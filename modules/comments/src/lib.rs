//! Module `comments`: bình luận hai cấp gắn vào đích bất kỳ (`target_type`, `target_id`), của
//! người dùng đăng nhập hoặc khách; pipeline kiểm duyệt có thứ tự qua bộ chống spam chung của
//! kernel (hai pha, D-119); báo cáo vi phạm, cấm, sửa trong thời hạn, xoá mềm giữ luồng; số đếm
//! theo đích; mở / đóng / ẩn đích; mail báo trả lời (chọn nhận, email đã xác minh).
//!
//! Chỉ API: website hiển thị khung bình luận (repo giao diện riêng). Mail đi qua `dyn Mailer` nên
//! module đăng ký SAU module `mail`; đích được kiểm qua sổ đối tượng của kernel (D-123) nên đăng
//! ký SAU module sở hữu đích (`content`). Hướng dẫn dùng: `docs/COMMENTS.md`.

mod authoring;
mod config;
mod entity;
mod events;
mod http;
mod migration;
mod moderation;
mod notify;
mod permissions;
mod posting;
mod privacy;
mod reports;
mod retention;
mod service;
mod targets;
mod text;
mod threads;
mod tools;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::antispam::SpamGuard;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Event, EventInfo, Subscription};
use autoxide_kernel::http::BodyLimit;
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::mail::Mailer;
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::secret::SecretKey;
use axum::http::Method;
use utoipa_axum::router::OpenApiRouter;

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
    /// Có sau `start`.
    service: Option<Arc<CommentsService>>,
    /// Event "đã lưu trữ / đã xoá" của module sở hữu đích có trong chương trình.
    owner_events: Vec<&'static str>,
}

impl Module for CommentsModule {
    fn name(&self) -> &'static str {
        "comments"
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
        self.config = Some(CommentsConfig::read(reader)?);
        Ok(())
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `comments` started before it was configured"))?;
        // Nghe event của module sở hữu đích bằng tên — chỉ khi module đó có trong chương trình
        // (đăng ký nhận một event không ai khai là lỗi khởi động) và loại đích được bật.
        self.owner_events = targets::OWNER_EVENTS
            .iter()
            .filter(|(name, kind, _)| {
                context.events().contains(name)
                    && config.target_types.iter().any(|known| known == kind)
            })
            .map(|(name, _, _)| *name)
            .collect();
        let parts = Parts {
            database: context.database().clone(),
            outbox: context.outbox()?,
            guard: context.require::<SpamGuard>()?,
            secret: (*context.require::<SecretKey>()?).clone(),
            mailer: context.require::<dyn Mailer>()?,
            translations: context.translations().clone(),
            resources: context.resources(),
        };
        let service = Arc::new(CommentsService::new(parts, config));
        context.personal_data(service.clone())?;
        self.service = Some(service);
        Ok(())
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        let Some(service) = self.service.clone() else {
            return Vec::new();
        };
        let mut subscriptions = vec![
            // Trả lời được hiện: mail báo cho người theo dõi (`notify.rs`).
            Subscription::new(
                events::CommentPublished::NAME,
                Arc::new(notify::ReplyNotifier(service.clone())),
            ),
            // Tài khoản bị xoá: xoá mềm bình luận, bỏ `user_id` (`privacy.rs`).
            Subscription::new(autoxide_kernel::privacy::USER_DELETED, service.clone()),
        ];
        for name in &self.owner_events {
            subscriptions
                .push(Subscription::new(name, Arc::new(targets::OwnerEvents(service.clone()))));
        }
        subscriptions
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![JobRegistration::new(retention::CLEANUP, Arc::new(retention::Cleanup(service)))]
        })
    }

    fn schedules(&self) -> Vec<Schedule> {
        let day = std::time::Duration::from_secs(24 * 3600);
        vec![Schedule::new(retention::CLEANUP, retention::CLEANUP, day)]
    }

    fn body_limits(&self) -> Vec<BodyLimit> {
        vec![
            BodyLimit::new(Method::POST, "/public/comments", MAX_BODY_BYTES),
            BodyLimit::new(Method::PATCH, "/public/comments/{id}", MAX_BODY_BYTES),
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

    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use autoxide_kernel::access::Principal;
    use autoxide_kernel::antispam::{AntispamConfig, SpamGuard};
    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::database::Database;
    use autoxide_kernel::error::Error;
    use autoxide_kernel::event::Outbox;
    use autoxide_kernel::i18n::Translations;
    use autoxide_kernel::mail::{Mail, Mailer};
    use autoxide_kernel::migration::{Migrator, ModuleMigrations};
    use autoxide_kernel::resource::{Resource, ResourceDirectory, ResourceResolver};
    use autoxide_kernel::secret::SecretKey;
    use autoxide_testkit::TestDatabase;
    use uuid::Uuid;

    use crate::config::CommentsConfig;
    use crate::service::{CommentsService, Parts};

    /// Bên gửi mail giả: giữ lại mọi mail, bỏ mail trùng `dedupe_key` như module `mail` thật.
    #[derive(Default)]
    pub struct FakeMailer(pub Mutex<Vec<Mail>>);

    impl FakeMailer {
        pub fn sent(&self) -> Vec<Mail> {
            self.0.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Mailer for FakeMailer {
        async fn send(&self, mail: Mail) -> Result<(), Error> {
            let mut mails = self.0.lock().unwrap();
            if mail.dedupe_key.is_none()
                || mails.iter().all(|sent| sent.dedupe_key != mail.dedupe_key)
            {
                mails.push(mail);
            }
            Ok(())
        }
    }

    /// Bộ tra giả cho loại `content.entry`: đích `public-…` công khai, `draft-…` chưa công khai,
    /// còn lại không có.
    pub struct Entries;

    #[async_trait::async_trait]
    impl ResourceResolver for Entries {
        async fn resolve(&self, id: &str) -> Result<Option<Resource>, Error> {
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

    pub struct Harness {
        pub service: Arc<CommentsService>,
        pub mailer: Arc<FakeMailer>,
        pub database: Database,
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
        let parts = Parts {
            database: database.clone(),
            outbox: Outbox::standalone("comments"),
            guard,
            secret,
            mailer: mailer.clone(),
            translations,
            resources,
        };
        Harness { service: Arc::new(CommentsService::new(parts, config)), mailer, database }
    }

    /// Người có các permission `permissions` (`comments.` thêm sẵn).
    pub fn user(permissions: &[&str]) -> Principal {
        Principal::new(Uuid::now_v7(), permissions.iter().map(|p| format!("comments.{p}")))
    }

    pub fn admin() -> Principal {
        user(&["post", "read", "moderate", "manage", "export"])
    }
}
