//! Module `audit`: audit log chỉ thêm — ai làm gì, với bản ghi nào, lúc nào, trước và
//! sau ra sao.
//!
//! Không module nào gọi audit trực tiếp: mọi module phát event về thay đổi của mình
//! (`autoxide_kernel::event`), audit đăng ký nhận MỌI event và ghi mỗi event thành
//! một dòng. Thêm module mới không phải sửa audit (D-055). Các dòng nối thành chuỗi băm;
//! `autoxide doctor` báo dòng đầu tiên bị sửa (D-183).

pub mod api;
mod chain;
mod entity;
mod http;
mod migration;
mod recorder;
mod service;
mod tools;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Subscription;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};

use crate::recorder::Recorder;
use crate::service::AuditService;
use utoipa_axum::router::OpenApiRouter;

/// Module `audit`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    AuditModule::default()
}

#[derive(Default)]
struct AuditModule {
    /// Có sau `start`.
    recorder: Option<Arc<Recorder>>,
    service: Option<Arc<AuditService>>,
    chain: Option<Arc<chain::ChainProbe>>,
}

impl Module for AuditModule {
    fn navigation(&self) -> autoxide_kernel::module::navigation::Navigation {
        use autoxide_kernel::module::navigation::{NavGroup, NavItem, Navigation};
        Navigation::default().group(NavGroup::new("settings", "Settings", "gear", 110)).item(
            NavItem::new("settings.audit", "settings", "Audit log", 40)
                .permissions(&[crate::http::AUDIT_READ]),
        )
    }

    fn name(&self) -> &'static str {
        "audit"
    }

    // Module nền: không gói nào tắt được (bước 35, D-323).
    fn always_enabled(&self) -> bool {
        true
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        self.recorder = Some(Arc::new(Recorder));
        self.service = Some(Arc::new(AuditService::new(context.resources())));
        self.chain = Some(Arc::new(chain::ChainProbe { databases: context.tenancy() }));
        Ok(())
    }

    fn seed_tenant(&self) -> Option<Arc<dyn autoxide_kernel::tenancy::TenantSeed>> {
        Some(Arc::new(HeadSeed))
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![http::AUDIT_READ]
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn tools(&self) -> Vec<autoxide_kernel::mcp::Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn checks(&self) -> Vec<autoxide_kernel::doctor::Check> {
        self.chain.clone().map_or_else(Vec::new, |probe| {
            vec![
                autoxide_kernel::doctor::Check::new(
                    "chain",
                    "No audit log entry was changed, removed or inserted after it was written \
                     (hash chain, D-183).",
                    probe,
                )
                // Đọc cả audit log: lâu hơn bước kiểm thường.
                .with_timeout(chain::VERIFY_TIMEOUT),
            ]
        })
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        match &self.recorder {
            Some(recorder) => vec![Subscription::new(Subscription::ALL, recorder.clone())],
            None => Vec::new(),
        }
    }
}

/// Gieo đầu chuỗi băm trống cho khách mới (bước 33, D-276).
struct HeadSeed;

#[async_trait::async_trait]
impl autoxide_kernel::tenancy::TenantSeed for HeadSeed {
    async fn seed(&self, tenant: &autoxide_kernel::tenancy::TenantContext) -> Result<(), Error> {
        chain::seed_head(tenant.database().write(), tenant.id().as_uuid()).await
    }
}
