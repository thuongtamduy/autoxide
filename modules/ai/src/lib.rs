//! Module `ai`: tầng kết nối AI cho mọi module (D-109).
//!
//! Công bố `dyn TextModel` (sinh kết quả có cấu trúc) và `dyn Embedder` (embedding) của
//! `autoxide_kernel::ai`. Hai adapter sinh chữ: `anthropic` (Messages API) và `openai` (Chat
//! Completions — OpenAI, Gemini, Ollama, vLLM…); embedding theo dạng `/embeddings`. Mặc định tắt:
//! module vẫn công bố bản "đã tắt" để module khác không phải rẽ nhánh. Hạn mức token mỗi ngày đếm
//! trong bảng `ai_usage`.

pub mod api;
mod config;
mod embedding;
mod errors;
mod http;
mod migration;
mod providers;
mod service;
mod usage;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::Permission;
use autoxide_kernel::ai::{Capabilities, Embedder, TextModel};
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::config::{AiConfig, Provider};
use crate::service::{Embeddings, Text};
use crate::usage::UsageBook;

/// Module `ai`, để binary đăng ký trong `crates/app/src/modules.rs` — TRƯỚC các module dùng AI.
pub fn module() -> impl Module {
    AiModule::default()
}

#[derive(Default)]
struct AiModule {
    /// Có sau `configure`.
    config: Option<AiConfig>,
    /// Có sau `start`.
    status: Option<Arc<http::Status>>,
}

impl Module for AiModule {
    fn name(&self) -> &'static str {
        "ai"
    }

    // Tách / gộp khách (bước 38, D-20261118-step38-relocation).
    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        use autoxide_kernel::tenancy::Relocation;
        Relocation::new().ephemeral(&["ai_usage"])
    }

    fn description(&self) -> &'static str {
        "Connection to AI models (text generation and embeddings) for every module; off by default."
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn tenant_settings(&self) -> Vec<autoxide_kernel::tenancy::TenantSetting> {
        vec![service::TENANT_API_KEY, service::TENANT_MODEL]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(AiConfig::read(reader)?);
        Ok(())
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![http::AI_VIEW]
    }

    fn limits(&self) -> Vec<autoxide_kernel::entitlement::Limit> {
        vec![usage::DAILY_TOKENS]
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `ai` started before it was configured"))?;
        let book = Arc::new(
            UsageBook::new(config.daily_token_limit).with_entitlements(context.entitlements()),
        );
        let (provider, capabilities): (Option<Arc<dyn providers::Provider>>, Capabilities) =
            match &config.text {
                None => (None, Capabilities::default()),
                Some(text) => {
                    let provider: Arc<dyn providers::Provider> = match text.provider {
                        Provider::Anthropic => Arc::new(providers::Anthropic::new(text)?),
                        Provider::OpenAi => Arc::new(providers::OpenAi::new(text)?),
                    };
                    tracing::info!(provider = text.provider.name(), model = %text.model, "AI text generation is on");
                    let capabilities =
                        Capabilities::enabled(text.provider.name().to_owned(), text.model.clone())
                            .images(text.images);
                    (Some(provider), capabilities)
                }
            };
        let client = match &config.embedding {
            None => None,
            Some(embedding) => {
                tracing::info!(model = %embedding.model, dimensions = embedding.dimensions, "AI embeddings are on");
                Some(embedding::OpenAiEmbeddings::new(embedding)?)
            }
        };
        let settings = context.require::<dyn autoxide_kernel::tenancy::TenantSettings>()?;
        let tenant_override =
            config.text.clone().map(|text| service::TenantOverride::new(text, settings));
        let text = Arc::new(Text { provider, capabilities, book: book.clone(), tenant_override });
        let embeddings = Arc::new(Embeddings { client, book: book.clone() });
        context.provide::<dyn TextModel>(text.clone())?;
        context.provide::<dyn Embedder>(embeddings.clone())?;
        self.status = Some(Arc::new(http::Status { text, embeddings, book }));
        Ok(())
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        match (&self.status, &self.config) {
            (Some(status), Some(config)) => vec![JobRegistration::new(
                usage::PRUNE,
                Arc::new(usage::Prune {
                    book: status.book.clone(),
                    days: config.usage_retention_days,
                }),
            )],
            _ => Vec::new(),
        }
    }

    fn schedules(&self) -> Vec<Schedule> {
        let day = Duration::from_secs(24 * 3600);
        vec![Schedule::new(usage::PRUNE, usage::PRUNE, day)]
    }

    fn tools(&self) -> Vec<Tool> {
        self.status.as_ref().map(http::tools).unwrap_or_default()
    }

    fn routes(&self) -> OpenApiRouter {
        self.status.clone().map_or_else(OpenApiRouter::new, http::routes)
    }
}
