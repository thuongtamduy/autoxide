//! Ghi event vào bảng event của module, trong transaction của bên gọi.

use std::sync::Arc;

use chrono::{SubsecRound, Utc};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Query};
use tokio::sync::Notify;
use uuid::Uuid;

use super::{Actor, Event, table_name};
use crate::error::Error;
use crate::tenancy::TenantContext;

/// Nơi một module ghi event của mình. Lấy bằng `StartContext::outbox()` lúc khởi
/// động; clone rẻ.
#[derive(Debug, Clone)]
pub struct Outbox {
    module: &'static str,
    table: Arc<str>,
    /// Đánh thức bộ phát khi có event mới, thay vì chờ tới lượt hỏi kế tiếp.
    wake: Arc<Notify>,
}

impl Outbox {
    pub(crate) fn with_wake(module: &'static str, wake: Arc<Notify>) -> Self {
        Outbox { module, table: table_name(module).into(), wake }
    }

    /// `Outbox` đứng riêng, không nối với bộ phát của chương trình — cho test của
    /// module. Trong chương trình thật, lấy bằng `StartContext::outbox`.
    pub fn standalone(module: &'static str) -> Self {
        Outbox::with_wake(module, Arc::new(Notify::new()))
    }

    /// Ghi `event` của khách `tenant` do `actor` gây ra, về bản ghi `subject` (dạng
    /// `<loại>:<id>`, nếu có). Gọi với transaction đang ghi thay đổi: event chỉ tồn tại khi
    /// transaction commit.
    pub async fn record<C, E>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        actor: Actor,
        subject: Option<String>,
        event: &E,
    ) -> Result<(), Error>
    where
        C: ConnectionTrait,
        E: Event,
    {
        // Lỗi của người viết module, không phải của người dùng: báo to để sửa sớm.
        if E::NAME.split_once('.').map(|(module, _)| module) != Some(self.module) {
            return Err(Error::internal(format!(
                "module `{}` cannot record event `{}`: events of a module start with `{}.`",
                self.module,
                E::NAME,
                self.module
            )));
        }
        let payload = serde_json::to_string(event).map_err(Error::internal)?;
        let now = Utc::now().trunc_subsecs(6);
        let mut insert = Query::insert();
        insert
            .into_table(Alias::new(&*self.table))
            .columns([
                Alias::new("tenant_id"),
                Alias::new("id"),
                Alias::new("name"),
                Alias::new("actor_user_id"),
                Alias::new("actor_api_key_id"),
                Alias::new("actor_kind"),
                Alias::new("subject"),
                Alias::new("payload"),
                Alias::new("occurred_at"),
                Alias::new("attempts"),
                Alias::new("next_attempt_at"),
            ])
            .values([
                tenant.id().as_uuid().into(),
                Uuid::now_v7().into(),
                E::NAME.into(),
                actor.user_id.into(),
                actor.api_key_id.into(),
                actor.kind().into(),
                subject.into(),
                payload.into(),
                now.into(),
                0i32.into(),
                now.into(),
            ])
            .map_err(|error| Error::internal(format!("event insert: {error}")))?;
        connection.execute(&insert).await?;
        // Đánh thức sớm; nếu bộ phát thức dậy trước khi transaction commit, nó sẽ thấy
        // event ở lượt hỏi kế tiếp.
        self.wake.notify_one();
        Ok(())
    }
}
