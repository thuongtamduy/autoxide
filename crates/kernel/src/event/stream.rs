//! Real-time event stream (step 40 P5, D-378): pushes to clients over Server-Sent Events the events that
//! modules **explicitly declare** streamable ([`EventInfo::streamable`](super::EventInfo::streamable)), with
//! the same content and permissions as reading the record through REST. NOT "per-table realtime" (pushing
//! every row change): tables are never exposed, only declared events.
//!
//! Reads from the `<module>_events` tables (transactional outbox) - correct with multiple servers: an event
//! written on server A is still received by a client connected to server B. One [`StreamHub`] per process: a
//! single shared polling loop for all connections (N connections do not become N queries per second),
//! querying only the databases of tenants that have listeners, then broadcasting to connections through
//! `tokio::sync::broadcast`.
//!
//! Why each poll looks back by a [`GRACE`] interval: the event ID (UUIDv7) and `occurred_at` are assigned at
//! WRITE time, while an event only becomes visible at COMMIT time. A transaction opened earlier and committed
//! later can carry an earlier timestamp than an event already broadcast; polling back `GRACE` and
//! de-duplicating by already-broadcast IDs does not lose it.
//!
//! **At-least-once** delivery: reconnecting with `Last-Event-ID` resends events within `GRACE` before
//! that event; the client de-duplicates by `id`.

use crate::time::Timestamp;
use jiff::SignedDuration;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Order, Query};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, watch};
use uuid::Uuid;

use super::table_name;
use crate::access::{Permission, Principal, Scope};
use crate::config::{ConfigReader, Invalid, Setting, parse_in_range};
use crate::database::Database;
use crate::error::{Error, ErrorKind, FieldError};
use crate::tenancy::{PlacementId, TenantContext, TenantId};

pub(crate) const STREAM: Setting = Setting::new(
    "AUTOXIDE_EVENTS_STREAM",
    "`true`: serve `GET /api/v1/kernel/events/stream`, a Server-Sent Events stream of the events \
     modules declare streamable. `false`: the endpoint answers 404 `kernel.stream_disabled`.",
)
.with_default("false");

pub(crate) const POLL_MS: Setting = Setting::new(
    "AUTOXIDE_EVENTS_STREAM_POLL_MS",
    "How often (milliseconds, 100 to 60000) each server reads new events from the database for \
     its open streams. One query per database and module, shared by every stream of the server.",
)
.with_default("1000");

pub(crate) const MAX_CONNECTIONS: Setting = Setting::new(
    "AUTOXIDE_EVENTS_STREAM_MAX_CONNECTIONS",
    "Most event streams one server keeps open at once (1 to 100000); more are answered with 429.",
)
.with_default("1000");

pub(crate) const MAX_PER_TENANT: Setting = Setting::new(
    "AUTOXIDE_EVENTS_STREAM_MAX_PER_TENANT",
    "Most event streams one server keeps open for one tenant (1 to 100000); more are answered \
     with 429.",
)
.with_default("200");

pub(crate) const MAX_PER_USER: Setting = Setting::new(
    "AUTOXIDE_EVENTS_STREAM_MAX_PER_USER",
    "Most event streams one server keeps open for one user (1 to 1000); more are answered with 429.",
)
.with_default("5");

pub(crate) const WS_RECHECK_SECONDS: Setting = Setting::new(
    "AUTOXIDE_EVENTS_WS_RECHECK_SECONDS",
    "How often (seconds, 1 to 3600) an open WebSocket of `GET /api/v1/kernel/events/ws` checks \
     its credentials again; when they no longer hold (token expired, account locked) the server \
     closes it with code 4401.",
)
.with_default("60");

/// Look back this far on each poll (see the top of the file): write transactions must be short (data rules), so
/// 5 seconds between writing the event and committing is more than enough.
pub(crate) const GRACE: Duration = Duration::from_secs(5);
/// Delivered events are kept this long and then cleaned up (`KEEP_DELIVERED` of the dispatcher); reconnecting
/// with an older ID makes the client receive `reset` (reload state) instead of a backfill.
const RETENTION: SignedDuration = SignedDuration::from_hours(24 * 7);
/// A connection lives at most this long and then the server closes it; the client reconnects with
/// `Last-Event-ID` and permissions are re-evaluated (a revoked permission does not keep a stream open
/// forever).
pub(crate) const LIFETIME: Duration = Duration::from_secs(30 * 60);
/// Backfills at most this many events; beyond that, `reset` is sent.
const BACKFILL_MAX: usize = 10_000;
/// Each query fetches at most this many rows of a table (repeating until exhausted).
const PAGE: u64 = 500;
/// Number of events the hub's broadcaster holds for slow connections; a connection that falls further
/// behind backfills from the database.
const BROADCAST_CAPACITY: usize = 4096;
/// Maximum number of `names` entries per connection.
const MAX_NAMES: usize = 50;

/// Stream configuration (read in the `kernel` module's `configure`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StreamConfig {
    pub enabled: bool,
    pub poll: Duration,
    pub max_connections: usize,
    pub max_per_tenant: usize,
    pub max_per_user: usize,
    /// The WebSocket re-evaluates credentials every this interval (step 69 P3).
    pub ws_recheck: Duration,
}

impl Default for StreamConfig {
    fn default() -> Self {
        StreamConfig {
            enabled: false,
            poll: Duration::from_millis(1000),
            max_connections: 1000,
            max_per_tenant: 200,
            max_per_user: 5,
            ws_recheck: Duration::from_secs(60),
        }
    }
}

impl StreamConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let enabled = reader.get_with(&STREAM, |raw| match raw {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("must be `true` or `false`".to_owned()),
        });
        let poll = reader.get_with(&POLL_MS, |raw| parse_in_range(raw, 100u64, 60_000));
        let total = reader.get_with(&MAX_CONNECTIONS, |raw| parse_in_range(raw, 1usize, 100_000));
        let tenant = reader.get_with(&MAX_PER_TENANT, |raw| parse_in_range(raw, 1usize, 100_000));
        let user = reader.get_with(&MAX_PER_USER, |raw| parse_in_range(raw, 1usize, 1000));
        let recheck = reader.get_with(&WS_RECHECK_SECONDS, |raw| parse_in_range(raw, 1u64, 3600));
        Ok(StreamConfig {
            enabled: enabled?,
            poll: Duration::from_millis(poll?),
            max_connections: total?,
            max_per_tenant: tenant?,
            max_per_user: user?,
            ws_recheck: Duration::from_secs(recheck?),
        })
    }
}

/// An event sent through the stream: the same shape as the `data:` of each SSE frame.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct StreamedEvent {
    pub id: Uuid,
    pub name: String,
    pub occurred_at: Timestamp,
    pub subject: Option<String>,
    pub actor_kind: String,
    pub payload: serde_json::Value,
    #[serde(skip)]
    pub tenant_id: Uuid,
    #[serde(skip)]
    pub actor_user_id: Option<Uuid>,
    /// People named in the payload's `audience` field (removed from `payload`).
    #[serde(skip)]
    pub audience: Vec<Uuid>,
}

/// A frame sent to the client.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Frame {
    Event(Arc<StreamedEvent>),
    /// The event to backfill has been cleaned up (or there are too many): the client reloads state through
    /// REST.
    Reset,
}

/// The stream hub of one process. Cheap to clone (the same hub).
#[derive(Clone)]
pub struct StreamHub(Arc<Hub>);

struct Hub {
    config: StreamConfig,
    /// Streamable events: name -> (emitting module, required permission, whether it reaches people in
    /// `audience`).
    events: BTreeMap<&'static str, (&'static str, Permission, bool)>,
    sender: broadcast::Sender<Arc<StreamedEvent>>,
    sources: Mutex<HashMap<PlacementId, Source>>,
    counts: Mutex<Counts>,
    closed: watch::Sender<bool>,
    poller_started: std::sync::atomic::AtomicBool,
}

/// A database that has listeners.
struct Source {
    database: Database,
    listeners: usize,
    /// Start time of the previous poll (`None`: never polled).
    since: Option<Timestamp>,
    /// IDs broadcast within the most recent `GRACE` window -> their timestamps.
    seen: HashMap<Uuid, Timestamp>,
}

#[derive(Default)]
struct Counts {
    total: usize,
    tenants: HashMap<TenantId, usize>,
    users: HashMap<(TenantId, Uuid), usize>,
}

/// How much of an event the caller may see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// Every event of the same tenant (permission at `tenant` depth, no node fence).
    Tenant,
    /// Only events caused by the caller. Events do not yet carry a node (`org_id`) so they cannot be filtered
    /// by branch: a permission narrower than `tenant` sees only its own actions - never another branch's
    /// records. Events declaring `to_audience`: also events naming the caller in `audience`.
    Own,
    /// No permission for the event, but the event declares `to_audience`: only events naming the caller
    /// in `audience` (e.g. a buyer receiving replies in their own conversation).
    Audience,
}

/// A connection's filter.
pub(crate) struct StreamFilter {
    tenant: Uuid,
    user: Uuid,
    names: HashMap<&'static str, Reach>,
}

impl StreamFilter {
    fn admits(&self, event: &StreamedEvent) -> bool {
        event.tenant_id == self.tenant
            && match self.names.get(event.name.as_str()) {
                Some(Reach::Tenant) => true,
                Some(Reach::Own) => {
                    event.actor_user_id == Some(self.user) || event.audience.contains(&self.user)
                }
                Some(Reach::Audience) => event.audience.contains(&self.user),
                None => false,
            }
    }
}

impl StreamHub {
    /// A hub for the streamable events in `catalog`.
    pub(crate) fn new(config: StreamConfig, catalog: &super::EventCatalog) -> Self {
        let events = catalog
            .streamable()
            .into_iter()
            .filter_map(|(module, event)| {
                event
                    .stream_permission()
                    .map(|permission| (event.name(), (module, permission, event.has_audience())))
            })
            .collect();
        let (sender, _) = broadcast::channel(BROADCAST_CAPACITY);
        let (closed, _) = watch::channel(false);
        StreamHub(Arc::new(Hub {
            config,
            events,
            sender,
            sources: Mutex::new(HashMap::new()),
            counts: Mutex::new(Counts::default()),
            closed,
            poller_started: std::sync::atomic::AtomicBool::new(false),
        }))
    }

    /// Closes every open stream and accepts no new ones - called when the process starts shutting down
    /// (graceful shutdown), so the server does not wait for connections that never end on their own. Clients
    /// reconnect to another server with `Last-Event-ID`.
    pub fn close(&self) {
        let _ = self.0.closed.send(true);
    }

    pub(crate) fn enabled(&self) -> bool {
        self.0.config.enabled
    }

    /// The interval at which the WebSocket re-evaluates credentials.
    pub(crate) fn ws_recheck(&self) -> Duration {
        self.0.config.ws_recheck
    }

    /// Number of streams open in this process.
    pub fn open_streams(&self) -> usize {
        lock(&self.0.counts).total
    }

    /// Validates `names` (`module.action` or `module.*`) and the permissions of `principal`, producing a
    /// filter.
    pub(crate) fn filter(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        names: &str,
    ) -> Result<StreamFilter, Error> {
        let patterns: Vec<&str> =
            names.split(',').map(str::trim).filter(|name| !name.is_empty()).collect();
        if patterns.is_empty() || patterns.len() > MAX_NAMES {
            return Err(Error::validation(vec![
                FieldError::new(
                    "names",
                    "invalid",
                    "give 1 to {max} event names, comma-separated (`module.event` or `module.*`)",
                )
                .with_param("max", MAX_NAMES),
            ]));
        }
        let mut chosen: HashMap<&'static str, Reach> = HashMap::new();
        // Permission thiếu đầu tiên: báo khi không còn event nào người gọi được xem.
        let mut denied: Option<Permission> = None;
        for pattern in patterns {
            let matched: Vec<(&'static str, Permission, bool)> = match pattern.strip_suffix(".*") {
                Some(module) => self
                    .0
                    .events
                    .iter()
                    .filter(|(_, (owner, _, _))| *owner == module)
                    .map(|(name, (_, permission, audience))| (*name, *permission, *audience))
                    .collect(),
                None => self
                    .0
                    .events
                    .get_key_value(pattern)
                    .map(|(name, (_, permission, audience))| (*name, *permission, *audience))
                    .into_iter()
                    .collect(),
            };
            if matched.is_empty() {
                return Err(unknown_event(pattern));
            }
            let exact = !pattern.ends_with(".*");
            for (name, permission, audience) in matched {
                match reach(principal, &permission) {
                    Some(reach) => {
                        chosen.insert(name, reach);
                    }
                    // Không có permission nhưng event tới người được nêu tên: chỉ nhận phần của mình.
                    None if audience => {
                        chosen.insert(name, Reach::Audience);
                    }
                    // Xin đích danh một event không có quyền xem: báo rõ permission thiếu.
                    None if exact => return Err(permission_error(principal, &permission)),
                    None => {
                        denied.get_or_insert(permission);
                    }
                }
            }
        }
        if chosen.is_empty()
            && let Some(permission) = denied
        {
            return Err(permission_error(principal, &permission));
        }
        Ok(StreamFilter { tenant: tenant.id().as_uuid(), user: principal.user_id(), names: chosen })
    }

    /// Opens a stream: frames come out through the returned channel, until the client disconnects, the hub
    /// closes or [`LIFETIME`] runs out. `last_event_id`: backfill from after that event. Exceeding the
    /// connection limit -> 429.
    pub(crate) fn open(
        &self,
        tenant: &TenantContext,
        filter: StreamFilter,
        last_event_id: Option<Uuid>,
    ) -> Result<mpsc::Receiver<Frame>, Error> {
        let ticket = self.session(tenant, filter.user)?;
        Ok(self.connect(tenant, filter, last_event_id, Some(ticket)))
    }

    /// Reserves a connection slot for a whole session (WebSocket, step 69 P3): a session changes its filter
    /// many times, each [`connect`](Self::connect) without a ticket, still counting as one connection. Hub
    /// closing -> 503.
    pub(crate) fn session(&self, tenant: &TenantContext, user: Uuid) -> Result<Ticket, Error> {
        if self.is_closed() {
            return Err(Error::unavailable(
                "shutting_down",
                "The server is shutting down. Connect again; another server will answer.",
            ));
        }
        self.admit(tenant.id(), user)
    }

    /// The hub is closed (the process is shutting down).
    pub(crate) fn is_closed(&self) -> bool {
        *self.0.closed.borrow()
    }

    /// Attaches a frame stream with filter `filter`; `ticket`: the reserved slot (`None` when the session
    /// holds the slot).
    pub(crate) fn connect(
        &self,
        tenant: &TenantContext,
        filter: StreamFilter,
        last_event_id: Option<Uuid>,
        ticket: Option<Ticket>,
    ) -> mpsc::Receiver<Frame> {
        let source = self.attach(tenant);
        // Đăng ký nhận TRƯỚC khi đọc bù: event phát trong lúc đọc bù không lọt giữa hai bên.
        let live = self.0.sender.subscribe();
        self.start_poller();
        let (sender, receiver) = mpsc::channel(256);
        // Client nối lại đã có event `Last-Event-ID`: coi như đã gửi, kẻo bản phát trực tiếp của
        // chính event đó đến trễ (poller chưa phát lúc client ngắt) lại gửi lần nữa.
        let delivered: HashMap<Uuid, Timestamp> =
            last_event_id.and_then(|id| time_of(id).map(|at| (id, at))).into_iter().collect();
        let connection = Connection {
            hub: self.clone(),
            database: tenant.database().clone(),
            filter,
            live,
            sender,
            delivered,
            last: last_event_id,
            _ticket: ticket,
            _source: source,
        };
        tokio::spawn(connection.run());
        receiver
    }

    /// Reserves a slot within the connection limit (returned when the ticket is dropped).
    fn admit(&self, tenant: TenantId, user: Uuid) -> Result<Ticket, Error> {
        let config = &self.0.config;
        let mut counts = lock(&self.0.counts);
        let in_tenant = counts.tenants.get(&tenant).copied().unwrap_or(0);
        let of_user = counts.users.get(&(tenant, user)).copied().unwrap_or(0);
        let limit = if of_user >= config.max_per_user {
            Some(config.max_per_user)
        } else if in_tenant >= config.max_per_tenant {
            Some(config.max_per_tenant)
        } else if counts.total >= config.max_connections {
            Some(config.max_connections)
        } else {
            None
        };
        if let Some(limit) = limit {
            metrics::counter!("kernel_event_streams_refused_total").increment(1);
            return Err(Error::new(
                ErrorKind::TooManyRequests,
                "kernel.stream_too_many",
                "Too many event streams are open ({limit}). Close some before opening another.",
            )
            .with_param("limit", limit));
        }
        counts.total += 1;
        *counts.tenants.entry(tenant).or_default() += 1;
        *counts.users.entry((tenant, user)).or_default() += 1;
        metrics::gauge!("kernel_event_streams_open").set(counts.total as f64);
        Ok(Ticket { hub: self.clone(), tenant, user })
    }

    /// Adds a listener for the database of tenant `tenant`.
    fn attach(&self, tenant: &TenantContext) -> Attachment {
        let placement = tenant.placement();
        let mut sources = lock(&self.0.sources);
        let source = sources.entry(placement).or_insert_with(|| Source {
            database: tenant.database().clone(),
            listeners: 0,
            since: None,
            seen: HashMap::new(),
        });
        source.listeners += 1;
        Attachment { hub: self.clone(), placement }
    }

    /// Runs the shared polling loop (once per process, starting from the first connection).
    fn start_poller(&self) {
        use std::sync::atomic::Ordering;
        if self.0.poller_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let hub = self.clone();
        tokio::spawn(async move {
            let mut closed = hub.0.closed.subscribe();
            loop {
                tokio::select! {
                    _ = closed.wait_for(|closed| *closed) => return,
                    () = tokio::time::sleep(hub.0.config.poll) => {}
                }
                if let Err(error) = hub.poll_once().await {
                    tracing::warn!(error = %error, "cannot read events for the event streams; retrying");
                }
            }
        });
    }

    /// One poll of every database that has listeners; broadcasts new events.
    pub(crate) async fn poll_once(&self) -> Result<usize, Error> {
        let targets: Vec<(PlacementId, Database, Option<Timestamp>)> = {
            let mut sources = lock(&self.0.sources);
            sources.retain(|_, source| source.listeners > 0);
            sources
                .iter()
                .map(|(placement, source)| (*placement, source.database.clone(), source.since))
                .collect()
        };
        let mut sent = 0;
        for (placement, database, since) in targets {
            let started = Timestamp::now();
            let from = since.unwrap_or(started) - grace();
            let events = self.read(&database, None, &self.all_names(), from, None).await?;
            let mut sources = lock(&self.0.sources);
            let Some(source) = sources.get_mut(&placement) else { continue };
            for event in events {
                if source.seen.insert(event.id, event.occurred_at).is_none() {
                    // Không ai nghe thì `send` báo lỗi: bỏ qua.
                    let _ = self.0.sender.send(Arc::new(event));
                    sent += 1;
                }
            }
            source.since = Some(started);
            let keep_after = started - grace();
            source.seen.retain(|_, at| *at >= keep_after);
        }
        if sent > 0 {
            metrics::counter!("kernel_event_stream_events_total").increment(sent as u64);
        }
        Ok(sent)
    }

    fn all_names(&self) -> Vec<&'static str> {
        self.0.events.keys().copied().collect()
    }

    /// Streamable events in `names`, with IDs from time `from` (and after `after` if given), of tenant
    /// `tenant` (or every tenant), in ID order. At most [`BACKFILL_MAX`] + 1 events.
    async fn read(
        &self,
        database: &Database,
        tenant: Option<Uuid>,
        names: &[&'static str],
        from: Timestamp,
        after: Option<Uuid>,
    ) -> Result<Vec<StreamedEvent>, Error> {
        let mut by_module: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
        for name in names {
            if let Some((module, _, _)) = self.0.events.get(name) {
                by_module.entry(*module).or_default().push(*name);
            }
        }
        let floor = lowest_id_at(from);
        let mut all = Vec::new();
        for (module, names) in by_module {
            let mut cursor: Option<Uuid> = None;
            loop {
                let mut select = Query::select();
                select
                    .columns([
                        "tenant_id",
                        "id",
                        "name",
                        "actor_user_id",
                        "actor_kind",
                        "subject",
                        "payload",
                        "occurred_at",
                    ])
                    .from(Alias::new(table_name(module)))
                    .and_where(Expr::col(Alias::new("name")).is_in(names.iter().copied()))
                    .order_by(Alias::new("id"), Order::Asc)
                    .limit(PAGE);
                match cursor {
                    Some(cursor) => select.and_where(Expr::col(Alias::new("id")).gt(cursor)),
                    None => select.and_where(Expr::col(Alias::new("id")).gte(floor)),
                };
                if let Some(after) = after {
                    select.and_where(Expr::col(Alias::new("id")).ne(after));
                }
                if let Some(tenant) = tenant {
                    select.and_where(Expr::col(Alias::new("tenant_id")).eq(tenant));
                }
                // Kết nối ghi: dòng vừa commit chưa tới read replica thì vẫn thấy.
                let rows = database.write().query_all(&select).await?;
                let count = rows.len();
                for row in rows {
                    let event = streamed(&row)?;
                    cursor = Some(event.id);
                    all.push(event);
                }
                if (count as u64) < PAGE || all.len() > BACKFILL_MAX {
                    break;
                }
            }
        }
        all.sort_by_key(|event| event.id);
        Ok(all)
    }
}

fn streamed(row: &sea_orm::QueryResult) -> Result<StreamedEvent, Error> {
    let payload: String = row.try_get("", "payload")?;
    let actor_user_id: Option<Uuid> = row.try_get("", "actor_user_id")?;
    let actor_kind: Option<String> = row.try_get("", "actor_kind")?;
    let mut payload: serde_json::Value =
        serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null);
    // `audience` chỉ để lọc: không gửi danh sách người nhận ra client.
    let audience = payload
        .as_object_mut()
        .and_then(|object| object.remove("audience"))
        .and_then(|value| serde_json::from_value::<Vec<Uuid>>(value).ok())
        .unwrap_or_default();
    Ok(StreamedEvent {
        id: row.try_get("", "id")?,
        name: row.try_get("", "name")?,
        occurred_at: row.try_get("", "occurred_at")?,
        subject: row.try_get("", "subject")?,
        actor_kind: actor_kind
            .unwrap_or_else(|| if actor_user_id.is_some() { "user" } else { "system" }.to_owned()),
        payload,
        tenant_id: row.try_get("", "tenant_id")?,
        actor_user_id,
        audience,
    })
}

/// An open connection.
struct Connection {
    hub: StreamHub,
    database: Database,
    filter: StreamFilter,
    live: broadcast::Receiver<Arc<StreamedEvent>>,
    sender: mpsc::Sender<Frame>,
    /// IDs sent recently -> their timestamps (de-duplication between backfill and live broadcast).
    delivered: HashMap<Uuid, Timestamp>,
    /// The last ID sent (or reported by the client through `Last-Event-ID`).
    last: Option<Uuid>,
    _ticket: Option<Ticket>,
    _source: Attachment,
}

impl Connection {
    async fn run(mut self) {
        let mut closed = self.hub.0.closed.subscribe();
        let deadline = tokio::time::sleep(LIFETIME);
        tokio::pin!(deadline);
        if let Some(last) = self.last
            && !self.backfill(last).await
        {
            return;
        }
        loop {
            let received = tokio::select! {
                _ = closed.wait_for(|closed| *closed) => return,
                () = &mut deadline => return,
                () = self.sender.closed() => return,
                received = self.live.recv() => received,
            };
            let keep_going = match received {
                Ok(event) => self.deliver(event).await,
                // Tụt hậu: phần bị bỏ đọc bù thẳng từ database.
                Err(broadcast::error::RecvError::Lagged(_)) => match self.last {
                    Some(last) => self.backfill(last).await,
                    None => self.send(Frame::Reset).await,
                },
                Err(broadcast::error::RecvError::Closed) => false,
            };
            if !keep_going {
                return;
            }
        }
    }

    /// Backfills from after `last` (looking back `GRACE`). `false`: the client has disconnected.
    async fn backfill(&mut self, last: Uuid) -> bool {
        let Some(at) = time_of(last) else {
            return self.send(Frame::Reset).await;
        };
        if at < Timestamp::now() - RETENTION {
            return self.send(Frame::Reset).await;
        }
        let names: Vec<&'static str> = self.filter.names.keys().copied().collect();
        let read = self
            .hub
            .read(&self.database, Some(self.filter.tenant), &names, at - grace(), Some(last))
            .await;
        let events = match read {
            Ok(events) => events,
            Err(error) => {
                tracing::warn!(error = %error, "cannot read missed events for a stream");
                return self.send(Frame::Reset).await;
            }
        };
        if events.len() > BACKFILL_MAX {
            return self.send(Frame::Reset).await;
        }
        for event in events {
            if !self.deliver(Arc::new(event)).await {
                return false;
            }
        }
        true
    }

    /// Sends `event` if the caller may see it and it has not been sent. `false`: the client has disconnected.
    async fn deliver(&mut self, event: Arc<StreamedEvent>) -> bool {
        if !self.filter.admits(&event) || self.delivered.contains_key(&event.id) {
            return true;
        }
        let at = event.occurred_at;
        self.delivered.insert(event.id, at);
        if self.delivered.len() > 2 * BACKFILL_MAX {
            let keep_after = at - grace() - grace();
            self.delivered.retain(|_, seen| *seen >= keep_after);
        }
        self.last = Some(event.id);
        self.send(Frame::Event(event)).await
    }

    async fn send(&self, frame: Frame) -> bool {
        self.sender.send(frame).await.is_ok()
    }
}

/// A slot reserved within the connection limit; dropping it returns the slot.
pub(crate) struct Ticket {
    hub: StreamHub,
    tenant: TenantId,
    user: Uuid,
}

impl Drop for Ticket {
    fn drop(&mut self) {
        let mut counts = lock(&self.hub.0.counts);
        counts.total = counts.total.saturating_sub(1);
        decrement(&mut counts.tenants, self.tenant);
        decrement(&mut counts.users, (self.tenant, self.user));
        metrics::gauge!("kernel_event_streams_open").set(counts.total as f64);
    }
}

/// A listener of a database; dropping it removes it.
struct Attachment {
    hub: StreamHub,
    placement: PlacementId,
}

impl Drop for Attachment {
    fn drop(&mut self) {
        if let Some(source) = lock(&self.hub.0.sources).get_mut(&self.placement) {
            source.listeners = source.listeners.saturating_sub(1);
        }
    }
}

fn decrement<K: std::hash::Hash + Eq>(map: &mut HashMap<K, usize>, key: K) {
    if let Some(count) = map.get_mut(&key) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            map.remove(&key);
        }
    }
}

/// A "poisoned" mutex (a thread panicked while holding it) is still used: the data inside is only counters
/// and sets of IDs, with no invariant broken halfway.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn grace() -> SignedDuration {
    SignedDuration::try_from(GRACE).unwrap_or_else(|_| SignedDuration::from_secs(5))
}

/// The smallest UUIDv7 of millisecond `at`: compares IDs (primary key, indexed) instead of `occurred_at`
/// (not indexed) - a UUIDv7 starts with its creation time, ordered by bytes on both databases.
fn lowest_id_at(at: Timestamp) -> Uuid {
    let millis = u128::try_from(at.as_unix_millis()).unwrap_or(0);
    Uuid::from_u128(millis << 80)
}

/// The creation time recorded in UUIDv7 `id`.
fn time_of(id: Uuid) -> Option<Timestamp> {
    let (seconds, nanos) = id.get_timestamp()?.to_unix();
    Timestamp::from_unix(i64::try_from(seconds).ok()?, nanos)
}

/// How far the caller sees an event that needs `permission`; `None`: does not see it.
fn reach(principal: &Principal, permission: &Permission) -> Option<Reach> {
    let grants = principal.grants(permission);
    if grants.is_empty() {
        None
    } else if grants.iter().any(|grant| grant.scope == Scope::Tenant) && principal.fence().is_none()
    {
        Some(Reach::Tenant)
    } else {
        Some(Reach::Own)
    }
}

fn permission_error(principal: &Principal, permission: &Permission) -> Error {
    principal
        .require(permission)
        .err()
        .unwrap_or_else(|| Error::internal("a permission without grants passed its check"))
}

fn unknown_event(name: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "kernel.stream_unknown_event",
        "Event `{name}` does not exist or cannot be streamed. The module catalog \
         (`GET /api/v1/kernel/modules`) lists each event with its `stream_permission`.",
    )
    .with_param("name", name)
}

/// The error code when the stream is disabled by configuration.
pub(crate) fn disabled() -> Error {
    Error::not_found(
        "kernel.stream_disabled",
        "Event streaming is not enabled on this server (`AUTOXIDE_EVENTS_STREAM`).",
    )
}

/// The ID in `Last-Event-ID` (or `last_event_id`); wrong format -> a `validation` error.
pub(crate) fn parse_last_event_id(raw: &str) -> Result<Uuid, Error> {
    Uuid::parse_str(raw.trim()).map_err(|_| {
        Error::validation(vec![FieldError::new(
            "last_event_id",
            "invalid",
            "must be the `id` of an event you received",
        )])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_nho_nhat_cua_mot_thoi_diem() {
        let now = Timestamp::now();
        let id = Uuid::now_v7();
        let floor = lowest_id_at(now - SignedDuration::from_secs(1));
        assert!(floor < id);
        assert!(lowest_id_at(now + SignedDuration::from_secs(1)) > id);
        let at = time_of(id).unwrap();
        assert!((at - now).as_secs().abs() < 2);
    }
}
