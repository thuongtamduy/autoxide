//! Module `comments`: bình luận hai cấp gắn vào đích bất kỳ (`target_type`, `target_id`), của
//! người dùng đăng nhập hoặc khách; pipeline kiểm duyệt có thứ tự qua bộ chống spam chung của
//! kernel (hai pha, D-119); báo cáo vi phạm, cấm, sửa trong thời hạn, xoá mềm giữ luồng; số đếm
//! theo đích; mở / đóng / ẩn đích; mail báo trả lời (chọn nhận, email đã xác minh).
//!
//! Chỉ API: website hiển thị khung bình luận (repo giao diện riêng). Mail đi qua `dyn Mailer` nên
//! module đăng ký SAU module `mail`; đích được kiểm qua sổ đối tượng của kernel (D-123) nên đăng
//! ký SAU module sở hữu đích (`content`). Hướng dẫn dùng: `docs/COMMENTS.md`.

#[cfg(any(test, feature = "bench-seed"))]
mod bench_seed;
mod blocks;
mod bulk;
mod config;
mod entity;
mod events;
mod http;
mod mcp;
mod migration;
mod moderation;
mod notify;
mod permissions;
mod posting;
mod privacy;
mod restriction;
mod retention;
mod service;
mod text;
mod threads;
mod tools;
mod work;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::antispam::SpamGuard;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::db::RecordInfo;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Event, Subscription};
use autoxide_kernel::http::BodyLimit;
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::mail::Mailer;
use autoxide_kernel::module::{Manifest, Module, StartContext};
use autoxide_kernel::privacy::PersonalData;
use autoxide_kernel::secret::SecretKey;

use crate::config::CommentsConfig;
use crate::service::{CommentsService, Parts};

/// Thân request lớn nhất của endpoint viết / sửa bình luận (chữ tối đa 20 000 ký tự).
const MAX_BODY_BYTES: usize = 128 * 1024;

/// Module `comments`, để binary đăng ký trong `crates/app/src/modules.rs` — SAU `mail` (gửi mail)
/// và module sở hữu đích (`content`).
pub fn module() -> impl Module {
    CommentsModule::default()
}

#[derive(Default)]
struct CommentsModule {
    /// Có sau `configure`.
    config: Option<CommentsConfig>,
}

// Hợp đồng module (D-20261126-module-manifest): `manifest` khai mọi thứ tĩnh (đọc lại sau `configure`, nên
// thời hạn lưu lấy được từ cấu hình), `start` dựng service rồi đăng ký những gì cần service.
impl Module for CommentsModule {
    fn manifest(&self) -> Manifest {
        use autoxide_kernel::module::navigation::{MobilePlacement, NavGroup, NavItem, Navigation};
        use autoxide_kernel::tenancy::Relocation;
        let navigation = Navigation::default()
            .group(NavGroup::new("marketing", "Marketing", "megaphone", 40))
            .item(
                NavItem::new("marketing.comments", "marketing", "Comments", 40)
                    .permissions(&[
                        crate::permissions::COMMENTS_READ,
                        crate::permissions::COMMENTS_MODERATE,
                    ])
                    .mobile(MobilePlacement::Primary, 40)
                    .counter("comments.pending"),
            );
        Manifest::new(
            "comments",
            "Comments and questions on any record (posts, products…), with threads, moderation, \
             reports, bans and answers.",
        )
        // Thử nghiệm (bước 70 P3): API và bảng còn có thể đổi; chỉ chạy khi `AUTOXIDE_MODULES` nêu tên.
        .stability(autoxide_kernel::module::Stability::Experimental)
        // Module buộc phải có (`context.require::<…>()?` trong `start`); bật module này thì chúng được
        // bật kèm (bước 70 P1). Thứ dùng "nếu có" (`.ok()`) không khai ở đây.
        .depends_on(["mail"])
        // Tách / gộp khách (bước 38, D-20261118-step38-relocation).
        .relocation(
            Relocation::new()
                .stay(&[
                    "comments_comments",
                    "comments_targets",
                    "comments_bans",
                    "comments_reports",
                    "comments_reply_subscriptions",
                    "comments_profiles",
                ])
                .ephemeral(&["comments_throttle", "comments_tokens", "comments_antispam_nonces"]),
        )
        .migrations(migration::all)
        .records([
            RecordInfo::of::<entity::comment::Model>(),
            RecordInfo::of::<entity::target::Model>(),
            RecordInfo::of::<entity::ban::Model>(),
            RecordInfo::of::<entity::report::Model>(),
            RecordInfo::of::<entity::subscription::Model>(),
            RecordInfo::of::<entity::profile::Model>(),
        ])
        .permissions(permissions::all())
        // Khối cho trang dựng bằng khối (bước 67): hạng thử nghiệm, khai trong `unstable`.
        .unstable(|unstable| unstable.blocks(blocks::all()))
        .events(events::with_activity())
        .retention(self.retention())
        .tenant_settings(config::tenant_settings())
        .translation("vi", include_str!("../locales/vi.json"))
        .schedules([Schedule::new(
            retention::CLEANUP,
            retention::CLEANUP,
            std::time::Duration::from_secs(24 * 3600),
        )])
        .body_limits([
            BodyLimit::post("/public/comments", MAX_BODY_BYTES),
            BodyLimit::patch("/public/comments/{id}", MAX_BODY_BYTES),
        ])
        .navigation(navigation)
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(CommentsConfig::read(reader)?);
        Ok(())
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `comments` started before it was configured"))?;
        // Nghe event vòng đời do module sở hữu đích khai (`Module::resource_events`) cho các loại
        // đích được bật — kernel đã kiểm event có thật, nên không cần biết module nào có mặt.
        let owner_events =
            posting::targets::owner_events(context.resource_events(), &config.target_types);
        let databases: autoxide_kernel::tenancy::DatabaseRegistry = context.kernel()?;
        let parts = Parts {
            databases: databases.clone(),
            outbox: context.kernel()?,
            guard: context.require::<SpamGuard>()?,
            secret: (*context.require::<SecretKey>()?).clone(),
            mailer: context.require::<dyn Mailer>()?,
            translations: context.translations().clone(),
            resources: context.kernel()?,
            verification: context.kernel()?,
            settings: context.require::<dyn autoxide_kernel::tenancy::TenantSettings>()?,
            links: context
                .require::<autoxide_kernel::tenancy::SiteLinks>()
                .map(|links| (*links).clone())
                .unwrap_or_default(),
        };
        let service = Arc::new(CommentsService::new(parts, config));
        // Vụ việc của `compliance` kiểm đích và bên bị phản ánh qua sổ đối tượng (D-363).
        context.resource_type(restriction::COMMENT, Arc::new(restriction::CommentResolver))?;
        context.contribute::<dyn PersonalData>(service.clone())?;
        context.contribute::<dyn autoxide_kernel::work::WorkSource>(Arc::new(work::Pending))?;
        context
            .contribute::<dyn autoxide_kernel::summary::SummaryMetric>(Arc::new(work::Pending))?;
        for action in bulk::actions(&service) {
            context.contribute::<dyn autoxide_kernel::bulk::BulkAction>(action)?;
        }
        // Trả lời được hiện: mail báo cho người theo dõi (`notify.rs`).
        context.subscribe(Subscription::new(
            events::CommentPublished::NAME,
            Arc::new(notify::ReplyNotifier(service.clone())),
        ))?;
        // Tài khoản bị xoá: xoá mềm bình luận, bỏ `user_id` (`privacy.rs`).
        context.subscribe(Subscription::new(
            autoxide_kernel::privacy::USER_DELETED,
            service.clone(),
        ))?;
        // Vụ việc của `compliance` ẩn / hiện lại bình luận (D-363, `restriction.rs`).
        for event in [
            autoxide_kernel::moderation::TARGET_RESTRICTED,
            autoxide_kernel::moderation::TARGET_RESTORED,
        ] {
            context.subscribe(Subscription::new(
                event,
                Arc::new(restriction::Listener(service.clone())),
            ))?;
        }
        let owner = Arc::new(posting::targets::OwnerEvents {
            service: service.clone(),
            events: owner_events.clone(),
        });
        for event in &owner_events {
            context.subscribe(Subscription::new(event.event, owner.clone()))?;
        }
        context.job(JobRegistration::new(
            retention::CLEANUP,
            Arc::new(retention::Cleanup { service: service.clone(), databases: databases.clone() }),
        ))?;
        // Lệnh sinh dữ liệu đo (bước 39 B1): chỉ có khi bật cargo feature `bench-seed`.
        #[cfg(feature = "bench-seed")]
        context.task(bench_seed::task(databases))?;
        for resource in mcp::resources() {
            context.mcp_resource(resource)?;
        }
        for prompt in mcp::prompts() {
            context.prompt(prompt)?;
        }
        context.tools(tools::all(&service))?;
        context.routes(http::routes(service))
    }

    // Chế độ nghỉ (bước 70 P9): module tắt vẫn xuất bình luận của người dùng và xoá chúng khi tài
    // khoản bị xoá — cùng hàm với lúc bật, không cần service (xem `privacy.rs`).
    fn dormant(
        &self,
        context: &mut autoxide_kernel::module::DormantContext<'_>,
    ) -> Result<(), Error> {
        context.personal_data(Arc::new(privacy::DormantPrivacy::new()))?;
        context.on_user_deleted(Arc::new(privacy::DormantPrivacy::new()))
    }
}

impl CommentsModule {
    // Thời hạn lưu dữ liệu cá nhân (bước 37 C5, D-360), đọc từ cấu hình đang có.
    fn retention(&self) -> Vec<autoxide_kernel::privacy::RetentionPolicy> {
        use autoxide_kernel::privacy::{DataCategory, Keep, RetentionPolicy};
        let Some(config) = &self.config else {
            return Vec::new();
        };
        vec![
            RetentionPolicy::new(
                "comments.published",
                "Comments that were shown (author, text); kept until a moderator or the author \
                 deletes them, or the account is deleted.",
                DataCategory::Basic,
                Keep::UntilDeleted,
            ),
            RetentionPolicy::new(
                "comments.unpublished",
                "Comments never shown (waiting for approval or spam).",
                DataCategory::Basic,
                Keep::Days(config.retention_days),
            )
            .setting(config::RETENTION_DAYS.name),
            RetentionPolicy::new(
                "comments.ip_hashes",
                "Hashed network address of comments and guest reports.",
                DataCategory::Security,
                Keep::Days(config.ip_retention_days),
            )
            .setting(config::IP_RETENTION_DAYS.name),
        ]
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! Đồ dùng chung cho test trong crate này.

    /// Bên gửi mail giả dùng chung (testkit).
    pub use autoxide_testkit::FakeMailer;

    use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext, TenantSettings};
    use std::sync::Arc;
    use std::time::Duration;

    use autoxide_kernel::access::{PostingVerification, Principal};
    use autoxide_kernel::antispam::{AntispamConfig, SpamGuard};
    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::database::Database;
    use autoxide_kernel::error::Error;
    use autoxide_kernel::event::Outbox;
    use autoxide_kernel::i18n::Translations;

    use autoxide_kernel::migration::{Migrator, ModuleMigrations};
    use autoxide_kernel::resource::{Resource, ResourceDirectory, ResourceResolver};
    use autoxide_kernel::secret::SecretKey;
    use autoxide_testkit::{TestApp, TestDatabase};
    use uuid::Uuid;

    use crate::config::CommentsConfig;
    use crate::service::{CommentsService, Parts};

    /// Bộ tra giả cho loại `content.entry`: đích `public-…` công khai, `draft-…` chưa công khai,
    /// còn lại không có.
    pub struct Entries;

    #[autoxide_kernel::async_trait]
    impl ResourceResolver for Entries {
        async fn resolve(
            &self,
            _tenant: &TenantContext,
            id: &str,
        ) -> Result<Option<Resource>, Error> {
            Ok((id.starts_with("public") || id.starts_with("draft")).then(|| {
                Resource::new("Hello world")
                    .public(id.starts_with("public"))
                    .url(Some(format!("/posts/{id}")))
            }))
        }
    }

    /// Cấu hình của test: mọi bình luận hiện ngay trừ khi test đổi; mail báo trả lời bật.
    pub fn config() -> CommentsConfig {
        CommentsConfig {
            target_types: vec!["content.entry".to_owned(), "media.item".to_owned()],
            verify_url: Some("https://www.example.com/verify?token={token}".to_owned()),
            unsubscribe_url: Some("https://www.example.com/unsubscribe?token={token}".to_owned()),
            site_url: Some("https://www.example.com".to_owned()),
            ..CommentsConfig::default()
        }
    }

    #[derive(Clone)]
    pub struct Harness {
        pub service: Arc<CommentsService>,
        pub mailer: Arc<FakeMailer>,
        pub database: Database,
        /// Khách mặc định trên database của test.
        pub tenant: TenantContext,
    }

    /// Service trên database (đã migrate) của test; chống spam không chờ 3 giây.
    pub async fn service(test: &TestDatabase, config: CommentsConfig) -> Harness {
        let antispam = AntispamConfig::default().min_fill(Duration::ZERO);
        service_with(test, config, antispam).await
    }

    /// Như [`service`], với cấu hình chống spam riêng.
    pub async fn service_with(
        test: &TestDatabase,
        config: CommentsConfig,
        antispam: AntispamConfig,
    ) -> Harness {
        build(test, config, antispam, None).await
    }

    /// Như [`service`], tra khách qua sổ và đọc cấu hình theo khách của `app` (một `TestApp`
    /// nhiều khách): cấu hình đặt qua REST của `app` có hiệu lực ở service này.
    pub async fn service_in(test: &TestDatabase, config: CommentsConfig, app: &TestApp) -> Harness {
        let antispam = AntispamConfig::default().min_fill(Duration::ZERO);
        let settings = app.api::<dyn TenantSettings>().unwrap();
        build(test, config, antispam, Some((app.registry().clone(), settings))).await
    }

    async fn build(
        test: &TestDatabase,
        config: CommentsConfig,
        antispam: AntispamConfig,
        tenancy: Option<(DatabaseRegistry, Arc<dyn TenantSettings>)>,
    ) -> Harness {
        let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
        let mut reader = ConfigReader::new(&source);
        let kernel = Config::read(&mut reader);
        let (kernel, _) = reader.finish(kernel).unwrap();
        let database = Database::connect(&kernel.database).await.unwrap();
        Migrator::new(vec![ModuleMigrations::new("comments", crate::migration::all())])
            .unwrap()
            .up(&database)
            .await
            .unwrap();
        let secret =
            SecretKey::new(database.clone(), Some(b"comments-test-secret-key-0123456789".to_vec()));
        let guard = Arc::new(SpamGuard::new(secret.clone(), antispam));
        let mailer = Arc::new(FakeMailer::default());
        let translations =
            Translations::new(vec![("comments", vec![("vi", include_str!("../locales/vi.json"))])])
                .unwrap();
        let resources = ResourceDirectory::default();
        resources.register("content", "content.entry", Arc::new(Entries)).unwrap();
        let (registry, settings) = tenancy.unzip();
        let settings =
            settings.unwrap_or_else(|| Arc::new(autoxide_testkit::FakeTenantSettings::default()));
        let parts = Parts {
            databases: registry.unwrap_or_else(|| DatabaseRegistry::single(database.clone())),
            outbox: Outbox::standalone("comments"),
            guard,
            secret,
            mailer: mailer.clone(),
            translations,
            resources,
            verification: PostingVerification::default(),
            links: autoxide_kernel::tenancy::SiteLinks::new(settings.clone()),
            settings,
        };
        let tenant = TenantContext::default_on(database.clone());
        let service = Arc::new(CommentsService::new(parts, config));
        Harness { service, mailer, database, tenant }
    }

    /// Người có các permission `permissions` (`comments.` thêm sẵn).
    pub fn user(permissions: &[&str]) -> Principal {
        Principal::new(Uuid::now_v7(), permissions.iter().map(|p| format!("comments.{p}")))
    }

    pub fn admin() -> Principal {
        user(&["post", "read", "moderate", "manage", "export"])
    }
}
