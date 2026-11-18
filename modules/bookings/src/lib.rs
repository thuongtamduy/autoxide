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

pub mod api;
mod availability;
mod entity;
mod events;
mod http_public;
mod http_setup;
mod migration;
mod openings;
mod permissions;
mod service;
mod settings;
mod setup;

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

    fn description(&self) -> &'static str {
        "Bookable services with staff, rooms and equipment, weekly schedules, public availability, \
         bookings that never overlap, deposits through a licensed partner, cancellation policies \
         and reminders."
    }

    fn navigation(&self) -> autoxide_kernel::module::navigation::Navigation {
        use autoxide_kernel::module::navigation::{NavGroup, NavItem, Navigation};
        Navigation::default().group(NavGroup::new("bookings", "Bookings", "calendar", 45)).item(
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
        });
        self.service = Some(service);
        Ok(())
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, |service| {
            http_setup::routes(service.clone()).merge(http_public::routes(service))
        })
    }
}
