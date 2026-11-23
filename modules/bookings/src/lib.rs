//! Module `bookings`: đặt lịch dịch vụ (bước 60, D-221, D-20261118-step60-bookings) — spa, salon,
//! phòng khám, studio, lớp học, sân thể thao.
//!
//! - Dịch vụ (thời lượng, đệm trước / sau, giá, cọc, sức chứa), nhân viên / phòng / thiết bị, lịch
//!   tuần theo giờ địa phương của nút, ngày nghỉ và ca thêm.
//! - Giờ trống công khai: hàm thuần [`availability`] (lịch − ngoại lệ − đã đặt − đệm).
//!
//! Đăng ký SAU `organizations` (múi giờ, chính sách theo nút qua `dyn OrgDirectory`), `ledger` (cọc
//! qua `dyn Ledger`) và `notifications` (nhắc lịch qua `dyn Notifier`); cả ba là tuỳ chọn.
//! Không cần module bán hàng (41–49).

mod actions;
pub mod api;
mod availability;
mod booking;
mod entity;
mod events;
mod guests;
mod http_bookings;
mod http_customer;
mod http_me;
mod http_public;
mod http_setup;
mod jobs;
mod listing;
mod migration;
mod openings;
mod permissions;
mod privacy;
mod reminders;
mod service;
mod settings;
mod setup;
mod tools;

mod relocation;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::EventInfo;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::tenancy::OrgSetting;
use utoipa_axum::router::OpenApiRouter;

use crate::service::BookingsService;
use crate::settings::Settings;

/// Module `bookings`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    BookingsModule::default()
}

#[derive(Default)]
struct BookingsModule {
    /// Có sau `start`.
    service: Option<Arc<BookingsService>>,
}

impl Module for BookingsModule {
    fn name(&self) -> &'static str {
        "bookings"
    }

    // Thử nghiệm (bước 70 P3): API và bảng còn có thể đổi; chỉ chạy khi `AUTOXIDE_MODULES` nêu tên.
    fn stability(&self) -> autoxide_kernel::module::Stability {
        autoxide_kernel::module::Stability::Experimental
    }

    fn description(&self) -> &'static str {
        "Bookable services with staff, rooms and equipment, weekly schedules, public availability, \
         bookings that never overlap, deposits through a licensed partner, cancellation policies \
         and reminders."
    }

    fn navigation(&self) -> autoxide_kernel::module::navigation::Navigation {
        use autoxide_kernel::module::navigation::{NavGroup, NavItem, Navigation};
        Navigation::default()
            .group(NavGroup::new("bookings", "Bookings", "calendar", 45))
            .item(NavItem::new("bookings.calendar", "bookings", "Appointments", 10).permissions(&[
                permissions::BOOKINGS_MANAGE,
                permissions::BOOKINGS_READ,
                permissions::BOOKINGS_STAFF,
            ]))
            .item(
                NavItem::new("bookings.services", "bookings", "Services and staff", 20)
                    .permissions(&[permissions::SERVICES_MANAGE]),
            )
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

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let service = Arc::new(BookingsService {
            outbox: context.outbox()?,
            settings: Settings {
                orgs: context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok(),
            },
            jobs: context.jobs(),
            databases: context.tenancy(),
            ledger: context.require::<dyn autoxide_ledger::api::Ledger>().ok(),
            notifier: context.require::<dyn autoxide_notifications::api::Notifier>().ok(),
            sms: context.require::<dyn autoxide_kernel::sms::SmsSender>().ok(),
            secret: context
                .require::<autoxide_kernel::secret::SecretKey>()
                .ok()
                .map(|key| (*key).clone()),
        });
        if let Some(notifier) = &service.notifier {
            reminders::declare(notifier.as_ref())?;
        }
        self.service = Some(service);
        // Lịch đặt bằng tài khoản: vào bản xuất dữ liệu cá nhân (rà soát bảo mật 2026-11).
        context.personal_data(Arc::new(privacy::BookingPrivacy))?;
        Ok(())
    }

    fn jobs(&self) -> Vec<autoxide_kernel::job::JobRegistration> {
        self.service.as_ref().map_or_else(Vec::new, |service| {
            let mut all = jobs::registrations(service);
            all.extend(reminders::registrations(service));
            all
        })
    }

    fn schedules(&self) -> Vec<autoxide_kernel::job::Schedule> {
        let mut all = jobs::schedules();
        all.extend(reminders::schedules());
        all
    }

    fn subscriptions(&self) -> Vec<autoxide_kernel::event::Subscription> {
        // Cọc đã thu; `ledger` có thể không được cài (`optional`).
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![
                autoxide_kernel::event::Subscription::optional(
                    "ledger.payment_succeeded",
                    Arc::new(jobs::PaymentListener(service)),
                ),
                // Tài khoản bị xoá: bỏ thông tin cá nhân khỏi lịch của người đó.
                autoxide_kernel::event::Subscription::new(
                    autoxide_kernel::privacy::USER_DELETED,
                    Arc::new(privacy::BookingPrivacy),
                ),
            ]
        })
    }

    fn tools(&self) -> Vec<autoxide_kernel::mcp::Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, |service| {
            http_setup::routes(service.clone())
                .merge(http_bookings::routes(service.clone()))
                .merge(http_customer::routes(service.clone()))
                .merge(http_me::routes(service.clone()))
                .merge(http_public::routes(service))
        })
    }

    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        relocation::relocation()
    }
}
