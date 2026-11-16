//! Lớp bọc chung quanh mọi nhà cung cấp — mỗi adapter không tự làm những việc này: tắt / bật,
//! hạn mức, kiểm kết quả theo schema (thử lại một lần), cộng token, log, metrics.
//!
//! Theo khách (bước 33, D-275): hạn mức đếm theo khách; khách đặt khoá
//! (`ai.connection.text.api_key`) hay mô hình (`ai.connection.text.model`) riêng thì lời gọi sinh
//! chữ của khách đi bằng khoá / mô hình đó, cùng nhà cung cấp và địa chỉ máy chủ đã cấu hình. Tên
//! theo quy ước kết nối đối tác; tên cũ `ai.api_key` / `ai.model` vẫn được nhận.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use autoxide_kernel::ai::{
    Capabilities, EmbedKind, Embedder, Generation, Part, Prompt, TextModel, Usage, schema,
};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{CONNECTION_GROUP, TenantContext, TenantSetting, TenantSettings};

use crate::config::{Provider as ProviderKind, TextConfig};
use crate::embedding::OpenAiEmbeddings;
use crate::errors;
use crate::providers::Provider;
use crate::usage::UsageBook;

/// Khoá API riêng của khách (lưu mã hoá, không bao giờ trả ra).
// Loại kết nối `text`: khoá này chỉ dùng cho sinh chữ; embedding có cấu hình riêng
// (`AUTOXIDE_EMBEDDING_…`), sau này cần theo khách thì thành `ai.connection.embedding.…`.
pub(crate) const TENANT_API_KEY: TenantSetting = TenantSetting::new(
    "ai.connection.text.api_key",
    "API key this tenant's AI requests use (its own account with the provider the server is \
     set up for); unset: the server's key.",
)
.secret()
.group(CONNECTION_GROUP)
.formerly("ai.api_key");

/// Mô hình riêng của khách. Cùng nhóm kết nối: mô hình là thứ chọn ở tài khoản của nhà cung cấp.
pub(crate) const TENANT_MODEL: TenantSetting = TenantSetting::new(
    "ai.connection.text.model",
    "Model this tenant's AI requests use, from the provider the server is set up for; unset: the \
     server's model.",
)
.group(CONNECTION_GROUP)
.formerly("ai.model");

/// Khoá / mô hình riêng của khách đè lên cấu hình chung.
pub(crate) struct TenantOverride {
    pub config: TextConfig,
    pub settings: Arc<dyn TenantSettings>,
    /// Adapter đã dựng theo (khoá, mô hình): dựng một lần, dùng lại.
    pub cache: Mutex<HashMap<(String, String), Arc<dyn Provider>>>,
}

impl TenantOverride {
    pub(crate) fn new(config: TextConfig, settings: Arc<dyn TenantSettings>) -> Self {
        TenantOverride { config, settings, cache: Mutex::new(HashMap::new()) }
    }

    /// Adapter của `tenant` khi khách đặt khoá hay mô hình riêng; `None`: dùng adapter chung.
    async fn provider(&self, tenant: &TenantContext) -> Result<Option<Arc<dyn Provider>>, Error> {
        let key = self.settings.get(tenant, &TENANT_API_KEY).await?;
        let model = self.settings.get(tenant, &TENANT_MODEL).await?;
        if key.is_none() && model.is_none() {
            return Ok(None);
        }
        let mut config = self.config.clone();
        if let Some(key) = &key {
            config.api_key = Some(key.as_str().to_owned());
        }
        if let Some(model) = &model {
            config.model = model.as_str().to_owned();
        }
        let id = (config.api_key.clone().unwrap_or_default(), config.model.clone());
        // Khoá chỉ giữ trong lúc tra / thêm (không qua `await`).
        if let Ok(cache) = self.cache.lock()
            && let Some(found) = cache.get(&id)
        {
            return Ok(Some(found.clone()));
        }
        let provider: Arc<dyn Provider> = match config.provider {
            ProviderKind::Anthropic => Arc::new(crate::providers::Anthropic::new(&config)?),
            ProviderKind::OpenAi => Arc::new(crate::providers::OpenAi::new(&config)?),
        };
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(id, provider.clone());
        }
        Ok(Some(provider))
    }
}

/// Sinh chữ (đã bật hay đã tắt).
pub(crate) struct Text {
    pub provider: Option<Arc<dyn Provider>>,
    pub capabilities: Capabilities,
    pub book: Arc<UsageBook>,
    /// Khoá / mô hình riêng của khách; `None` khi AI tắt (hay trong test).
    pub tenant_override: Option<TenantOverride>,
}

#[async_trait::async_trait]
impl TextModel for Text {
    fn capabilities(&self) -> Capabilities {
        self.capabilities.clone()
    }

    fn ready(&self) -> Result<(), Error> {
        if self.provider.is_some() { Ok(()) } else { Err(errors::disabled()) }
    }

    async fn generate(&self, tenant: &TenantContext, prompt: Prompt) -> Result<Generation, Error> {
        let Some(shared) = &self.provider else {
            return Err(errors::disabled());
        };
        let own = match &self.tenant_override {
            Some(tenant_override) => tenant_override.provider(tenant).await?,
            None => None,
        };
        let provider = own.as_ref().unwrap_or(shared);
        let purpose = prompt.purpose;
        if !self.capabilities.images
            && prompt.input.iter().any(|part| matches!(part, Part::Image { .. }))
        {
            return Err(errors::unsupported_input());
        }
        self.book.check(tenant).await?;
        let started = Instant::now();
        // Hai lần: mô hình đôi khi trả sai dạng (nhất là máy chủ bỏ qua schema mà không báo).
        for attempt in 1..=2 {
            let raw = match provider.generate(tenant, &prompt).await {
                Ok(raw) => raw,
                Err(error) => {
                    count(purpose, error.code());
                    return Err(error);
                }
            };
            // Token đã tốn kể cả khi kết quả sai dạng.
            self.book.record(tenant, purpose, raw.usage).await?;
            tokens(purpose, raw.usage);
            let problems = schema::problems(&prompt.output_schema, &raw.output);
            if problems.is_empty() {
                count(purpose, "ok");
                metrics::histogram!("ai_request_duration_seconds", "purpose" => purpose)
                    .record(started.elapsed().as_secs_f64());
                tracing::info!(
                    purpose,
                    model = %raw.model,
                    input_tokens = raw.usage.input_tokens,
                    output_tokens = raw.usage.output_tokens,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "AI answer"
                );
                return Ok(Generation::new(raw.output, raw.usage, raw.model));
            }
            // Ghi vị trí lỗi (không ghi nội dung) để người vận hành biết mô hình yếu tới đâu.
            tracing::warn!(purpose, attempt, problems = problems.len(), first = %problems[0], "AI answer has the wrong shape");
        }
        count(purpose, errors::output_invalid().code());
        Err(errors::output_invalid())
    }
}

/// Embedding (đã bật hay đã tắt).
pub(crate) struct Embeddings {
    pub client: Option<OpenAiEmbeddings>,
    pub book: Arc<UsageBook>,
}

#[async_trait::async_trait]
impl Embedder for Embeddings {
    fn enabled(&self) -> bool {
        self.client.is_some()
    }

    fn ready(&self) -> Result<(), Error> {
        if self.client.is_some() { Ok(()) } else { Err(errors::embedding_disabled()) }
    }

    fn model_id(&self) -> &str {
        self.client.as_ref().map_or("", |client| client.model.as_str())
    }

    fn dimensions(&self) -> usize {
        self.client.as_ref().map_or(0, |client| client.dimensions)
    }

    async fn embed(
        &self,
        tenant: &TenantContext,
        purpose: &'static str,
        texts: &[String],
        _kind: EmbedKind,
    ) -> Result<Vec<Vec<f32>>, Error> {
        let Some(client) = &self.client else {
            return Err(errors::embedding_disabled());
        };
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        self.book.check(tenant).await?;
        match client.embed(tenant, purpose, texts).await {
            Ok((vectors, usage)) => {
                self.book.record(tenant, purpose, usage).await?;
                tokens(purpose, usage);
                count(purpose, "ok");
                Ok(vectors)
            }
            Err(error) => {
                count(purpose, error.code());
                Err(error)
            }
        }
    }
}

/// `outcome`: `ok` hay mã lỗi — tập nhỏ cố định (`ai.*`, `internal_error`).
fn count(purpose: &'static str, outcome: &str) {
    let outcome = match outcome {
        "ok" => "ok",
        "ai.refused" => "refused",
        "ai.output_truncated" => "truncated",
        "ai.output_invalid" => "invalid",
        "ai.provider_unavailable" => "unavailable",
        "ai.misconfigured" => "misconfigured",
        "ai.quota_exceeded" => "quota_exceeded",
        "ai.request_rejected" => "rejected",
        _ => "error",
    };
    metrics::counter!("ai_requests_total", "purpose" => purpose, "outcome" => outcome).increment(1);
}

fn tokens(purpose: &'static str, usage: Usage) {
    metrics::counter!("ai_tokens_total", "purpose" => purpose, "direction" => "input")
        .increment(usage.input_tokens);
    metrics::counter!("ai_tokens_total", "purpose" => purpose, "direction" => "output")
        .increment(usage.output_tokens);
}
