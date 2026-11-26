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
mod relocation;
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
use autoxide_kernel::http::ModuleRouter;

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

    fn description(&self) -> &'static str {
        "Append-only audit log: who changed what, on which record, when, before and after."
    }

    // Audit log nối chuỗi băm (D-183) nên không cắt bớt từng dòng theo tuổi: giữ tới khi xoá hẳn
    // khách. Phần dữ liệu cá nhân (`before` / `after` / `details`) bị xoá khi tài khoản bị xoá.
    fn retention(&self) -> Vec<autoxide_kernel::privacy::RetentionPolicy> {
        use autoxide_kernel::privacy::{DataCategory, Keep, RetentionPolicy};
        vec![RetentionPolicy::new(
            "audit.entries",
            "Who did what and when. The personal details of an entry (before / after values) are \
             erased when the account they describe is deleted; the entry itself stays until the \
             tenant is deleted.",
            DataCategory::Security,
            Keep::UntilDeleted,
        )]
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
        let service = Arc::new(
            AuditService::new(context.resources())
                .with_events(context.events().clone(), context.translations().clone()),
        );
        // Lịch sử theo bản ghi cho dòng hoạt động của kernel (bước 51 P13).
        context.provide::<dyn autoxide_kernel::resource::RecordHistory>(service.clone())?;
        self.service = Some(service);
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

    fn routes(&self) -> ModuleRouter {
        self.service.clone().map_or_else(ModuleRouter::new, |service| http::routes(service).into())
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

    // Tách / gộp khách (bước 38): ghi lại dòng vào chuỗi băm của khách nhận, nguồn giữ nguyên.
    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        relocation::relocation()
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
