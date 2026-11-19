//! Hợp đồng của module `conversations` cho module khác (`autoxide_conversations::api::…`): tên event
//! để nghe bằng `Module::subscriptions`. Event không mang nội dung tin nhắn.

/// Người mua bắt đầu một cuộc trò chuyện với một nút. `after`: `id`, `org_id`, `customer_user_id`,
/// `subject`, `status`…
pub const THREAD_OPENED: &str = "conversations.thread_opened";
/// Một tin mới (không kèm chữ). `after`: `id`, `thread_id`, `author_kind`, `author_user_id`,
/// `auto_reply`.
pub const MESSAGE_CREATED: &str = "conversations.message_created";
/// Một cuộc trò chuyện được đóng.
pub const THREAD_CLOSED: &str = "conversations.thread_closed";
