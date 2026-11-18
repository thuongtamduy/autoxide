//! Module `media`: thư viện media — ảnh, video, âm thanh, tài liệu trong thư mục lồng nhau,
//! cho website và module khác dùng lại (D-086).
//!
//! Nội dung lưu ở nơi lưu chung của chương trình (`dyn Storage` do module `files` công
//! bố — ổ đĩa hay S3), dưới khoá `media/<item_id>/<version_id>` (khách khác khách mặc định: dưới
//! `tenants/<khách>/`); thông tin nằm trong `media_item` / `media_folder`. Module không đọc, ghi
//! hay tham chiếu bảng của `files`. Mọi bảng theo khách (bước 33).
//!
//! REST ở `/api/v1/media` (hướng dẫn: `docs/MEDIA.md`), tool MCP, trait [`api::Media`]
//! cho module khác, hai job định kỳ (dọn thùng rác, dọn nội dung mồ côi).

pub mod api;
mod client_config;
mod config;
mod entity;
mod events;
mod find;
mod http;
mod images;
mod jobs;
mod library;
mod migration;
mod permissions;
mod service;
mod summary;
mod tools;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventInfo, Subscription};
use autoxide_kernel::http::BodyLimit;
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::storage::Storage;
use axum::http::Method;
use utoipa_axum::router::OpenApiRouter;

use crate::config::MediaConfig;
use crate::images::variants::{Generator, MAKE_VARIANTS};
use crate::jobs::{CLEAN_ORPHANS, CleanOrphans, PURGE_TRASH, PurgeTrash};
use crate::service::MediaService;

/// Module `media`, để binary đăng ký trong `crates/app/src/modules.rs` — SAU module `files`
/// (cần `dyn Storage` của nó).
pub fn module() -> impl Module {
    MediaModule::default()
}

#[derive(Default)]
struct MediaModule {
    /// Có sau `configure`.
    config: Option<MediaConfig>,
    /// Có sau `start`.
    service: Option<Arc<MediaService>>,
    /// Sổ database, có sau `start`: job dọn dẹp và lệnh `refresh_variants` tra ngữ cảnh từng khách.
    databases: Option<autoxide_kernel::tenancy::DatabaseRegistry>,
}

impl Module for MediaModule {
    fn navigation(&self) -> autoxide_kernel::module::navigation::Navigation {
        use autoxide_kernel::module::navigation::{MobilePlacement, NavGroup, NavItem, Navigation};
        Navigation::default().group(NavGroup::new("content", "Content", "file", 50)).item(
            NavItem::new("content.media", "content", "Media library", 40)
                .permissions(&[crate::permissions::MEDIA_READ])
                .mobile(MobilePlacement::Primary, 50),
        )
    }

    fn name(&self) -> &'static str {
        "media"
    }

    fn description(&self) -> &'static str {
        "Media library: images, video, audio and documents in nested folders, with tags, alt \
         text and resized copies."
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn permissions(&self) -> Vec<Permission> {
        permissions::all()
    }

    fn events(&self) -> Vec<EventInfo> {
        events::with_activity()
    }

    fn body_limits(&self) -> Vec<BodyLimit> {
        // Tải lên và thay nội dung nhận tới giới hạn của item, thay cho giới hạn chung.
        let max = self.config.as_ref().map_or(0, |config| config.limits.max_bytes);
        let bytes = usize::try_from(max).unwrap_or(usize::MAX);
        vec![
            BodyLimit::new(Method::POST, "/", bytes),
            BodyLimit::new(Method::PUT, "/{id}/content", bytes),
        ]
    }

    fn client_config(
        &self,
    ) -> Result<Option<autoxide_kernel::module::ClientSection>, autoxide_kernel::error::Error> {
        let Some(config) = &self.config else { return Ok(None) };
        let section = client_config::section(config)?;
        // Ngôn ngữ dịch `alt` là cấu hình theo khách: request xác định được khách thì lấy của khách.
        Ok(match &self.service {
            Some(service) => section.map(|section| {
                section.per_tenant(Arc::new(client_config::TenantAltLocales(service.clone())))
            }),
            None => section,
        })
    }

    fn tenant_settings(&self) -> Vec<autoxide_kernel::tenancy::TenantSetting> {
        vec![config::TENANT_ALT_LOCALES]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(MediaConfig::read(reader)?);
        Ok(())
    }

    // Thời hạn lưu dữ liệu cá nhân (bước 37 C5, D-360), đọc từ cấu hình đang có.
    fn retention(&self) -> Vec<autoxide_kernel::privacy::RetentionPolicy> {
        use autoxide_kernel::privacy::{DataCategory, Keep, RetentionPolicy};
        let Some(config) = &self.config else {
            return Vec::new();
        };
        vec![
            RetentionPolicy::new(
                "media.trash",
                "Media items in the trash (may show people) before they are deleted for good.",
                DataCategory::Basic,
                Keep::Days(config.trash_days),
            )
            .setting(config::TRASH_DAYS.name),
        ]
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `media` started before it was configured"))?;
        let storage = context.require::<dyn Storage>()?;
        let ai = context.require::<dyn autoxide_kernel::ai::TextModel>()?;
        let service = Arc::new(
            MediaService::new(storage, context.outbox()?, context.jobs(), ai, config)
                .with_resources(context.resources())
                .with_settings(context.require::<dyn autoxide_kernel::tenancy::TenantSettings>()?),
        );
        self.service = Some(service.clone());
        self.databases = Some(context.tenancy());
        context.personal_data(service.clone())?;
        context.finder_source(Arc::new(find::Items))?;
        context.summary_metric(Arc::new(summary::Items))?;
        context.provide::<dyn api::Media>(service)
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        // Tài khoản bị xoá: bỏ liên kết item tới người đó (item vẫn giữ).
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![Subscription::new(autoxide_kernel::privacy::USER_DELETED, service)]
        })
    }

    fn seed_tenant(&self) -> Option<Arc<dyn autoxide_kernel::tenancy::TenantSeed>> {
        Some(Arc::new(LockSeed))
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        match (self.service.clone(), self.databases.clone()) {
            (Some(service), Some(databases)) => vec![
                JobRegistration::new(
                    PURGE_TRASH,
                    Arc::new(PurgeTrash { service: service.clone(), databases: databases.clone() }),
                ),
                JobRegistration::new(
                    CLEAN_ORPHANS,
                    Arc::new(CleanOrphans {
                        service: service.clone(),
                        databases: databases.clone(),
                    }),
                ),
                JobRegistration::new(
                    library::direct::PURGE_UPLOADS,
                    Arc::new(library::direct::PurgeUploads { service: service.clone(), databases }),
                ),
                JobRegistration::new(MAKE_VARIANTS, Arc::new(Generator(service))),
            ],
            _ => Vec::new(),
        }
    }

    fn schedules(&self) -> Vec<Schedule> {
        let day = Duration::from_secs(24 * 3600);
        vec![
            Schedule::new(PURGE_TRASH, PURGE_TRASH, day),
            // Một lần cho cả bản cài: job quét mọi nơi lưu, đối chiếu với database của đúng khách.
            Schedule::new(CLEAN_ORPHANS, CLEAN_ORPHANS, day).on_control_only(),
            // Lượt tải thẳng bỏ dở (bước 40 P4): bảng ở database của từng khách.
            Schedule::new(
                library::direct::PURGE_UPLOADS,
                library::direct::PURGE_UPLOADS,
                Duration::from_secs(3600),
            ),
        ]
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn tasks(&self) -> Vec<autoxide_kernel::task::Task> {
        match (self.service.clone(), self.databases.clone()) {
            (Some(service), Some(databases)) => vec![images::refresh::task(service, databases)],
            _ => Vec::new(),
        }
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }
}

/// Gieo dòng khoá cây thư mục (`media_lock.folder_tree`) cho khách mới (bước 33). Khách mặc định
/// có dòng từ migration `m0001`.
struct LockSeed;

#[async_trait::async_trait]
impl autoxide_kernel::tenancy::TenantSeed for LockSeed {
    async fn seed(&self, tenant: &autoxide_kernel::tenancy::TenantContext) -> Result<(), Error> {
        library::folders::seed_lock(tenant.database().write(), tenant).await
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! Đồ dùng chung cho test trong crate này.

    use std::sync::Arc;

    use autoxide_kernel::access::Principal;
    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::database::Database;
    use autoxide_kernel::event::Outbox;
    use autoxide_kernel::migration::{Migrator, ModuleMigrations};
    use autoxide_kernel::storage::ByteStream;
    use autoxide_testkit::{MemoryStorage, TestDatabase};
    use bytes::Bytes;
    use futures_util::StreamExt as _;
    use uuid::Uuid;

    use crate::config::{Limits, MediaConfig};
    use crate::service::MediaService;

    pub fn limits() -> Limits {
        let source = ConfigSource::default();
        let mut reader = ConfigReader::new(&source);
        let config = MediaConfig::read(&mut reader);
        reader.finish(config).unwrap().0.limits
    }

    /// Service trên database (đã migrate) của test, nội dung trong bộ nhớ; AI tắt.
    pub async fn service(
        test: &TestDatabase,
        limits: Limits,
    ) -> (Arc<MediaService>, Arc<MemoryStorage>, Database) {
        let ai = Arc::new(autoxide_testkit::ai::FakeTextModel::disabled());
        service_with_ai(test, limits, ai, false).await
    }

    /// Như [`service`], với mô hình AI cho trước; `send_private`: cho gửi ảnh riêng tư.
    pub async fn service_with_ai(
        test: &TestDatabase,
        limits: Limits,
        ai: Arc<dyn autoxide_kernel::ai::TextModel>,
        send_private: bool,
    ) -> (Arc<MediaService>, Arc<MemoryStorage>, Database) {
        let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
        let mut reader = ConfigReader::new(&source);
        let config = Config::read(&mut reader);
        let (config, _) = reader.finish(config).unwrap();
        let database = Database::connect(&config.database).await.unwrap();
        // Bảng của kernel: cây tổ chức (bước 34) mà kiểm quyền theo nút đọc tới.
        autoxide_kernel::module::Modules::new(Vec::new())
            .unwrap()
            .migrator()
            .unwrap()
            .up(&database)
            .await
            .unwrap();
        Migrator::new(vec![ModuleMigrations::new("media", crate::migration::all())])
            .unwrap()
            .up(&database)
            .await
            .unwrap();
        let storage = Arc::new(MemoryStorage::default());
        let config = MediaConfig {
            limits,
            trash_days: 30,
            image_widths: vec![100, 200],
            ai_send_private: send_private,
            alt_locales: vec!["en".to_owned(), "vi".to_owned()],
            direct: crate::config::Direct::default(),
        };
        let service = MediaService::new(
            storage.clone(),
            Outbox::standalone("media"),
            autoxide_kernel::job::JobQueue::new("media"),
            ai,
            config,
        );
        (Arc::new(service), storage, database)
    }

    pub fn body(bytes: &[u8]) -> ByteStream {
        futures_util::stream::iter([Ok(Bytes::copy_from_slice(bytes))]).boxed()
    }

    /// Người có các permission `permissions` (`media.` thêm sẵn).
    pub fn user(permissions: &[&str]) -> Principal {
        Principal::new(Uuid::now_v7(), permissions.iter().map(|p| format!("media.{p}")))
    }

    pub fn editor() -> Principal {
        user(&["read", "write", "purge"])
    }
}
