//! Module `media`: thư viện media — ảnh, video, âm thanh, tài liệu trong thư mục lồng nhau,
//! cho website và module khác dùng lại (D-086).
//!
//! Nội dung lưu ở nơi lưu chung của chương trình (`dyn Storage` do module `files` công
//! bố — ổ đĩa hay S3), dưới khoá `media/<item_id>/<version_id>`; thông tin nằm trong
//! `media_item` / `media_folder`. Module không đọc, ghi hay tham chiếu bảng của `files`.
//!
//! REST ở `/api/v1/media` (hướng dẫn: `docs/MEDIA.md`), tool MCP, trait [`api::Media`]
//! cho module khác, hai job định kỳ (dọn thùng rác, dọn nội dung mồ côi).

pub mod api;
mod client_config;
mod config;
mod entity;
mod events;
mod http;
mod images;
mod jobs;
mod library;
mod migration;
mod permissions;
mod service;
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
}

impl Module for MediaModule {
    fn name(&self) -> &'static str {
        "media"
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
        self.config.as_ref().map_or(Ok(None), client_config::section)
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(MediaConfig::read(reader)?);
        Ok(())
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
            MediaService::new(
                context.database().clone(),
                storage,
                context.outbox()?,
                context.jobs(),
                ai,
                config,
            )
            .with_resources(context.resources()),
        );
        self.service = Some(service.clone());
        context.personal_data(service.clone())?;
        context.provide::<dyn api::Media>(service)
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        // Tài khoản bị xoá: bỏ liên kết item tới người đó (item vẫn giữ).
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![Subscription::new(autoxide_kernel::privacy::USER_DELETED, service)]
        })
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![
                JobRegistration::new(PURGE_TRASH, Arc::new(PurgeTrash(service.clone()))),
                JobRegistration::new(CLEAN_ORPHANS, Arc::new(CleanOrphans(service.clone()))),
                JobRegistration::new(MAKE_VARIANTS, Arc::new(Generator(service))),
            ]
        })
    }

    fn schedules(&self) -> Vec<Schedule> {
        let day = Duration::from_secs(24 * 3600);
        vec![
            Schedule::new(PURGE_TRASH, PURGE_TRASH, day),
            Schedule::new(CLEAN_ORPHANS, CLEAN_ORPHANS, day),
        ]
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn tasks(&self) -> Vec<autoxide_kernel::task::Task> {
        self.service.clone().map_or_else(Vec::new, |service| vec![images::refresh::task(service)])
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
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
        };
        let service = MediaService::new(
            database.clone(),
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
