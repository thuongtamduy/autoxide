//! Cấu hình của module `ai` (D-109). Biến sinh chữ mang tiền tố `AUTOXIDE_AI_`, biến embedding
//! mang tiền tố `AUTOXIDE_EMBEDDING_`. Không đặt gì thì AI tắt và mọi thứ khác chạy như cũ.

use std::time::Duration;

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const PROVIDER: Setting = Setting::new(
    "AUTOXIDE_AI_PROVIDER",
    "Which AI provider generates text: `none` (AI features are off), `anthropic` (Claude, \
     Messages API) or `openai` (the OpenAI Chat Completions format — OpenAI itself, Google \
     Gemini through its OpenAI-compatible endpoint, Ollama, vLLM, LM Studio, OpenRouter… chosen \
     with AUTOXIDE_AI_BASE_URL).",
)
.with_default("none");

pub(crate) const API_KEY: Setting = Setting::new(
    "AUTOXIDE_AI_API_KEY",
    "API key of the AI provider. Required for `anthropic`; for `openai`, leave it empty only \
     for a local server that needs none (Ollama).",
)
.secret();

pub(crate) const BASE_URL: Setting = Setting::new(
    "AUTOXIDE_AI_BASE_URL",
    "Base address of the provider's API. Defaults: `https://api.anthropic.com` for `anthropic`, \
     `https://api.openai.com/v1` for `openai`. Examples for `openai`: \
     `https://generativelanguage.googleapis.com/v1beta/openai` (Gemini), \
     `http://localhost:11434/v1` (Ollama).",
)
.with_example("http://localhost:11434/v1");

pub(crate) const MODEL: Setting = Setting::new(
    "AUTOXIDE_AI_MODEL",
    "Model that generates text. Default for `anthropic`: `claude-opus-5-5`. Required for \
     `openai` (names differ between services and change often).",
)
.with_example("claude-opus-5-5");

pub(crate) const IMAGES: Setting = Setting::new(
    "AUTOXIDE_AI_IMAGES",
    "Whether the model reads images (needed for alt text suggestions): `true` or `false`. Set \
     `false` for a text-only local model.",
)
.with_default("true");

pub(crate) const TIMEOUT_SECONDS: Setting = Setting::new(
    "AUTOXIDE_AI_TIMEOUT_SECONDS",
    "Longest wait for one answer of the AI provider, 1 to 3600 seconds.",
)
.with_default("600");

pub(crate) const DAILY_TOKEN_LIMIT: Setting = Setting::new(
    "AUTOXIDE_AI_DAILY_TOKEN_LIMIT",
    "Tokens (input plus output, text and embeddings together) the server may use per UTC day; \
     past it, AI features answer 429 `ai.quota_exceeded` until the next day. Counted in the \
     database, shared by every server process; concurrent calls may go slightly over. `0` \
     means no limit.",
)
.with_default("2000000");

pub(crate) const USAGE_RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_AI_USAGE_RETENTION_DAYS",
    "Days the daily token counts are kept, 1 to 3650.",
)
.with_default("90");

pub(crate) const ANTHROPIC_FALLBACK: Setting = Setting::new(
    "AUTOXIDE_AI_ANTHROPIC_FALLBACK",
    "For `anthropic`: when Claude's safety classifier declines a request, let Anthropic rerun \
     it on another model in the same call (beta `server-side-fallback`). `true` or `false`; set \
     `false` for models that do not support it (Claude Haiku).",
)
.with_default("true");

pub(crate) const OPENAI_SCHEMA_MODE: Setting = Setting::new(
    "AUTOXIDE_AI_OPENAI_SCHEMA_MODE",
    "For `openai`: `strict` asks the server to follow the JSON Schema of each answer \
     (`response_format` `json_schema`); `json_object` only asks for JSON, for servers that do \
     not take a schema. The server checks every answer either way.",
)
.with_default("strict");

pub(crate) const OPENAI_SEND_EFFORT: Setting = Setting::new(
    "AUTOXIDE_AI_OPENAI_SEND_EFFORT",
    "For `openai`: send `reasoning_effort` (`true` or `false`). Only reasoning models accept it; \
     others answer 400.",
)
.with_default("false");

pub(crate) const OPENAI_MAX_TOKENS_PARAM: Setting = Setting::new(
    "AUTOXIDE_AI_OPENAI_MAX_TOKENS_PARAM",
    "For `openai`: the request field that caps the answer length, `max_completion_tokens` \
     (OpenAI) or `max_tokens` (older compatible servers).",
)
.with_default("max_completion_tokens");

pub(crate) const EMBEDDING_PROVIDER: Setting = Setting::new(
    "AUTOXIDE_EMBEDDING_PROVIDER",
    "Which service computes embeddings for search by meaning: `none` (off; search matches words \
     only) or `openai` (the `/embeddings` format of OpenAI, also served by Gemini, Ollama, vLLM \
     and Voyage AI). Independent of AUTOXIDE_AI_PROVIDER.",
)
.with_default("none");

pub(crate) const EMBEDDING_BASE_URL: Setting =
    Setting::new("AUTOXIDE_EMBEDDING_BASE_URL", "Base address of the embeddings API.")
        .with_default("https://api.openai.com/v1");

pub(crate) const EMBEDDING_API_KEY: Setting = Setting::new(
    "AUTOXIDE_EMBEDDING_API_KEY",
    "API key of the embeddings service, if it needs one.",
)
.secret();

pub(crate) const EMBEDDING_MODEL: Setting = Setting::new(
    "AUTOXIDE_EMBEDDING_MODEL",
    "Embedding model; required when embeddings are on. Prefer a multilingual model of about \
     1024 dimensions (for example `bge-m3` on Ollama).",
)
.with_example("bge-m3");

pub(crate) const EMBEDDING_DIMENSIONS: Setting = Setting::new(
    "AUTOXIDE_EMBEDDING_DIMENSIONS",
    "Length of the vectors the model returns, 1 to 8192; required when embeddings are on. \
     Changing the model or this value rebuilds the search index.",
)
.with_example("1024");

pub(crate) const EMBEDDING_BATCH_SIZE: Setting = Setting::new(
    "AUTOXIDE_EMBEDDING_BATCH_SIZE",
    "Texts sent in one embeddings request, 1 to 2048.",
)
.with_default("64");

pub(crate) const EMBEDDING_SEND_DIMENSIONS: Setting = Setting::new(
    "AUTOXIDE_EMBEDDING_SEND_DIMENSIONS",
    "Send `dimensions` with each request so the model shortens its vectors (OpenAI \
     `text-embedding-3-*` accepts it): `true` or `false`.",
)
.with_default("false");

/// Nhà cung cấp sinh chữ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Provider {
    Anthropic,
    OpenAi,
}

impl Provider {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAi => "openai",
        }
    }
}

/// Cấu hình sinh chữ khi đã bật.
#[derive(Debug, Clone)]
pub(crate) struct TextConfig {
    pub provider: Provider,
    pub api_key: Option<String>,
    /// Không có `/` ở cuối.
    pub base_url: String,
    pub model: String,
    pub images: bool,
    pub timeout: Duration,
    pub anthropic_fallback: bool,
    /// `true`: `response_format` `json_schema`; `false`: `json_object`.
    pub openai_strict: bool,
    pub openai_send_effort: bool,
    pub openai_max_tokens_param: String,
}

/// Cấu hình embedding khi đã bật.
#[derive(Debug, Clone)]
pub(crate) struct EmbeddingConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub dimensions: usize,
    pub batch_size: usize,
    pub send_dimensions: bool,
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
pub(crate) struct AiConfig {
    /// `None`: sinh chữ tắt.
    pub text: Option<TextConfig>,
    /// `None`: embedding tắt.
    pub embedding: Option<EmbeddingConfig>,
    /// `0`: không giới hạn.
    pub daily_token_limit: u64,
    pub usage_retention_days: u32,
}

impl AiConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let provider = reader.get_with(&PROVIDER, |raw| match raw.trim() {
            "none" => Ok(None),
            "anthropic" => Ok(Some(Provider::Anthropic)),
            "openai" => Ok(Some(Provider::OpenAi)),
            _ => Err("must be `none`, `anthropic` or `openai`".to_owned()),
        });
        let api_key = reader.optional_with(&API_KEY, non_empty);
        let base_url = reader.optional_with(&BASE_URL, url);
        let model = reader.optional_with(&MODEL, non_empty);
        let images = reader.get_with(&IMAGES, parse_bool);
        let timeout = reader.get_with(&TIMEOUT_SECONDS, |raw| parse_in_range(raw, 1u64, 3600));
        let limit = reader.get_with(&DAILY_TOKEN_LIMIT, |raw| parse_in_range(raw, 0u64, u64::MAX));
        let retention =
            reader.get_with(&USAGE_RETENTION_DAYS, |raw| parse_in_range(raw, 1u32, 3650));
        let fallback = reader.get_with(&ANTHROPIC_FALLBACK, parse_bool);
        let strict = reader.get_with(&OPENAI_SCHEMA_MODE, |raw| match raw.trim() {
            "strict" => Ok(true),
            "json_object" => Ok(false),
            _ => Err("must be `strict` or `json_object`".to_owned()),
        });
        let send_effort = reader.get_with(&OPENAI_SEND_EFFORT, parse_bool);
        let max_tokens_param = reader.get_with(&OPENAI_MAX_TOKENS_PARAM, |raw| match raw.trim() {
            param @ ("max_completion_tokens" | "max_tokens") => Ok(param.to_owned()),
            _ => Err("must be `max_completion_tokens` or `max_tokens`".to_owned()),
        });

        let embedding_on = reader.get_with(&EMBEDDING_PROVIDER, |raw| match raw.trim() {
            "none" => Ok(false),
            "openai" => Ok(true),
            _ => Err("must be `none` or `openai`".to_owned()),
        });
        let embedding_base_url = reader.get_with(&EMBEDDING_BASE_URL, url);
        let embedding_key = reader.optional_with(&EMBEDDING_API_KEY, non_empty);
        let embedding_model = reader.optional_with(&EMBEDDING_MODEL, non_empty);
        let dimensions =
            reader.optional_with(&EMBEDDING_DIMENSIONS, |raw| parse_in_range(raw, 1usize, 8192));
        let batch_size =
            reader.get_with(&EMBEDDING_BATCH_SIZE, |raw| parse_in_range(raw, 1usize, 2048));
        let send_dimensions = reader.get_with(&EMBEDDING_SEND_DIMENSIONS, parse_bool);

        let timeout = Duration::from_secs(timeout?);
        let text = match provider? {
            None => None,
            Some(provider) => {
                let api_key = api_key?.flatten();
                if provider == Provider::Anthropic && api_key.is_none() {
                    return reader
                        .reject(&API_KEY, "is required when AUTOXIDE_AI_PROVIDER is `anthropic`");
                }
                let model = match (model?.flatten(), provider) {
                    (Some(model), _) => model,
                    (None, Provider::Anthropic) => "claude-opus-5-5".to_owned(),
                    (None, Provider::OpenAi) => {
                        return reader.reject(
                            &MODEL,
                            "is required when AUTOXIDE_AI_PROVIDER is `openai`: name the model of \
                             your service (for example `gpt-…`, `gemini-…` or a local model)",
                        );
                    }
                };
                let base_url = base_url?.unwrap_or_else(|| {
                    match provider {
                        Provider::Anthropic => "https://api.anthropic.com",
                        Provider::OpenAi => "https://api.openai.com/v1",
                    }
                    .to_owned()
                });
                Some(TextConfig {
                    provider,
                    api_key,
                    base_url,
                    model,
                    images: images?,
                    timeout,
                    anthropic_fallback: fallback?,
                    openai_strict: strict?,
                    openai_send_effort: send_effort?,
                    openai_max_tokens_param: max_tokens_param?,
                })
            }
        };
        let embedding = if embedding_on? {
            let Some(model) = embedding_model?.flatten() else {
                return reader.reject(
                    &EMBEDDING_MODEL,
                    "is required when AUTOXIDE_EMBEDDING_PROVIDER is `openai`",
                );
            };
            let Some(dimensions) = dimensions? else {
                return reader.reject(
                    &EMBEDDING_DIMENSIONS,
                    "is required when AUTOXIDE_EMBEDDING_PROVIDER is `openai`",
                );
            };
            Some(EmbeddingConfig {
                base_url: embedding_base_url?,
                api_key: embedding_key?.flatten(),
                model,
                dimensions,
                batch_size: batch_size?,
                send_dimensions: send_dimensions?,
                timeout,
            })
        } else {
            None
        };
        Ok(AiConfig {
            text,
            embedding,
            daily_token_limit: limit?,
            usage_retention_days: retention?,
        })
    }
}

/// Chuỗi rỗng (sau khi bỏ khoảng trắng) coi như không đặt.
fn non_empty(raw: &str) -> Result<Option<String>, String> {
    let trimmed = raw.trim();
    Ok((!trimmed.is_empty()).then(|| trimmed.to_owned()))
}

/// Địa chỉ `http://` hay `https://`, bỏ `/` ở cuối.
fn url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/');
    let rest = trimmed.strip_prefix("https://").or_else(|| trimmed.strip_prefix("http://"));
    match rest {
        Some(host) if !host.is_empty() && !host.contains(char::is_whitespace) => {
            Ok(trimmed.to_owned())
        }
        _ => Err("must be an address starting with `https://` or `http://`".to_owned()),
    }
}

fn parse_bool(raw: &str) -> Result<bool, String> {
    match raw.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("must be `true` or `false`".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::config::ConfigSource;

    use super::*;

    pub(crate) fn read(pairs: &[(&str, &str)]) -> Result<AiConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = AiConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mac_dinh_tat_het() {
        let config = read(&[]).unwrap();
        assert!(config.text.is_none() && config.embedding.is_none());
        assert_eq!((config.daily_token_limit, config.usage_retention_days), (2_000_000, 90));
    }

    #[test]
    fn anthropic_can_key_va_co_model_mac_dinh() {
        let error = read(&[("AUTOXIDE_AI_PROVIDER", "anthropic")]).unwrap_err();
        assert!(error.contains("AUTOXIDE_AI_API_KEY"), "{error}");
        let config =
            read(&[("AUTOXIDE_AI_PROVIDER", "anthropic"), ("AUTOXIDE_AI_API_KEY", "k")]).unwrap();
        let text = config.text.unwrap();
        assert_eq!(
            (text.model.as_str(), text.base_url.as_str()),
            ("claude-opus-5-5", "https://api.anthropic.com")
        );
    }

    #[test]
    fn openai_can_model_khong_can_key() {
        let error = read(&[("AUTOXIDE_AI_PROVIDER", "openai")]).unwrap_err();
        assert!(error.contains("AUTOXIDE_AI_MODEL"), "{error}");
        let config = read(&[
            ("AUTOXIDE_AI_PROVIDER", "openai"),
            ("AUTOXIDE_AI_MODEL", "llama3"),
            ("AUTOXIDE_AI_BASE_URL", "http://localhost:11434/v1/"),
        ])
        .unwrap();
        let text = config.text.unwrap();
        assert_eq!((text.base_url.as_str(), text.api_key), ("http://localhost:11434/v1", None));
        assert!(read(&[("AUTOXIDE_AI_BASE_URL", "ftp://x")]).unwrap_err().contains("https://"));
    }

    #[test]
    fn embedding_can_model_va_so_chieu() {
        let error = read(&[("AUTOXIDE_EMBEDDING_PROVIDER", "openai")]).unwrap_err();
        assert!(error.contains("AUTOXIDE_EMBEDDING_MODEL"), "{error}");
        let config = read(&[
            ("AUTOXIDE_EMBEDDING_PROVIDER", "openai"),
            ("AUTOXIDE_EMBEDDING_MODEL", "bge-m3"),
            ("AUTOXIDE_EMBEDDING_DIMENSIONS", "1024"),
        ])
        .unwrap();
        assert_eq!(config.embedding.unwrap().dimensions, 1024);
    }
}
