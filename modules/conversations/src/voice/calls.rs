//! Cuộc gọi (D-232): người mua gọi cửa hàng từ trình duyệt, phiên của nhân viên, dòng thời gian,
//! webhook của nhà cung cấp (ghi nguyên văn + job áp trạng thái, không lùi trạng thái), tin hệ thống
//! tóm tắt cuộc gọi đúng một lần.

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::rows::lock_one;
use autoxide_kernel::secret::sha256;
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use autoxide_kernel::voice::{BrowserSession, BrowserSessionRequest, CallStatus, SessionRole};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use super::callbacks::recording_notice_required;
use super::lines::{Line, voice_unavailable};
use crate::entity::{call, callback, voice_event, voice_line};
use crate::events::{self, Change};
use crate::messages::NewMessage;
use crate::permissions::CALL;
use crate::service::{AuthorKind, CLOSED, ConversationsService, Thread, Viewer, audience, now};

pub(crate) type Call = call::Model;

pub(crate) const PROCESS_VOICE_EVENT: &str = "conversations.process_voice_event";

/// Câu mẫu dịch qua `text:<câu>` của `locales/vi.json`. Macro chỉ trả lại chuỗi; tên `answer_text!` là
/// dấu để `cargo xtask check` thấy câu và đòi bản dịch (như `shipping`).
macro_rules! answer_text {
    ($text:literal) => {
        $text
    };
}

/// Thông tin người mua cần trước khi gọi.
#[derive(Debug, Clone)]
pub(crate) struct VoiceInfo {
    pub available: bool,
    pub browser_calls: bool,
    pub callbacks: bool,
    pub recording: bool,
    pub recording_notice: Option<String>,
    pub line: Option<Line>,
}

/// Một thông báo đã đọc từ webhook, lưu trong `conversations_voice_events.notification`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredNotification {
    pub provider_call_id: String,
    pub call_id: Option<Uuid>,
    pub status: String,
    pub agent_identity: Option<String>,
    pub occurred_at: Timestamp,
    pub duration_seconds: Option<u32>,
    pub provider_recording_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ProcessVoiceEvent {
    pub event_id: Uuid,
}

impl Job for ProcessVoiceEvent {
    const KIND: &'static str = PROCESS_VOICE_EVENT;
}

pub(crate) struct ProcessEvents(pub std::sync::Arc<ConversationsService>);

#[async_trait::async_trait]
impl JobHandler for ProcessEvents {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let Some(tenant) = &job.tenant else {
            return Ok(());
        };
        let payload: ProcessVoiceEvent =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.process_voice_event(tenant, payload.event_id).await
    }
}

impl ConversationsService {
    /// Câu thông báo ghi âm theo ngôn ngữ `locale`.
    pub(crate) fn recording_notice(&self, locale: Option<&str>) -> String {
        let language = locale.and_then(Language::from_tag).unwrap_or(Language::Vi);
        // Câu dịch qua `text:` của module.
        self.parts.translations.describe(
            answer_text!(
                "This call is recorded to keep the quality of our service and to settle disputes."
            ),
            language,
        )
    }

    /// Thông tin gọi điện của cuộc `thread_id` cho người mua.
    pub(crate) async fn voice_info(
        &self,
        tenant: &TenantContext,
        customer: Uuid,
        thread_id: Uuid,
    ) -> Result<VoiceInfo, Error> {
        let thread = self.get(tenant, Viewer::Customer(customer), thread_id).await?;
        let line = self.line_of(tenant, thread.org_id).await?;
        let capabilities =
            line.as_ref().and_then(|line| self.provider(&line.provider)).map(|p| p.capabilities());
        let Some(capabilities) = capabilities else {
            return Ok(VoiceInfo {
                available: false,
                browser_calls: false,
                callbacks: false,
                recording: false,
                recording_notice: None,
                line: None,
            });
        };
        let recording = line.as_ref().is_some_and(|line| line.recording_enabled);
        Ok(VoiceInfo {
            available: true,
            browser_calls: capabilities.browser_calls,
            callbacks: capabilities.outbound,
            recording,
            recording_notice: recording.then(|| self.recording_notice(thread.locale.as_deref())),
            line,
        })
    }

    /// Người mua gọi cửa hàng từ trình duyệt: ghi cuộc gọi `inbound` `queued`, rồi NGOÀI transaction
    /// xin phiên WebRTC của nhà cung cấp.
    pub(crate) async fn start_call(
        &self,
        tenant: &TenantContext,
        customer: Uuid,
        thread_id: Uuid,
        recording_ack: bool,
    ) -> Result<(Call, BrowserSession), Error> {
        let thread = self.get(tenant, Viewer::Customer(customer), thread_id).await?;
        if thread.status == CLOSED {
            return Err(crate::messages::thread_closed(thread_id));
        }
        let line = self.line_of(tenant, thread.org_id).await?.ok_or_else(voice_unavailable)?;
        let provider = self.provider(&line.provider).ok_or_else(voice_unavailable)?;
        if !provider.capabilities().browser_calls {
            return Err(voice_unavailable());
        }
        if line.recording_enabled && !recording_ack {
            return Err(recording_notice_required());
        }
        self.guard_customer(tenant, customer, OrgId::from_uuid(thread.org_id)).await?;
        self.count_call(tenant, customer).await?;
        let at = now();
        let id = Uuid::now_v7();
        let row = Call {
            id,
            tenant_id: tenant.id().as_uuid(),
            org_id: thread.org_id,
            thread_id,
            callback_id: None,
            direction: "inbound".to_owned(),
            status: CallStatus::Queued.as_str().to_owned(),
            provider: line.provider.clone(),
            provider_call_id: None,
            agent_user_id: None,
            caller_user_id: Some(customer),
            started_at: Some(at),
            answered_at: None,
            ended_at: None,
            duration_seconds: None,
            recording_enabled: line.recording_enabled,
            recording_ack_at: line.recording_enabled.then_some(at),
            provider_recording_id: None,
            recording_ref: None,
            recording_expires_at: None,
            created_at: at,
            updated_at: at,
        };
        let transaction = tenant.database().begin_write().await?;
        call::Entity::insert(row.clone().into_active_model())
            .exec_without_returning(&transaction)
            .await?;
        self.record_call(&transaction, tenant, Actor::user(customer), &thread, None, &row).await?;
        transaction.commit().await?;
        let hotline = PhoneNumber::parse(&line.hotline, "84")
            .ok_or_else(|| Error::internal("the hotline is not E.164"))?;
        let request = BrowserSessionRequest {
            role: SessionRole::Customer,
            identity: customer.to_string(),
            hotline,
            call_id: Some(id),
            recording_notice: line
                .recording_enabled
                .then(|| self.recording_notice(thread.locale.as_deref())),
        };
        // Ngoài transaction ghi: gọi nhà cung cấp.
        match provider.browser_session(tenant.id(), &request).await {
            Ok(session) => Ok((row, session)),
            Err(error) => {
                self.advance(tenant, id, CallStatus::Failed, None, None, None, None, at).await?;
                Err(error)
            }
        }
    }

    /// Phiên WebRTC của nhân viên để nghe máy / gọi ra ở đường dây của nút `org`.
    pub(crate) async fn agent_session(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
    ) -> Result<BrowserSession, Error> {
        let target = OrgTarget::new(OrgId::from_uuid(org), None);
        principal.require_at(tenant.database().read(), &CALL, target).await?;
        let line = self.line_of(tenant, org).await?.ok_or_else(voice_unavailable)?;
        let provider = self.provider(&line.provider).ok_or_else(voice_unavailable)?;
        let hotline = PhoneNumber::parse(&line.hotline, "84")
            .ok_or_else(|| Error::internal("the hotline is not E.164"))?;
        let request = BrowserSessionRequest {
            role: SessionRole::Agent,
            identity: principal.user_id().to_string(),
            hotline,
            call_id: None,
            recording_notice: None,
        };
        provider.browser_session(tenant.id(), &request).await
    }

    /// Cuộc gọi `id`.
    pub(crate) async fn call(&self, tenant: &TenantContext, id: Uuid) -> Result<Call, Error> {
        call::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| call_not_found(id))
    }

    /// Cuộc gọi của cuộc trò chuyện `thread_id` (`conversations.call` ở nút của nó), mới trước.
    pub(crate) async fn calls_of(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        thread_id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<Call>, Error> {
        let thread = self.reload(tenant, thread_id).await?;
        self.require_call(tenant, principal, &thread).await?;
        // Một trang theo id (mới trước mặc định), keyset trong database (index
        // `conversations_calls_thread_page_idx`, m0009).
        let select =
            call::Entity::find().filter(call::Column::ThreadId.eq(thread_id)).scoped(tenant);
        fetch_page(select, call::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// `conversations.call` ở nút của cuộc trò chuyện; ngoài phạm vi → 404 như không có.
    pub(crate) async fn require_call(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        thread: &Thread,
    ) -> Result<(), Error> {
        let target = OrgTarget::new(OrgId::from_uuid(thread.org_id), thread.assignee_user_id);
        let result = principal.require_at(tenant.database().read(), &CALL, target).await;
        crate::service::hidden(result, thread.id)
    }

    /// Ghi event của một lần đổi cuộc gọi: `call_updated` (stream), và `call_ended` khi vừa tới trạng
    /// thái cuối. Không số điện thoại, không `recording_ref`.
    pub(crate) async fn record_call<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        actor: Actor,
        thread: &Thread,
        before: Option<&Call>,
        after: &Call,
    ) -> Result<(), Error> {
        let subject = events::thread_subject(thread.id);
        let event = events::CallUpdated(Change {
            before: before.map(call_snapshot),
            after: Some(call_snapshot(after)),
            audience: audience(thread),
        });
        self.parts.outbox.record(connection, tenant, actor, subject.clone(), &event).await?;
        let was_final = before.is_some_and(|row| is_final(&row.status));
        if is_final(&after.status) && !was_final {
            let ended = events::CallEnded(Change {
                before: None,
                after: Some(call_snapshot(after)),
                audience: Vec::new(),
            });
            self.parts.outbox.record(connection, tenant, actor, subject, &ended).await?;
        }
        Ok(())
    }

    /// Đưa cuộc gọi `call_id` tới `status` (không lùi; trạng thái cuối không đổi nữa). Tới trạng thái
    /// cuối: đúng một tin hệ thống tóm tắt, yêu cầu gọi lại đổi theo, xếp job tải bản ghi.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn advance(
        &self,
        tenant: &TenantContext,
        call_id: Uuid,
        status: CallStatus,
        provider_call_id: Option<String>,
        agent_identity: Option<String>,
        duration_seconds: Option<u32>,
        provider_recording_id: Option<String>,
        at: Timestamp,
    ) -> Result<bool, Error> {
        let transaction = tenant.database().begin_write().await?;
        let Some(before) =
            lock_one(&transaction, call::Entity::find_by_id(call_id).scoped(tenant)).await?
        else {
            transaction.rollback().await?;
            return Ok(false);
        };
        let current = CallStatus::parse(&before.status).unwrap_or(CallStatus::Queued);
        if current.is_final() || status.rank() <= current.rank() {
            transaction.rollback().await?;
            return Ok(false);
        }
        let mut row = before.clone().into_active_model();
        use sea_orm::{ActiveModelTrait, Set};
        row.status = Set(status.as_str().to_owned());
        row.updated_at = Set(now());
        if before.provider_call_id.is_none()
            && let Some(id) = provider_call_id
        {
            row.provider_call_id = Set(Some(id));
        }
        if let Some(agent) = agent_identity.as_deref().and_then(|raw| raw.parse::<Uuid>().ok())
            && before.direction == "inbound"
        {
            row.agent_user_id = Set(Some(agent));
        }
        if status == CallStatus::InProgress {
            row.answered_at = Set(Some(at));
        }
        if status.is_final() {
            row.ended_at = Set(Some(at));
            row.duration_seconds =
                Set(duration_seconds.map(|seconds| i32::try_from(seconds).unwrap_or(i32::MAX)));
            if before.recording_enabled {
                row.provider_recording_id = Set(provider_recording_id.clone());
            }
        }
        let after = row.update(&transaction).await?;
        let thread = crate::entity::thread::Entity::find_by_id(after.thread_id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| crate::service::not_found(after.thread_id))?;
        if status.is_final() {
            self.summarize(&transaction, tenant, &thread, &after).await?;
            if let Some(callback_id) = after.callback_id {
                let (next, attempt) = match status {
                    CallStatus::Completed => ("done", 0),
                    CallStatus::NoAnswer | CallStatus::Busy => ("open", 1),
                    _ => ("open", 0),
                };
                let open_key = (next == "open").then(|| after.thread_id.to_string());
                callback::Entity::update_many()
                    .col_expr(callback::Column::Status, Expr::value(next))
                    .col_expr(callback::Column::OpenKey, Expr::value(open_key))
                    .col_expr(
                        callback::Column::Attempts,
                        sea_orm::sea_query::ExprTrait::add(
                            Expr::col(callback::Column::Attempts),
                            attempt,
                        ),
                    )
                    .col_expr(callback::Column::UpdatedAt, Expr::value(now()))
                    .filter(callback::Column::Id.eq(callback_id))
                    .filter(callback::Column::Status.eq("calling"))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
            }
            if after.recording_enabled && after.provider_recording_id.is_some() {
                let job = super::recordings::FetchRecording { call_id };
                let options = EnqueueOptions::default().unique_key(format!("recording:{call_id}"));
                self.parts.jobs.enqueue(&transaction, tenant, &job, options).await?;
            }
        }
        self.record_call(&transaction, tenant, Actor::system(), &thread, Some(&before), &after)
            .await?;
        transaction.commit().await?;
        tracing::info!(call_id = %call_id, status = status.as_str(), "call status changed");
        Ok(true)
    }

    /// Tin hệ thống tóm tắt cuộc gọi (`client_id = call:<id>`: nhận lại webhook không thêm tin thứ hai).
    async fn summarize<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        thread: &Thread,
        row: &Call,
    ) -> Result<(), Error> {
        let language =
            thread.locale.as_deref().and_then(Language::from_tag).unwrap_or(Language::Vi);
        let duration = row.duration_seconds.unwrap_or(0).to_string();
        let body = self.parts.translations.describe_with(
            answer_text!("Call ({direction}): {outcome}, {duration} seconds."),
            &[("direction", &row.direction), ("outcome", &row.status), ("duration", &duration)],
            language,
        );
        let new = NewMessage {
            body: String::new(),
            client_id: format!("call:{}", row.id),
            media_ids: Vec::new(),
        };
        let at = row.ended_at.unwrap_or_else(now);
        let inserted = self
            .insert_row(
                connection,
                tenant,
                thread,
                AuthorKind::System,
                None,
                body,
                &new,
                false,
                Some(row.id),
                at,
            )
            .await?;
        if let Some(message) = inserted {
            let event = events::MessageCreated(Change {
                before: None,
                after: Some(crate::service::message_snapshot(&message)),
                audience: audience(thread),
            });
            let subject = events::thread_subject(thread.id);
            self.parts.outbox.record(connection, tenant, Actor::system(), subject, &event).await?;
        }
        Ok(())
    }

    /// Nhận webhook của nhà cung cấp `provider` cho đường dây có token `token`: kiểm chữ ký (sai →
    /// 401, không lưu), rồi MỘT transaction ghi nguyên văn từng thông báo (trùng thì bỏ) và xếp job.
    pub(crate) async fn receive_voice_webhook(
        &self,
        tenant: &TenantContext,
        provider_name: &str,
        token: &str,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<usize, Error> {
        if body.len() > self.config.voice_webhook_max_bytes {
            return Err(Error::new(
                autoxide_kernel::error::ErrorKind::PayloadTooLarge,
                "conversations.voice_webhook_too_large",
                "The notification is larger than {max} bytes.",
            )
            .with_param("max", self.config.voice_webhook_max_bytes));
        }
        let line = voice_line::Entity::find()
            .filter(voice_line::Column::LineTokenHash.eq(sha256(token)))
            .filter(voice_line::Column::Provider.eq(provider_name))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        let (Some(_line), Some(provider)) = (line, self.provider(provider_name)) else {
            return Err(voice_unavailable());
        };
        let notifications = match provider.verify_webhook(tenant.id(), headers, body) {
            Ok(notifications) => notifications,
            Err(error) => {
                let label = if self.provider(provider_name).is_some() {
                    provider.provider()
                } else {
                    "unknown"
                };
                metrics::counter!("conversations_voice_webhook_rejected_total", "provider" => label).increment(1);
                tracing::warn!(provider = label, "voice webhook rejected: bad signature");
                return Err(error);
            }
        };
        let raw = String::from_utf8_lossy(body).into_owned();
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        let mut stored = 0;
        for notification in notifications {
            let id = Uuid::now_v7();
            let parsed = StoredNotification {
                provider_call_id: notification.provider_call_id.clone(),
                call_id: notification.call_id,
                status: notification.status.as_str().to_owned(),
                agent_identity: notification.agent_identity.clone(),
                occurred_at: notification.occurred_at,
                duration_seconds: notification.duration_seconds,
                provider_recording_id: notification.provider_recording_id.clone(),
            };
            let row = voice_event::Model {
                id,
                tenant_id: tenant.id().as_uuid(),
                provider: provider_name.to_owned(),
                provider_event_id: notification.provider_event_id.clone(),
                call_id: notification.call_id,
                notification: serde_json::to_string(&parsed).map_err(Error::internal)?,
                body: raw.clone(),
                status: "received".to_owned(),
                received_at: at,
            };
            let inserted = voice_event::Entity::insert(row.into_active_model())
                .on_conflict(
                    OnConflict::columns([
                        voice_event::Column::TenantId,
                        voice_event::Column::Provider,
                        voice_event::Column::ProviderEventId,
                    ])
                    .do_nothing()
                    .to_owned(),
                )
                .exec_without_returning(&transaction)
                .await?;
            if inserted == 0 {
                continue;
            }
            stored += 1;
            let options = EnqueueOptions::default()
                .unique_key(format!("voice:{provider_name}:{}", notification.provider_event_id));
            self.parts
                .jobs
                .enqueue(&transaction, tenant, &ProcessVoiceEvent { event_id: id }, options)
                .await?;
        }
        transaction.commit().await?;
        Ok(stored)
    }

    /// Job: áp một thông báo đã lưu. Chạy lại không sao.
    pub(crate) async fn process_voice_event(
        &self,
        tenant: &TenantContext,
        event_id: Uuid,
    ) -> Result<(), Error> {
        let Some(row) = voice_event::Entity::find_by_id(event_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
        else {
            return Ok(());
        };
        if row.status == "processed" {
            return Ok(());
        }
        let parsed: StoredNotification =
            serde_json::from_str(&row.notification).map_err(Error::internal)?;
        // Ghép cuộc gọi bằng mã phía mình; không có thì bằng mã của nhà cung cấp — không bao giờ bằng số.
        let call = match parsed.call_id {
            Some(id) => {
                call::Entity::find_by_id(id).scoped(tenant).one(tenant.database().read()).await?
            }
            None => {
                call::Entity::find()
                    .filter(call::Column::Provider.eq(row.provider.as_str()))
                    .filter(call::Column::ProviderCallId.eq(parsed.provider_call_id.as_str()))
                    .scoped(tenant)
                    .one(tenant.database().read())
                    .await?
            }
        };
        let outcome = match (call, CallStatus::parse(&parsed.status)) {
            (Some(call), Some(status)) => self
                .advance(
                    tenant,
                    call.id,
                    status,
                    Some(parsed.provider_call_id.clone()),
                    parsed.agent_identity.clone(),
                    parsed.duration_seconds,
                    parsed.provider_recording_id.clone(),
                    parsed.occurred_at,
                )
                .await
                .map(|_| ()),
            _ => Ok(()),
        };
        let status = if outcome.is_ok() { "processed" } else { "failed" };
        voice_event::Entity::update_many()
            .col_expr(voice_event::Column::Status, Expr::value(status))
            .filter(voice_event::Column::Id.eq(event_id))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        outcome
    }
}

pub(crate) fn is_final(status: &str) -> bool {
    CallStatus::parse(status).is_some_and(CallStatus::is_final)
}

/// Những gì event cần biết về một cuộc gọi — không số điện thoại, không khoá bản ghi.
pub(crate) fn call_snapshot(row: &Call) -> Value {
    json!({
        "id": row.id,
        "thread_id": row.thread_id,
        "org_id": row.org_id,
        "callback_id": row.callback_id,
        "direction": row.direction,
        "status": row.status,
        "agent_user_id": row.agent_user_id,
        "duration_seconds": row.duration_seconds,
        "recording_enabled": row.recording_enabled,
        "recorded": row.recording_ref.is_some() || row.provider_recording_id.is_some(),
    })
}

pub(crate) fn call_not_found(id: Uuid) -> Error {
    Error::not_found("conversations.call_not_found", "There is no call `{id}` you can see.")
        .with_param("id", id)
}
