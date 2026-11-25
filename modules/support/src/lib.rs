//! Module `support`: phiếu hỗ trợ khách hàng (bước 53, D-204).
//!
//! Người mua (có tài khoản hay vãng lai) gửi yêu cầu; nhân viên của nút (cửa hàng, gian hàng) nhận,
//! trả lời, theo dõi tới khi xong. Khiếu nại có hạn luật định ở `compliance`, tranh chấp ở
//! `marketplace`: `support` chỉ chuyển phiếu sang đó, không làm lại. Hướng dẫn: `docs/SUPPORT.md`.

mod ai;
pub mod api;
mod autoreply;
mod autoreply_admin;
mod config;
mod entity;
mod events;
mod extras;
mod facts;
mod guest;
mod hours;
mod http;
mod inbound;
mod intake;
mod messages;
mod migration;
mod notify;
mod permissions;
mod policy;
mod privacy;
mod report;
mod service;
mod sla;
mod status;
mod teams;
mod tickets;
mod tools;

mod relocation;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::antispam::SpamGuard;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Event, EventInfo, Subscription};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::secret::SecretKey;
use utoipa_axum::router::OpenApiRouter;

use crate::config::SupportConfig;
use crate::service::{Parts, SupportService};

/// Module `support`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    SupportModule::default()
}

#[derive(Default)]
struct SupportModule {
    config: Option<SupportConfig>,
    /// Có sau `start`.
    service: Option<Arc<SupportService>>,
    /// Sổ database: job theo lịch tra ngữ cảnh từng khách có phiếu trên database của job.
    databases: Option<autoxide_kernel::tenancy::DatabaseRegistry>,
}

impl Module for SupportModule {
    fn name(&self) -> &'static str {
        "support"
    }

    // Thử nghiệm (bước 70 P3): API và bảng còn có thể đổi; chỉ chạy khi `AUTOXIDE_MODULES` nêu tên.
    fn stability(&self) -> autoxide_kernel::module::Stability {
        autoxide_kernel::module::Stability::Experimental
    }

    fn description(&self) -> &'static str {
        "Customer support tickets: messages, internal notes, guest access by ticket token, staff \
         scope by organization unit, and hand-over to consumer complaints."
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

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(config::read(reader)?);
        Ok(())
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `support` started before it was configured"))?;
        // `notifications`, `media`, `compliance` đăng ký trước nhưng không bắt buộc: thiếu thì
        // không gửi mã / thông báo, không nhận tệp đính kèm, không gộp phiếu theo số đã xác thực.
        let notifier = context.require::<dyn autoxide_notifications::api::Notifier>().ok();
        if let Some(notifier) = &notifier {
            notify::declare(notifier.as_ref())?;
        }
        let parts = Parts {
            outbox: context.outbox()?,
            guard: context.require::<SpamGuard>()?,
            secret: (*context.require::<SecretKey>()?).clone(),
            notifier,
            media: context.require::<dyn autoxide_media::api::Media>().ok(),
            accounts: context.require::<dyn autoxide_compliance::api::AccountVerification>().ok(),
            // Hỏi quyền của người được giao (`principal_of`); không có thì không tự giao cho ai.
            authenticator: context.require::<dyn autoxide_kernel::access::Authenticator>().ok(),
            resources: Some(context.resources()),
            complaints: context.require::<dyn autoxide_compliance::api::ComplaintIntake>().ok(),
            ai: context.require::<dyn autoxide_kernel::ai::TextModel>().ok(),
            facts: facts::FactRegistry::default(),
            jobs: context.jobs(),
        };
        let service = Arc::new(SupportService::new(parts, config));
        context.personal_data(service.clone())?;
        context.provide::<dyn api::SupportIntake>(service.clone())?;
        // Email trả lời vào lại phiếu (module nhận thư gọi vào; tắt theo chính sách).
        context.provide::<dyn api::InboundMail>(service.clone())?;
        // Điểm cắm cho module đăng ký sau (`shipping`, `orders`): nguồn câu trả lời từ dữ liệu (D-230).
        context.provide::<dyn api::SupportFacts>(Arc::new(service.parts.facts.clone()))?;
        // Bộ trả lời tự động cho chat (bước 61).
        context.provide::<dyn api::AutoResponder>(service.clone())?;
        // Phiếu là một loại đối tượng (D-123): vụ việc khiếu nại của `compliance` trỏ tới nó.
        context.resource_type("support.ticket", Arc::new(intake::TicketResolver))?;
        // Hộp việc (bước 51): phiếu đang chờ nhân viên.
        context.work_source(Arc::new(sla::Waiting))?;
        self.databases = Some(context.tenancy());
        self.service = Some(service);
        Ok(())
    }

    // Chế độ nghỉ (bước 70 P9): module tắt vẫn xuất phiếu của người dùng và ẩn danh hoá khi tài khoản bị
    // xoá — cùng hàm với lúc bật, không cần service (xem `privacy.rs`).
    fn dormant(
        &self,
        context: &mut autoxide_kernel::module::DormantContext<'_>,
    ) -> Result<(), Error> {
        context.personal_data(Arc::new(privacy::DormantPrivacy))?;
        context.on_user_deleted(Arc::new(privacy::DormantPrivacy))
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        let Some(service) = self.service.clone() else {
            return Vec::new();
        };
        vec![
            // Tài khoản bị xoá: ẩn danh hoá người hỏi, giữ phiếu cho thống kê (`privacy.rs`).
            Subscription::new(autoxide_kernel::privacy::USER_DELETED, service.clone()),
            // Tin mới: báo người mua / người được giao / người được nhắc tên (`notify.rs`).
            Subscription::new(
                events::MessageAdded::NAME,
                Arc::new(notify::MessageNotices(service)),
            ),
        ]
    }

    fn jobs(&self) -> Vec<autoxide_kernel::job::JobRegistration> {
        use autoxide_kernel::job::JobRegistration;
        let (Some(service), Some(databases)) = (self.service.clone(), self.databases.clone())
        else {
            return Vec::new();
        };
        [sla::SLA_CHECK, sla::AUTO_CLOSE]
            .into_iter()
            .map(|kind| {
                let sweep =
                    sla::Sweep { service: service.clone(), databases: databases.clone(), kind };
                JobRegistration::new(kind, Arc::new(sweep))
            })
            .chain(std::iter::once(JobRegistration::new(
                autoreply::AUTO_REPLY,
                Arc::new(autoreply::Runner(service.clone())),
            )))
            .collect()
    }

    fn schedules(&self) -> Vec<autoxide_kernel::job::Schedule> {
        use autoxide_kernel::job::Schedule;
        use std::time::Duration;
        vec![
            // Mỗi phút: đánh dấu phiếu quá hạn.
            Schedule::new(sla::SLA_CHECK, sla::SLA_CHECK, Duration::from_secs(60)),
            // Mỗi giờ: đóng phiếu đã xử lý quá số ngày tự đóng (hạn tính theo ngày, một giờ là đủ).
            Schedule::new(sla::AUTO_CLOSE, sla::AUTO_CLOSE, Duration::from_secs(3600)),
        ]
    }

    fn tools(&self) -> Vec<autoxide_kernel::mcp::Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        relocation::relocation()
    }
}
