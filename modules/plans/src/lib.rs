//! Module `plans` (bước 35, D-185, D-323): bán phần mềm theo gói mà không sửa code. Gói là dữ liệu
//! (module + tính năng + hạn mức + giá theo kỳ), có gói thêm và ghi đè riêng cho từng khách; module
//! công bố quyền dùng gộp lại của mỗi khách cho kernel (`dyn Entitlements`), kernel cưỡng chế.
//!
//! Không có gói nào (hay không có gói mặc định) thì khách không thuê bao không bị giới hạn: cài
//! module này vào bản một khách không đổi gì.

// Bản release tính bố cục (layout) của future `async` lồng nhau sâu ở job vòng đời thuê bao vượt mức
// mặc định 128 của trình biên dịch ("queries overflow the depth limit"); bản debug không gặp. Nâng mức
// theo đúng gợi ý của trình biên dịch — chỉ ảnh hưởng lúc biên dịch, không đổi code sinh ra.
#![recursion_limit = "256"]

pub mod api;
mod billing;
mod billing_profile;
mod config;
mod entity;
mod events;
mod http;
mod http_lifecycle;
mod jobs;
mod lifecycle;
mod merge;
mod migration;
mod notices;
mod permissions;
mod provider;
mod provisioning;
mod service;
mod signup;
mod signup_http;
mod tools;
mod usage;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::EventInfo;
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::config::PlansConfig;
use crate::service::{Cache, PlansService};

pub fn module() -> impl Module {
    PlansModule::default()
}

#[derive(Default)]
struct PlansModule {
    config: Option<PlansConfig>,
    service: Option<Arc<PlansService>>,
}

impl Module for PlansModule {
    fn name(&self) -> &'static str {
        "plans"
    }

    // Gói không tự tắt được chính module quản lý gói.
    fn always_enabled(&self) -> bool {
        true
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn control_plane_tables(&self) -> Vec<&'static str> {
        vec![
            "plans_plan",
            "plans_plan_module",
            "plans_plan_feature",
            "plans_plan_limit",
            "plans_price",
            "plans_subscription",
            "plans_subscription_addon",
            "plans_override",
            "plans_settings",
            "plans_usage",
            "plans_reservation",
            "plans_usage_record",
            "plans_billing_period",
            "plans_billing_profile",
            "plans_provisioning",
            "plans_signup",
        ]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(PlansConfig::read(reader)?);
        Ok(())
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

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `plans` started before `configure`"))?;
        let service = Arc::new(PlansService {
            databases: context.tenancy(),
            outbox: context.outbox()?,
            entitlements: context.entitlements(),
            cache: Cache::new(config.cache),
            hold: config.hold,
            tenants: context.require::<autoxide_kernel::tenancy::TenantAdmin>().ok(),
            mailer: context.require::<dyn autoxide_kernel::mail::Mailer>().ok(),
            translations: context.translations().clone(),
            reminder_days: config.reminder_days.clone(),
            exports: context.require::<autoxide_kernel::tenancy::TenantExports>().ok(),
            units: context.require::<dyn autoxide_address::api::AdministrativeUnits>().ok(),
            jobs: context.jobs(),
            invitations: context.require::<dyn autoxide_auth::api::Invitations>().ok(),
            invoices: context.require::<dyn autoxide_invoices::api::Invoices>().ok(),
            ledger: context.require::<dyn autoxide_ledger::api::Ledger>().ok(),
            tax_category: config.tax_category.clone(),
            notifier: context.require::<dyn autoxide_notifications::api::Notifier>().ok(),
            kyc: context.require::<dyn autoxide_compliance::api::KycGate>().ok(),
            acceptance: context.require::<dyn autoxide_compliance::api::AcceptanceGate>().ok(),
            guard: context.require::<autoxide_kernel::antispam::SpamGuard>()?,
            signup: config.signup.clone(),
        });
        if let Some(notifier) = &service.notifier {
            notices::declare(notifier.as_ref())?;
        }
        self.service = Some(service.clone());
        context.provide::<dyn api::Provisioning>(Arc::new(provisioning::Provisioner(
            service.clone(),
        )))?;
        context.provide::<dyn autoxide_kernel::entitlement::Entitlements>(Arc::new(
            provider::Provider(service),
        ))
    }

    fn jobs(&self) -> Vec<autoxide_kernel::job::JobRegistration> {
        self.service.as_ref().map_or_else(Vec::new, jobs::registrations)
    }

    fn schedules(&self) -> Vec<autoxide_kernel::job::Schedule> {
        jobs::schedules()
    }

    // Tiền thuê bao về (module `ledger`): đánh dấu kỳ đã trả. Thuê bao đổi trạng thái: báo quản trị
    // viên khách (module `notifications`). Định danh của khách tự đăng ký xong (module `compliance`).
    fn subscriptions(&self) -> Vec<autoxide_kernel::event::Subscription> {
        // Chỉ khi có module tương ứng (event chưa ai khai thì kernel từ chối đăng ký nghe).
        let Some(service) = self.service.clone() else {
            return Vec::new();
        };
        let mut subscriptions = Vec::new();
        if service.ledger.is_some() {
            subscriptions.push(autoxide_kernel::event::Subscription::new(
                "ledger.payment_succeeded",
                Arc::new(billing::PaymentListener(service.clone())),
            ));
        }
        // Thuê bao quá hạn / tạm khoá: báo quản trị viên khách (module `notifications`).
        if service.notifier.is_some() {
            subscriptions.push(autoxide_kernel::event::Subscription::new(
                "plans.subscription_status_changed",
                Arc::new(notices::StatusNotices(service.clone())),
            ));
        }
        // Định danh của khách tự đăng ký xong (module `compliance`): cấp phát hay từ chối.
        if service.kyc.is_some() {
            for event in
                [autoxide_compliance::api::KYC_VERIFIED, autoxide_compliance::api::KYC_REJECTED]
            {
                subscriptions.push(autoxide_kernel::event::Subscription::new(
                    event,
                    Arc::new(signup::KycListener(service.clone())),
                ));
            }
        }
        subscriptions
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }
}
