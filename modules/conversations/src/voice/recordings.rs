//! Bản ghi âm (D-232, ghi âm tắt mặc định): job tải về từ nhà cung cấp → lưu vào `Storage` của khách
//! (riêng tư, khoá `conversations/recordings/<call_id>`) → xoá ở nhà cung cấp; nghe (ghi event mỗi lần
//! mở), xoá theo yêu cầu chủ thể, xoá khi quá hạn lưu và khi tài khoản bị xoá.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{Job, JobContext, JobHandler};
use autoxide_kernel::storage::{Storage, StoredFile, TenantStorage};
use autoxide_kernel::tenancy::{DatabaseRegistry, OrgId, ScopeExt, TenantContext, TenantId};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use super::calls::{Call, call_not_found};
use crate::entity::{call, callback};
use crate::events::{self, Change};
use crate::permissions::{RECORDINGS_READ, VOICE_MANAGE};
use crate::service::{ConversationsService, now};

pub(crate) const FETCH_RECORDING: &str = "conversations.fetch_recording";
pub(crate) const PURGE_RECORDINGS: &str = "conversations.purge_recordings";

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct FetchRecording {
    pub call_id: Uuid,
}

impl Job for FetchRecording {
    const KIND: &'static str = FETCH_RECORDING;
}

pub(crate) struct Fetcher(pub Arc<ConversationsService>);

#[async_trait::async_trait]
impl JobHandler for Fetcher {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let Some(tenant) = &job.tenant else {
            return Ok(());
        };
        let payload: FetchRecording =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.fetch_recording(tenant, payload.call_id).await
    }
}

/// Job mỗi ngày: xoá bản ghi quá hạn của mọi khách trên database của job.
pub(crate) struct Purger(pub Arc<ConversationsService>, pub DatabaseRegistry);

#[async_trait::async_trait]
impl JobHandler for Purger {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let at = now();
        let tenants: Vec<Uuid> = call::Entity::find()
            .select_only()
            .column(call::Column::TenantId)
            .distinct()
            .filter(call::Column::RecordingExpiresAt.lt(at))
            .into_tuple()
            .all(job.database.read())
            .await?;
        for id in tenants {
            let tenant = match self.1.tenant(TenantId::from_uuid(id)).await {
                Ok(tenant) => tenant,
                Err(error) => {
                    tracing::warn!(tenant_id = %id, code = error.code(), "recording purge skipped a tenant");
                    continue;
                }
            };
            self.0.purge_recordings(&tenant, at).await?;
        }
        Ok(())
    }
}

fn key_of(call_id: Uuid) -> String {
    format!("conversations/recordings/{call_id}")
}

impl ConversationsService {
    fn store(&self, tenant: &TenantContext) -> Result<TenantStorage, Error> {
        let storage = self.parts.storage.clone().ok_or_else(|| {
            Error::unavailable(
                "conversations.storage_unavailable",
                "Call recordings cannot be stored: module `files` is not installed.",
            )
        })?;
        Ok(TenantStorage::new(storage, tenant))
    }

    /// Tải bản ghi của cuộc gọi `call_id` về `Storage` rồi xoá ở nhà cung cấp. Chạy lại không sao.
    pub(crate) async fn fetch_recording(
        &self,
        tenant: &TenantContext,
        call_id: Uuid,
    ) -> Result<(), Error> {
        let row = self.call(tenant, call_id).await?;
        let Some(provider_id) = row.provider_recording_id.clone() else {
            return Ok(());
        };
        let provider = self.provider(&row.provider).ok_or_else(super::lines::voice_unavailable)?;
        if row.recording_ref.is_none() {
            // Ngoài transaction ghi: tải từ nhà cung cấp, ghi vào nơi lưu.
            let content = provider.fetch_recording(tenant.id(), &provider_id).await?;
            let key = key_of(call_id);
            self.store(tenant)?.put(&key, content).await?;
            let ended = row.ended_at.unwrap_or_else(now);
            let expires = ended
                + SignedDuration::from_hours(
                    24 * (i64::from(self.config.recording_retention_days)),
                );
            call::Entity::update_many()
                .col_expr(call::Column::RecordingRef, Expr::value(Some(key)))
                .col_expr(call::Column::RecordingExpiresAt, Expr::value(Some(expires)))
                .col_expr(call::Column::UpdatedAt, Expr::value(now()))
                .filter(call::Column::Id.eq(call_id))
                .scoped(tenant)
                .exec(tenant.database().write())
                .await?;
        }
        // Đã lưu về: xoá ở nhà cung cấp (xoá lại cái đã xoá vẫn thành công).
        provider.delete_recording(tenant.id(), &provider_id).await?;
        call::Entity::update_many()
            .col_expr(call::Column::ProviderRecordingId, Expr::value(Option::<String>::None))
            .filter(call::Column::Id.eq(call_id))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        Ok(())
    }

    /// Mở bản ghi của cuộc gọi `call_id` (`conversations.recordings.read`); ghi event mỗi lần mở.
    pub(crate) async fn open_recording(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        call_id: Uuid,
    ) -> Result<StoredFile, Error> {
        let row = self.scoped_call(tenant, principal, call_id, &RECORDINGS_READ).await?;
        let key = row.recording_ref.clone().ok_or_else(|| recording_not_found(call_id))?;
        let file =
            self.store(tenant)?.get(&key).await?.ok_or_else(|| recording_not_found(call_id))?;
        let transaction = tenant.database().begin_write().await?;
        let event = events::RecordingOpened(Change {
            before: None,
            after: Some(json!({"call_id": call_id, "thread_id": row.thread_id})),
            audience: Vec::new(),
        });
        let subject = events::thread_subject(row.thread_id);
        self.parts
            .outbox
            .record(&transaction, tenant, Actor::from(principal), subject, &event)
            .await?;
        transaction.commit().await?;
        Ok(file)
    }

    /// Xoá bản ghi theo yêu cầu (`conversations.voice.manage`).
    pub(crate) async fn delete_recording(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        call_id: Uuid,
    ) -> Result<Call, Error> {
        let row = self.scoped_call(tenant, principal, call_id, &VOICE_MANAGE).await?;
        self.erase_recording(tenant, Actor::from(principal), &row, "request").await?;
        self.call(tenant, call_id).await
    }

    async fn scoped_call(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        call_id: Uuid,
        permission: &autoxide_kernel::access::Permission,
    ) -> Result<Call, Error> {
        let row = self.call(tenant, call_id).await?;
        let target = OrgTarget::new(OrgId::from_uuid(row.org_id), row.agent_user_id);
        principal.require_at(tenant.database().read(), permission, target).await.map_err(
            |error| {
                if error.code() == "org_out_of_scope" { call_not_found(call_id) } else { error }
            },
        )?;
        Ok(row)
    }

    /// Xoá bản ghi của `row` ở nơi lưu và ở nhà cung cấp; ghi event. Không có bản ghi thì thôi.
    async fn erase_recording(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        row: &Call,
        reason: &str,
    ) -> Result<bool, Error> {
        if row.recording_ref.is_none() && row.provider_recording_id.is_none() {
            return Ok(false);
        }
        if let Some(key) = &row.recording_ref {
            self.store(tenant)?.delete(key).await?;
        }
        if let (Some(id), Some(provider)) =
            (&row.provider_recording_id, self.provider(&row.provider))
        {
            provider.delete_recording(tenant.id(), id).await?;
        }
        let transaction = tenant.database().begin_write().await?;
        call::Entity::update_many()
            .col_expr(call::Column::RecordingRef, Expr::value(Option::<String>::None))
            .col_expr(call::Column::ProviderRecordingId, Expr::value(Option::<String>::None))
            .col_expr(call::Column::RecordingExpiresAt, Expr::value(Option::<Timestamp>::None))
            .col_expr(call::Column::UpdatedAt, Expr::value(now()))
            .filter(call::Column::Id.eq(row.id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let event = events::RecordingDeleted(Change {
            before: None,
            after: Some(json!({"call_id": row.id, "thread_id": row.thread_id, "reason": reason})),
            audience: Vec::new(),
        });
        let subject = events::thread_subject(row.thread_id);
        self.parts.outbox.record(&transaction, tenant, actor, subject, &event).await?;
        transaction.commit().await?;
        Ok(true)
    }

    /// Xoá mọi bản ghi quá hạn lưu của khách `tenant`; trả số đã xoá.
    pub(crate) async fn purge_recordings(
        &self,
        tenant: &TenantContext,
        at: Timestamp,
    ) -> Result<u64, Error> {
        let expired = call::Entity::find()
            .filter(call::Column::RecordingExpiresAt.lt(at))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let mut count = 0;
        for row in expired {
            if self.erase_recording(tenant, Actor::system(), &row, "retention").await? {
                count += 1;
            }
        }
        Ok(count)
    }
}

/// Tài khoản `user` bị xoá: xoá bản ghi các cuộc gọi của người đó, xoá số trong yêu cầu gọi lại.
/// `service` là `None` khi module đang tắt: bản ghi giữ lại tới khi bật lại (xem trong hàm).
pub(crate) async fn forget_calls(
    service: Option<&ConversationsService>,
    tenant: &TenantContext,
    user: Uuid,
    threads: &[Uuid],
) -> Result<(), Error> {
    let calls = call::Entity::find()
        .filter(
            sea_orm::Condition::any()
                .add(call::Column::CallerUserId.eq(user))
                .add(call::Column::ThreadId.is_in(threads.to_vec())),
        )
        .scoped(tenant)
        .all(tenant.database().read())
        .await?;
    for row in calls {
        if let Some(service) = service {
            service.erase_recording(tenant, Actor::system(), &row, "account_deleted").await?;
        } else if row.recording_ref.is_some() || row.provider_recording_id.is_some() {
            // Module đang tắt (chế độ nghỉ, bước 70 P9): không có nơi lưu của nó lẫn bộ nối nhà cung
            // cấp thoại, nên bản ghi giữ nguyên (cột không đổi) để job hạn lưu xoá khi bật lại; phần
            // còn lại (người gọi, số gọi lại) vẫn gỡ ngay bên dưới.
            tracing::warn!(call_id = %row.id, "call recording kept: module `conversations` is turned off");
        }
    }
    let at = now();
    // Yêu cầu còn mở của người đó: huỷ (không ai gọi tới số đã xoá).
    callback::Entity::update_many()
        .col_expr(callback::Column::Status, Expr::value("cancelled"))
        .filter(callback::Column::RequesterUserId.eq(user))
        .filter(callback::Column::Status.is_in(["open", "calling"]))
        .scoped(tenant)
        .exec(tenant.database().write())
        .await?;
    callback::Entity::update_many()
        .col_expr(callback::Column::PhoneSealed, Expr::value(Option::<String>::None))
        .col_expr(callback::Column::PhoneMasked, Expr::value(""))
        .col_expr(callback::Column::RequesterUserId, Expr::value(Option::<Uuid>::None))
        .col_expr(callback::Column::OpenKey, Expr::value(Option::<String>::None))
        .col_expr(callback::Column::UpdatedAt, Expr::value(at))
        .filter(callback::Column::RequesterUserId.eq(user))
        .scoped(tenant)
        .exec(tenant.database().write())
        .await?;
    call::Entity::update_many()
        .col_expr(call::Column::CallerUserId, Expr::value(Option::<Uuid>::None))
        .filter(call::Column::CallerUserId.eq(user))
        .scoped(tenant)
        .exec(tenant.database().write())
        .await?;
    Ok(())
}

pub(crate) fn recording_not_found(id: Uuid) -> Error {
    Error::not_found("conversations.recording_not_found", "Call `{id}` has no recording.")
        .with_param("id", id)
}
