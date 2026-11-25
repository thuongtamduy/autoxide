//! Module `conversations`: chat chăm sóc khách (bước 61, D-221).
//!
//! Người mua nhắn tin với cửa hàng hay gian hàng trên sàn, về một sản phẩm hay một đơn; nhân viên trả
//! lời từ hộp thư chung, nhận / giao cuộc trò chuyện, chuyển thành phiếu hỗ trợ (`support`). Tin mới
//! tới gần như tức thì qua stream SSE của kernel; người không online nhận thông báo gom lô. Câu đơn
//! giản được trả lời tự động bằng `AutoResponder` của `support` (D-230). Hướng dẫn:
//! `docs/CONVERSATIONS.md`.

mod abuse;
pub mod api;
mod assist;
mod config;
mod entity;
mod events;
mod http;
mod mask;
mod messages;
mod migration;
mod notify;
mod permissions;
mod privacy;
mod relocation;
mod resource;
mod service;
mod settings;
mod threads;
mod tools;
mod voice;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventInfo, Subscription};
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::tenancy::OrgSetting;
use utoipa_axum::router::OpenApiRouter;

use crate::config::ConversationsConfig;
use crate::service::{ConversationsService, Parts};

/// Module `conversations`, để binary đăng ký trong `crates/app/src/modules.rs` — SAU `support`.
pub fn module() -> impl Module {
    ConversationsModule::default()
}

#[derive(Default)]
struct ConversationsModule {
    config: Option<ConversationsConfig>,
    /// Có sau `start`.
    service: Option<Arc<ConversationsService>>,
    /// Sổ database: job theo lịch tra ngữ cảnh từng khách.
    databases: Option<autoxide_kernel::tenancy::DatabaseRegistry>,
}

impl Module for ConversationsModule {
    fn name(&self) -> &'static str {
        "conversations"
    }

    // Thử nghiệm (bước 70 P3): API và bảng còn có thể đổi; chỉ chạy khi `AUTOXIDE_MODULES` nêu tên.
    fn stability(&self) -> autoxide_kernel::module::Stability {
        autoxide_kernel::module::Stability::Experimental
    }

    fn description(&self) -> &'static str {
        "Customer chat with a shop or a marketplace store: shared inbox, assignment, read cursors, \
         real-time events, notices for people offline, hand-over to support tickets and automatic \
         answers from approved replies."
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn permissions(&self) -> Vec<Permission> {
        permissions::all()
    }

    fn events(&self) -> Vec<EventInfo> {
        events::all()
    }

    fn org_settings(&self) -> Vec<OrgSetting> {
        settings::all()
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(config::read(reader)?);
        Ok(())
    }

    // Thời hạn lưu dữ liệu (bước 37, D-360), đọc từ cấu hình đang có.
    fn retention(&self) -> Vec<autoxide_kernel::privacy::RetentionPolicy> {
        use autoxide_kernel::privacy::{DataCategory, Keep, RetentionPolicy};
        let Some(config) = &self.config else {
            return Vec::new();
        };
        vec![
            RetentionPolicy::new(
                "conversations.messages",
                "Chat conversations and their messages, kept this many days after the last \
                 message; the customer's own messages are erased when their account is deleted.",
                DataCategory::Basic,
                Keep::Days(config.retention_days),
            )
            .setting(config::RETENTION_DAYS.name),
            RetentionPolicy::new(
                "conversations.recordings",
                "Call recordings (off unless the shop switches them on), kept this many days after \
                 the call; deleted on request and with the customer's account.",
                DataCategory::Sensitive,
                Keep::Days(config.recording_retention_days),
            )
            .setting(config::RECORDING_RETENTION_DAYS.name),
            RetentionPolicy::new(
                "conversations.callback_requests",
                "Call-back requests with the customer's verified phone number (encrypted); the \
                 number is erased with the account, the request goes with its conversation.",
                DataCategory::Basic,
                Keep::Days(config.retention_days),
            )
            .setting(config::RETENTION_DAYS.name),
        ]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self.config.clone().ok_or_else(|| {
            Error::internal("module `conversations` started before it was configured")
        })?;
        // `organizations`, `notifications`, `media`, `support`, `ai` đăng ký trước nhưng không bắt
        // buộc: thiếu thì mặc định chính sách, không gửi thông báo, không nhận ảnh, không chuyển phiếu
        // / trả lời tự động, không soạn nháp.
        let notifier = context.require::<dyn autoxide_notifications::api::Notifier>().ok();
        if let Some(notifier) = &notifier {
            notify::declare(notifier.as_ref())?;
        }
        let parts = Parts {
            outbox: context.outbox()?,
            jobs: context.jobs(),
            settings: settings::Settings {
                orgs: context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok(),
            },
            verification: context.posting_verification(),
            notifier,
            media: context.require::<dyn autoxide_media::api::Media>().ok(),
            intake: context.require::<dyn autoxide_support::api::SupportIntake>().ok(),
            responder: context.require::<dyn autoxide_support::api::AutoResponder>().ok(),
            ai: context.require::<dyn autoxide_kernel::ai::TextModel>().ok(),
            authenticator: context.require::<dyn autoxide_kernel::access::Authenticator>().ok(),
            resources: Some(context.resources()),
            // Bộ nối nhà cung cấp thoại đăng ký TRƯỚC module này (D-232); không có thì không gọi điện.
            voice: context.contributions::<dyn autoxide_kernel::voice::VoiceProvider>(),
            accounts: context.require::<dyn autoxide_compliance::api::AccountVerification>().ok(),
            storage: context.require::<dyn autoxide_kernel::storage::Storage>().ok(),
            secret: context
                .require::<autoxide_kernel::secret::SecretKey>()
                .ok()
                .map(|key| (*key).clone()),
            translations: context.translations().clone(),
        };
        let service = Arc::new(ConversationsService::new(parts, config));
        context.personal_data(service.clone())?;
        // Cuộc trò chuyện là một loại đối tượng (D-123): phiếu hỗ trợ chuyển từ chat trỏ về nó.
        context.resource_type("conversations.thread", Arc::new(resource::ThreadResolver))?;
        // Hộp việc (bước 51): yêu cầu gọi lại đang mở.
        context.work_source(Arc::new(voice::work::Callbacks))?;
        self.databases = Some(context.tenancy());
        self.service = Some(service);
        Ok(())
    }

    // Chế độ nghỉ (bước 70 P9): module tắt vẫn xuất cuộc trò chuyện của người dùng và ẩn danh hoá khi
    // tài khoản bị xoá — cùng hàm với lúc bật, không cần service (xem `privacy.rs`). Bản ghi âm xoá qua kho
    // dùng chung và bộ nối thoại đang bật; thiếu thì báo lỗi để event được thử lại.
    fn dormant(
        &self,
        context: &mut autoxide_kernel::module::DormantContext<'_>,
    ) -> Result<(), Error> {
        let part = Arc::new(privacy::DormantPrivacy {
            tenancy: context.tenancy(),
            voice: context.contributions::<dyn autoxide_kernel::voice::VoiceProvider>(),
        });
        context.personal_data(part.clone())?;
        context.on_user_deleted(part)
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        let Some(service) = self.service.clone() else {
            return Vec::new();
        };
        // Tài khoản bị xoá: ẩn danh hoá phía người mua (`privacy.rs`).
        vec![Subscription::new(autoxide_kernel::privacy::USER_DELETED, service)]
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        let (Some(service), Some(databases)) = (self.service.clone(), self.databases.clone())
        else {
            return Vec::new();
        };
        use voice::{callbacks, calls, recordings};
        vec![
            JobRegistration::new(
                assist::AUTO_REPLY,
                Arc::new(assist::AutoReplies(service.clone())),
            ),
            JobRegistration::new(notify::NOTIFY_UNREAD, Arc::new(notify::Unread(service.clone()))),
            JobRegistration::new(privacy::PURGE, Arc::new(privacy::Purge(service.clone()))),
            JobRegistration::new(
                calls::PROCESS_VOICE_EVENT,
                Arc::new(calls::ProcessEvents(service.clone())),
            ),
            JobRegistration::new(
                recordings::FETCH_RECORDING,
                Arc::new(recordings::Fetcher(service.clone())),
            ),
            JobRegistration::new(
                recordings::PURGE_RECORDINGS,
                Arc::new(recordings::Purger(service.clone(), databases.clone())),
            ),
            JobRegistration::new(
                callbacks::EXPIRE_CALLBACKS,
                Arc::new(callbacks::ExpireCallbacks(service, databases)),
            ),
        ]
    }

    fn schedules(&self) -> Vec<Schedule> {
        use voice::{callbacks, recordings};
        let day = Duration::from_secs(24 * 3600);
        vec![
            // Mỗi ngày: xoá cuộc trò chuyện quá hạn lưu (hạn tính theo ngày).
            Schedule::new(privacy::PURGE, privacy::PURGE, day),
            // Mỗi ngày: xoá bản ghi âm quá hạn lưu.
            Schedule::new(recordings::PURGE_RECORDINGS, recordings::PURGE_RECORDINGS, day),
            // Mỗi 15 phút: yêu cầu gọi lại quá hạn.
            Schedule::new(
                callbacks::EXPIRE_CALLBACKS,
                callbacks::EXPIRE_CALLBACKS,
                Duration::from_secs(15 * 60),
            ),
        ]
    }

    fn tools(&self) -> Vec<autoxide_kernel::mcp::Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn body_limits(&self) -> Vec<autoxide_kernel::http::BodyLimit> {
        let max = self.config.as_ref().map_or(65536, |config| config.voice_webhook_max_bytes);
        vec![autoxide_kernel::http::BodyLimit::new(
            axum::http::Method::POST,
            "/voice/webhooks/{provider}/{line_token}",
            max,
        )]
    }

    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        relocation::relocation()
    }
}
