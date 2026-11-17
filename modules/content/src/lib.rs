//! Module `content`: bài viết và trang (có cấp cha con), bản nháp tách khỏi bản công khai,
//! lịch sử phiên bản, hẹn giờ, bản dịch, danh mục, thẻ, tác giả, menu, chuyển hướng, dữ liệu
//! SEO, endpoint công khai cho website (D-096, D-098, D-101, D-102).
//!
//! Chỉ API: giao diện quản trị và website là repo riêng (D-097). Ảnh là ID của module
//! `media` (đăng ký SAU `media`, kiểm qua `autoxide_media::api::Media`). Hướng dẫn dùng:
//! `docs/CONTENT.md`.
//!
//! Theo khách (bước 33): mọi bảng mang `tenant_id`, mọi truy vấn lọc theo khách; endpoint công
//! khai trả site của khách theo tên miền / header; job định kỳ (dọn revision, lập index bù) chạy
//! một lần mỗi database rồi làm theo từng khách có dữ liệu ở đó.

mod ai;
pub mod api;
mod client_config;
mod config;
mod entity;
mod entries;
mod events;
mod export;
mod find;
mod http;
mod migration;
mod permissions;
mod privacy;
mod publishing;
mod resource;
mod restriction;
mod service;
mod setup;
mod taxonomy;
mod templates;
mod tools;
mod work;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventInfo, Subscription};
use autoxide_kernel::http::Deprecation;
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::resource::{ResourceChange, ResourceEvent};
use autoxide_media::api::Media;
use axum::http::Method;
use utoipa_axum::router::OpenApiRouter;

use crate::ai::{semantic, translate};
use crate::config::ContentConfig;
use crate::entries::revisions;
use crate::publishing::schedules;
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
    fn custom_field_targets(&self) -> Vec<autoxide_kernel::custom_fields::CustomFieldTarget> {
        // Trường tự định nghĩa của khách trên bài viết / trang (bước 51 P19): chỉ nội bộ.
        vec![autoxide_kernel::custom_fields::CustomFieldTarget::new(
            resource::ENTRY,
            permissions::CONTENT_READ,
            permissions::CONTENT_WRITE,
        )]
    }

    fn overridable_texts(&self) -> Vec<autoxide_kernel::texts::OverridableText> {
        // Thuật ngữ khách đổi được (bước 51 P21): giao diện và website hiện tên loại mục theo khách.
        vec![
            autoxide_kernel::texts::OverridableText::new("content.term.post", "Post"),
            autoxide_kernel::texts::OverridableText::new("content.term.page", "Page"),
        ]
    }

    fn navigation(&self) -> autoxide_kernel::module::navigation::Navigation {
        use autoxide_kernel::module::navigation::{NavGroup, NavItem, Navigation};
        Navigation::default()
            .group(NavGroup::new("content", "Content", "file", 50))
            .item(
                NavItem::new("content.entries", "content", "Posts and pages", 10)
                    .permissions(&[crate::permissions::CONTENT_READ])
                    .counter("content.review"),
            )
            .item(
                NavItem::new("content.structure", "content", "Categories and redirects", 20)
                    .permissions(&[crate::permissions::CONTENT_READ]),
            )
    }

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
        events::with_activity()
    }

    fn resource_events(&self) -> Vec<ResourceEvent> {
        // Bài lưu trữ: thôi nhận bình luận mới, bình luận cũ vẫn hiện. Bài xoá: ẩn luôn (D-202).
        vec![
            ResourceEvent::new("content.entry_archived", resource::ENTRY, ResourceChange::Closed),
            ResourceEvent::new("content.entry_deleted", resource::ENTRY, ResourceChange::Removed),
        ]
    }

    fn deprecations(&self) -> Vec<Deprecation> {
        // Menu của website chuyển sang module `menus` (D-238); hành vi cũ giữ tới ngày sunset.
        const SINCE: &str = "2026-11-14";
        const SUNSET: &str = "2027-05-14";
        vec![
            Deprecation::new(Method::GET, "/menus", SINCE)
                .sunset(SUNSET)
                .successor("/api/v1/menus"),
            Deprecation::new(Method::GET, "/menus/{key}", SINCE)
                .sunset(SUNSET)
                .successor("/api/v1/menus/{id}"),
            Deprecation::new(Method::PUT, "/menus/{key}", SINCE)
                .sunset(SUNSET)
                .successor("/api/v1/menus/{id}"),
            Deprecation::new(Method::GET, "/public/menus/{key}", SINCE)
                .sunset(SUNSET)
                .successor("/api/v1/menus/public/{key}"),
        ]
    }

    fn client_config(
        &self,
    ) -> Result<Option<autoxide_kernel::module::ClientSection>, autoxide_kernel::error::Error> {
        let Some(config) = &self.config else { return Ok(None) };
        let section = client_config::section(config)?;
        // Ngôn ngữ là cấu hình theo khách: request xác định được khách thì lấy của khách.
        Ok(match &self.service {
            Some(service) => section.map(|section| {
                section.per_tenant(Arc::new(client_config::TenantLanguages(service.clone())))
            }),
            None => section,
        })
    }

    fn tenant_settings(&self) -> Vec<autoxide_kernel::tenancy::TenantSetting> {
        vec![config::TENANT_LOCALES, config::TENANT_DEFAULT_LOCALE]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(ContentConfig::read(reader)?);
        Ok(())
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn features(&self) -> Vec<autoxide_kernel::entitlement::Feature> {
        vec![service::AI_SUGGESTIONS]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `content` started before it was configured"))?;
        let dependencies = crate::service::Dependencies {
            media: context.require::<dyn Media>()?,
            ai: context.require::<dyn autoxide_kernel::ai::TextModel>()?,
            embedder: context.require::<dyn autoxide_kernel::ai::Embedder>()?,
            settings: context.require::<dyn autoxide_kernel::tenancy::TenantSettings>()?,
        };
        let service = Arc::new(
            ContentService::new(
                context.tenancy(),
                context.outbox()?,
                context.jobs(),
                dependencies,
                config,
            )
            .with_translations(context.translations().clone())
            // Tuỳ chọn: app đăng ký `organizations` trước `content`; test riêng của module thì không.
            .with_orgs(context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok())
            .with_entitlements(context.entitlements())
            .with_resources(context.resources())
            .with_custom_fields(context.custom_fields())
            // Mẫu trang bắt buộc (D-359): biến từ hồ sơ pháp lý của `compliance` khi app có module đó.
            .with_templates(
                (*context.require::<autoxide_kernel::template::TemplateEngine>()?).clone(),
                context.require::<dyn autoxide_compliance::api::LegalProfiles>().ok(),
            ),
        );
        context.personal_data(service.clone())?;
        context.work_source(Arc::new(work::Review))?;
        context.summary_metric(Arc::new(work::Review))?;
        context.finder_source(Arc::new(find::Entries))?;
        context.code_resolver(Arc::new(find::BySlug))?;
        context.setup_step(Arc::new(setup::FirstPage))?;
        // Nhãn nội bộ (bước 51 P20): gắn / gỡ cần quyền sửa bài.
        context.taggable(resource::ENTRY, permissions::CONTENT_WRITE)?;
        context.export_source(Arc::new(export::Entries(service.clone())))?;
        context.bulk_action(Arc::new(export::Archive(service.clone())))?;
        context.resource_type(resource::ENTRY, Arc::new(resource::EntryResolver))?;
        context.resource_type(resource::TERM, Arc::new(resource::TermResolver(service.clone())))?;
        // Menu cũ cho module `menus` nhập sang (D-238).
        context.provide::<dyn api::LegacyMenus>(service.clone())?;
        // Ảnh chụp bản đã đăng cho module khác (bước 37 C1).
        context.provide::<dyn api::PublishedEntries>(service.clone())?;
        self.service = Some(service);
        Ok(())
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        // Tài khoản bị xoá: gỡ hồ sơ tác giả, bỏ liên kết, huỷ lịch (xem `privacy.rs`).
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![
                Subscription::new(autoxide_kernel::privacy::USER_DELETED, service),
                // Vụ việc của `compliance` ẩn / hiện lại mục (D-363, `restriction.rs`).
                Subscription::new(
                    autoxide_kernel::moderation::TARGET_RESTRICTED,
                    Arc::new(restriction::Listener),
                ),
                Subscription::new(
                    autoxide_kernel::moderation::TARGET_RESTORED,
                    Arc::new(restriction::Listener),
                ),
            ]
        })
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![
                JobRegistration::new(schedules::RUN, Arc::new(schedules::Runner(service.clone()))),
                JobRegistration::new(revisions::PRUNE, Arc::new(revisions::Prune(service.clone()))),
                JobRegistration::new(
                    translate::TRANSLATE,
                    Arc::new(translate::Translator(service.clone())),
                ),
                JobRegistration::new(semantic::INDEX, Arc::new(semantic::Indexer(service.clone()))),
                JobRegistration::new(semantic::CATCH_UP, Arc::new(semantic::CatchUp(service))),
            ]
        })
    }

    fn schedules(&self) -> Vec<Schedule> {
        let day = std::time::Duration::from_secs(24 * 3600);
        vec![
            Schedule::new(revisions::PRUNE, revisions::PRUNE, day),
            // Lập index tìm kiếm cho mục còn thiếu (vừa bật embedding, đổi mô hình): mỗi giờ.
            Schedule::new(
                semantic::CATCH_UP,
                semantic::CATCH_UP,
                std::time::Duration::from_secs(3600),
            ),
        ]
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

    use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext};
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
        async fn find_many(
            &self,
            _tenant: &TenantContext,
            ids: &[Uuid],
        ) -> Result<Vec<MediaRef>, Error> {
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
                        alt_ai_assisted: false,
                        url: format!("/api/v1/media/{id}/content?v=1"),
                        variants: Vec::new(),
                    })
                })
                .collect())
        }

        /// `alt` theo ngôn ngữ: `[<ngôn ngữ>]`, để test thấy ảnh được tra theo ngôn ngữ nào.
        async fn find_many_in(
            &self,
            tenant: &TenantContext,
            ids: &[Uuid],
            locale: &str,
        ) -> Result<Vec<MediaRef>, Error> {
            let mut found = self.find_many(tenant, ids).await?;
            for item in &mut found {
                item.alt = format!("[{locale}]");
            }
            Ok(found)
        }
    }

    pub fn config(pairs: &[(&str, &str)]) -> ContentConfig {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = ContentConfig::read(&mut reader);
        reader.finish(config).unwrap().0
    }

    /// Service trên database (đã migrate) của test; AI và embedding tắt.
    pub async fn service(
        test: &TestDatabase,
        config: ContentConfig,
    ) -> (Arc<ContentService>, Arc<FakeMedia>, Database) {
        let ai = Arc::new(autoxide_testkit::ai::FakeTextModel::disabled());
        let embedder = Arc::new(autoxide_testkit::ai::FakeEmbedder::disabled());
        service_with(test, config, ai, embedder).await
    }

    /// Như [`service`], với mô hình AI và embedding cho trước.
    pub async fn service_with(
        test: &TestDatabase,
        config: ContentConfig,
        ai: Arc<dyn autoxide_kernel::ai::TextModel>,
        embedder: Arc<dyn autoxide_kernel::ai::Embedder>,
    ) -> (Arc<ContentService>, Arc<FakeMedia>, Database) {
        build(test, config, ai, embedder, None).await
    }

    /// Như [`service_with`], tra khách qua sổ và đọc cấu hình theo khách của `app` (một `TestApp`
    /// nhiều khách): cấu hình đặt qua REST của `app` có hiệu lực ở service này.
    pub async fn service_in(
        test: &TestDatabase,
        config: ContentConfig,
        ai: Arc<dyn autoxide_kernel::ai::TextModel>,
        embedder: Arc<dyn autoxide_kernel::ai::Embedder>,
        app: &autoxide_testkit::TestApp,
    ) -> (Arc<ContentService>, Arc<FakeMedia>, Database) {
        let settings = app.api::<dyn autoxide_kernel::tenancy::TenantSettings>().unwrap();
        build(test, config, ai, embedder, Some((app.registry().clone(), settings))).await
    }

    async fn build(
        test: &TestDatabase,
        config: ContentConfig,
        ai: Arc<dyn autoxide_kernel::ai::TextModel>,
        embedder: Arc<dyn autoxide_kernel::ai::Embedder>,
        tenancy: Option<(DatabaseRegistry, Arc<dyn autoxide_kernel::tenancy::TenantSettings>)>,
    ) -> (Arc<ContentService>, Arc<FakeMedia>, Database) {
        let (registry, settings) = tenancy.unzip();
        let settings =
            settings.unwrap_or_else(|| Arc::new(autoxide_testkit::FakeTenantSettings::default()));
        let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
        let mut reader = ConfigReader::new(&source);
        let kernel = Config::read(&mut reader);
        let (kernel, _) = reader.finish(kernel).unwrap();
        let database = Database::connect(&kernel.database).await.unwrap();
        // Bảng của kernel: cây tổ chức (bước 34) mà kiểm quyền theo nút đọc tới.
        autoxide_kernel::module::Modules::new(Vec::new())
            .unwrap()
            .migrator()
            .unwrap()
            .up(&database)
            .await
            .unwrap();
        Migrator::new(vec![ModuleMigrations::new("content", crate::migration::all())])
            .unwrap()
            .up(&database)
            .await
            .unwrap();
        let media = Arc::new(FakeMedia::default());
        let dependencies =
            crate::service::Dependencies { media: media.clone(), ai, embedder, settings };
        let service = ContentService::new(
            registry.unwrap_or_else(|| DatabaseRegistry::single(database.clone())),
            Outbox::standalone("content"),
            autoxide_kernel::job::JobQueue::new("content"),
            dependencies,
            config,
        );
        (Arc::new(service), media, database)
    }

    /// Người có các permission `permissions` (`content.` thêm sẵn).
    pub fn user(permissions: &[&str]) -> Principal {
        Principal::new(Uuid::now_v7(), permissions.iter().map(|p| format!("content.{p}")))
    }

    pub fn editor() -> Principal {
        user(&["read", "write", "publish", "delete", "manage"])
    }
}
