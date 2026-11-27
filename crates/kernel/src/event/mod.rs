//! Events: side effects of a change (audit, notifications...) travel through events, following the
//! "transactional outbox" pattern.
//!
//! 1. A module declares the events it emits through `Module::events` and adds
//!    [`outbox_migration`] to its migrations: the `<module>_events` table.
//! 2. In the SAME transaction as the data change, the module records the event with
//!    [`Outbox::record`]. If the transaction fails the event does not exist either; once committed it
//!    certainly exists - never "changed the data but forgot to notify", nor "notified but the data
//!    did not change".
//! 3. After the commit, the kernel's [`Dispatcher`] reads the event tables and delivers them to the
//!    modules that subscribed (`Module::subscriptions`). When a receiver fails it is retried with
//!    backoff; after [`MAX_ATTEMPTS`] attempts the event is marked failed and logged.
//!
//! Delivery is **at-least-once**: a receiver must tolerate receiving the same event again
//! (compared by `id`), because the server may shut down in the middle of delivery.

mod dispatcher;
mod migration;
mod outbox;
pub(crate) mod signals;
pub(crate) mod stream;

use crate::time::Timestamp;
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::Arc;
use uuid::Uuid;

use crate::error::Error;
use crate::module::is_identifier;
use crate::tenancy::TenantContext;

pub use dispatcher::{Dispatcher, MAX_ATTEMPTS};
pub use migration::{
    outbox_access_migration, outbox_actor_migration, outbox_migration, tenant_table,
};
pub use outbox::Outbox;
pub use signals::SignalTopics;
pub use stream::StreamHub;

/// A kind of event, declared by the module that emits it.
///
/// ```
/// use autoxide_kernel::event::EventInfo;
///
/// pub const NOTE_CREATED: EventInfo = EventInfo::new("notes.note_created", "A note was created.");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EventInfo {
    name: &'static str,
    description: &'static str,
    /// Activity sentence (step 51 P5) and the permission needed to see it.
    activity: Option<(&'static str, crate::access::Permission)>,
    /// Permission needed to receive this event through the SSE stream (step 40 P5); `None`: not streamed.
    stream: Option<crate::access::Permission>,
    /// The event also reaches the people named in the payload's `audience` field (step 61,
    /// D-20261119-step61-stream-audience).
    audience: bool,
}

impl EventInfo {
    /// `name`: `<module>.<action>` (lowercase, digits, `_`). `description`: an English sentence.
    pub const fn new(name: &'static str, description: &'static str) -> Self {
        EventInfo { name, description, activity: None, stream: None, audience: false }
    }

    /// This event appears in "recent activity" (step 51 P5, `GET /api/v1/audit/activity`) with the English
    /// sentence `sentence` (translated through the module's `text:`): `{actor}` is the name of the person who
    /// acted, other `{name}` placeholders take the same-named field of the event's `after` (or `before`)
    /// data. Only people with `reader` see this line.
    pub const fn activity(
        mut self,
        sentence: &'static str,
        reader: crate::access::Permission,
    ) -> Self {
        self.activity = Some((sentence, reader));
        self
    }

    /// This event is pushed through the real-time stream (`GET /api/v1/kernel/events/stream`, step 40 P5) to
    /// people with `reader` - the same permission as reading the record through REST, because the payload
    /// contains the record's data. Not streamed by default: only events a module explicitly declares go out
    /// (there is no "per-table realtime", D-378).
    pub const fn streamable(mut self, reader: crate::access::Permission) -> Self {
        self.stream = Some(reader);
        self
    }

    /// This event also reaches people named in the payload: the payload's `audience` field (an array of user
    /// ids) lists the people who receive the event through the stream even without the `reader` permission of
    /// [`streamable`](Self::streamable) at tenant-wide depth - e.g. the buyer and the assigned staff of a
    /// conversation (step 61). The `audience` field is removed from the payload sent to the client. Only
    /// effective together with `streamable`.
    pub const fn to_audience(mut self) -> Self {
        self.audience = true;
        self
    }

    /// Event khai [`to_audience`](Self::to_audience).
    pub fn has_audience(&self) -> bool {
        self.audience
    }

    /// Permission to receive the event through the stream, when the event declares
    /// [`streamable`](Self::streamable).
    pub fn stream_permission(&self) -> Option<crate::access::Permission> {
        self.stream
    }

    /// The activity sentence and the permission to see it, when the event declares them.
    pub fn activity_line(&self) -> Option<(&'static str, crate::access::Permission)> {
        self.activity
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn description(&self) -> &'static str {
        self.description
    }
}

/// The content of an event: a type convertible to JSON, tied to the event name.
///
/// ```
/// use autoxide_kernel::event::Event;
/// use serde::Serialize;
///
/// #[derive(Serialize)]
/// struct NoteCreated {
///     title: String,
/// }
///
/// impl Event for NoteCreated {
///     const NAME: &'static str = "notes.note_created";
/// }
/// ```
pub trait Event: Serialize {
    const NAME: &'static str;
}

/// Who caused a change. Recorded with every event (so audit knows "who"). Built with
/// [`Actor::system`], [`Actor::user`], [`Actor::api_key`], [`Actor::public`] or from
/// `&Principal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Actor {
    /// The user; `None` when it is the system (CLI command, job...).
    pub user_id: Option<Uuid>,
    /// The API key, when the user acts through a key.
    pub api_key_id: Option<Uuid>,
    /// An anonymous stranger who is not logged in (form submission, newsletter signup, a visitor's comment)
    /// — different from the system, even though neither has a `user_id`.
    pub public: bool,
    /// The path through which the actor entered the tenant (step 34): directly, delegated from a parent
    /// tenant, or the platform staff's support access - the tenant's audit shows who came in by which path.
    pub access: crate::access::AccessPath,
    /// Kind of actor (step 51 P12); `None`: inferred from `user_id` / `public` as before.
    pub kind: Option<crate::access::ActorKind>,
}

impl Actor {
    /// The system: CLI commands run by an operator, background jobs.
    pub fn system() -> Self {
        Actor::default()
    }

    /// The user `user_id` (not through an API key).
    pub fn user(user_id: Uuid) -> Self {
        Actor { user_id: Some(user_id), ..Actor::default() }
    }

    /// User `user_id` acting through API key `api_key_id`.
    pub const fn api_key(user_id: Uuid, api_key_id: Uuid) -> Self {
        Actor {
            user_id: Some(user_id),
            api_key_id: Some(api_key_id),
            public: false,
            access: crate::access::AccessPath::Direct,
            kind: None,
        }
    }

    /// An anonymous stranger who is not logged in.
    pub fn public() -> Self {
        Actor { public: true, ..Actor::default() }
    }

    /// `public`, `system`, `user` (a person), `agent`, `service`, `support`... - the value of the
    /// `actor_kind` column.
    pub fn kind(&self) -> &'static str {
        if self.public {
            "public"
        } else if let Some(kind) = self.kind {
            kind.as_str()
        } else if self.user_id.is_some() {
            "user"
        } else {
            "system"
        }
    }
}

impl From<&crate::access::Principal> for Actor {
    fn from(principal: &crate::access::Principal) -> Self {
        Actor {
            user_id: Some(principal.user_id()),
            api_key_id: principal.api_key_id(),
            public: false,
            // Client OAuth làm thay người dùng: audit ghi grant của nó (D-20261126-mcp-oauth).
            // Đường vào khác (uỷ quyền, hỗ trợ, luật tự động — chốt chống lặp của `automations` đọc nó) giữ nguyên.
            access: match (principal.oauth(), principal.access()) {
                (Some(oauth), crate::access::AccessPath::Direct) => {
                    crate::access::AccessPath::OAuthGrant(oauth.grant_id())
                }
                (_, access) => access,
            },
            // Người (`Human`) suy được từ `user_id`: chỉ ghi loại khi khác (agent, service, hỗ trợ…).
            kind: Some(principal.actor_kind())
                .filter(|kind| *kind != crate::access::ActorKind::Human),
        }
    }
}

/// An event being delivered to a receiver. The receiver reads it; tests build it with [`Delivered::new`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Delivered {
    pub id: Uuid,
    /// The event's tenant: the receiver reads / writes data of exactly this tenant (database:
    /// `tenant.database()`).
    pub tenant: TenantContext,
    /// The module that emitted the event.
    pub module: &'static str,
    pub name: String,
    pub actor: Actor,
    /// The record that was changed, in the form `<kind>:<id>` (e.g. `auth.user:0190...`), if any.
    pub subject: Option<String>,
    pub payload: serde_json::Value,
    pub occurred_at: Timestamp,
}

impl Delivered {
    /// Event `name` of module `module` for `tenant`, with a new id, done by the system just now,
    /// about no particular record.
    pub fn new(
        tenant: TenantContext,
        module: &'static str,
        name: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Delivered {
            id: Uuid::now_v7(),
            tenant,
            module,
            name: name.into(),
            actor: Actor::system(),
            subject: None,
            payload,
            occurred_at: Timestamp::now(),
        }
    }

    /// Sets the event id.
    pub const fn id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }

    /// Sets who caused the change.
    pub const fn actor(mut self, actor: Actor) -> Self {
        self.actor = actor;
        self
    }

    /// Sets the changed record, as `<kind>:<id>`.
    pub fn subject(mut self, subject: Option<String>) -> Self {
        self.subject = subject;
        self
    }

    /// Sets when the change happened.
    pub const fn occurred_at(mut self, occurred_at: Timestamp) -> Self {
        self.occurred_at = occurred_at;
        self
    }

    /// The organization node the event is about, read the one uniform way (D-20261125-event-org): the
    /// payload's top-level `org_id`, else `after.org_id`, else `before.org_id` (the `{"before", "after"}`
    /// snapshot modules record). `None`: the event names no node - it concerns the whole tenant, or its
    /// module does not record one; a consumer limited to one branch must then treat it as outside that
    /// branch.
    pub fn org_id(&self) -> Option<crate::tenancy::OrgId> {
        ["/org_id", "/after/org_id", "/before/org_id"]
            .iter()
            .find_map(|pointer| self.payload.pointer(pointer).and_then(serde_json::Value::as_str))
            .and_then(|raw| raw.parse::<Uuid>().ok())
            .map(crate::tenancy::OrgId::from_uuid)
    }
}

/// An event receiver. When it returns an error the event is delivered again later.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`.
#[async_trait::async_trait]
pub trait EventHandler: Send + Sync {
    async fn handle(&self, event: &Delivered) -> Result<(), Error>;
}

/// Subscribes to events named `event` (or to every event, with [`Subscription::ALL`]).
#[derive(Clone)]
pub struct Subscription {
    event: &'static str,
    handler: Arc<dyn EventHandler>,
    optional: bool,
}

impl Subscription {
    /// Use as `event` to receive every event of every module (e.g. the audit log).
    pub const ALL: &'static str = "*";

    pub fn new(event: &'static str, handler: Arc<dyn EventHandler>) -> Self {
        Subscription { event, handler, optional: false }
    }

    /// Like [`new`](Self::new), but for events of a module that MAY NOT be installed
    /// (D-20261118-step41-kernel-prep): when no module declares `event` the registry only logs at `info`
    /// level instead of failing startup. Used when a module registered earlier listens to a module registered
    /// later (e.g. `catalog` listens to `inventory.cost_changed`).
    pub fn optional(event: &'static str, handler: Arc<dyn EventHandler>) -> Self {
        Subscription { event, handler, optional: true }
    }

    pub fn event(&self) -> &'static str {
        self.event
    }

    /// `true` when created with [`optional`](Self::optional).
    pub fn is_optional(&self) -> bool {
        self.optional
    }

    fn matches(&self, name: &str) -> bool {
        self.event == Self::ALL || self.event == name
    }
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription")
            .field("event", &self.event)
            .field("optional", &self.optional)
            .finish_non_exhaustive()
    }
}

/// Every kind of event in the program, in name order.
#[derive(Debug, Clone, Default)]
pub struct EventCatalog {
    events: Arc<Vec<(&'static str, EventInfo)>>,
}

impl EventCatalog {
    /// Validates and collects the modules' events (`(module name, its events)`).
    pub fn new(modules: Vec<(&'static str, Vec<EventInfo>)>) -> Result<Self, Vec<String>> {
        let mut problems = Vec::new();
        let mut seen = BTreeSet::new();
        let mut events = Vec::new();
        for (module, declared) in modules {
            for event in declared {
                if !has_module_prefix(module, event.name) {
                    problems.push(format!(
                        "event `{}` of module `{module}` must be named `{module}.<what_happened>`, \
                         in lowercase letters, digits and `_` (e.g. `{module}.item_created`)",
                        event.name
                    ));
                }
                if event.description.trim().is_empty() {
                    problems.push(format!("event `{}` has no description", event.name));
                }
                if !seen.insert(event.name) {
                    problems.push(format!("event `{}` is declared twice", event.name));
                }
                events.push((module, event));
            }
        }
        if !problems.is_empty() {
            return Err(problems);
        }
        events.sort_by_key(|(_, event)| *event);
        Ok(EventCatalog { events: Arc::new(events) })
    }

    pub fn contains(&self, name: &str) -> bool {
        self.events.iter().any(|(_, event)| event.name == name)
    }

    /// Every kind of event, in name order.
    pub fn iter(&self) -> impl Iterator<Item = &EventInfo> {
        self.events.iter().map(|(_, event)| event)
    }

    /// Streamable events (step 40 P5): (emitting module, event), in name order.
    pub fn streamable(&self) -> Vec<(&'static str, EventInfo)> {
        self.events.iter().filter(|(_, event)| event.stream.is_some()).copied().collect()
    }

    /// Modules that emit at least one event (i.e. that have a `<module>_events` table).
    pub fn modules(&self) -> Vec<&'static str> {
        let modules: BTreeSet<&'static str> =
            self.events.iter().map(|(module, _)| *module).collect();
        modules.into_iter().collect()
    }
}

/// Whether `name` has the form `<module>.<segment>`.
fn has_module_prefix(module: &str, name: &str) -> bool {
    name.split_once('.').is_some_and(|(prefix, rest)| prefix == module && is_identifier(rest))
}

/// The event table name of module `module`.
pub(crate) fn table_name(module: &str) -> String {
    format!("{module}_events")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn danh_muc_event_kiem_ten() {
        let catalog = EventCatalog::new(vec![
            ("notes", vec![EventInfo::new("notes.note_created", "Created.")]),
            ("auth", vec![EventInfo::new("auth.user_created", "Created.")]),
        ])
        .unwrap();
        assert!(catalog.contains("notes.note_created"));
        assert_eq!(catalog.modules(), ["auth", "notes"]);
        let names: Vec<&str> = catalog.iter().map(EventInfo::name).collect();
        assert_eq!(names, ["auth.user_created", "notes.note_created"]);

        let problems = EventCatalog::new(vec![(
            "notes",
            vec![
                EventInfo::new("tags.created", "Wrong module."),
                EventInfo::new("notes.Created", "Upper case."),
                EventInfo::new("notes.done", " "),
                EventInfo::new("notes.done", "Twice."),
            ],
        )])
        .unwrap_err();
        assert_eq!(problems.len(), 4, "{problems:#?}");
    }

    #[test]
    fn actor_lay_tu_principal() {
        let user = Uuid::now_v7();
        let key = Uuid::now_v7();
        let principal = crate::access::Principal::new(user, []).via_api_key(key);

        assert_eq!(Actor::from(&principal), Actor::api_key(user, key));
        assert_eq!(Actor::system().user_id, None);
    }
}
