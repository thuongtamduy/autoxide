//! Module `analytics`: số liệu truy cập và hành vi ngay trong Autoxide (bước 57, D-220) — không gửi dữ
//! liệu người đọc cho bên thứ ba, mặc định không cookie, chỉ giữ số đã gộp lâu dài.
//!
//! - Website gửi sự kiện qua `POST /api/v1/analytics/public/events`; module khác khai loại sự kiện
//!   (`Module::analytics_events`) và phễu (`Module::funnels`) của mình; sự kiện phía máy chủ rút từ
//!   event của module (`ingest.rs`).
//! - Phiên ẩn danh với khoá xoay mỗi ngày, không lưu IP hay user agent (`ingest.rs`).
//! - Job mỗi giờ gộp theo ngày (`rollup.rs`); báo cáo chỉ đọc bảng gộp (`reports.rs`).
//! - Theo dõi theo người chỉ khi đồng ý `analytics.personal` ở sổ đồng ý của `compliance`; rút đồng
//!   ý thì gỡ phần gắn người; hạn lưu là cấu hình (`privacy.rs`).
//!
//! Đăng ký SAU `compliance` (đọc sổ đồng ý qua `dyn ConsentLedger`). Hướng dẫn: `docs/ANALYTICS.md`.

pub mod api;
mod client;
mod config;
mod entity;
mod errors;
mod http;
mod ingest;
mod migration;
mod permissions;
mod privacy;
mod reports;
mod rollup;
mod service;
mod shared;
mod tools;

mod relocation;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::Permission;
use autoxide_kernel::analytics::{AnalyticsEvent, Source};
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler, Subscription};
use autoxide_kernel::http::BodyLimit;
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::secret::SecretKey;
use axum::http::Method;
use utoipa_axum::router::OpenApiRouter;

use crate::config::AnalyticsConfig;
use crate::service::AnalyticsService;

/// Module `analytics`, để binary đăng ký trong `crates/app/src/modules.rs` — SAU `compliance`.
pub fn module() -> impl Module {
    AnalyticsModule::default()
}

/// Loại sự kiện "xem trang" của chính module; `object` tuỳ chọn (trang là một đối tượng, vd bài
/// viết).
const PAGE_VIEW: AnalyticsEvent =
    AnalyticsEvent::browser(api::PAGE_VIEW, "A page of the website was shown.");

#[derive(Default)]
struct AnalyticsModule {
    /// Có sau `configure`.
    config: Option<AnalyticsConfig>,
    /// Có sau `start`.
    service: Option<Arc<AnalyticsService>>,
    /// Event `compliance.consent_withdrawn` có người khai (module `compliance` đã cài).
    listen_consents: bool,
}

impl Module for AnalyticsModule {
    fn name(&self) -> &'static str {
        "analytics"
    }

    fn description(&self) -> &'static str {
        "Cookieless website analytics kept in your own database: pages, sources, UTM campaigns, \
         most viewed objects and funnels; per-person tracking only with consent."
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn permissions(&self) -> Vec<Permission> {
        permissions::all()
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(AnalyticsConfig::read(reader)?);
        Ok(())
    }

    // Thời hạn lưu dữ liệu (bước 37 C5, D-360), đọc từ cấu hình đang có.
    fn retention(&self) -> Vec<autoxide_kernel::privacy::RetentionPolicy> {
        use autoxide_kernel::privacy::{DataCategory, Keep, RetentionPolicy};
        let Some(config) = &self.config else {
            return Vec::new();
        };
        vec![
            RetentionPolicy::new(
                "analytics.events",
                "Single visit events (page, referring site, campaign, anonymous daily session; the \
                 account only with consent).",
                DataCategory::Basic,
                Keep::Days(config.raw_retention_days),
            )
            .setting(config::RAW_RETENTION_DAYS.name),
            RetentionPolicy::new(
                "analytics.daily_totals",
                "Daily totals of visits per website (no person in them).",
                DataCategory::Basic,
                Keep::Days(config.retention_days),
            )
            .setting(config::RETENTION_DAYS.name),
        ]
    }

    fn analytics_events(&self) -> Vec<AnalyticsEvent> {
        vec![PAGE_VIEW]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self.config.clone().ok_or_else(|| {
            Error::internal("module `analytics` started before it was configured")
        })?;
        let service = Arc::new(AnalyticsService {
            config,
            catalog: context.analytics_catalog(),
            secret: (*context.require::<SecretKey>()?).clone(),
            consents: context.require::<dyn autoxide_compliance::api::ConsentLedger>().ok(),
            registry: context.tenancy(),
        });
        self.listen_consents =
            context.events().contains(autoxide_compliance::api::CONSENT_WITHDRAWN);
        context.personal_data(service.clone())?;
        context.summary_metric(Arc::new(shared::Visitors))?;
        context.provide::<dyn api::Analytics>(service.clone())?;
        self.service = Some(service);
        Ok(())
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        let Some(service) = self.service.clone() else {
            return Vec::new();
        };
        let forget = Arc::new(privacy::Forget(service.clone()));
        let mut subscriptions =
            vec![Subscription::new(autoxide_kernel::privacy::USER_DELETED, forget.clone())];
        if self.listen_consents {
            subscriptions
                .push(Subscription::new(autoxide_compliance::api::CONSENT_WITHDRAWN, forget));
        }
        // Sự kiện phía máy chủ: mỗi loại nghe event của module khai nó.
        for declared in service.catalog.events() {
            if let Source::Server { event, .. } = declared.source {
                subscriptions.push(Subscription::new(
                    event,
                    Arc::new(ServerHits { service: service.clone(), declared: *declared }),
                ));
            }
        }
        subscriptions
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![
                JobRegistration::new(rollup::ROLLUP, Arc::new(rollup::Rollup(service.clone()))),
                JobRegistration::new(privacy::CLEANUP, Arc::new(privacy::Cleanup(service))),
            ]
        })
    }

    fn schedules(&self) -> Vec<Schedule> {
        let hour = Duration::from_secs(3600);
        vec![
            Schedule::new(rollup::ROLLUP, rollup::ROLLUP, hour),
            Schedule::new(privacy::CLEANUP, privacy::CLEANUP, hour * 24),
        ]
    }

    fn body_limits(&self) -> Vec<BodyLimit> {
        // 20 sự kiện, mỗi sự kiện vài trăm byte.
        vec![BodyLimit::new(Method::POST, "/public/events", 64 * 1024)]
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        relocation::relocation()
    }
}

/// Bên nhận event của module cho một loại sự kiện phía máy chủ.
struct ServerHits {
    service: Arc<AnalyticsService>,
    declared: AnalyticsEvent,
}

#[async_trait::async_trait]
impl EventHandler for ServerHits {
    async fn handle(&self, delivered: &Delivered) -> Result<(), Error> {
        if !self.service.config.enabled {
            return Ok(());
        }
        let Source::Server { extract, .. } = self.declared.source else {
            return Ok(());
        };
        match extract(delivered)? {
            Some(hit) => self.service.record_server_hit(&self.declared, delivered, hit).await,
            None => Ok(()),
        }
    }
}
