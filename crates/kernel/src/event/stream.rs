//! Stream event thời gian thực (bước 40 P5, D-378): đẩy tới client qua Server-Sent Events những
//! event module **chủ động khai** stream được ([`EventInfo::streamable`](super::EventInfo::streamable)),
//! cùng nội dung và quyền như đọc bản ghi qua REST. KHÔNG phải "realtime theo bảng" (đẩy mọi thay đổi
//! dòng): bảng không bao giờ lộ ra ngoài, chỉ event đã khai.
//!
//! Đọc từ bảng `<module>_events` (transactional outbox) — đúng khi nhiều máy chủ: event ghi ở máy chủ
//! A, client nối với máy chủ B vẫn nhận. Mỗi tiến trình một [`StreamHub`]: một vòng hỏi chung cho mọi
//! kết nối (N kết nối không thành N câu hỏi mỗi giây), chỉ hỏi database của khách đang có người nghe,
//! rồi phát cho các kết nối qua `tokio::sync::broadcast`.
//!
//! Vì sao lùi lại một khoảng [`GRACE`] mỗi lượt hỏi: ID event (UUIDv7) và `occurred_at` gán lúc GHI,
//! còn event chỉ thấy được lúc COMMIT. Transaction mở trước, commit sau có thể mang thời điểm sớm hơn
//! event đã phát; hỏi lùi lại `GRACE` rồi khử trùng theo ID đã phát thì không mất nó.
//!
//! Giao **ít nhất một lần**: nối lại bằng `Last-Event-ID` gửi lại event trong khoảng `GRACE` trước
//! event đó; client khử trùng theo `id`.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Order, Query};
use serde::Serialize;
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

/// Lùi lại chừng này mỗi lượt hỏi (xem đầu file): transaction ghi phải ngắn (luật dữ liệu), nên
/// 5 giây giữa lúc ghi event và lúc commit là dư.
pub(crate) const GRACE: Duration = Duration::from_secs(5);
/// Event đã giao được giữ chừng này rồi bị dọn (`KEEP_DELIVERED` của bộ phát); nối lại với ID cũ
/// hơn thì client nhận `reset` (tải lại trạng thái) thay vì đọc bù.
const RETENTION: chrono::Duration = chrono::Duration::days(7);
/// Một kết nối sống tối đa chừng này rồi máy chủ đóng; client nối lại bằng `Last-Event-ID` và
/// quyền được xét lại (permission bị thu hồi không giữ stream mãi).
pub(crate) const LIFETIME: Duration = Duration::from_secs(30 * 60);
/// Đọc bù tối đa chừng này event; nhiều hơn thì gửi `reset`.
const BACKFILL_MAX: usize = 10_000;
/// Mỗi câu hỏi lấy tối đa chừng này dòng một bảng (lặp tới khi hết).
const PAGE: u64 = 500;
/// Số event bộ phát của hub giữ cho kết nối chậm; tụt quá thì kết nối đọc bù từ database.
const BROADCAST_CAPACITY: usize = 4096;
/// Số mục `names` tối đa của một kết nối.
const MAX_NAMES: usize = 50;

/// Cấu hình stream (đọc trong `configure` của module `kernel`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StreamConfig {
    pub enabled: bool,
    pub poll: Duration,
    pub max_connections: usize,
    pub max_per_tenant: usize,
    pub max_per_user: usize,
}

impl Default for StreamConfig {
    fn default() -> Self {
        StreamConfig {
            enabled: false,
            poll: Duration::from_millis(1000),
            max_connections: 1000,
            max_per_tenant: 200,
            max_per_user: 5,
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
        Ok(StreamConfig {
            enabled: enabled?,
            poll: Duration::from_millis(poll?),
            max_connections: total?,
            max_per_tenant: tenant?,
            max_per_user: user?,
        })
    }
}

/// Một event gửi qua stream: cùng hình dạng với `data:` của mỗi khung SSE.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct StreamedEvent {
    pub id: Uuid,
    pub name: String,
    pub occurred_at: DateTime<Utc>,
    pub subject: Option<String>,
    pub actor_kind: String,
    pub payload: serde_json::Value,
    #[serde(skip)]
    pub tenant_id: Uuid,
    #[serde(skip)]
    pub actor_user_id: Option<Uuid>,
    /// Người được nêu tên trong trường `audience` của payload (đã bỏ khỏi `payload`).
    #[serde(skip)]
    pub audience: Vec<Uuid>,
}

/// Một khung gửi cho client.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Frame {
    Event(Arc<StreamedEvent>),
    /// Event cần đọc bù đã bị dọn (hay quá nhiều): client tải lại trạng thái qua REST.
    Reset,
}

/// Hub stream của một tiến trình. Clone rẻ (cùng một hub).
#[derive(Clone)]
pub struct StreamHub(Arc<Hub>);

struct Hub {
    config: StreamConfig,
    /// Event stream được: tên → (module phát, permission cần, có tới người trong `audience`).
    events: BTreeMap<&'static str, (&'static str, Permission, bool)>,
    sender: broadcast::Sender<Arc<StreamedEvent>>,
    sources: Mutex<HashMap<PlacementId, Source>>,
    counts: Mutex<Counts>,
    closed: watch::Sender<bool>,
    poller_started: std::sync::atomic::AtomicBool,
}

/// Một database đang có người nghe.
struct Source {
    database: Database,
    listeners: usize,
    /// Lúc bắt đầu lượt hỏi trước (`None`: chưa hỏi lần nào).
    since: Option<DateTime<Utc>>,
    /// ID đã phát trong khoảng `GRACE` gần nhất → thời điểm của nó.
    seen: HashMap<Uuid, DateTime<Utc>>,
}

#[derive(Default)]
struct Counts {
    total: usize,
    tenants: HashMap<TenantId, usize>,
    users: HashMap<(TenantId, Uuid), usize>,
}

/// Người gọi thấy một event tới mức nào.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// Mọi event cùng khách (permission ở độ sâu `tenant`, không hàng rào nút).
    Tenant,
    /// Chỉ event do chính người gọi gây ra. Event chưa mang nút (`org_id`) nên chưa lọc được theo
    /// nhánh: quyền hẹp hơn `tenant` chỉ thấy việc của mình — không bao giờ thấy bản ghi của nhánh khác.
    /// Event khai `to_audience`: cả event nêu tên người gọi trong `audience`.
    Own,
    /// Không có permission của event, nhưng event khai `to_audience`: chỉ event nêu tên người gọi
    /// trong `audience` (vd người mua nhận tin trả lời trong cuộc trò chuyện của mình).
    Audience,
}

/// Bộ lọc của một kết nối.
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
    /// Hub cho các event stream được trong `catalog`.
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

    /// Đóng mọi stream đang mở và không nhận stream mới — gọi khi tiến trình bắt đầu tắt (graceful
    /// shutdown), để máy chủ không chờ những kết nối không bao giờ tự kết thúc. Client nối lại
    /// sang máy chủ khác bằng `Last-Event-ID`.
    pub fn close(&self) {
        let _ = self.0.closed.send(true);
    }

    pub(crate) fn enabled(&self) -> bool {
        self.0.config.enabled
    }

    /// Số stream đang mở ở tiến trình này.
    pub fn open_streams(&self) -> usize {
        lock(&self.0.counts).total
    }

    /// Kiểm `names` (`module.việc` hay `module.*`) và quyền của `principal`, ra bộ lọc.
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

    /// Mở một stream: khung đi ra qua kênh trả về, tới khi client ngắt, hub đóng hay hết
    /// [`LIFETIME`]. `last_event_id`: đọc bù từ sau event đó. Vượt giới hạn kết nối → 429.
    pub(crate) fn open(
        &self,
        tenant: &TenantContext,
        filter: StreamFilter,
        last_event_id: Option<Uuid>,
    ) -> Result<mpsc::Receiver<Frame>, Error> {
        if *self.0.closed.borrow() {
            return Err(Error::unavailable(
                "shutting_down",
                "The server is shutting down. Connect again; another server will answer.",
            ));
        }
        let ticket = self.admit(tenant.id(), filter.user)?;
        let source = self.attach(tenant);
        // Đăng ký nhận TRƯỚC khi đọc bù: event phát trong lúc đọc bù không lọt giữa hai bên.
        let live = self.0.sender.subscribe();
        self.start_poller();
        let (sender, receiver) = mpsc::channel(256);
        let connection = Connection {
            hub: self.clone(),
            database: tenant.database().clone(),
            filter,
            live,
            sender,
            delivered: HashMap::new(),
            last: last_event_id,
            _ticket: ticket,
            _source: source,
        };
        tokio::spawn(connection.run());
        Ok(receiver)
    }

    /// Giữ một chỗ trong giới hạn kết nối (trả lại khi vé bị bỏ).
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

    /// Thêm một người nghe cho database của khách `tenant`.
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

    /// Chạy vòng hỏi chung (một lần mỗi tiến trình, từ lúc có kết nối đầu tiên).
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

    /// Một lượt hỏi mọi database đang có người nghe; phát event mới.
    pub(crate) async fn poll_once(&self) -> Result<usize, Error> {
        let targets: Vec<(PlacementId, Database, Option<DateTime<Utc>>)> = {
            let mut sources = lock(&self.0.sources);
            sources.retain(|_, source| source.listeners > 0);
            sources
                .iter()
                .map(|(placement, source)| (*placement, source.database.clone(), source.since))
                .collect()
        };
        let mut sent = 0;
        for (placement, database, since) in targets {
            let started = Utc::now();
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

    /// Event stream được trong `names`, có ID từ thời điểm `from` (và sau `after` nếu có), của khách
    /// `tenant` (hay mọi khách), theo thứ tự ID. Tối đa [`BACKFILL_MAX`] + 1 event.
    async fn read(
        &self,
        database: &Database,
        tenant: Option<Uuid>,
        names: &[&'static str],
        from: DateTime<Utc>,
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

/// Một kết nối đang mở.
struct Connection {
    hub: StreamHub,
    database: Database,
    filter: StreamFilter,
    live: broadcast::Receiver<Arc<StreamedEvent>>,
    sender: mpsc::Sender<Frame>,
    /// ID đã gửi gần đây → thời điểm (khử trùng giữa đọc bù và phát trực tiếp).
    delivered: HashMap<Uuid, DateTime<Utc>>,
    /// ID cuối đã gửi (hay client báo qua `Last-Event-ID`).
    last: Option<Uuid>,
    _ticket: Ticket,
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

    /// Đọc bù từ sau `last` (lùi lại `GRACE`). `false`: client đã ngắt.
    async fn backfill(&mut self, last: Uuid) -> bool {
        let Some(at) = time_of(last) else {
            return self.send(Frame::Reset).await;
        };
        if at < Utc::now() - RETENTION {
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

    /// Gửi `event` nếu người gọi được thấy và chưa gửi. `false`: client đã ngắt.
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

/// Chỗ đã giữ trong giới hạn kết nối; bỏ thì trả lại.
struct Ticket {
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

/// Một người nghe của database; bỏ thì bớt.
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

/// Mutex bị "đầu độc" (một thread panic khi đang giữ) vẫn dùng tiếp: dữ liệu bên trong chỉ là bộ
/// đếm và tập ID, không có bất biến nào bị phá dở.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn grace() -> chrono::Duration {
    chrono::Duration::from_std(GRACE).unwrap_or_else(|_| chrono::Duration::seconds(5))
}

/// UUIDv7 nhỏ nhất của mili giây `at`: so sánh ID (khoá chính, có index) thay cho `occurred_at`
/// (không có index) — UUIDv7 bắt đầu bằng thời điểm tạo, xếp theo byte trên cả hai database.
fn lowest_id_at(at: DateTime<Utc>) -> Uuid {
    let millis = u128::try_from(at.timestamp_millis()).unwrap_or(0);
    Uuid::from_u128(millis << 80)
}

/// Thời điểm tạo ghi trong UUIDv7 `id`.
fn time_of(id: Uuid) -> Option<DateTime<Utc>> {
    let (seconds, nanos) = id.get_timestamp()?.to_unix();
    DateTime::from_timestamp(i64::try_from(seconds).ok()?, nanos)
}

/// Người gọi thấy event cần `permission` tới đâu; `None`: không thấy.
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

/// Mã lỗi khi stream bị tắt bằng cấu hình.
pub(crate) fn disabled() -> Error {
    Error::not_found(
        "kernel.stream_disabled",
        "Event streaming is not enabled on this server (`AUTOXIDE_EVENTS_STREAM`).",
    )
}

/// ID trong `Last-Event-ID` (hay `last_event_id`); sai dạng → lỗi `validation`.
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
        let now = Utc::now();
        let id = Uuid::now_v7();
        let floor = lowest_id_at(now - chrono::Duration::seconds(1));
        assert!(floor < id);
        assert!(lowest_id_at(now + chrono::Duration::seconds(1)) > id);
        let at = time_of(id).unwrap();
        assert!((at - now).num_seconds().abs() < 2);
    }
}
