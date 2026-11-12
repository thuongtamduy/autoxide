//! Module `content`: bài viết và trang (có cấp cha con), bản nháp tách khỏi bản công khai,
//! lịch sử phiên bản, hẹn giờ, bản dịch, danh mục, thẻ, tác giả, menu, chuyển hướng, dữ liệu
//! SEO, endpoint công khai cho website (D-096, D-098, D-101, D-102).
//!
//! Chỉ API: giao diện quản trị và website là repo riêng (D-097). Ảnh là ID của module
//! `media` (đăng ký SAU `media`, kiểm qua `autoxide_media::api::Media`). Hướng dẫn dùng:
//! `docs/CONTENT.md`.

mod authors;
mod body;
mod config;
mod entity;
mod entries;
mod events;
mod http;
mod menus;
mod migration;
mod paths;
mod permissions;
mod preview;
mod privacy;
mod publishing;
mod reading;
mod redirects;
mod resource;
mod revisions;
mod schedules;
mod service;
mod slug;
mod terms;
mod tools;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventInfo, Subscription};
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_media::api::Media;
use utoipa_axum::router::OpenApiRouter;

use crate::config::ContentConfig;
use crate::service::ContentService;

/// Module `content`, để binary đăng ký trong `crates/app/src/modules.rs` — SAU module `media`.
pub fn module() -> impl Module {
    ContentModule::default()
}

#[derive(Default)]
struct ContentModule {
    /// Có sau `configure`.
    config: Option<ContentConfig>,
    /// Có sau `start`.
    service: Option<Arc<ContentService>>,
}

impl Module for ContentModule {
    fn name(&self) -> &'static str {
        "content"
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

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(ContentConfig::read(reader)?);
        Ok(())
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `content` started before it was configured"))?;
        let media = context.require::<dyn Media>()?;
        let service = Arc::new(ContentService::new(
            context.database().clone(),
            context.outbox()?,
            context.jobs(),
            media,
            config,
        ));
        context.personal_data(service.clone())?;
        context
            .resource_type(resource::ENTRY, Arc::new(resource::EntryResolver(service.clone())))?;
        self.service = Some(service);
        Ok(())
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        // Tài khoản bị xoá: gỡ hồ sơ tác giả, bỏ liên kết, huỷ lịch (xem `privacy.rs`).
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![Subscription::new(autoxide_kernel::privacy::USER_DELETED, service)]
        })
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![
                JobRegistration::new(schedules::RUN, Arc::new(schedules::Runner(service.clone()))),
                JobRegistration::new(revisions::PRUNE, Arc::new(revisions::Prune(service))),
            ]
        })
    }

    fn schedules(&self) -> Vec<Schedule> {
        let day = std::time::Duration::from_secs(24 * 3600);
        vec![Schedule::new(revisions::PRUNE, revisions::PRUNE, day)]
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! Đồ dùng chung cho test trong crate này.

    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use autoxide_kernel::access::Principal;
    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::database::Database;
    use autoxide_kernel::error::Error;
    use autoxide_kernel::event::Outbox;
    use autoxide_kernel::migration::{Migrator, ModuleMigrations};
    use autoxide_media::api::{Media, MediaRef, MediaVisibility};
    use autoxide_testkit::TestDatabase;
    use uuid::Uuid;

    use crate::config::ContentConfig;
    use crate::service::ContentService;

    /// Thư viện media giả: ID → công khai hay không.
    #[derive(Default)]
    pub struct FakeMedia(pub Mutex<HashMap<Uuid, MediaVisibility>>);

    impl FakeMedia {
        pub fn add(&self, public: bool) -> Uuid {
            let id = Uuid::now_v7();
            let visibility =
                if public { MediaVisibility::Public } else { MediaVisibility::Private };
            self.0.lock().unwrap().insert(id, visibility);
            id
        }
    }

    #[async_trait::async_trait]
    impl Media for FakeMedia {
        async fn find_many(&self, ids: &[Uuid]) -> Result<Vec<MediaRef>, Error> {
            let known = self.0.lock().unwrap();
            Ok(ids
                .iter()
                .filter_map(|id| {
                    known.get(id).map(|visibility| MediaRef {
                        id: *id,
                        visibility: *visibility,
                        content_type: "image/png".to_owned(),
                        width: Some(10),
                        height: Some(10),
                        alt: String::new(),
                        url: format!("/api/v1/media/{id}/content?v=1"),
                        variants: Vec::new(),
                    })
                })
                .collect())
        }
    }

    pub fn config(pairs: &[(&str, &str)]) -> ContentConfig {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = ContentConfig::read(&mut reader);
        reader.finish(config).unwrap().0
    }

    /// Service trên database (đã migrate) của test.
    pub async fn service(
        test: &TestDatabase,
        config: ContentConfig,
    ) -> (Arc<ContentService>, Arc<FakeMedia>, Database) {
        let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
        let mut reader = ConfigReader::new(&source);
        let kernel = Config::read(&mut reader);
        let (kernel, _) = reader.finish(kernel).unwrap();
        let database = Database::connect(&kernel.database).await.unwrap();
        Migrator::new(vec![ModuleMigrations::new("content", crate::migration::all())])
            .unwrap()
            .up(&database)
            .await
            .unwrap();
        let media = Arc::new(FakeMedia::default());
        let service = ContentService::new(
            database.clone(),
            Outbox::standalone("content"),
            autoxide_kernel::job::JobQueue::new("content"),
            media.clone(),
            config,
        );
        (Arc::new(service), media, database)
    }

    /// Người có các permission `permissions` (`content.` thêm sẵn).
    pub fn user(permissions: &[&str]) -> Principal {
        Principal::new(Uuid::now_v7(), permissions.iter().map(|p| format!("content.{p}")))
    }

    pub fn editor() -> Principal {
        user(&["read", "write", "publish", "manage"])
    }
}
