//! Records an event into the module's event table, inside the caller's transaction.

use crate::time::Timestamp;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Query};
use std::sync::Arc;
use tokio::sync::Notify;
use uuid::Uuid;

use super::{Actor, Event, table_name};
use crate::error::Error;
use crate::tenancy::TenantContext;

/// Where a module records its events. Obtained with `StartContext::outbox()` at
/// startup; cheap to clone.
#[derive(Debug, Clone)]
pub struct Outbox {
    module: &'static str,
    table: Arc<str>,
    /// Wakes the dispatcher when a new event arrives, instead of waiting for the next polling turn.
    wake: Arc<Notify>,
}

impl Outbox {
    pub(crate) fn with_wake(module: &'static str, wake: Arc<Notify>) -> Self {
        Outbox { module, table: table_name(module).into(), wake }
    }

    /// A standalone `Outbox`, not connected to the program's dispatcher - for a module's
    /// tests. In the real program, obtain it with `StartContext::outbox`.
    pub fn standalone(module: &'static str) -> Self {
        Outbox::with_wake(module, Arc::new(Notify::new()))
    }

    /// Records `event` of tenant `tenant` caused by `actor`, about the record `subject` (in the form
    /// `<kind>:<id>`, if any). Call it with the transaction that writes the change: the event exists only
    /// when the transaction commits.
    pub async fn record<C, E>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        actor: Actor,
        subject: Option<String>,
        event: &E,
    ) -> Result<(), Error>
    where
        C: crate::db::Executor,
        E: Event,
    {
        let connection = &crate::db::Via::of(connection);
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
        let now = Timestamp::now();
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
                Alias::new("actor_access"),
                Alias::new("actor_access_id"),
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
                actor.access.as_str().into(),
                actor.access.id().into(),
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
