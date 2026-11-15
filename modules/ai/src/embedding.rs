//! Client embedding theo dạng `POST {base}/embeddings` (`{"model", "input": [...]}` →
//! `data[].embedding`): OpenAI, Gemini (endpoint tương thích), Ollama, vLLM, Voyage AI.

use autoxide_kernel::ai::Usage;
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use serde_json::{Value, json};

use crate::config::EmbeddingConfig;
use crate::errors;
use crate::providers::{client, post_json};

pub(crate) struct OpenAiEmbeddings {
    client: reqwest::Client,
    url: String,
    authorization: Option<String>,
    pub(crate) model: String,
    pub(crate) dimensions: usize,
    batch_size: usize,
    send_dimensions: bool,
}

impl OpenAiEmbeddings {
    pub(crate) fn new(config: &EmbeddingConfig) -> Result<Self, Error> {
        Ok(OpenAiEmbeddings {
            client: client(config.timeout)?,
            url: format!("{}/embeddings", config.base_url),
            authorization: config.api_key.as_ref().map(|key| format!("Bearer {key}")),
            model: config.model.clone(),
            dimensions: config.dimensions,
            batch_size: config.batch_size,
            send_dimensions: config.send_dimensions,
        })
    }

    /// Một vector cho mỗi chữ, cùng thứ tự; gửi theo lô `batch_size`.
    pub(crate) async fn embed(
        &self,
        _tenant: &TenantContext,
        purpose: &'static str,
        texts: &[String],
    ) -> Result<(Vec<Vec<f32>>, Usage), Error> {
        let mut vectors = Vec::with_capacity(texts.len());
        let mut usage = Usage::default();
        let mut headers = Vec::new();
        if let Some(authorization) = &self.authorization {
            headers.push(("authorization", authorization.as_str()));
        }
        for batch in texts.chunks(self.batch_size) {
            let mut body = json!({"model": self.model, "input": batch});
            if self.send_dimensions
                && let Some(object) = body.as_object_mut()
            {
                object.insert("dimensions".to_owned(), json!(self.dimensions));
            }
            let answer = post_json(&self.client, &self.url, &headers, &body, purpose).await?;
            let (found, used) = self.read(&answer, batch.len(), purpose)?;
            vectors.extend(found);
            usage.input_tokens += used;
        }
        Ok((vectors, usage))
    }

    /// Đọc câu trả lời của một lô `expected` chữ.
    pub(crate) fn read(
        &self,
        answer: &Value,
        expected: usize,
        purpose: &'static str,
    ) -> Result<(Vec<Vec<f32>>, u64), Error> {
        let mut rows: Vec<(u64, Vec<f32>)> = Vec::with_capacity(expected);
        for (position, item) in
            answer.get("data").and_then(Value::as_array).into_iter().flatten().enumerate()
        {
            let index = item.get("index").and_then(Value::as_u64).unwrap_or(position as u64);
            let vector: Vec<f32> = item
                .get("embedding")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                // `f64` → `f32`: embedding chỉ cần độ chính xác của `f32` (4 byte mỗi chiều).
                .filter_map(|value| value.as_f64().map(|value| value as f32))
                .collect();
            rows.push((index, vector));
        }
        rows.sort_by_key(|(index, _)| *index);
        let wrong_size = rows.iter().any(|(_, vector)| vector.len() != self.dimensions);
        if rows.len() != expected || wrong_size {
            tracing::warn!(
                purpose,
                expected,
                received = rows.len(),
                dimensions = self.dimensions,
                "embedding answer has the wrong number or size of vectors; check \
                 AUTOXIDE_EMBEDDING_DIMENSIONS"
            );
            return Err(errors::output_invalid());
        }
        let usage = answer.get("usage");
        let tokens = usage
            .and_then(|usage| usage.get("prompt_tokens").or_else(|| usage.get("total_tokens")))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        Ok((rows.into_iter().map(|(_, vector)| vector).collect(), tokens))
    }
}
