//! Module `newsletter`: danh sách bản tin; người đăng ký với xác nhận hai bước; huỷ một chạm
//! (RFC 8058) và trang lựa chọn; chiến dịch (nháp, hẹn giờ, gửi, tạm dừng, tiếp tục, huỷ) gửi
//! theo lô có giới hạn tốc độ, không gửi trùng khi chạy lại; danh sách chặn và bounce; chiến dịch
//! tự tạo khi `content` đăng bài (tuỳ chọn theo danh sách). Không đo lượt mở / lượt bấm.
//!
//! Chỉ API: website hiển thị ô đăng ký, trang xác nhận, trang lựa chọn (repo giao diện riêng).
//! Mail đi qua `dyn Mailer` nên module đăng ký SAU module `mail`; bài mới lấy qua sổ đối tượng
//! của kernel nên đăng ký SAU `content`. Hướng dẫn dùng: `docs/NEWSLETTER.md`.

mod audience;
mod campaigns;
mod config;
mod entity;
mod events;
mod http;
mod links;
mod migration;
mod permissions;
mod privacy;
mod public;
mod retention;
mod service;
mod text;
mod tools;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::antispam::SpamGuard;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventInfo, Subscription};
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::mail::Mailer;
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::public_url::PublicUrl;
use autoxide_kernel::secret::SecretKey;
use utoipa_axum::router::OpenApiRouter;

use crate::config::NewsletterConfig;
use crate::service::{NewsletterService, Parts};

/// Module `newsletter`, để binary đăng ký trong `crates/app/src/modules.rs` — SAU `mail` (gửi
/// mail) và `content` (bài mới).
pub fn module() -> impl Module {
    NewsletterModule::default()
}

#[derive(Default)]
struct NewsletterModule {
    /// Có sau `configure`.
    config: Option<NewsletterConfig>,
    /// Có sau `start`.
    service: Option<Arc<NewsletterService>>,
    /// Event của module khác có trong chương trình (chỉ nghe khi có người phát).
    listens_published: bool,
    listens_failures: bool,
    /// Sổ database, có sau `start`: job dọn dẹp tra ngữ cảnh từng khách.
    databases: Option<autoxide_kernel::tenancy::DatabaseRegistry>,
}

impl Module for NewsletterModule {
    fn name(&self) -> &'static str {
        "newsletter"
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
        self.config = Some(NewsletterConfig::read(reader)?);
        Ok(())
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self.config.clone().ok_or_else(|| {
            Error::internal("module `newsletter` started before it was configured")
        })?;
        let parts = Parts {
            outbox: context.outbox()?,
            guard: context.require::<SpamGuard>()?,
            secret: (*context.require::<SecretKey>()?).clone(),
            mailer: context.require::<dyn Mailer>()?,
            translations: context.translations().clone(),
            jobs: context.jobs(),
            public_url: context.require::<PublicUrl>()?,
            resources: context.resources(),
            databases: context.tenancy(),
        };
        // Nghe event của module khác bằng tên — chỉ khi module đó có trong chương trình (đăng ký
        // nhận một event không ai khai là lỗi khởi động).
        self.listens_published = context.events().contains(campaigns::new_posts::ENTRY_PUBLISHED);
        self.listens_failures = context.events().contains(autoxide_kernel::mail::DELIVERY_FAILED);
        let service = Arc::new(NewsletterService::new(parts, config));
        context.personal_data(service.clone())?;
        self.service = Some(service);
        self.databases = Some(context.tenancy());
        Ok(())
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        let Some(service) = self.service.clone() else {
            return Vec::new();
        };
        let mut subscriptions = vec![
            // Tài khoản bị xoá: bỏ `created_by` (`privacy.rs`).
            Subscription::new(autoxide_kernel::privacy::USER_DELETED, service.clone()),
        ];
        if self.listens_published {
            subscriptions.push(Subscription::new(
                campaigns::new_posts::ENTRY_PUBLISHED,
                Arc::new(campaigns::new_posts::NewPostHandler(service.clone())),
            ));
        }
        if self.listens_failures {
            subscriptions.push(Subscription::new(
                autoxide_kernel::mail::DELIVERY_FAILED,
                Arc::new(audience::bounces::DeliveryFailures(service)),
            ));
        }
        subscriptions
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        match (self.service.clone(), self.databases.clone()) {
            (Some(service), Some(databases)) => vec![
                JobRegistration::new(
                    campaigns::sending::PREPARE,
                    Arc::new(campaigns::sending::Preparer(service.clone())),
                ),
                JobRegistration::new(
                    campaigns::sending::SEND,
                    Arc::new(campaigns::sending::Sender(service.clone())),
                ),
                JobRegistration::new(
                    public::SUBSCRIBE,
                    Arc::new(public::Subscriber(service.clone())),
                ),
                JobRegistration::new(
                    retention::CLEANUP,
                    Arc::new(retention::Cleanup { service, databases }),
                ),
            ],
            _ => Vec::new(),
        }
    }

    fn schedules(&self) -> Vec<Schedule> {
        let day = std::time::Duration::from_secs(24 * 3600);
        vec![Schedule::new(retention::CLEANUP, retention::CLEANUP, day)]
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use autoxide_kernel::access::Principal;
    use autoxide_kernel::antispam::{AntispamConfig, SpamGuard};
    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::database::Database;
    use autoxide_kernel::error::Error;
    use autoxide_kernel::event::Outbox;
    use autoxide_kernel::i18n::Translations;
    use autoxide_kernel::job::JobQueue;
    use autoxide_kernel::mail::{Mail, Mailer};
    use autoxide_kernel::migration::{Migrator, ModuleMigrations};
    use autoxide_kernel::public_url::PublicUrl;
    use autoxide_kernel::resource::ResourceDirectory;
    use autoxide_kernel::secret::SecretKey;
    use autoxide_testkit::TestDatabase;
    use uuid::Uuid;

    use crate::config::NewsletterConfig;
    use crate::service::{NewsletterService, Parts};

    /// Bên gửi mail giả: giữ lại mọi mail, bỏ mail trùng `dedupe_key` như module `mail` thật.
    #[derive(Default)]
    pub struct FakeMailer {
        pub mails: Mutex<Vec<Mail>>,
        /// Số lần `send` được gọi (kể cả mail trùng khoá bị bỏ).
        pub calls: AtomicUsize,
    }

    impl FakeMailer {
        pub fn sent(&self) -> Vec<Mail> {
            self.mails.lock().unwrap().clone()
        }

        pub fn to(&self, address: &str) -> Vec<Mail> {
            self.sent().into_iter().filter(|mail| mail.to == address).collect()
        }

        pub fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl Mailer for FakeMailer {
        async fn send(&self, _tenant: &TenantContext, mail: Mail) -> Result<(), Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut mails = self.mails.lock().unwrap();
            let duplicate = mail.dedupe_key.is_some()
                && mails.iter().any(|sent| sent.dedupe_key == mail.dedupe_key);
            if !duplicate {
                mails.push(mail);
            }
            Ok(())
        }
    }

    /// Cấu hình đủ để gửi: trang xác nhận, trang lựa chọn, website, địa chỉ người gửi.
    pub fn config() -> NewsletterConfig {
        NewsletterConfig {
            confirm_url: Some("https://www.example.com/confirm?token={token}".to_owned()),
            preferences_url: Some("https://www.example.com/preferences?token={token}".to_owned()),
            site_url: Some("https://www.example.com".to_owned()),
            postal_address: Some("Example Ltd, 1 Example Street, Hanoi".to_owned()),
            ..NewsletterConfig::default()
        }
    }

    #[derive(Clone)]
    pub struct Harness {
        pub service: Arc<NewsletterService>,
        pub mailer: Arc<FakeMailer>,
        pub database: Database,
        /// Khách mặc định trên database của test.
        pub tenant: TenantContext,
        pub resources: ResourceDirectory,
    }

    /// Service trên database (đã migrate) của test; chống spam không chờ 3 giây.
    pub async fn service(test: &TestDatabase, config: NewsletterConfig) -> Harness {
        let antispam = AntispamConfig::default().min_fill(Duration::ZERO);
        service_with(test, config, antispam).await
    }

    /// Như [`service`], với cấu hình chống spam riêng.
    pub async fn service_with(
        test: &TestDatabase,
        config: NewsletterConfig,
        antispam: AntispamConfig,
    ) -> Harness {
        build(test, config, antispam, None).await
    }

    /// Như [`service`], tra khách qua sổ `registry` (vd sổ nhiều khách của một `TestApp`).
    pub async fn service_in(
        test: &TestDatabase,
        config: NewsletterConfig,
        registry: DatabaseRegistry,
    ) -> Harness {
        let antispam = AntispamConfig::default().min_fill(Duration::ZERO);
        build(test, config, antispam, Some(registry)).await
    }

    async fn build(
        test: &TestDatabase,
        config: NewsletterConfig,
        antispam: AntispamConfig,
        registry: Option<DatabaseRegistry>,
    ) -> Harness {
        let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
        let mut reader = ConfigReader::new(&source);
        let kernel = Config::read(&mut reader);
        let (kernel, _) = reader.finish(kernel).unwrap();
        let database = Database::connect(&kernel.database).await.unwrap();
        Migrator::new(vec![ModuleMigrations::new("newsletter", crate::migration::all())])
            .unwrap()
            .up(&database)
            .await
            .unwrap();
        let secret = SecretKey::new(
            database.clone(),
            Some(b"newsletter-test-secret-key-0123456789".to_vec()),
        );
        let guard = Arc::new(SpamGuard::new(secret.clone(), antispam));
        let mailer = Arc::new(FakeMailer::default());
        let translations = Translations::new(vec![(
            "newsletter",
            vec![("vi", include_str!("../locales/vi.json"))],
        )])
        .unwrap();
        let resources = ResourceDirectory::default();
        let parts = Parts {
            outbox: Outbox::standalone("newsletter"),
            guard,
            secret,
            mailer: mailer.clone(),
            translations,
            jobs: JobQueue::new("newsletter"),
            public_url: Arc::new(PublicUrl::new(Some("https://api.example.com".to_owned()))),
            resources: resources.clone(),
            databases: registry.unwrap_or_else(|| DatabaseRegistry::single(database.clone())),
        };
        let service = Arc::new(NewsletterService::new(parts, config));
        let tenant = TenantContext::default_on(database.clone());
        Harness { service, mailer, database, tenant, resources }
    }

    /// Người có các permission `permissions` (`newsletter.` thêm sẵn).
    pub fn user(permissions: &[&str]) -> Principal {
        Principal::new(Uuid::now_v7(), permissions.iter().map(|p| format!("newsletter.{p}")))
    }

    pub fn admin() -> Principal {
        user(&["read", "manage", "send", "export"])
    }
}
