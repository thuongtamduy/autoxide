//! Gọi nhà cung cấp AI qua HTTP: phần chung (client, gửi JSON, đổi mã HTTP thành lỗi) và hai
//! bộ chuyển đổi — `anthropic` (Messages API gốc) và `openai` (Chat Completions, dùng được với
//! OpenAI, Gemini, Ollama, vLLM…). Dạng request / response và nguồn: `docs/research/ai.md`.
//!
//! Không bao giờ ghi prompt, nội dung hay key vào log — chỉ việc gì, mã HTTP, thời gian.

use std::time::Duration;

use autoxide_kernel::ai::{Effort, Part, Prompt, Usage};
use autoxide_kernel::error::Error;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use crate::config::TextConfig;
use crate::errors;

/// Phiên bản Messages API của Anthropic. Nâng có chủ đích, kèm test (máy chủ giả giữ dạng).
pub(crate) const ANTHROPIC_VERSION: &str = "2026-10-21";
/// Beta "chạy lại trên mô hình khác khi bộ lọc an toàn từ chối" của Anthropic.
pub(crate) const ANTHROPIC_FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// Kết quả thô của nhà cung cấp: `output` là JSON đã đọc (hay chuỗi gốc nếu không đọc được —
/// bước kiểm schema sẽ từ chối và thử lại), chưa kiểm schema.
#[derive(Debug, Clone)]
pub(crate) struct Raw {
    pub output: Value,
    pub usage: Usage,
    pub model: String,
}

/// Một nhà cung cấp sinh chữ.
#[async_trait::async_trait]
pub(crate) trait Provider: Send + Sync {
    async fn generate(&self, prompt: &Prompt) -> Result<Raw, Error>;
}

/// HTTP client dùng chung: giới hạn thời gian chờ, không theo chuyển hướng (gửi key tới địa chỉ
/// khác địa chỉ đã cấu hình là rủi ro).
pub(crate) fn client(timeout: Duration) -> Result<reqwest::Client, Error> {
    autoxide_kernel::tls::install_crypto_provider();
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Autoxide/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| Error::internal(format!("AI HTTP client: {error}")))
}

/// Gửi `body` (JSON) tới `url`, trả JSON của câu trả lời 2xx. Mã khác thành lỗi `ai.*`.
pub(crate) async fn post_json(
    client: &reqwest::Client,
    url: &str,
    headers: &[(&str, &str)],
    body: &Value,
    purpose: &'static str,
) -> Result<Value, Error> {
    let bytes = serde_json::to_vec(body).map_err(Error::internal)?;
    let mut request = client.post(url).header("content-type", "application/json").body(bytes);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            // Lỗi mạng, hết giờ chờ: tạm thời. Thông báo của reqwest không chứa key.
            tracing::warn!(purpose, error = %error, "AI provider unreachable");
            return Err(errors::provider_unavailable(None));
        }
    };
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| seconds.clamp(1, 3600));
    let text = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(purpose, status, error = %error, "AI provider answer was cut off");
            return Err(errors::provider_unavailable(retry_after));
        }
    };
    let json: Option<Value> = serde_json::from_slice(&text).ok();
    if (200..300).contains(&status) {
        return json.ok_or_else(|| {
            tracing::warn!(purpose, status, "AI provider answered with something that is not JSON");
            errors::provider_unavailable(None)
        });
    }
    // Thông báo lỗi của nhà cung cấp (không chứa prompt) giúp người vận hành sửa cấu hình.
    let message = json
        .as_ref()
        .and_then(|json| json.pointer("/error/message").and_then(Value::as_str))
        .unwrap_or("")
        .chars()
        .take(300)
        .collect::<String>();
    match status {
        401..=404 => {
            tracing::warn!(purpose, status, message, "AI provider refused the server's settings");
            Err(errors::misconfigured())
        }
        408 | 409 | 429 | 500..=599 => {
            tracing::warn!(purpose, status, message, "AI provider busy or failing");
            Err(errors::provider_unavailable(retry_after))
        }
        _ => Err(Error::internal(format!(
            "AI provider rejected the request for `{purpose}` (HTTP {status}): {message}"
        ))),
    }
}

/// Bộ chuyển đổi Anthropic: `POST {base}/v1/messages`.
pub(crate) struct Anthropic {
    client: reqwest::Client,
    url: String,
    api_key: String,
    model: String,
    fallback: bool,
}

impl Anthropic {
    pub(crate) fn new(config: &TextConfig) -> Result<Self, Error> {
        Ok(Anthropic {
            client: client(config.timeout)?,
            url: format!("{}/v1/messages", config.base_url),
            api_key: config.api_key.clone().unwrap_or_default(),
            model: config.model.clone(),
            fallback: config.anthropic_fallback,
        })
    }

    /// Thân request (tách riêng để test kiểm được).
    pub(crate) fn body(&self, prompt: &Prompt) -> Value {
        // Ảnh đặt trước chữ (tài liệu của Anthropic khuyên vậy).
        let mut content: Vec<Value> = prompt
            .input
            .iter()
            .filter_map(|part| match part {
                Part::Image { media_type, bytes } => Some(json!({
                    "type": "image",
                    "source": {"type": "base64", "media_type": media_type, "data": STANDARD.encode(bytes)}
                })),
                Part::Text(_) => None,
            })
            .collect();
        content.extend(prompt.input.iter().filter_map(|part| match part {
            Part::Text(text) => Some(json!({"type": "text", "text": text})),
            Part::Image { .. } => None,
        }));
        let mut body = json!({
            "model": self.model,
            "max_tokens": prompt.max_output_tokens,
            "system": prompt.instructions,
            "messages": [{"role": "user", "content": content}],
            "output_config": {
                "effort": effort(prompt.effort),
                "format": {"type": "json_schema", "schema": prompt.output_schema},
            },
        });
        if self.fallback
            && let Some(object) = body.as_object_mut()
        {
            object.insert("fallbacks".to_owned(), json!("default"));
        }
        body
    }

    pub(crate) fn read(answer: &Value) -> Result<Raw, Error> {
        match answer.get("stop_reason").and_then(Value::as_str) {
            Some("refusal") => return Err(errors::refused()),
            Some("max_tokens" | "model_context_window_exceeded") => {
                return Err(errors::output_truncated());
            }
            _ => {}
        }
        // Kết quả nằm trong khối `text` (có thể sau khối `thinking`): chọn theo loại, không
        // theo vị trí.
        let text: String = answer
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect();
        let usage = answer.get("usage");
        let count = |name: &str| usage.and_then(|usage| usage.get(name)).and_then(Value::as_u64);
        let input = count("input_tokens").unwrap_or(0)
            + count("cache_creation_input_tokens").unwrap_or(0)
            + count("cache_read_input_tokens").unwrap_or(0);
        Ok(Raw {
            output: parse_output(&text),
            usage: Usage {
                input_tokens: input,
                output_tokens: count("output_tokens").unwrap_or(0),
            },
            model: answer.get("model").and_then(Value::as_str).unwrap_or_default().to_owned(),
        })
    }
}

#[async_trait::async_trait]
impl Provider for Anthropic {
    async fn generate(&self, prompt: &Prompt) -> Result<Raw, Error> {
        let mut headers =
            vec![("x-api-key", self.api_key.as_str()), ("anthropic-version", ANTHROPIC_VERSION)];
        if self.fallback {
            headers.push(("anthropic-beta", ANTHROPIC_FALLBACK_BETA));
        }
        let answer =
            post_json(&self.client, &self.url, &headers, &self.body(prompt), prompt.purpose)
                .await?;
        Anthropic::read(&answer)
    }
}

/// Bộ chuyển đổi Chat Completions: `POST {base}/chat/completions`.
pub(crate) struct OpenAi {
    client: reqwest::Client,
    url: String,
    authorization: Option<String>,
    model: String,
    strict: bool,
    send_effort: bool,
    max_tokens_param: String,
}

impl OpenAi {
    pub(crate) fn new(config: &TextConfig) -> Result<Self, Error> {
        Ok(OpenAi {
            client: client(config.timeout)?,
            url: format!("{}/chat/completions", config.base_url),
            authorization: config.api_key.as_ref().map(|key| format!("Bearer {key}")),
            model: config.model.clone(),
            strict: config.openai_strict,
            send_effort: config.openai_send_effort,
            max_tokens_param: config.openai_max_tokens_param.clone(),
        })
    }

    pub(crate) fn body(&self, prompt: &Prompt) -> Value {
        let mut instructions = prompt.instructions.clone();
        let response_format = if self.strict {
            // Tên schema: chữ, số, `_`, `-` (yêu cầu của OpenAI).
            let name: String = prompt
                .purpose
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
                .collect();
            json!({"type": "json_schema", "json_schema": {"name": name, "schema": prompt.output_schema, "strict": true}})
        } else {
            // Chế độ JSON thường cần chữ "JSON" trong tin nhắn; đưa luôn schema để mô hình theo.
            instructions.push_str(&format!(
                "\n\nAnswer with one JSON object that matches this JSON Schema:\n{}",
                prompt.output_schema
            ));
            json!({"type": "json_object"})
        };
        let mut content: Vec<Value> = prompt
            .input
            .iter()
            .filter_map(|part| match part {
                Part::Text(text) => Some(json!({"type": "text", "text": text})),
                Part::Image { .. } => None,
            })
            .collect();
        content.extend(prompt.input.iter().filter_map(|part| match part {
            Part::Image { media_type, bytes } => Some(json!({
                "type": "image_url",
                "image_url": {"url": format!("data:{media_type};base64,{}", STANDARD.encode(bytes))}
            })),
            Part::Text(_) => None,
        }));
        let mut body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": instructions},
                {"role": "user", "content": content},
            ],
            "response_format": response_format,
        });
        if let Some(object) = body.as_object_mut() {
            object.insert(self.max_tokens_param.clone(), json!(prompt.max_output_tokens));
            if self.send_effort {
                object.insert("reasoning_effort".to_owned(), json!(effort(prompt.effort)));
            }
        }
        body
    }

    pub(crate) fn read(answer: &Value) -> Result<Raw, Error> {
        let choice = answer.pointer("/choices/0");
        match choice.and_then(|choice| choice.get("finish_reason")).and_then(Value::as_str) {
            Some("length") => return Err(errors::output_truncated()),
            Some("content_filter") => return Err(errors::refused()),
            _ => {}
        }
        if choice
            .and_then(|choice| choice.pointer("/message/refusal"))
            .and_then(Value::as_str)
            .is_some_and(|refusal| !refusal.is_empty())
        {
            return Err(errors::refused());
        }
        let text = choice
            .and_then(|choice| choice.pointer("/message/content"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let count = |name: &str| {
            answer.get("usage").and_then(|usage| usage.get(name)).and_then(Value::as_u64)
        };
        Ok(Raw {
            output: parse_output(text),
            usage: Usage {
                input_tokens: count("prompt_tokens").unwrap_or(0),
                output_tokens: count("completion_tokens").unwrap_or(0),
            },
            model: answer.get("model").and_then(Value::as_str).unwrap_or_default().to_owned(),
        })
    }
}

#[async_trait::async_trait]
impl Provider for OpenAi {
    async fn generate(&self, prompt: &Prompt) -> Result<Raw, Error> {
        let mut headers = Vec::new();
        if let Some(authorization) = &self.authorization {
            headers.push(("authorization", authorization.as_str()));
        }
        let answer =
            post_json(&self.client, &self.url, &headers, &self.body(prompt), prompt.purpose)
                .await?;
        OpenAi::read(&answer)
    }
}

fn effort(effort: Effort) -> &'static str {
    effort.as_str()
}

/// JSON trong câu trả lời; mô hình ở chế độ JSON thường đôi khi bọc trong khối ```` ``` ````.
/// Không đọc được thì trả nguyên chuỗi — bước kiểm schema sẽ từ chối.
fn parse_output(text: &str) -> Value {
    let trimmed = text.trim();
    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.strip_suffix("```"))
        .unwrap_or(trimmed);
    serde_json::from_str(unfenced.trim()).unwrap_or_else(|_| Value::String(text.to_owned()))
}
