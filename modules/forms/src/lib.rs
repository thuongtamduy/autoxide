//! Module `forms`: định nghĩa form với danh sách kiểu trường đóng, nhận bài gửi công khai qua bộ
//! chống spam chung của kernel (hai pha, D-119), hộp thư quản trị, xuất CSV, mail báo quản trị
//! viên, xuất / xoá theo email, tự xoá theo thời hạn.
//!
//! Chỉ API: website hiển thị form và gửi bài (repo giao diện riêng). Mail báo đi qua `dyn
//! Mailer` nên module đăng ký SAU module `mail`. Hướng dẫn dùng: `docs/FORMS.md`.

mod config;
mod entity;
mod events;
mod forms;
mod http;
mod migration;
mod permissions;
mod privacy;
mod retention;
mod service;
mod submissions;
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

use crate::config::FormsConfig;
use crate::service::{FormsService, Parts};

/// Module `forms`, để binary đăng ký trong `crates/app/src/modules.rs` — SAU module `mail`.
pub fn module() -> impl Module {
    FormsModule::default()
}

#[derive(Default)]
struct FormsModule {
    /// Có sau `configure`.
    config: Option<FormsConfig>,
    /// Có sau `start`.
    service: Option<Arc<FormsService>>,
}

impl Module for FormsModule {
    fn name(&self) -> &'static str {
        "forms"
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
        self.config = Some(FormsConfig::read(reader)?);
        Ok(())
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `forms` started before it was configured"))?;
        let parts = Parts {
            database: context.database().clone(),
            outbox: context.outbox()?,
            guard: context.require::<SpamGuard>()?,
            secret: (*context.require::<SecretKey>()?).clone(),
            mailer: context.require::<dyn Mailer>()?,
            translations: context.translations().clone(),
        };
        let service = Arc::new(FormsService::new(parts, config));
        context.personal_data(service.clone())?;
        self.service = Some(service);
        Ok(())
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![
                // Bài vào hộp thư: mail báo quản trị viên (`submissions/notify.rs`).
                Subscription::new(
                    events::SubmissionReceived::NAME,
                    Arc::new(submissions::notify::Notifier(service.clone())),
                ),
                // Tài khoản bị xoá: bỏ `created_by` (`privacy.rs`).
                Subscription::new(autoxide_kernel::privacy::USER_DELETED, service),
            ]
        })
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
        let max = self.config.as_ref().map_or(65_536, |config| config.max_submission_bytes);
        vec![BodyLimit::new(Method::POST, "/public/{key}/submissions", max)]
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

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use autoxide_kernel::access::Principal;
    use autoxide_kernel::antispam::{AntispamConfig, SpamGuard};
    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::database::Database;
    use autoxide_kernel::error::{Error, FieldError};
    use autoxide_kernel::event::Outbox;
    use autoxide_kernel::i18n::Translations;
    use autoxide_kernel::mail::{Mail, Mailer};
    use autoxide_kernel::migration::{Migrator, ModuleMigrations};
    use autoxide_kernel::secret::SecretKey;
    use autoxide_testkit::TestDatabase;
    use uuid::Uuid;

    use crate::config::FormsConfig;
    use crate::service::{FormsService, Parts};

    /// Bên gửi mail giả: giữ lại mọi mail. Bật `refuse_reply_to` thì từ chối mọi mail có
    /// `reply_to`, như module `mail` từ chối địa chỉ nó không gửi được.
    #[derive(Default)]
    pub struct FakeMailer {
        pub mails: Mutex<Vec<Mail>>,
        pub refuse_reply_to: AtomicBool,
    }

    impl FakeMailer {
        pub fn sent(&self) -> Vec<Mail> {
            self.mails.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Mailer for FakeMailer {
        async fn send(&self, mail: Mail) -> Result<(), Error> {
            if mail.reply_to.is_some() && self.refuse_reply_to.load(Ordering::SeqCst) {
                return Err(Error::validation(vec![FieldError::new(
                    "reply_to",
                    "email",
                    "must be a mail address, like `ada@example.com`",
                )]));
            }
            self.mails.lock().unwrap().push(mail);
            Ok(())
        }
    }

    /// Service trên database (đã migrate) của test; chống spam không chờ 3 giây.
    pub async fn service(
        test: &TestDatabase,
        config: FormsConfig,
    ) -> (Arc<FormsService>, Arc<FakeMailer>, Database) {
        let antispam = AntispamConfig::default().min_fill(Duration::ZERO);
        service_with(test, config, antispam).await
    }

    /// Như [`service`], với cấu hình chống spam riêng.
    pub async fn service_with(
        test: &TestDatabase,
        config: FormsConfig,
        antispam: AntispamConfig,
    ) -> (Arc<FormsService>, Arc<FakeMailer>, Database) {
        let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
        let mut reader = ConfigReader::new(&source);
        let kernel = Config::read(&mut reader);
        let (kernel, _) = reader.finish(kernel).unwrap();
        let database = Database::connect(&kernel.database).await.unwrap();
        Migrator::new(vec![ModuleMigrations::new("forms", crate::migration::all())])
            .unwrap()
            .up(&database)
            .await
            .unwrap();
        let secret =
            SecretKey::new(database.clone(), Some(b"forms-test-secret-key-0123456789".to_vec()));
        let guard = Arc::new(SpamGuard::new(secret.clone(), antispam));
        let mailer = Arc::new(FakeMailer::default());
        let translations =
            Translations::new(vec![("forms", vec![("vi", include_str!("../locales/vi.json"))])])
                .unwrap();
        let parts = Parts {
            database: database.clone(),
            outbox: Outbox::standalone("forms"),
            guard,
            secret,
            mailer: mailer.clone(),
            translations,
        };
        (Arc::new(FormsService::new(parts, config)), mailer, database)
    }

    /// Người có các permission `permissions` (`forms.` thêm sẵn).
    pub fn user(permissions: &[&str]) -> Principal {
        Principal::new(Uuid::now_v7(), permissions.iter().map(|p| format!("forms.{p}")))
    }

    /// Mọi quyền quản trị của module (không có `forms.submit`: quản trị viên gửi form như mọi
    /// người).
    pub fn admin() -> Principal {
        user(&["read", "manage", "review", "export"])
    }
}
