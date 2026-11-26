//! Module `mail`: gửi mail cho mọi module (D-061).
//!
//! Module khác gọi `dyn Mailer` (`autoxide_kernel::mail`); mail vào job queue của module
//! này và được gửi nền, lỗi thì thử lại. Có `AUTOXIDE_MAIL_SMTP_URL` thì gửi qua SMTP;
//! không có thì ghi file `.eml` vào `AUTOXIDE_MAIL_FILE_DIR` (cho dev và test).

pub mod api;
mod bounces;
mod client_config;
mod config;
mod doctor;
mod service;
mod transport;

#[cfg(test)]
mod bounce_tests;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::EventInfo;
use autoxide_kernel::job::JobRegistration;
use autoxide_kernel::mail::Mailer;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};

use crate::config::MailConfig;
use crate::service::{MailService, Sender};
use crate::transport::Transport;

/// Module `mail`, để binary đăng ký trong `crates/app/src/modules.rs` — TRƯỚC các module
/// gửi mail.
pub fn module() -> impl Module {
    MailModule::default()
}

#[derive(Default)]
struct MailModule {
    /// Có sau `configure`.
    config: Option<MailConfig>,
    /// Có sau `start`.
    sender: Option<Arc<Sender>>,
    /// Bước kiểm của `autoxide doctor`; có sau `start`.
    probe: Option<Arc<doctor::SmtpProbe>>,
    /// Sổ thư bị dội; có sau `start`.
    bounces: Option<Arc<bounces::Bounces>>,
}

impl Module for MailModule {
    fn name(&self) -> &'static str {
        "mail"
    }

    fn description(&self) -> &'static str {
        "Outgoing mail for every module, queued and retried, over SMTP or to files in development."
    }

    // Module nền: không gói nào tắt được (bước 35, D-323).
    fn always_enabled(&self) -> bool {
        true
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            autoxide_kernel::job::jobs_migration("m0001_create_jobs", "mail"),
            // Bảng `mail_events` (outbox) cho event mail bị từ chối hẳn.
            autoxide_kernel::event::outbox_migration("m0002_create_events", "mail"),
            autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "mail"),
            // Bảng phụ trợ của kernel (event, job, throttle, token, nonce) mang `tenant_id` (D-253).
            autoxide_kernel::tenancy::migration::adopt(
                "m0004_tenancy_helpers",
                vec![
                    autoxide_kernel::job::tenant_table("mail"),
                    autoxide_kernel::event::tenant_table("mail"),
                ],
            ),
            // Đường người làm vào khách (bước 34, D-310).
            autoxide_kernel::event::outbox_access_migration("k0001_event_access", "mail"),
            // Sổ thư bị dội và danh sách chặn (D-20261125-mail-bounces).
            Box::new(bounces::migration::Migration),
        ]
    }

    // Tách / gộp khách (bước 38): sổ dội không đi theo — khách mới học lại từ lần dội kế (gộp mà mang theo thì đụng
    // khoá duy nhất (khách, địa chỉ) khi hai khách cùng có một địa chỉ).
    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        autoxide_kernel::tenancy::Relocation::new().ephemeral(&["mail_bounces"])
    }

    fn permissions(&self) -> Vec<autoxide_kernel::access::Permission> {
        vec![bounces::BOUNCES_READ, bounces::BOUNCES_MANAGE]
    }

    fn retention(&self) -> Vec<autoxide_kernel::privacy::RetentionPolicy> {
        use autoxide_kernel::privacy::{DataCategory, Keep, RetentionPolicy};
        let days = self.config.as_ref().map_or(365, |config| config.bounce_retention_days);
        vec![
            RetentionPolicy::new(
                "mail.bounces",
                "Email addresses whose mail bounced, why, and whether they are suppressed.",
                DataCategory::Basic,
                Keep::Days(days),
            )
            .setting(config::BOUNCE_RETENTION_DAYS.name),
        ]
    }

    fn routes(&self) -> utoipa_axum::router::OpenApiRouter {
        self.bounces
            .clone()
            .map_or_else(utoipa_axum::router::OpenApiRouter::new, bounces::http::routes)
    }

    fn schedules(&self) -> Vec<autoxide_kernel::job::Schedule> {
        let day = std::time::Duration::from_secs(24 * 3600);
        vec![autoxide_kernel::job::Schedule::new(bounces::CLEANUP, bounces::CLEANUP, day)]
    }

    fn events(&self) -> Vec<EventInfo> {
        vec![
            bounces::ADDRESS_SUPPRESSED,
            bounces::SUPPRESSION_RELEASED,
            EventInfo::new(
                autoxide_kernel::mail::DELIVERY_FAILED,
                "A mail was refused for good because of its recipient address (SMTP enhanced status \
             5.1.x or 5.2.x: no such mailbox, for example); `to` is the address, `dedupe_key` \
             the sender's key.",
            ),
        ]
    }

    fn client_config(
        &self,
    ) -> Result<Option<autoxide_kernel::module::ClientSection>, autoxide_kernel::error::Error> {
        self.config.as_ref().map_or(Ok(None), client_config::section)
    }

    fn tenant_settings(&self) -> Vec<autoxide_kernel::tenancy::TenantSetting> {
        vec![service::TENANT_SMTP_URL, service::TENANT_FROM]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(MailConfig::read(reader)?);
        Ok(())
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `mail` started before it was configured"))?;
        if config.smtp_url.is_none() {
            tracing::warn!(
                directory = %config.file_dir.display(),
                "AUTOXIDE_MAIL_SMTP_URL is not set: mail is written to files instead of being \
                 sent; set it for production"
            );
        }
        service::allow_private_networks(config.allow_private_networks);
        let transport = Arc::new(Transport::new(config.smtp_url.as_deref(), config.file_dir)?);
        self.probe = Some(Arc::new(doctor::SmtpProbe { transport: transport.clone() }));
        let settings = context.require::<dyn autoxide_kernel::tenancy::TenantSettings>()?;
        let bounces = Arc::new(bounces::Bounces {
            hard_limit: config.hard_bounce_limit,
            retention_days: config.bounce_retention_days,
            outbox: context.outbox()?,
        });
        self.bounces = Some(bounces.clone());
        let sender = Arc::new(Sender::new(
            transport,
            config.from,
            context.outbox()?,
            settings,
            bounces.clone(),
            config.allow_private_networks,
        ));
        // Thử kết nối SMTP từ trang cài đặt (`POST /kernel/settings/{name}/test`).
        context.connection_probe(sender.clone())?;
        self.sender = Some(sender);
        let service = MailService::new(context.jobs(), bounces);
        context.provide::<dyn Mailer>(Arc::new(service))
    }

    fn checks(&self) -> Vec<autoxide_kernel::doctor::Check> {
        self.probe.clone().map_or_else(Vec::new, |probe| {
            vec![autoxide_kernel::doctor::Check::new(
                "smtp",
                "Mail can be handed to the configured SMTP server.",
                probe,
            )]
        })
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        let mut jobs = Vec::new();
        if let Some(sender) = &self.sender {
            jobs.push(JobRegistration::new(service::SEND, sender.clone()));
        }
        if let Some(bounces) = &self.bounces {
            jobs.push(JobRegistration::new(
                bounces::CLEANUP,
                Arc::new(bounces::Cleanup(bounces.clone())),
            ));
        }
        jobs
    }
}
