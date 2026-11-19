//! Yêu cầu gọi lại (D-232): người mua xin cửa hàng gọi lại vào số đã xác thực, trong giờ nhận gọi
//! lại; mỗi cuộc trò chuyện một yêu cầu mở (`open_key`), gửi lại cùng `client_id` trả yêu cầu cũ.
//! Nhân viên bấm gọi: transaction ngắn khoá dòng, `open → calling`, ghi cuộc gọi `queued` → commit →
//! NGOÀI transaction gọi nhà cung cấp với số hiển thị là hotline của cửa hàng.

use std::time::Duration as StdDuration;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::rows::lock_one;
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::throttle::{Outcome, Throttle};
use autoxide_kernel::voice::{CallStatus, OutboundCall};
use chrono::{DateTime, Duration, Utc};
use sea_orm::sea_query::{Condition, Expr, OnConflict};
use sea_orm::{ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter};
use serde_json::json;
use uuid::Uuid;

use super::hours::covers;
use super::lines::{hours_of, voice_unavailable};
use crate::entity::{call, callback};
use crate::events::{self, Change};
use crate::permissions::CALL;
use crate::service::{CLOSED, ConversationsService, MODULE, Viewer, audience, now};

pub(crate) type Callback = callback::Model;

/// Mục đích mã hoá số điện thoại của yêu cầu gọi lại (`seal`).
pub(crate) const PHONE_PURPOSE: &str = "conversations.callback_phone";
/// Yêu cầu mở quá `window_end` chừng này thì hết hạn.
const EXPIRE_AFTER: Duration = Duration::hours(2);
pub(crate) const EXPIRE_CALLBACKS: &str = "conversations.expire_callbacks";

/// Yêu cầu gọi lại của người mua.
#[derive(Debug, Clone)]
pub(crate) struct NewCallback {
    pub client_id: String,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    /// `account`: số đã xác thực của tài khoản.
    pub phone_source: String,
    pub recording_ack: bool,
}

/// Bộ lọc danh sách yêu cầu gọi lại của nhân viên.
#[derive(Debug, Clone, Default)]
pub(crate) struct CallbackFilter {
    pub status: Option<String>,
    pub org_id: Option<Uuid>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

impl ConversationsService {
    /// Đếm một lượt gọi / yêu cầu gọi lại của người mua trong ngày; quá → 429.
    pub(crate) async fn count_call(&self, tenant: &TenantContext, user: Uuid) -> Result<(), Error> {
        let limit = self.config.calls_per_day;
        let throttle = Throttle::new(MODULE, limit, StdDuration::from_secs(24 * 3600))?;
        match throttle.hit(tenant, &format!("calls:{user}")).await? {
            Outcome::Allowed => Ok(()),
            Outcome::Limited { retry_after } => Err(Error::new(
                ErrorKind::TooManyRequests,
                "conversations.calls_too_many",
                "You made too many calls today ({limit}); try again later.",
            )
            .with_param("limit", limit)
            .with_param("retry_after", retry_after)),
        }
    }

    /// Người mua xin gọi lại trong cuộc `thread_id`. `Ok((yêu cầu, mới tạo))`.
    pub(crate) async fn request_callback(
        &self,
        tenant: &TenantContext,
        customer: Uuid,
        thread_id: Uuid,
        new: NewCallback,
    ) -> Result<(Callback, bool), Error> {
        let thread = self.get(tenant, Viewer::Customer(customer), thread_id).await?;
        if let Some(existing) = self.callback_by_client(tenant, thread_id, &new.client_id).await? {
            return Ok((existing, false));
        }
        if thread.status == CLOSED {
            return Err(crate::messages::thread_closed(thread_id));
        }
        let line = self.line_of(tenant, thread.org_id).await?.ok_or_else(voice_unavailable)?;
        let provider = self.provider(&line.provider).ok_or_else(voice_unavailable)?;
        if !provider.capabilities().outbound {
            return Err(voice_unavailable());
        }
        let mut problems = Vec::new();
        let client_id = new.client_id.trim().to_owned();
        if !(1..=100).contains(&client_id.chars().count()) {
            problems.push(FieldError::new("client_id", "invalid", "must be 1 to 100 characters"));
        }
        if new.phone_source != "account" {
            problems.push(FieldError::new(
                "phone_source",
                "invalid",
                "must be `account` (the verified phone number of your account)",
            ));
        }
        let at = now();
        let days = i64::from(self.config.callback_days_ahead);
        let in_hours = new.window_start < new.window_end
            && new.window_start >= at - Duration::minutes(5)
            && new.window_end <= at + Duration::days(days)
            && covers(&hours_of(&line), line.utc_offset_minutes, new.window_start, new.window_end);
        if !in_hours {
            return Err(Error::new(
                ErrorKind::Validation,
                "conversations.callback_window_invalid",
                "The call-back window must be within the shop's call-back hours, in the next {days} days.",
            )
            .with_param("days", days));
        }
        crate::service::finish(problems)?;
        if line.recording_enabled && !new.recording_ack {
            return Err(recording_notice_required());
        }
        let phone = self.verified_phone(tenant, customer).await?;
        self.count_call(tenant, customer).await?;
        let id = Uuid::now_v7();
        let sealed = self.seal_phone(tenant, id, &phone).await?;
        let row = callback::Model {
            id,
            tenant_id: tenant.id().as_uuid(),
            org_id: thread.org_id,
            thread_id,
            requester_user_id: Some(customer),
            client_id: client_id.clone(),
            phone_sealed: Some(sealed),
            phone_masked: phone.masked(),
            phone_source: "account".to_owned(),
            window_start: new.window_start,
            window_end: new.window_end,
            recording_ack_at: (line.recording_enabled && new.recording_ack).then_some(at),
            status: "open".to_owned(),
            open_key: Some(thread_id.to_string()),
            assigned_user_id: None,
            attempts: 0,
            last_call_id: None,
            created_at: at,
            updated_at: at,
        };
        let transaction = tenant.database().begin_write().await?;
        // Trùng `client_id` hay còn yêu cầu mở: không chèn (`ON CONFLICT DO NOTHING`, D-158).
        let inserted = callback::Entity::insert(row.clone().into_active_model())
            .on_conflict(OnConflict::new().do_nothing().to_owned())
            .exec_without_returning(&transaction)
            .await?;
        if inserted == 0 {
            transaction.rollback().await?;
            if let Some(existing) = self.callback_by_client(tenant, thread_id, &client_id).await? {
                return Ok((existing, false));
            }
            return Err(Error::new(
                ErrorKind::Conflict,
                "conversations.callback_pending",
                "You already have a call-back request waiting.",
            ));
        }
        let event = events::CallbackRequested(Change {
            before: None,
            after: Some(callback_snapshot(&row)),
            audience: audience(&thread),
        });
        let subject = events::thread_subject(thread_id);
        self.parts
            .outbox
            .record(&transaction, tenant, Actor::user(customer), subject, &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(callback_id = %id, phone = %row.phone_masked, "call-back requested");
        Ok((row, true))
    }

    async fn callback_by_client(
        &self,
        tenant: &TenantContext,
        thread_id: Uuid,
        client_id: &str,
    ) -> Result<Option<Callback>, Error> {
        Ok(callback::Entity::find()
            .filter(callback::Column::ThreadId.eq(thread_id))
            .filter(callback::Column::ClientId.eq(client_id.trim()))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?)
    }

    /// Số đã xác thực của tài khoản; không có → 403.
    async fn verified_phone(
        &self,
        tenant: &TenantContext,
        user: Uuid,
    ) -> Result<PhoneNumber, Error> {
        let phone = match &self.parts.accounts {
            Some(accounts) => accounts.verified_phone(tenant.id(), user).await?,
            None => None,
        };
        phone.ok_or_else(|| {
            Error::new(
                ErrorKind::Forbidden,
                "conversations.phone_unverified",
                "Verify a phone number on your account to be called back.",
            )
        })
    }

    async fn seal_phone(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        phone: &PhoneNumber,
    ) -> Result<String, Error> {
        let secret = self.parts.secret.as_ref().ok_or_else(|| Error::internal("no secret key"))?;
        let sealer = secret.sealer().await?;
        sealer.seal(tenant.id(), PHONE_PURPOSE, &id.to_string(), phone.as_str().as_bytes())
    }

    async fn open_phone(
        &self,
        tenant: &TenantContext,
        row: &Callback,
    ) -> Result<PhoneNumber, Error> {
        let sealed = row.phone_sealed.as_deref().ok_or_else(|| {
            Error::new(
                ErrorKind::Conflict,
                "conversations.callback_not_found",
                "The phone number of this request was deleted.",
            )
        })?;
        let secret = self.parts.secret.as_ref().ok_or_else(|| Error::internal("no secret key"))?;
        let opened =
            secret.sealer().await?.open(tenant.id(), PHONE_PURPOSE, &row.id.to_string(), sealed)?;
        let raw = String::from_utf8(opened.to_vec()).map_err(Error::internal)?;
        PhoneNumber::parse(&raw, "84")
            .ok_or_else(|| Error::internal("a sealed phone number is not E.164"))
    }

    /// Người mua huỷ yêu cầu gọi lại còn mở của mình.
    pub(crate) async fn cancel_callback(
        &self,
        tenant: &TenantContext,
        customer: Uuid,
        thread_id: Uuid,
        callback_id: Uuid,
    ) -> Result<Callback, Error> {
        self.get(tenant, Viewer::Customer(customer), thread_id).await?;
        let row = callback::Entity::find_by_id(callback_id)
            .filter(callback::Column::ThreadId.eq(thread_id))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| callback_not_found(callback_id))?;
        self.finish_callback(tenant, Actor::user(customer), &row, "cancelled").await
    }

    /// Nhân viên đóng yêu cầu (đã liên lạc được bằng đường khác…).
    pub(crate) async fn close_callback(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        callback_id: Uuid,
    ) -> Result<Callback, Error> {
        let row = self.staff_callback(tenant, principal, callback_id).await?;
        self.finish_callback(tenant, Actor::from(principal), &row, "done").await
    }

    /// Đổi yêu cầu còn `open` sang trạng thái cuối `to`; đã cuối thì trả nguyên.
    async fn finish_callback(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        row: &Callback,
        to: &str,
    ) -> Result<Callback, Error> {
        if !matches!(row.status.as_str(), "open" | "calling") {
            return Ok(row.clone());
        }
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        callback::Entity::update_many()
            .col_expr(callback::Column::Status, Expr::value(to))
            .col_expr(callback::Column::OpenKey, Expr::value(Option::<String>::None))
            .col_expr(callback::Column::UpdatedAt, Expr::value(at))
            .filter(callback::Column::Id.eq(row.id))
            .filter(callback::Column::Status.eq("open"))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let after = callback::Entity::find_by_id(row.id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| callback_not_found(row.id))?;
        if after.status == "calling" {
            transaction.rollback().await?;
            return Err(callback_taken());
        }
        let _ = actor;
        transaction.commit().await?;
        Ok(after)
    }

    /// Yêu cầu `id` cho nhân viên có `conversations.call` ở nút của nó; ngoài phạm vi → 404.
    async fn staff_callback(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Callback, Error> {
        let read = tenant.database().read();
        let row = callback::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| callback_not_found(id))?;
        let target = autoxide_kernel::access::OrgTarget::new(
            autoxide_kernel::tenancy::OrgId::from_uuid(row.org_id),
            row.assigned_user_id,
        );
        principal.require_at(read, &CALL, target).await.map_err(|error| {
            if error.code() == "org_out_of_scope" { callback_not_found(id) } else { error }
        })?;
        Ok(row)
    }

    /// Một trang yêu cầu gọi lại trong phạm vi `conversations.call`, sớm trước (mặc định `asc`
    /// của `order` thì cũ trước).
    pub(crate) async fn callbacks(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &CallbackFilter,
        page: &PageRequest,
    ) -> Result<Page<Callback>, Error> {
        let scope = principal.scope_condition(
            &CALL,
            callback::Column::OrgId,
            Some(callback::Column::AssignedUserId),
        )?;
        let mut condition = Condition::all().add(scope);
        if let Some(status) = &filter.status {
            condition = condition.add(callback::Column::Status.eq(status.as_str()));
        }
        if let Some(org) = filter.org_id {
            condition = condition.add(callback::Column::OrgId.eq(org));
        }
        if let Some(from) = filter.from {
            condition = condition.add(callback::Column::WindowEnd.gte(from));
        }
        if let Some(to) = filter.to {
            condition = condition.add(callback::Column::WindowStart.lte(to));
        }
        let select = callback::Entity::find().filter(condition).scoped(tenant);
        fetch_page(select, callback::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Nhân viên gọi lại cho người mua. Hai người bấm cùng lúc → một cuộc gọi, người sau 409.
    pub(crate) async fn call_back(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        callback_id: Uuid,
    ) -> Result<call::Model, Error> {
        let row = self.staff_callback(tenant, principal, callback_id).await?;
        let line = self.line_of(tenant, row.org_id).await?.ok_or_else(voice_unavailable)?;
        let provider = self.provider(&line.provider).ok_or_else(voice_unavailable)?;
        let thread = self.reload(tenant, row.thread_id).await?;
        let at = now();
        let call_id = Uuid::now_v7();
        let transaction = tenant.database().begin_write().await?;
        let locked =
            lock_one(&transaction, callback::Entity::find_by_id(callback_id).scoped(tenant))
                .await?
                .ok_or_else(|| callback_not_found(callback_id))?;
        if locked.status != "open" {
            transaction.rollback().await?;
            return Err(if locked.status == "calling" {
                callback_taken()
            } else {
                callback_not_found(callback_id)
            });
        }
        callback::Entity::update_many()
            .col_expr(callback::Column::Status, Expr::value("calling"))
            .col_expr(callback::Column::AssignedUserId, Expr::value(Some(principal.user_id())))
            .col_expr(callback::Column::LastCallId, Expr::value(Some(call_id)))
            .col_expr(callback::Column::UpdatedAt, Expr::value(at))
            .filter(callback::Column::Id.eq(callback_id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let record = line.recording_enabled && locked.recording_ack_at.is_some();
        let new_call = call::Model {
            id: call_id,
            tenant_id: tenant.id().as_uuid(),
            org_id: row.org_id,
            thread_id: row.thread_id,
            callback_id: Some(callback_id),
            direction: "outbound".to_owned(),
            status: CallStatus::Queued.as_str().to_owned(),
            provider: line.provider.clone(),
            provider_call_id: None,
            agent_user_id: Some(principal.user_id()),
            caller_user_id: None,
            started_at: Some(at),
            answered_at: None,
            ended_at: None,
            duration_seconds: None,
            recording_enabled: record,
            recording_ack_at: locked.recording_ack_at,
            provider_recording_id: None,
            recording_ref: None,
            recording_expires_at: None,
            created_at: at,
            updated_at: at,
        };
        call::Entity::insert(new_call.clone().into_active_model())
            .exec_without_returning(&transaction)
            .await?;
        self.record_call(&transaction, tenant, Actor::from(principal), &thread, None, &new_call)
            .await?;
        transaction.commit().await?;
        // Ngoài transaction ghi: gọi nhà cung cấp. Số đầy đủ chỉ mở ra ngay đây.
        let outbound = async {
            let to = self.open_phone(tenant, &locked).await?;
            let from = PhoneNumber::parse(&line.hotline, "84")
                .ok_or_else(|| Error::internal("the hotline is not E.164"))?;
            let notice = record.then(|| self.recording_notice(thread.locale.as_deref()));
            provider
                .place_call(
                    tenant.id(),
                    &OutboundCall {
                        call_id,
                        from,
                        to,
                        agent_identity: principal.user_id().to_string(),
                        record,
                        recording_notice: notice,
                    },
                )
                .await
        };
        match outbound.await {
            Ok(placed) => {
                call::Entity::update_many()
                    .col_expr(
                        call::Column::ProviderCallId,
                        Expr::value(Some(placed.provider_call_id)),
                    )
                    .filter(call::Column::Id.eq(call_id))
                    .filter(call::Column::ProviderCallId.is_null())
                    .scoped(tenant)
                    .exec(tenant.database().write())
                    .await?;
                self.call(tenant, call_id).await
            }
            Err(error) => {
                tracing::warn!(call_id = %call_id, code = error.code(), "call-back could not be placed");
                self.advance(tenant, call_id, CallStatus::Failed, None, None, None, None, at)
                    .await?;
                Err(error)
            }
        }
    }

    /// Job: yêu cầu còn mở quá `window_end` + 2 giờ → `expired`, báo người mua một lần.
    pub(crate) async fn expire_callbacks(&self, tenant: &TenantContext) -> Result<u64, Error> {
        let before = now() - EXPIRE_AFTER;
        let expired = callback::Entity::find()
            .filter(callback::Column::Status.eq("open"))
            .filter(callback::Column::WindowEnd.lt(before))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let mut count = 0;
        for row in expired {
            let changed = callback::Entity::update_many()
                .col_expr(callback::Column::Status, Expr::value("expired"))
                .col_expr(callback::Column::OpenKey, Expr::value(Option::<String>::None))
                .col_expr(callback::Column::UpdatedAt, Expr::value(now()))
                .filter(callback::Column::Id.eq(row.id))
                .filter(callback::Column::Status.eq("open"))
                .scoped(tenant)
                .exec(tenant.database().write())
                .await?
                .rows_affected;
            if changed == 0 {
                continue;
            }
            count += 1;
            if let (Some(notifier), Some(user)) = (&self.parts.notifier, row.requester_user_id) {
                let notice = autoxide_notifications::api::Notification::new(
                    crate::notify::CALLBACK_EXPIRED.key(),
                    autoxide_notifications::api::Recipient::User(user),
                    json!({}),
                    format!("conversations.callback_expired:{}", row.id),
                )
                .link(format!("/conversations/me/threads/{}", row.thread_id));
                notifier.notify(tenant.id(), notice).await?;
            }
        }
        Ok(count)
    }
}

/// Job quét yêu cầu quá hạn của mọi khách có yêu cầu trên database của job.
pub(crate) struct ExpireCallbacks(
    pub std::sync::Arc<ConversationsService>,
    pub autoxide_kernel::tenancy::DatabaseRegistry,
);

#[async_trait::async_trait]
impl JobHandler for ExpireCallbacks {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        use sea_orm::QuerySelect;
        let tenants: Vec<Uuid> = callback::Entity::find()
            .select_only()
            .column(callback::Column::TenantId)
            .distinct()
            .filter(callback::Column::Status.eq("open"))
            .into_tuple()
            .all(job.database.read())
            .await?;
        for tenant in tenants {
            let id = autoxide_kernel::tenancy::TenantId::from_uuid(tenant);
            let context = match self.1.tenant(id).await {
                Ok(context) => context,
                Err(error) => {
                    tracing::warn!(tenant_id = %tenant, code = error.code(), "call-back sweep skipped a tenant");
                    continue;
                }
            };
            self.0.expire_callbacks(&context).await?;
        }
        Ok(())
    }
}

/// Những gì event cần biết về một yêu cầu — không số điện thoại.
pub(crate) fn callback_snapshot(row: &Callback) -> serde_json::Value {
    json!({
        "id": row.id,
        "org_id": row.org_id,
        "thread_id": row.thread_id,
        "status": row.status,
        "window_start": row.window_start,
        "window_end": row.window_end,
        "attempts": row.attempts,
    })
}

pub(crate) fn callback_not_found(id: Uuid) -> Error {
    Error::not_found(
        "conversations.callback_not_found",
        "There is no call-back request `{id}` you can see.",
    )
    .with_param("id", id)
}

pub(crate) fn callback_taken() -> Error {
    Error::new(
        ErrorKind::Conflict,
        "conversations.callback_taken",
        "Another agent is already handling this call-back request.",
    )
}

pub(crate) fn recording_notice_required() -> Error {
    Error::new(
        ErrorKind::Validation,
        "conversations.recording_notice_required",
        "Confirm you know the call is recorded (`recording_ack: true`) before calling.",
    )
}
