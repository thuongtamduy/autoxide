//! Module `customers`: khách hàng và giữ chân khách (bước 47, D-194, D-222, D-226).
//!
//! - Hồ sơ bán hàng của từng người mua trong khách: theo tài khoản người mua và / hoặc số điện
//!   thoại (khoá HMAC theo khách, như `orders`) — khách quen ở quầy không cần tài khoản.
//! - Lịch sử mua dựng từ event của `orders`, tổng chi, số đơn.
//! - Liên hệ đã che cho nhân viên; xem đủ cần permission riêng và ghi audit (D-226).
//! - Nhóm khách tay hay theo quy tắc, cắm vào `orders` cho bảng giá và khuyến mại (D-222).
//!
//! Đăng ký SAU `orders` (nghe event của nó, cắm vào `dyn OrderExtensions`). Đặc tả:
//! `docs/steps/47-customers.md`.

pub mod api;
mod config;
mod engagement;
mod entity;
mod errors;
mod events;
mod facts;
mod http;
mod jobs;
mod listing;
mod loyalty;
mod membership;
mod migration;
mod permissions;
mod privacy;
mod profiles;
mod referrals;
mod relocation;
mod resource;
mod segments;
mod service;
mod tools;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventInfo, Subscription};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::service::{CUSTOMER, CustomersService};

/// Module `customers`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    CustomersModule::default()
}

#[derive(Default)]
struct CustomersModule {
    config: Option<config::CustomersConfig>,
    /// Có sau `start`.
    service: Option<Arc<CustomersService>>,
}

impl Module for CustomersModule {
    fn name(&self) -> &'static str {
        "customers"
    }

    fn description(&self) -> &'static str {
        "Customer profiles keyed by buyer account or phone number, purchase history from orders, \
         masked contact details with audited reveal, and manual or rule-based customer groups for \
         price lists and promotions."
    }

    fn navigation(&self) -> autoxide_kernel::module::navigation::Navigation {
        use autoxide_kernel::module::navigation::{NavGroup, NavItem, Navigation};
        Navigation::default()
            .group(NavGroup::new("customers", "Customers", "users", 60))
            .item(
                NavItem::new("customers.list", "customers", "Customers", 20)
                    .permissions(&[permissions::READ]),
            )
            .item(
                NavItem::new("customers.groups", "customers", "Customer groups", 30)
                    .permissions(&[permissions::SEGMENTS_MANAGE]),
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

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn custom_field_targets(&self) -> Vec<autoxide_kernel::custom_fields::CustomFieldTarget> {
        // Trường tự định nghĩa của hồ sơ khách (bước 51 P19, D-237): nội bộ, nhân viên điền.
        vec![autoxide_kernel::custom_fields::CustomFieldTarget::new(
            CUSTOMER,
            permissions::READ,
            permissions::MANAGE,
        )]
    }

    fn configure(
        &mut self,
        reader: &mut autoxide_kernel::config::ConfigReader<'_>,
    ) -> Result<(), autoxide_kernel::config::Invalid> {
        self.config = Some(config::read(reader)?);
        Ok(())
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let notifier = context.require::<dyn autoxide_notifications::api::Notifier>().ok();
        if let Some(notifier) = &notifier {
            notifier.declare(engagement::CART_REMINDER)?;
        }
        let service = Arc::new(CustomersService {
            databases: context.tenancy(),
            outbox: context.outbox()?,
            jobs: context.jobs(),
            secret: (*context.require::<autoxide_kernel::secret::SecretKey>()?).clone(),
            // `compliance` đăng ký trước; thiếu thì không gắn hồ sơ theo số đã xác thực.
            accounts: context.require::<dyn autoxide_compliance::api::AccountVerification>().ok(),
            custom_fields: context.custom_fields(),
            ledger: context.require::<dyn autoxide_ledger::api::Ledger>().ok(),
            promotions: context.require::<dyn autoxide_promotions::api::Promotions>().ok(),
            consents: context.require::<dyn autoxide_compliance::api::ConsentLedger>().ok(),
            notifier,
            config: self.config.clone().unwrap_or_default(),
        });
        context.personal_data(service.clone())?;
        // Hồ sơ là một loại đối tượng: nhãn và ghi chú nội bộ của bước 51 gắn lên nó (D-237).
        context.resource_type(CUSTOMER, Arc::new(resource::CustomerResolver))?;
        context.taggable(CUSTOMER, permissions::MANAGE)?;
        let provider = Arc::new(membership::Provider(service.clone()));
        context.provide::<dyn api::Customers>(provider.clone())?;
        // `orders` đăng ký trước nên không `require` được `customers`: tự cắm nhóm khách vào điểm cắm
        // của nó. Không có `orders` (vd test) thì bỏ qua.
        if let Ok(extensions) = context.require::<dyn autoxide_orders::api::OrderExtensions>() {
            extensions.set_buyer_groups(provider)?;
            // Điểm thưởng là một cách trả tiền của đơn (D-222).
            extensions.add_tender(Arc::new(loyalty::tender::PointsTender(service.clone())))?;
        }
        self.service = Some(service);
        Ok(())
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        let Some(service) = self.service.clone() else {
            return Vec::new();
        };
        use autoxide_orders::api::events as orders;
        let handler = Arc::new(facts::OrderEvents(service.clone()));
        let holds = Arc::new(loyalty::tender::OrderHolds(service.clone()));
        let engagement = Arc::new(engagement::EngagementEvents(service));
        vec![
            Subscription::new(
                autoxide_kernel::privacy::USER_DELETED,
                Arc::new(privacy::ErasedUsers),
            ),
            Subscription::optional(orders::ORDER_PLACED, handler.clone()),
            Subscription::optional(orders::ORDER_STATUS_CHANGED, handler.clone()),
            Subscription::optional(orders::REFUND_COMPLETED, handler),
            Subscription::optional(orders::ORDER_CONFIRMED, holds.clone()),
            Subscription::optional(orders::ORDER_CANCELLED, holds),
            Subscription::optional(orders::CART_ABANDONED, engagement.clone()),
            Subscription::optional(autoxide_compliance::api::CONSENT_WITHDRAWN, engagement),
        ]
    }

    fn jobs(&self) -> Vec<autoxide_kernel::job::JobRegistration> {
        self.service.as_ref().map_or_else(Vec::new, |service| {
            let mut all = membership::registrations(service);
            all.extend(jobs::registrations(service));
            all
        })
    }

    fn schedules(&self) -> Vec<autoxide_kernel::job::Schedule> {
        let mut all = membership::schedules();
        all.extend(jobs::schedules());
        all
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
