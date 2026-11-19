//! Tín hiệu tạm qua WebSocket (bước 69 P3, D-20261119-step69-ws): "đang gõ", "đang online" trong
//! một chủ đề (vd một cuộc trò chuyện). KHÔNG lưu: chỉ chuyển giữa các kết nối đang mở ở cùng tiến
//! trình — mất tín hiệu thì chỉ mất một dấu "đang gõ", không mất dữ liệu (dữ liệu đi qua REST và
//! event).
//!
//! Kernel không biết chủ đề nào tồn tại hay ai được vào: module sở hữu loại chủ đề góp một
//! [`SignalTopics`] (`context.contribute::<dyn SignalTopics>(…)`) trả lời "người này có được gửi
//! và nhận tín hiệu ở chủ đề `<loại>:<id>` không". Chưa module nào góp loại đó thì không ai vào được.

use std::sync::{Arc, OnceLock};

use crate::access::Principal;
use crate::error::Error;
use crate::tenancy::TenantContext;

/// Module góp để mở chủ đề tín hiệu của mình (vd `conversations.thread`).
#[async_trait::async_trait]
pub trait SignalTopics: Send + Sync {
    /// Loại chủ đề, `<module>.<loại>`; chủ đề là `<loại>:<id>`.
    fn kind(&self) -> &'static str;

    /// `principal` có được gửi và nhận tín hiệu ở chủ đề `<kind>:<id>` của khách `tenant` không —
    /// thường: có đọc được bản ghi `id` không.
    async fn allows(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: &str,
    ) -> Result<bool, Error>;
}

/// Các [`SignalTopics`] module đã góp; điền sau khi mọi module khởi động. Clone rẻ.
#[derive(Clone, Default)]
pub(crate) struct SignalTopicsSlot(Arc<OnceLock<Vec<Arc<dyn SignalTopics>>>>);

impl SignalTopicsSlot {
    pub(crate) fn fill(&self, topics: Vec<Arc<dyn SignalTopics>>) {
        let _ = self.0.set(topics);
    }

    /// Bên trả lời cho loại `kind`.
    pub(crate) fn find(&self, kind: &str) -> Option<Arc<dyn SignalTopics>> {
        self.0.get()?.iter().find(|topics| topics.kind() == kind).cloned()
    }
}
