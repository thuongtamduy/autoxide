//! Module `audit`: audit log chỉ thêm — ai làm gì, với bản ghi nào, lúc nào, trước và
//! sau ra sao.
//!
//! Không module nào gọi audit trực tiếp: mọi module phát event về thay đổi của mình
//! (`autoxide_kernel::event`), audit đăng ký nhận MỌI event và ghi mỗi event thành
//! một dòng. Thêm module mới không phải sửa audit (D-055).

pub mod api;
mod entity;
mod migration;
mod recorder;

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Subscription;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};

use crate::recorder::Recorder;

/// Module `audit`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    AuditModule::default()
}

#[derive(Default)]
struct AuditModule {
    /// Có sau `start`.
    recorder: Option<Arc<Recorder>>,
}

impl Module for AuditModule {
    fn name(&self) -> &'static str {
        "audit"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        self.recorder = Some(Arc::new(Recorder::new(context.database().clone())));
        Ok(())
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        match &self.recorder {
            Some(recorder) => vec![Subscription::new(Subscription::ALL, recorder.clone())],
            None => Vec::new(),
        }
    }
}
