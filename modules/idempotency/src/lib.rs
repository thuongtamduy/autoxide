//! Module `idempotency`: lưu idempotency key cho kernel (D-058).
//!
//! Kernel xử lý header `Idempotency-Key` ở lớp HTTP cho MỌI module; module này chỉ
//! cung cấp chỗ lưu (`dyn IdempotencyStore`) và dọn key hết hạn mỗi giờ. Không có
//! module này thì kernel bỏ qua header.

pub mod api;
mod entity;
mod migration;
mod store;

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::idempotency::IdempotencyStore;
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};

/// Module `idempotency`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    IdempotencyModule::default()
}

#[derive(Default)]
struct IdempotencyModule {
    /// Có sau `start`.
    cleanup: Option<Arc<store::Cleanup>>,
}

impl Module for IdempotencyModule {
    fn name(&self) -> &'static str {
        "idempotency"
    }

    fn description(&self) -> &'static str {
        "Storage of `Idempotency-Key` results, so retried writes run once."
    }

    // Module nền: không gói nào tắt được (bước 35, D-323).
    fn always_enabled(&self) -> bool {
        true
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        self.cleanup = Some(Arc::new(store::Cleanup));
        context.provide::<dyn IdempotencyStore>(Arc::new(store::Store))
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        match &self.cleanup {
            Some(cleanup) => vec![JobRegistration::new(store::CLEANUP, cleanup.clone())],
            None => Vec::new(),
        }
    }

    fn schedules(&self) -> Vec<Schedule> {
        vec![store::cleanup_schedule()]
    }
}
