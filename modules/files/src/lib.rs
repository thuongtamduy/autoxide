//! Module `files`: nơi lưu file cho mọi module (D-066).
//!
//! Có `AUTOXIDE_FILES_S3_BUCKET` thì lưu trên S3 (hay dịch vụ tương thích S3); không có
//! thì lưu trong thư mục `AUTOXIDE_FILES_DIR`. Module khác dùng `dyn Storage`
//! (`autoxide_kernel::storage`) mà không cần biết file nằm ở đâu.

pub mod api;
mod config;
mod store;

use std::sync::Arc;

use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::storage::Storage;

use crate::config::{Backend, FilesConfig};
use crate::store::ObjectStorage;

/// Module `files`, để binary đăng ký trong `crates/app/src/modules.rs` — TRƯỚC các
/// module lưu file.
pub fn module() -> impl Module {
    FilesModule::default()
}

#[derive(Default)]
struct FilesModule {
    /// Có sau `configure`.
    config: Option<FilesConfig>,
}

impl Module for FilesModule {
    fn name(&self) -> &'static str {
        "files"
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(FilesConfig::read(reader)?);
        Ok(())
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `files` started before it was configured"))?;
        match &config.backend {
            Backend::Local { dir } => {
                tracing::info!(directory = %dir.display(), "files are stored on local disk");
            }
            Backend::S3(s3) => tracing::info!(bucket = %s3.bucket, "files are stored in S3"),
        }
        let storage = Arc::new(ObjectStorage::open(&config.backend)?);
        context.provide::<dyn Storage>(storage)
    }
}

#[cfg(test)]
mod tests;
