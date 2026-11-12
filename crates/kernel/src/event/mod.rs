//! Event: việc phụ của một thay đổi (audit, thông báo…) đi qua event, theo mẫu
//! "transactional outbox".
//!
//! 1. Module khai các event nó phát qua `Module::events` và thêm
//!    [`outbox_migration`] vào migration của mình: bảng `<module>_events`.
//! 2. Trong CÙNG transaction với thay đổi dữ liệu, module ghi event bằng
//!    [`Outbox::record`]. Transaction hỏng thì event cũng không có; commit thì chắc chắn
//!    có — không bao giờ "đổi dữ liệu mà quên báo", hay "báo mà dữ liệu không đổi".
//! 3. Sau khi commit, [`Dispatcher`] của kernel đọc các bảng event và giao cho những
//!    module đăng ký nhận (`Module::subscriptions`). Bên nhận lỗi thì thử lại với
//!    backoff; quá [`MAX_ATTEMPTS`] lần thì đánh dấu hỏng và ghi log.
//!
//! Giao **ít nhất một lần** (at-least-once): bên nhận phải chịu được việc nhận lại
//! cùng một event (so theo `id`), vì máy chủ có thể tắt giữa lúc giao.

mod dispatcher;
mod migration;
mod outbox;

use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::error::Error;
use crate::module::is_identifier;

pub use dispatcher::{Dispatcher, MAX_ATTEMPTS};
pub use migration::{outbox_actor_migration, outbox_migration};
pub use outbox::Outbox;

/// Một loại event, khai bởi module phát nó.
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
}

impl EventInfo {
    /// `name`: `<module>.<việc>` (chữ thường, số, `_`). `description`: câu tiếng Anh.
    pub const fn new(name: &'static str, description: &'static str) -> Self {
        EventInfo { name, description }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn description(&self) -> &'static str {
        self.description
    }
}

/// Nội dung của một event: kiểu dữ liệu đổi được sang JSON, gắn với tên event.
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

/// Ai gây ra một thay đổi. Ghi kèm mọi event (để audit biết "ai").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Actor {
    /// Người dùng; `None` khi là hệ thống (lệnh CLI, job…).
    pub user_id: Option<Uuid>,
    /// API key, khi người dùng hành động qua key.
    pub api_key_id: Option<Uuid>,
    /// Người lạ chưa đăng nhập (gửi form, đăng ký bản tin, bình luận của khách) — khác hệ
    /// thống, dù cả hai đều không có `user_id`.
    pub public: bool,
}

impl Actor {
    /// Hệ thống: lệnh CLI chạy bởi người vận hành, job chạy nền.
    pub fn system() -> Self {
        Actor::default()
    }

    /// Người dùng `user_id` (không qua API key).
    pub fn user(user_id: Uuid) -> Self {
        Actor { user_id: Some(user_id), ..Actor::default() }
    }

    /// Người lạ chưa đăng nhập.
    pub fn public() -> Self {
        Actor { public: true, ..Actor::default() }
    }

    /// `system`, `user` hay `public` — giá trị cột `actor_kind`.
    pub fn kind(&self) -> &'static str {
        if self.public {
            "public"
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
        }
    }
}

/// Một event đang được giao cho bên nhận.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivered {
    pub id: Uuid,
    /// Module phát event.
    pub module: &'static str,
    pub name: String,
    pub actor: Actor,
    /// Bản ghi bị thay đổi, dạng `<loại>:<id>` (vd `auth.user:0190…`), nếu có.
    pub subject: Option<String>,
    pub payload: serde_json::Value,
    pub occurred_at: DateTime<Utc>,
}

/// Bên nhận event. Trả lỗi thì event được giao lại sau.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`.
#[async_trait::async_trait]
pub trait EventHandler: Send + Sync {
    async fn handle(&self, event: &Delivered) -> Result<(), Error>;
}

/// Đăng ký nhận event tên `event` (hoặc mọi event, với [`Subscription::ALL`]).
#[derive(Clone)]
pub struct Subscription {
    event: &'static str,
    handler: Arc<dyn EventHandler>,
}

impl Subscription {
    /// Dùng làm `event` để nhận mọi event của mọi module (vd audit log).
    pub const ALL: &'static str = "*";

    pub fn new(event: &'static str, handler: Arc<dyn EventHandler>) -> Self {
        Subscription { event, handler }
    }

    pub fn event(&self) -> &'static str {
        self.event
    }

    fn matches(&self, name: &str) -> bool {
        self.event == Self::ALL || self.event == name
    }
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription").field("event", &self.event).finish_non_exhaustive()
    }
}

/// Mọi loại event của chương trình, theo thứ tự tên.
#[derive(Debug, Clone, Default)]
pub struct EventCatalog {
    events: Arc<Vec<(&'static str, EventInfo)>>,
}

impl EventCatalog {
    /// Kiểm và gom event của các module (`(tên module, event của nó)`).
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

    /// Mọi loại event, theo thứ tự tên.
    pub fn iter(&self) -> impl Iterator<Item = &EventInfo> {
        self.events.iter().map(|(_, event)| event)
    }

    /// Các module có phát ít nhất một event (tức có bảng `<module>_events`).
    pub fn modules(&self) -> Vec<&'static str> {
        let modules: BTreeSet<&'static str> =
            self.events.iter().map(|(module, _)| *module).collect();
        modules.into_iter().collect()
    }
}

/// `name` có dạng `<module>.<đoạn>` không.
fn has_module_prefix(module: &str, name: &str) -> bool {
    name.split_once('.').is_some_and(|(prefix, rest)| prefix == module && is_identifier(rest))
}

/// Tên bảng event của module `module`.
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

        assert_eq!(
            Actor::from(&principal),
            Actor { user_id: Some(user), api_key_id: Some(key), public: false }
        );
        assert_eq!(Actor::system().user_id, None);
    }
}
