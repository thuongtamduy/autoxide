//! Contract of the `conversations` module for other modules (`autoxide_conversations::api::…`): event names
//! to listen to with `Module::subscriptions`. Events do not carry message content.

/// A buyer starts a conversation with a branch. `after`: `id`, `org_id`, `customer_user_id`,
/// `subject`, `status`…
pub const THREAD_OPENED: &str = "conversations.thread_opened";
/// A new message (without text). `after`: `id`, `thread_id`, `author_kind`, `author_user_id`,
/// `auto_reply`.
pub const MESSAGE_CREATED: &str = "conversations.message_created";
/// A conversation is closed.
pub const THREAD_CLOSED: &str = "conversations.thread_closed";
