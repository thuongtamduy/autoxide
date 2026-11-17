//! Module `notifications` (bước 37 phần B, D-348): thông báo tới người dùng qua email, SMS, Zalo ZNS,
//! web push, đẩy di động và hộp thư trong ứng dụng.
//!
//! - Module khác khai loại thông báo ([`api::NotificationKind`]) và gửi qua [`api::Notifier`].
//! - Mỗi loại thuộc một nhóm: bảo mật (không tắt được), giao dịch, quảng cáo (cần đồng ý trong sổ của
//!   `compliance`, khung giờ và nhãn quảng cáo theo quy tắc có ngày hiệu lực, link huỷ nhận).
//! - Bên gửi của từng kênh là trait của kernel (`Mailer`, `SmsSender`, `ZnsSender`, `WebPushSender`,
//!   `MobilePushSender`) do module khác cung cấp; thiếu bên nào thì kênh đó bị bỏ qua
//!   (`channel_disabled`).
//! - Thông báo của nền tảng tới quản trị viên mọi khách (bước 35).

pub mod api;
mod branding;
mod config;
mod custom;
mod delivery;
mod entity;
mod events;
mod http_admin;
mod http_me;
mod http_templates;
mod layout;
mod me;
mod messages;
mod migration;
mod permissions;
mod platform;
mod privacy;
mod provider;
mod registry;
mod service;
mod templates;
mod tools;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventInfo, Subscription};
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::throttle::Throttle;
use utoipa_axum::router::OpenApiRouter;

use crate::config::NotificationsConfig;
use crate::registry::Kinds;
use crate::service::{Deliver, NotificationsService};

/// Số lần gửi thử mẫu mỗi người mỗi giờ.
const TEST_SENDS_PER_HOUR: u32 = 10;

pub fn module() -> impl Module {
    NotificationsModule {
        service: None,
        config: NotificationsConfig::default(),
        listen_consents: false,
        listen_users: false,
    }
}

struct NotificationsModule {
    service: Option<Arc<NotificationsService>>,
    config: NotificationsConfig,
    /// Có module khai event rút đồng ý (`compliance`) và có sổ đồng ý: nghe để huỷ tin quảng cáo.
    listen_consents: bool,
    /// Có module khai event xoá tài khoản (`auth`): nghe để xoá dữ liệu của người đó.
    listen_users: bool,
}

impl Module for NotificationsModule {
    fn navigation(&self) -> autoxide_kernel::module::navigation::Navigation {
        use autoxide_kernel::module::navigation::{NavGroup, NavItem, Navigation};
        Navigation::default().group(NavGroup::new("settings", "Settings", "gear", 110)).item(
            NavItem::new("settings.notifications", "settings", "Notification templates", 60)
                .permissions(&[crate::permissions::TEMPLATES_MANAGE]),
        )
    }

    fn name(&self) -> &'static str {
        "notifications"
    }

    // Tin bảo mật và giao dịch là nền của mọi module khác, không bán theo gói.
    fn always_enabled(&self) -> bool {
        true
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = config::read(reader)?;
        Ok(())
    }

    // Thời hạn lưu dữ liệu cá nhân (bước 37 C5, D-360), đọc từ cấu hình đang có.
    fn retention(&self) -> Vec<autoxide_kernel::privacy::RetentionPolicy> {
        use autoxide_kernel::privacy::{DataCategory, Keep, RetentionPolicy};
        let days = |value: i64| Keep::Days(u32::try_from(value).unwrap_or(u32::MAX));
        vec![
            RetentionPolicy::new(
                "notifications.send_log",
                "Address and filled-in values of sent notifications (the log row stays, blanked).",
                DataCategory::Basic,
                days(self.config.retention_days),
            )
            .setting(config::RETENTION_DAYS.name),
            RetentionPolicy::new(
                "notifications.inbox",
                "Notifications in the in-app inbox.",
                DataCategory::Basic,
                days(self.config.inbox_retention_days),
            )
            .setting(config::INBOX_RETENTION_DAYS.name),
        ]
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
        let kinds = Arc::new(Kinds::default());
        kinds.declare(registry::PLATFORM_NOTICE)?;
        kinds.declare(registry::PLATFORM_SECURITY_NOTICE)?;
        let throttle = Throttle::new(
            "notifications",
            self.config.per_recipient_per_hour,
            Duration::from_secs(3_600),
        )?;
        let service = Arc::new(NotificationsService {
            databases: context.tenancy(),
            outbox: context.outbox()?,
            jobs: context.jobs(),
            kinds,
            translations: context.translations().clone(),
            config: self.config.clone(),
            throttle,
            secret: context
                .require::<autoxide_kernel::secret::SecretKey>()
                .ok()
                .map(|key| (*key).clone()),
            users: context.require::<dyn autoxide_auth::api::Users>().ok(),
            admins: context.require::<dyn autoxide_auth::api::TenantAdmins>().ok(),
            tenants: context.require::<autoxide_kernel::tenancy::TenantAdmin>().ok(),
            consents: context.require::<dyn autoxide_compliance::api::ConsentLedger>().ok(),
            rules: context.require::<dyn autoxide_compliance::api::ComplianceRules>().ok(),
            mailer: context.require::<dyn autoxide_kernel::mail::Mailer>().ok(),
            sms: context.require::<dyn autoxide_kernel::sms::SmsSender>().ok(),
            zns: context.require::<dyn autoxide_kernel::zns::ZnsSender>().ok(),
            web_push: context.require::<dyn autoxide_kernel::push::WebPushSender>().ok(),
            mobile_push: context.require::<dyn autoxide_kernel::push::MobilePushSender>().ok(),
            templates: context
                .require::<autoxide_kernel::template::TemplateEngine>()
                .map(|engine| (*engine).clone())
                .unwrap_or_default(),
            legal: context.require::<dyn autoxide_compliance::api::LegalProfiles>().ok(),
            test_throttle: Throttle::new(
                "notifications",
                TEST_SENDS_PER_HOUR,
                Duration::from_secs(3_600),
            )?,
        });
        self.listen_consents = service.consents.is_some()
            && context.events().contains(autoxide_compliance::api::CONSENT_WITHDRAWN);
        self.listen_users = context.events().contains(autoxide_kernel::privacy::USER_DELETED);
        context.personal_data(Arc::new(privacy::NotificationsPersonalData))?;
        self.service = Some(service.clone());
        context.provide::<dyn api::Notifier>(Arc::new(provider::Provider(service)))
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, |service| {
            http_me::routes(service.clone())
                .merge(http_templates::routes(service.clone()))
                .merge(http_admin::routes(service))
        })
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        use autoxide_kernel::job::Job;
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![
                JobRegistration::new(
                    Deliver::KIND,
                    Arc::new(delivery::DeliverJob(service.clone())),
                ),
                JobRegistration::new(
                    delivery::PURGE,
                    Arc::new(delivery::PurgeJob(service.clone())),
                ),
                JobRegistration::new(
                    platform::PlatformNoticeJob::KIND,
                    Arc::new(platform::PlatformNoticeRunner(service)),
                ),
            ]
        })
    }

    fn schedules(&self) -> Vec<Schedule> {
        vec![Schedule::new(delivery::PURGE, delivery::PURGE, Duration::from_secs(86_400))]
    }

    // Người nhận rút đồng ý quảng cáo (module `compliance`): huỷ tin quảng cáo đang chờ của họ. Tài
    // khoản bị xoá (module `auth`): xoá dữ liệu của người đó.
    fn subscriptions(&self) -> Vec<Subscription> {
        let Some(service) = self.service.clone() else {
            return Vec::new();
        };
        let mut subscriptions = Vec::new();
        if self.listen_consents {
            subscriptions.push(Subscription::new(
                autoxide_compliance::api::CONSENT_WITHDRAWN,
                Arc::new(delivery::ConsentWithdrawnListener(service.clone())),
            ));
        }
        if self.listen_users {
            subscriptions.push(Subscription::new(
                autoxide_kernel::privacy::USER_DELETED,
                Arc::new(privacy::UserDeletedListener(service)),
            ));
        }
        subscriptions
    }
}
