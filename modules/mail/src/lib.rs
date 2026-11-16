//! Module `mail`: gửi mail cho mọi module (D-061).
//!
//! Module khác gọi `dyn Mailer` (`autoxide_kernel::mail`); mail vào job queue của module
//! này và được gửi nền, lỗi thì thử lại. Có `AUTOXIDE_MAIL_SMTP_URL` thì gửi qua SMTP;
//! không có thì ghi file `.eml` vào `AUTOXIDE_MAIL_FILE_DIR` (cho dev và test).

pub mod api;
mod client_config;
mod config;
mod doctor;
mod service;
mod transport;

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
}

impl Module for MailModule {
    fn name(&self) -> &'static str {
        "mail"
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
        ]
    }

    fn events(&self) -> Vec<EventInfo> {
        vec![EventInfo::new(
            autoxide_kernel::mail::DELIVERY_FAILED,
            "A mail was refused for good because of its recipient address (SMTP enhanced status \
             5.1.x or 5.2.x: no such mailbox, for example); `to` is the address, `dedupe_key` \
             the sender's key.",
        )]
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
        let transport = Arc::new(Transport::new(config.smtp_url.as_deref(), config.file_dir)?);
        self.probe = Some(Arc::new(doctor::SmtpProbe { transport: transport.clone() }));
        let settings = context.require::<dyn autoxide_kernel::tenancy::TenantSettings>()?;
        self.sender =
            Some(Arc::new(Sender::new(transport, config.from, context.outbox()?, settings)));
        let service = MailService::new(context.jobs());
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
        match &self.sender {
            Some(sender) => vec![JobRegistration::new(service::SEND, sender.clone())],
            None => Vec::new(),
        }
    }
}
