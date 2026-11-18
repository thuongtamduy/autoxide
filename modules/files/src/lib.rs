//! Module `files`: nơi lưu file cho mọi module (D-066).
//!
//! Có `AUTOXIDE_FILES_S3_BUCKET` thì lưu trên S3 (hay dịch vụ tương thích S3); không có
//! thì lưu trong thư mục `AUTOXIDE_FILES_DIR`. Module khác dùng `dyn Storage`
//! (`autoxide_kernel::storage`) mà không cần biết file nằm ở đâu.
//!
//! Người dùng tải file lên, xem, tải xuống và xoá qua `/api/v1/files` (D-067); thông
//! tin file nằm trong bảng `files_file`, có giới hạn kích thước và loại.

pub mod api;
mod cleanup;
mod config;
mod doctor;
mod entity;
mod events;
mod http;
mod migration;
mod permissions;
mod service;
mod store;
mod tools;
mod uploads;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventInfo, Subscription};
use autoxide_kernel::http::BodyLimit;
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::storage::Storage;
use axum::http::Method;
use utoipa_axum::router::OpenApiRouter;

use crate::config::{Backend, FilesConfig};
use crate::service::FileService;
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
    /// Có sau `start`.
    service: Option<Arc<FileService>>,
    /// Bước kiểm của `autoxide doctor`; có sau `start`.
    probe: Option<Arc<doctor::StorageProbe>>,
    /// Sổ database, cho job dọn mồ côi (đối chiếu khoá với database của đúng khách); có sau `start`.
    databases: Option<autoxide_kernel::tenancy::DatabaseRegistry>,
}

impl Module for FilesModule {
    fn name(&self) -> &'static str {
        "files"
    }

    // Tách / gộp khách (bước 38, D-20261118-step38-relocation).
    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        use autoxide_kernel::tenancy::Relocation;
        Relocation::new()
            // File không gắn nút: ở lại khách nguồn khi tách; file của thứ đi theo (media…) được
            // kernel chép theo khoá `tenants/<khách>/<module>/<id>/…`.
            .stay(&["files_file"])
            .ephemeral(&["files_uploads"])
    }

    fn description(&self) -> &'static str {
        "File storage for every module, on disk or on S3-compatible storage."
    }

    // Module nền: không gói nào tắt được (bước 35, D-323).
    fn limits(&self) -> Vec<autoxide_kernel::entitlement::Limit> {
        vec![service::STORAGE_LIMIT]
    }

    fn always_enabled(&self) -> bool {
        true
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

    fn body_limits(&self) -> Vec<BodyLimit> {
        // Tải lên nhận tới giới hạn file, thay cho giới hạn chung của request.
        let max = self.config.as_ref().map_or(0, |config| config.limits.max_bytes);
        let bytes = usize::try_from(max).unwrap_or(usize::MAX);
        vec![BodyLimit::new(Method::POST, "/", bytes)]
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
        let location = match &config.backend {
            Backend::Local { dir } => {
                tracing::info!(directory = %dir.display(), "files are stored on local disk");
                format!("local disk at {}", dir.display())
            }
            Backend::S3(s3) => {
                tracing::info!(bucket = %s3.bucket, "files are stored in S3");
                format!("S3 bucket `{}`", s3.bucket)
            }
        };
        let storage: Arc<dyn Storage> = Arc::new(ObjectStorage::open(&config.backend)?);
        self.probe = Some(Arc::new(doctor::StorageProbe { storage: storage.clone(), location }));
        let outbox = context.outbox()?;
        self.databases = Some(context.tenancy());
        let mut service = FileService::new(storage.clone(), outbox, config.limits);
        service.entitlements = context.entitlements();
        service.direct = config.direct;
        context.usage_counter(&service::STORAGE_LIMIT, Arc::new(service::StorageCounter))?;
        let service = Arc::new(service);
        self.service = Some(service.clone());
        context.personal_data(service)?;
        // Nơi lưu riêng của placement (bước 33) mở bằng cùng cài đặt.
        context.provide::<dyn autoxide_kernel::storage::StorageOpener>(Arc::new(store::Opener))?;
        context.provide::<dyn Storage>(storage)
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        // Tài khoản bị xoá: xoá file của người đó.
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![Subscription::new(autoxide_kernel::privacy::USER_DELETED, service)]
        })
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn checks(&self) -> Vec<autoxide_kernel::doctor::Check> {
        self.probe.clone().map_or_else(Vec::new, |probe| {
            vec![autoxide_kernel::doctor::Check::new(
                "storage",
                "Files can be written, read back and deleted where they are stored.",
                probe,
            )]
        })
    }

    fn tools(&self) -> Vec<autoxide_kernel::mcp::Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        match (self.service.clone(), self.databases.clone()) {
            (Some(service), Some(databases)) => vec![
                JobRegistration::new(
                    cleanup::CLEAN_ORPHANS,
                    Arc::new(cleanup::CleanOrphans {
                        service: service.clone(),
                        databases: databases.clone(),
                    }),
                ),
                JobRegistration::new(
                    uploads::PURGE_UPLOADS,
                    Arc::new(uploads::PurgeUploads { service, databases }),
                ),
            ],
            _ => Vec::new(),
        }
    }

    fn schedules(&self) -> Vec<Schedule> {
        let day = std::time::Duration::from_secs(24 * 3600);
        // Một lần mỗi ngày cho cả bản cài (database điều khiển): job quét mọi nơi lưu file và đối
        // chiếu từng khoá với database của đúng khách — chạy trên mọi database chỉ lặp lại việc đó.
        let hour = std::time::Duration::from_secs(3600);
        vec![
            Schedule::new(cleanup::CLEAN_ORPHANS, cleanup::CLEAN_ORPHANS, day).on_control_only(),
            // Lượt tải thẳng bỏ dở (bước 40 P4): bảng `files_uploads` nằm ở database của từng khách,
            // nên chạy trên mọi database.
            Schedule::new(uploads::PURGE_UPLOADS, uploads::PURGE_UPLOADS, hour),
        ]
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod testing {
    //! Đồ dùng chung cho test trong crate này.

    use std::sync::Arc;

    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::database::Database;
    use autoxide_kernel::event::Outbox;
    use autoxide_kernel::migration::{Migrator, ModuleMigrations};
    use autoxide_testkit::TestDatabase;

    use crate::config::{Backend, Limits};
    use crate::service::FileService;
    use crate::store::ObjectStorage;

    /// Service trên database (đã migrate) của test, lưu file trong một thư mục tạm.
    pub async fn service(test: &TestDatabase, limits: Limits) -> (Arc<FileService>, Database) {
        let dir = std::env::temp_dir().join(format!("autoxide-files-{}", uuid::Uuid::now_v7()));
        let storage = Arc::new(ObjectStorage::open(&Backend::Local { dir }).unwrap());
        service_on(test, limits, storage).await
    }

    /// Như [`service`], lưu file ở `storage`.
    pub async fn service_on(
        test: &TestDatabase,
        limits: Limits,
        storage: Arc<dyn autoxide_kernel::storage::Storage>,
    ) -> (Arc<FileService>, Database) {
        let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
        let mut reader = ConfigReader::new(&source);
        let config = Config::read(&mut reader);
        let (config, _) = reader.finish(config).unwrap();
        let database = Database::connect(&config.database).await.unwrap();
        Migrator::new(vec![ModuleMigrations::new("files", crate::migration::all())])
            .unwrap()
            .up(&database)
            .await
            .unwrap();
        let outbox = Outbox::standalone("files");
        (Arc::new(FileService::new(storage, outbox, limits)), database)
    }

    pub fn no_limits() -> Limits {
        Limits { max_bytes: 1024 * 1024, allowed_types: None }
    }
}
