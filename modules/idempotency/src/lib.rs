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

use autoxide_kernel::db::RecordInfo;
use autoxide_kernel::error::Error;
use autoxide_kernel::idempotency::IdempotencyStore;
use autoxide_kernel::job::JobRegistration;
use autoxide_kernel::module::{Manifest, Module, StartContext};
use autoxide_kernel::tenancy::Relocation;

/// Module `idempotency`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    IdempotencyModule
}

struct IdempotencyModule;

// Hợp đồng module (D-20261126-module-manifest): khai tĩnh trong `manifest`, đăng ký thứ cần service trong `start`.
impl Module for IdempotencyModule {
    fn manifest(&self) -> Manifest {
        Manifest::new(
            "idempotency",
            "Storage of `Idempotency-Key` results, so retried writes run once.",
        )
        // Core không chạy được nếu thiếu module này: `AUTOXIDE_MODULES` không tắt được nó (bước 70 P1).
        .essential()
        // Module nền: không gói nào tắt được (bước 35, D-323).
        .always_enabled()
        .migrations(migration::all)
        .records([RecordInfo::of::<entity::IdempotencyRecord>()])
        .translation("vi", include_str!("../locales/vi.json"))
        .schedules([store::cleanup_schedule()])
        // Tách / gộp khách (bước 38, D-20261118-step38-relocation): key là dữ liệu tạm, không chép.
        .relocation(Relocation::new().ephemeral(&["idempotency_record"]))
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.job(JobRegistration::new(store::CLEANUP, Arc::new(store::Cleanup)))?;
        context.provide::<dyn IdempotencyStore>(Arc::new(store::Store))
    }
}
