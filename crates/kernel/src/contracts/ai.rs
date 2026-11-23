//! AI: shared traits that let modules use language models and embeddings without depending on any specific
//! provider (D-109).
//!
//! One module (`ai`) publishes `dyn TextModel` and `dyn Embedder`; a module that needs AI gets them with
//! `context.require::<dyn TextModel>()` (register AFTER the `ai` module). When the installer has not enabled
//! AI, the `ai` module still publishes a "disabled" implementation: every call returns the `ai.disabled`
//! error, so a module using it does not have to branch; it only asks [`TextModel::capabilities`] to learn
//! whether to offer the feature to users.
//!
//! Provider concepts (parameter names, `stop_reason`, ...) never leave the `ai` module; business concepts
//! (articles, SEO, ...) do not enter here.

use serde_json::Value;

use crate::error::Error;
use crate::tenancy::TenantContext;

/// How hard the model thinks: trades cost and latency for quality. Each provider maps this to its own parameter
/// (or ignores it if it has none).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Effort {
    Low,
    Medium,
    High,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
        }
    }
}

/// One piece of data passed to the model.
#[derive(Debug, Clone, PartialEq, Eq)]
// Cố ý KHÔNG `#[non_exhaustive]` (D-167): thêm một loại dữ liệu mới phải buộc bên cài
// `TextModel` xử lý nó; nhánh `_` sẽ lặng lẽ bỏ dữ liệu khỏi yêu cầu.
pub enum Part {
    Text(String),
    /// Raster image (`image/jpeg`, `image/png`, `image/gif`, `image/webp`).
    Image {
        media_type: String,
        bytes: Vec<u8>,
    },
}

/// A request for structured output. Build it with [`Prompt::new`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Prompt {
    /// `<module>.<action>` (e.g. `content.translate`): used for quotas, logs and metrics. It is a small fixed
    /// set declared in code and is never user data.
    pub purpose: &'static str,
    /// Rules for the model (system prompt). Written by code only; user data never goes here.
    pub instructions: String,
    /// Data to process. It may contain user-written text, so treat it as untrusted.
    pub input: Vec<Part>,
    /// JSON Schema the output must follow, restricted to the subset every provider accepts (see
    /// [`schema`]).
    pub output_schema: Value,
    pub effort: Effort,
    pub max_output_tokens: u32,
}

impl Prompt {
    /// A request with no input yet, [`Effort::Low`] and at most
    /// [`DEFAULT_MAX_OUTPUT_TOKENS`](Self::DEFAULT_MAX_OUTPUT_TOKENS) output tokens.
    pub const fn new(purpose: &'static str, instructions: String, output_schema: Value) -> Self {
        Prompt {
            purpose,
            instructions,
            input: Vec::new(),
            output_schema,
            effort: Effort::Low,
            max_output_tokens: Self::DEFAULT_MAX_OUTPUT_TOKENS,
        }
    }

    /// Output token limit of [`Prompt::new`].
    pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 1024;

    /// Sets the data to process.
    pub fn input(mut self, input: Vec<Part>) -> Self {
        self.input = input;
        self
    }

    /// Sets how much effort the model spends thinking.
    pub const fn effort(mut self, effort: Effort) -> Self {
        self.effort = effort;
        self
    }

    /// Sets the maximum number of output tokens.
    pub const fn max_output_tokens(mut self, max_output_tokens: u32) -> Self {
        self.max_output_tokens = max_output_tokens;
        self
    }
}

/// Tokens used by one call (for quota and cost accounting).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Usage {
    /// Tokens read and written by one call.
    pub const fn new(input_tokens: u64, output_tokens: u64) -> Self {
        Usage { input_tokens, output_tokens }
    }
}

/// Result of [`TextModel::generate`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Generation {
    /// Matches `output_schema` (re-validated; the provider is not trusted).
    pub output: Value,
    pub usage: Usage,
    /// The model that answered (may differ from the configured model when the provider switches by itself).
    pub model: String,
}

impl Generation {
    /// A result: the output, the tokens used and the model that answered.
    pub const fn new(output: Value, usage: Usage, model: String) -> Self {
        Generation { output, usage, model }
    }
}

/// What the current model can do. Build it with [`Capabilities::disabled`] or
/// [`Capabilities::enabled`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Capabilities {
    /// AI is enabled and configured.
    pub enabled: bool,
    /// Provider name (`anthropic`, `openai`): for display, not for branching.
    pub provider: String,
    pub model: String,
    /// Accepts images as input (for alt text suggestions).
    pub images: bool,
}

impl Capabilities {
    /// AI is off: no provider, no model, no images (same as `Default`).
    pub const fn disabled() -> Self {
        Capabilities {
            enabled: false,
            provider: String::new(),
            model: String::new(),
            images: false,
        }
    }

    /// AI is on with this provider and model; images are not accepted until
    /// [`images`](Self::images) says so.
    pub const fn enabled(provider: String, model: String) -> Self {
        Capabilities { enabled: true, provider, model, images: false }
    }

    /// Sets whether the model accepts images as input.
    pub const fn images(mut self, images: bool) -> Self {
        self.images = images;
        self
    }
}

/// A text-generating model.
//
// `Send + Sync`: một đối tượng dùng chung giữa mọi request (qua `Arc`), có thể ở nhiều thread
// cùng lúc. `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`.
#[async_trait::async_trait]
pub trait TextModel: Send + Sync {
    fn capabilities(&self) -> Capabilities;

    /// `Ok` when enabled; otherwise exactly the error [`generate`](Self::generate) would return
    /// (`ai.disabled`). Call it before expensive work (reading images, queueing jobs) to fail early.
    fn ready(&self) -> Result<(), Error>;

    /// Generates one result matching `prompt.output_schema`. Errors carry a stable `code` (`ai.disabled`,
    /// `ai.quota_exceeded`, `ai.provider_unavailable`, `ai.refused`, `ai.output_truncated`,
    /// `ai.output_invalid`, `ai.unsupported_input`, `ai.misconfigured`).
    ///
    /// `tenant`: the tenant this work is done for; keys, quotas and counters are per tenant.
    async fn generate(&self, tenant: &TenantContext, prompt: Prompt) -> Result<Generation, Error>;
}

/// What a piece of text is embedded for (some models need to know; others ignore it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EmbedKind {
    /// Content being indexed.
    Document,
    /// A search query.
    Query,
}

/// A model that computes embeddings (numeric vectors representing the meaning of a text).
#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    /// Enabled and configured.
    fn enabled(&self) -> bool;
    /// `Ok` when enabled; otherwise the `ai.embedding_disabled` error.
    fn ready(&self) -> Result<(), Error>;
    /// Model name, stored with the vector to know when the index must be rebuilt.
    fn model_id(&self) -> &str;
    /// Number of dimensions of each vector.
    fn dimensions(&self) -> usize;
    /// One vector per element of `texts`, in the same order. `purpose`: same as [`Prompt::purpose`].
    async fn embed(
        &self,
        tenant: &TenantContext,
        purpose: &'static str,
        texts: &[String],
        kind: EmbedKind,
    ) -> Result<Vec<Vec<f32>>, Error>;
}

/// Validates a JSON value against the JSON Schema subset Autoxide uses, which every provider accepts: `type`
/// (`object`, `array`, `string`, `integer`, `number`, `boolean`, `null`, or a list of types), `properties`,
/// `required`, `additionalProperties: false`, `items`, `enum`, `description`. Providers do **not** guarantee
/// string length or item count limits (Anthropic states it does not enforce them), so they are not used in
/// schemas sent out; feature code checks them itself.
pub mod schema {
    use serde_json::Value;

    /// Every violation, one sentence each with its location (`$.items[2].id: ...`). Empty: valid.
    pub fn problems(schema: &Value, value: &Value) -> Vec<String> {
        let mut found = Vec::new();
        check(schema, value, "$", &mut found);
        found
    }

    fn check(schema: &Value, value: &Value, at: &str, found: &mut Vec<String>) {
        if let Some(types) = schema.get("type") {
            let allowed: Vec<&str> = match types {
                Value::String(one) => vec![one.as_str()],
                Value::Array(many) => many.iter().filter_map(Value::as_str).collect(),
                _ => Vec::new(),
            };
            if !allowed.iter().any(|expected| is_type(value, expected)) {
                found.push(format!("{at}: expected {}", allowed.join(" or ")));
                return;
            }
        }
        if let Some(Value::Array(options)) = schema.get("enum")
            && !options.contains(value)
        {
            found.push(format!("{at}: not one of the allowed values"));
        }
        match value {
            Value::Object(object) => {
                let properties = schema.get("properties").and_then(Value::as_object);
                if let Some(Value::Array(required)) = schema.get("required") {
                    for name in required.iter().filter_map(Value::as_str) {
                        if !object.contains_key(name) {
                            found.push(format!("{at}.{name}: missing"));
                        }
                    }
                }
                let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
                for (name, item) in object {
                    match properties.and_then(|properties| properties.get(name)) {
                        Some(inner) => check(inner, item, &format!("{at}.{name}"), found),
                        None if closed => found.push(format!("{at}.{name}: not allowed")),
                        None => {}
                    }
                }
            }
            Value::Array(items) => {
                if let Some(inner) = schema.get("items") {
                    for (index, item) in items.iter().enumerate() {
                        check(inner, item, &format!("{at}[{index}]"), found);
                    }
                }
            }
            _ => {}
        }
    }

    fn is_type(value: &Value, expected: &str) -> bool {
        match expected {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => false,
        }
    }

    #[cfg(test)]
    mod tests {
        use serde_json::json;

        use super::problems;

        #[test]
        fn kiem_kieu_truong_bat_buoc_va_truong_thua() {
            let schema = json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "tags": {"type": "array", "items": {"type": "string", "enum": ["a", "b"]}},
                    "note": {"type": ["string", "null"]}
                },
                "required": ["title", "tags", "note"],
                "additionalProperties": false
            });
            assert!(
                problems(&schema, &json!({"title": "x", "tags": ["a"], "note": null})).is_empty()
            );
            let found = problems(&schema, &json!({"title": 1, "tags": ["c"], "extra": true}));
            assert_eq!(
                found,
                [
                    "$.note: missing",
                    "$.extra: not allowed",
                    "$.tags[0]: not one of the allowed values",
                    "$.title: expected string"
                ]
            );
            assert_eq!(problems(&schema, &json!([])), ["$: expected object"]);
        }
    }
}

/// Embedding vector: encoding for storage in the database (a binary column, identical on SQLite and
/// Postgres), normalization and comparison. Computed in Rust so both databases give the same result (D-109).
pub mod vector {
    /// `f32` little-endian values concatenated: 4 bytes per dimension, with a fixed byte order on every machine.
    pub fn encode(vector: &[f32]) -> Vec<u8> {
        vector.iter().flat_map(|value| value.to_le_bytes()).collect()
    }

    /// Inverse of [`encode`]. `None` when the byte count is not divisible by 4.
    pub fn decode(bytes: &[u8]) -> Option<Vec<f32>> {
        if !bytes.len().is_multiple_of(4) {
            return None;
        }
        // `as_chunks::<4>()`: cắt thành các mảng đúng 4 byte (phần dư rỗng vì đã kiểm ở trên).
        Some(bytes.as_chunks::<4>().0.iter().map(|chunk| f32::from_le_bytes(*chunk)).collect())
    }

    /// Divides by the length so the vector has length 1; cosine then reduces to the dot product ([`dot`]).
    /// An all-zero vector is left unchanged.
    pub fn normalize(vector: &mut [f32]) {
        let length = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        if length > 0.0 {
            for value in vector.iter_mut() {
                *value /= length;
            }
        }
    }

    /// Dot product; for two [`normalize`]d vectors it is the cosine (-1 ... 1). Two vectors of different
    /// length: computed over the common part.
    //
    // Cộng vào 8 tổng riêng rồi gộp (bước 39 B6, `docs/BENCHMARKS.md` "Tìm theo ý nghĩa"): cộng
    // số thực tuần tự vào MỘT tổng thì trình biên dịch không được đổi thứ tự (kết quả số thực phụ
    // thuộc thứ tự), nên không dùng lệnh SIMD của CPU; 8 tổng độc lập thì được — nhanh gấp ~2,6 lần
    // (100 000 vector × 1 024 chiều: 137 → 52 ms), không cần `unsafe`. Kết quả lệch ở chữ số cuối,
    // thứ hạng tìm không đổi.
    pub fn dot(a: &[f32], b: &[f32]) -> f32 {
        const LANES: usize = 8;
        let length = a.len().min(b.len());
        let (a, b) = (&a[..length], &b[..length]);
        // `as_chunks`: mảng đúng `LANES` phần tử (độ dài biết lúc biên dịch) và phần dư.
        let ((a_lanes, a_tail), (b_lanes, b_tail)) =
            (a.as_chunks::<LANES>(), b.as_chunks::<LANES>());
        let mut sums = [0.0f32; LANES];
        for (x, y) in a_lanes.iter().zip(b_lanes) {
            for lane in 0..LANES {
                sums[lane] += x[lane] * y[lane];
            }
        }
        let tail: f32 = a_tail.iter().zip(b_tail).map(|(x, y)| x * y).sum();
        sums.iter().sum::<f32>() + tail
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn dot_khop_cach_tinh_tuan_tu() {
            for length in [0, 1, 7, 8, 9, 17, 1024, 1027] {
                let a: Vec<f32> =
                    (0..length).map(|i| ((i * 37 % 101) as f32 - 50.0) / 50.0).collect();
                let b: Vec<f32> =
                    (0..length).map(|i| ((i * 53 % 97) as f32 - 48.0) / 48.0).collect();
                let plain: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
                assert!((dot(&a, &b) - plain).abs() < 1e-3, "{length}");
            }
            // Khác độ dài: phần chung.
            assert!((dot(&[1.0, 2.0, 3.0], &[1.0, 1.0]) - 3.0).abs() < f32::EPSILON);
        }

        #[test]
        fn ma_hoa_giai_ma_va_cosine() {
            let mut a = vec![3.0, 4.0];
            normalize(&mut a);
            assert_eq!(a, [0.6, 0.8]);
            assert_eq!(decode(&encode(&a)).unwrap(), a);
            assert_eq!(decode(&[1, 2, 3]), None);
            let mut b = vec![4.0, 3.0];
            normalize(&mut b);
            assert!((dot(&a, &b) - 0.96).abs() < 1e-6);
            let mut zero = vec![0.0, 0.0];
            normalize(&mut zero);
            assert_eq!(zero, [0.0, 0.0]);
        }
    }
}
