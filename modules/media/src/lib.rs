//! Module `media`: thư viện media — ảnh, video, âm thanh, tài liệu trong thư mục lồng nhau,
//! cho website và module khác dùng lại (D-086).
//!
//! Nội dung lưu ở nơi lưu chung của chương trình (`dyn Storage` do module `files` công
//! bố — ổ đĩa hay S3), dưới khoá `media/<item_id>/<version_id>`; thông tin nằm trong
//! `media_item` / `media_folder`. Module không đọc, ghi hay tham chiếu bảng của `files`.
//!
//! REST ở `/api/v1/media` (hướng dẫn: `docs/MEDIA.md`), tool MCP, trait [`api::Media`]
//! cho module khác, hai job định kỳ (dọn thùng rác, dọn nội dung mồ côi).

mod alt;
pub mod api;
mod config;
mod entity;
mod events;
mod folders;
mod http;
mod jobs;
mod migration;
mod permissions;
mod refresh;
mod service;
mod sniff;
mod tags;
mod tools;
mod upload;
mod variants;

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
use crate::jobs::{CLEAN_ORPHANS, CleanOrphans, PURGE_TRASH, PurgeTrash};
use crate::service::MediaService;
use crate::variants::{Generator, MAKE_VARIANTS};

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
        let service = Arc::new(MediaService::new(
            context.database().clone(),
            storage,
            context.outbox()?,
            context.jobs(),
            ai,
            config,
        ));
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
        self.service.clone().map_or_else(Vec::new, |service| vec![refresh::task(service)])
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! Đồ dùng chung cho test trong crate này.

    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use autoxide_kernel::access::Principal;
    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::database::Database;
    use autoxide_kernel::error::Error;
    use autoxide_kernel::event::Outbox;
    use autoxide_kernel::migration::{Migrator, ModuleMigrations};
    use autoxide_kernel::storage::{ByteStream, Storage, StoredFile};
    use autoxide_testkit::TestDatabase;
    use bytes::Bytes;
    use futures_util::{StreamExt as _, TryStreamExt as _};
    use uuid::Uuid;

    use crate::config::{Limits, MediaConfig};
    use crate::service::MediaService;

    /// Nơi lưu trong bộ nhớ: đủ cho test của module (nơi lưu thật — ổ đĩa, S3 — được thử
    /// ở test đầu-cuối của `crates/app`). Không cài lại `get_range` / `keys_with_prefix`:
    /// thử luôn cài đặt mặc định của kernel.
    #[derive(Default)]
    pub struct MemoryStorage(pub Mutex<BTreeMap<String, Bytes>>);

    #[async_trait::async_trait]
    impl Storage for MemoryStorage {
        async fn put(&self, key: &str, content: ByteStream) -> Result<u64, Error> {
            let chunks: Vec<Bytes> = content.try_collect().await?;
            let bytes = Bytes::from(chunks.concat());
            let size = bytes.len() as u64;
            self.0.lock().unwrap().insert(key.to_owned(), bytes);
            Ok(size)
        }

        async fn get(&self, key: &str) -> Result<Option<StoredFile>, Error> {
            let bytes = self.0.lock().unwrap().get(key).cloned();
            Ok(bytes.map(|bytes| {
                // Chia thành khúc 3 byte để thử việc cắt khoảng qua nhiều khúc.
                let content = futures_util::stream::iter(
                    bytes
                        .chunks(3)
                        .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
                        .collect::<Vec<_>>(),
                )
                .boxed();
                StoredFile::new(bytes.len() as u64, content)
            }))
        }

        async fn delete(&self, key: &str) -> Result<(), Error> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }

        async fn keys(&self) -> Result<Vec<String>, Error> {
            Ok(self.0.lock().unwrap().keys().cloned().collect())
        }
    }

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
