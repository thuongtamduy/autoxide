//! AI: trait chung để module dùng mô hình ngôn ngữ và embedding mà không phụ thuộc nhà cung cấp
//! nào (D-109).
//!
//! Một module (`ai`) công bố `dyn TextModel` và `dyn Embedder`; module cần AI lấy bằng
//! `context.require::<dyn TextModel>()` (đăng ký SAU module `ai`). Khi người cài chưa bật AI,
//! module `ai` vẫn công bố bản "đã tắt": mọi lời gọi trả lỗi `ai.disabled`, nên module dùng nó
//! không phải rẽ nhánh — chỉ hỏi [`TextModel::capabilities`] để biết có nên mời người dùng không.
//!
//! Khái niệm của nhà cung cấp (tên tham số, `stop_reason`…) không ra khỏi module `ai`; khái niệm
//! nghiệp vụ (bài viết, SEO…) không vào đây.

use serde_json::Value;

use crate::error::Error;
use crate::tenancy::TenantContext;

/// Mức độ kỹ khi mô hình suy nghĩ: đổi lấy chi phí và thời gian. Mỗi nhà cung cấp tự đổi sang
/// tham số của mình (hay bỏ qua nếu không có).
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

/// Một phần dữ liệu đưa cho mô hình.
#[derive(Debug, Clone, PartialEq, Eq)]
// Cố ý KHÔNG `#[non_exhaustive]` (D-167): thêm một loại dữ liệu mới phải buộc bên cài
// `TextModel` xử lý nó; nhánh `_` sẽ lặng lẽ bỏ dữ liệu khỏi yêu cầu.
pub enum Part {
    Text(String),
    /// Ảnh raster (`image/jpeg`, `image/png`, `image/gif`, `image/webp`).
    Image {
        media_type: String,
        bytes: Vec<u8>,
    },
}

/// Một yêu cầu sinh kết quả có cấu trúc. Dựng bằng [`Prompt::new`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Prompt {
    /// `<module>.<việc>` (vd `content.translate`): dùng cho hạn mức, log và metrics — là một
    /// tập nhỏ cố định do code khai, không bao giờ là dữ liệu người dùng.
    pub purpose: &'static str,
    /// Luật cho mô hình (system prompt). Chỉ do code viết; dữ liệu người dùng không vào đây.
    pub instructions: String,
    /// Dữ liệu cần xử lý — có thể chứa chữ người dùng viết, nên coi là không tin được.
    pub input: Vec<Part>,
    /// JSON Schema kết quả phải theo, trong phần con mà mọi nhà cung cấp nhận (xem
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

/// Số token một lời gọi đã dùng (để tính hạn mức và chi phí).
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

/// Kết quả của [`TextModel::generate`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Generation {
    /// Đã khớp `output_schema` (đã được kiểm lại, không tin nhà cung cấp).
    pub output: Value,
    pub usage: Usage,
    /// Mô hình đã trả lời (có thể khác mô hình cấu hình khi nhà cung cấp tự chuyển).
    pub model: String,
}

impl Generation {
    /// A result: the output, the tokens used and the model that answered.
    pub const fn new(output: Value, usage: Usage, model: String) -> Self {
        Generation { output, usage, model }
    }
}

/// Mô hình đang dùng làm được gì. Dựng bằng [`Capabilities::disabled`] hay
/// [`Capabilities::enabled`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Capabilities {
    /// AI đã được bật và cấu hình.
    pub enabled: bool,
    /// Tên nhà cung cấp (`anthropic`, `openai`) — để hiển thị, không để rẽ nhánh.
    pub provider: String,
    pub model: String,
    /// Nhận ảnh làm đầu vào (cho gợi ý alt text).
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

/// Mô hình sinh chữ.
//
// `Send + Sync`: một đối tượng dùng chung giữa mọi request (qua `Arc`), có thể ở nhiều thread
// cùng lúc. `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`.
#[async_trait::async_trait]
pub trait TextModel: Send + Sync {
    fn capabilities(&self) -> Capabilities;

    /// `Ok` khi đã bật; không thì đúng lỗi mà [`generate`](Self::generate) sẽ trả
    /// (`ai.disabled`). Gọi trước khi làm việc tốn kém (đọc ảnh, xếp job) để báo lỗi sớm.
    fn ready(&self) -> Result<(), Error>;

    /// Sinh một kết quả khớp `prompt.output_schema`. Lỗi có `code` ổn định (`ai.disabled`,
    /// `ai.quota_exceeded`, `ai.provider_unavailable`, `ai.refused`, `ai.output_truncated`,
    /// `ai.output_invalid`, `ai.unsupported_input`, `ai.misconfigured`).
    ///
    /// `tenant`: khách mà việc này làm cho — khoá, hạn mức và số đếm theo khách.
    async fn generate(&self, tenant: &TenantContext, prompt: Prompt) -> Result<Generation, Error>;
}

/// Đoạn chữ được tính embedding để làm gì (một số mô hình cần biết; mô hình khác bỏ qua).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EmbedKind {
    /// Nội dung được lập index.
    Document,
    /// Câu tìm kiếm.
    Query,
}

/// Mô hình tính embedding (dãy số biểu diễn ý nghĩa của đoạn chữ).
#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    /// Đã bật và cấu hình.
    fn enabled(&self) -> bool;
    /// `Ok` khi đã bật; không thì lỗi `ai.embedding_disabled`.
    fn ready(&self) -> Result<(), Error>;
    /// Tên mô hình — lưu cùng vector để biết khi nào phải lập lại index.
    fn model_id(&self) -> &str;
    /// Số chiều của mỗi vector.
    fn dimensions(&self) -> usize;
    /// Một vector cho mỗi phần tử của `texts`, cùng thứ tự. `purpose`: như [`Prompt::purpose`].
    async fn embed(
        &self,
        tenant: &TenantContext,
        purpose: &'static str,
        texts: &[String],
        kind: EmbedKind,
    ) -> Result<Vec<Vec<f32>>, Error>;
}

/// Kiểm một giá trị JSON theo phần con của JSON Schema mà Autoxide dùng — phần mọi nhà cung cấp
/// đều nhận: `type` (`object`, `array`, `string`, `integer`, `number`, `boolean`, `null`, hay
/// danh sách các kiểu), `properties`, `required`, `additionalProperties: false`, `items`,
/// `enum`, `description`. Độ dài chuỗi và số phần tử nhà cung cấp **không** bảo đảm (Anthropic
/// ghi rõ không ép), nên không dùng trong schema gửi đi — code của tính năng tự kiểm.
pub mod schema {
    use serde_json::Value;

    /// Mọi chỗ sai, mỗi chỗ một câu có vị trí (`$.items[2].id: …`). Rỗng: hợp lệ.
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

/// Vector embedding: mã hoá để lưu trong database (cột nhị phân, như nhau trên SQLite và
/// Postgres), chuẩn hoá, so sánh. Tính trong Rust để hai database cho cùng kết quả (D-109).
pub mod vector {
    /// `f32` little-endian nối nhau — 4 byte mỗi chiều, thứ tự byte cố định trên mọi máy.
    pub fn encode(vector: &[f32]) -> Vec<u8> {
        vector.iter().flat_map(|value| value.to_le_bytes()).collect()
    }

    /// Ngược của [`encode`]. `None` khi số byte không chia hết cho 4.
    pub fn decode(bytes: &[u8]) -> Option<Vec<f32>> {
        if !bytes.len().is_multiple_of(4) {
            return None;
        }
        // `as_chunks::<4>()`: cắt thành các mảng đúng 4 byte (phần dư rỗng vì đã kiểm ở trên).
        Some(bytes.as_chunks::<4>().0.iter().map(|chunk| f32::from_le_bytes(*chunk)).collect())
    }

    /// Chia cho độ dài để vector dài 1: khi đó cosine chỉ còn là tích vô hướng ([`dot`]).
    /// Vector toàn số 0 giữ nguyên.
    pub fn normalize(vector: &mut [f32]) {
        let length = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        if length > 0.0 {
            for value in vector.iter_mut() {
                *value /= length;
            }
        }
    }

    /// Tích vô hướng; với hai vector đã [`normalize`] thì là cosine (−1 … 1). Hai vector khác
    /// độ dài: tính trên phần chung.
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
