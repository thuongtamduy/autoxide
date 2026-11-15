//! Lớp bọc chung quanh mọi nhà cung cấp — mỗi adapter không tự làm những việc này: tắt / bật,
//! hạn mức, kiểm kết quả theo schema (thử lại một lần), cộng token, log, metrics.

use std::sync::Arc;
use std::time::Instant;

use autoxide_kernel::ai::{
    Capabilities, EmbedKind, Embedder, Generation, Part, Prompt, TextModel, Usage, schema,
};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;

use crate::embedding::OpenAiEmbeddings;
use crate::errors;
use crate::providers::Provider;
use crate::usage::UsageBook;

/// Sinh chữ (đã bật hay đã tắt).
pub(crate) struct Text {
    pub provider: Option<Box<dyn Provider>>,
    pub capabilities: Capabilities,
    pub book: Arc<UsageBook>,
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
        let Some(provider) = &self.provider else {
            return Err(errors::disabled());
        };
        let purpose = prompt.purpose;
        if !self.capabilities.images
            && prompt.input.iter().any(|part| matches!(part, Part::Image { .. }))
        {
            return Err(errors::unsupported_input());
        }
        self.book.check().await?;
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
            self.book.record(purpose, raw.usage).await?;
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
        self.book.check().await?;
        match client.embed(tenant, purpose, texts).await {
            Ok((vectors, usage)) => {
                self.book.record(purpose, usage).await?;
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
