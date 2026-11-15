//! Module `webhooks`: gửi event của mọi module tới hệ thống khác qua HTTPS, có chữ ký
//! theo chuẩn Standard Webhooks, thử lại khi lỗi và lưu lịch sử gửi (D-081).
//!
//! Quản trị viên (permission `webhooks.manage`) khai nơi nhận và các event muốn nhận ở
//! `/api/v1/webhooks`. Module nhận mọi event qua `Module::subscriptions`, mỗi lần gửi là
//! một job trong `webhooks_jobs`.

pub mod api;
mod config;
mod delivery;
mod entity;
mod events;
mod http;
mod migration;
mod network;
mod permissions;
mod service;
mod signature;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventInfo, Subscription};
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::config::WebhooksConfig;
use crate::delivery::{CLEANUP_KIND, Cleanup, Fanout, SEND_KIND, Sender};
use crate::service::WebhookService;

pub fn module() -> impl Module {
    WebhooksModule::default()
}

#[derive(Default)]
struct WebhooksModule {
    /// Có sau `configure`.
    config: Option<WebhooksConfig>,
    /// Có sau `start`.
    running: Option<Running>,
}

struct Running {
    service: Arc<WebhookService>,
    fanout: Arc<Fanout>,
    sender: Arc<Sender>,
    cleanup: Arc<Cleanup>,
}

impl Module for WebhooksModule {
    fn name(&self) -> &'static str {
        "webhooks"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn upgrades(&self) -> Vec<Box<dyn autoxide_kernel::migration::DataUpgrade>> {
        migration::upgrades()
    }

    fn permissions(&self) -> Vec<Permission> {
        permissions::all()
    }

    fn events(&self) -> Vec<EventInfo> {
        events::all()
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(WebhooksConfig::read(reader)?);
        Ok(())
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `webhooks` started before it was configured"))?;
        let secrets = context.require::<autoxide_kernel::secret::SecretKey>()?;
        let service = Arc::new(
            WebhookService::new(
                secrets.as_ref().clone(),
                context.outbox()?,
                context.events().clone(),
                config.allow_http,
                config.allow_private_networks,
            )
            .with_translations(context.translations().clone()),
        );
        let sender = Arc::new(Sender {
            secrets: secrets.as_ref().clone(),
            client: Sender::client(config.timeout, config.allow_private_networks)?,
            allow_private_networks: config.allow_private_networks,
        });
        self.running = Some(Running {
            service,
            fanout: delivery::fanout(),
            sender,
            cleanup: Arc::new(Cleanup),
        });
        Ok(())
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        self.running.as_ref().map_or_else(Vec::new, |running| {
            vec![Subscription::new(Subscription::ALL, running.fanout.clone())]
        })
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        self.running.as_ref().map_or_else(Vec::new, |running| {
            vec![
                JobRegistration::new(SEND_KIND, running.sender.clone()),
                JobRegistration::new(CLEANUP_KIND, running.cleanup.clone()),
            ]
        })
    }

    fn schedules(&self) -> Vec<Schedule> {
        vec![Schedule::new(CLEANUP_KIND, CLEANUP_KIND, Duration::from_secs(24 * 3600))]
    }

    fn routes(&self) -> OpenApiRouter {
        self.running
            .as_ref()
            .map_or_else(OpenApiRouter::new, |running| http::routes(running.service.clone()))
    }
}
