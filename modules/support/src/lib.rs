//! Module `support`: phiếu hỗ trợ khách hàng (bước 53, D-204).
//!
//! Người mua (có tài khoản hay vãng lai) gửi yêu cầu; nhân viên của nút (cửa hàng, gian hàng) nhận,
//! trả lời, theo dõi tới khi xong. Khiếu nại có hạn luật định ở `compliance`, tranh chấp ở
//! `marketplace`: `support` chỉ chuyển phiếu sang đó, không làm lại. Hướng dẫn: `docs/SUPPORT.md`.

pub mod api;
mod config;
mod entity;
mod events;
mod guest;
mod http;
mod intake;
mod messages;
mod migration;
mod notify;
mod permissions;
mod privacy;
mod service;
mod status;
mod tickets;

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
}

impl Module for SupportModule {
    fn name(&self) -> &'static str {
        "support"
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
        };
        let service = Arc::new(SupportService::new(parts, config));
        context.personal_data(service.clone())?;
        context.provide::<dyn api::SupportIntake>(service.clone())?;
        self.service = Some(service);
        Ok(())
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

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        relocation::relocation()
    }
}
