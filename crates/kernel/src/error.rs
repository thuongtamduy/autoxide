//! Kiểu lỗi chung cho mọi layer.
//!
//! Service, handler REST và tool MCP đều trả [`Error`] này. Nó mang đủ thông tin
//! cho hai người đọc khác nhau:
//!
//! - **client** (người hoặc AI agent): `code` ổn định để máy rẽ nhánh, thông báo
//!   nói rõ phải sửa gì, và danh sách lỗi theo từng trường với lỗi đầu vào;
//! - **người vận hành**: nguyên nhân gốc (`source`) — chỉ ghi vào log, KHÔNG BAO
//!   GIỜ gửi cho client.
//!
//! `code` là hợp đồng với client: đổi một mã là thay đổi phá vỡ. `code` cũng
//! chính là i18n key của thông báo; câu tiếng Anh viết ở chỗ tạo lỗi là bản mặc
//! định, bản dịch tra theo `code` (docs/DECISIONS.md D-039).
//!
//! ```
//! use autoxide_kernel::error::Error;
//!
//! fn find(id: u32) -> Result<&'static str, Error> {
//!     Err(Error::not_found("notes.note_not_found", "Note {id} does not exist.")
//!         .with_param("id", id))
//! }
//! let error = find(7).unwrap_err();
//! assert_eq!(error.code(), "notes.note_not_found");
//! assert_eq!(error.message(), "Note 7 does not exist.");
//! ```

use std::borrow::Cow;
use std::fmt;
use std::num::NonZeroU32;

use validator::{ValidationErrors, ValidationErrorsKind};

/// Loại lỗi; quyết định mã trạng thái HTTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// Request không đọc được (JSON hỏng, thiếu tham số, sai kiểu). HTTP 400.
    BadRequest,
    /// Chưa xác thực, hoặc thông tin xác thực không hợp lệ. HTTP 401.
    Unauthenticated,
    /// Đã xác thực nhưng không có permission. HTTP 403.
    Forbidden,
    /// Không có tài nguyên đó. HTTP 404.
    NotFound,
    /// Trái với trạng thái hiện tại (trùng giá trị unique, sửa bản đã cũ). HTTP 409.
    Conflict,
    /// Điều kiện của request không còn đúng (`If-Match` cũ: bản ghi đã bị sửa). HTTP 412.
    PreconditionFailed,
    /// Thân request lớn hơn giới hạn. HTTP 413.
    PayloadTooLarge,
    /// Kiểu nội dung của request không được hỗ trợ. HTTP 415.
    UnsupportedMediaType,
    /// Khoảng byte (`Range`) xin đọc nằm ngoài nội dung. HTTP 416.
    RangeNotSatisfiable,
    /// Request đọc được nhưng giá trị không qua kiểm tra. HTTP 422.
    Validation,
    /// Endpoint bắt buộc gửi điều kiện (`If-Match`) mà request không gửi. HTTP 428.
    PreconditionRequired,
    /// Gọi quá nhiều. HTTP 429.
    TooManyRequests,
    /// Tạm ẩn vì lý do pháp lý (vd vụ việc vi phạm đang xử lý). HTTP 451 (RFC 7725).
    UnavailableForLegalReasons,
    /// Lỗi phía máy chủ; chi tiết chỉ nằm trong log. HTTP 500.
    Internal,
    /// Tạm thời chưa phục vụ được (database chưa sẵn sàng, đang tắt). HTTP 503.
    Unavailable,
}

impl ErrorKind {
    /// Mã trạng thái HTTP tương ứng.
    pub fn status(self) -> u16 {
        match self {
            ErrorKind::BadRequest => 400,
            ErrorKind::Unauthenticated => 401,
            ErrorKind::Forbidden => 403,
            ErrorKind::NotFound => 404,
            ErrorKind::Conflict => 409,
            ErrorKind::PreconditionFailed => 412,
            ErrorKind::PayloadTooLarge => 413,
            ErrorKind::UnsupportedMediaType => 415,
            ErrorKind::RangeNotSatisfiable => 416,
            ErrorKind::Validation => 422,
            ErrorKind::PreconditionRequired => 428,
            ErrorKind::TooManyRequests => 429,
            ErrorKind::UnavailableForLegalReasons => 451,
            ErrorKind::Internal => 500,
            ErrorKind::Unavailable => 503,
        }
    }
}

/// Lỗi của một trường trong dữ liệu đầu vào.
///
/// Câu viết thành **mẫu** tiếng Anh, chỗ thay đổi là `{tên}` điền bằng
/// [`with_param`](Self::with_param) — không ghép sẵn bằng `format!`: bản dịch tra theo đúng câu
/// mẫu (khoá `field:<câu mẫu>` trong `locales/vi.json`, như gettext dùng câu gốc làm khoá), rồi
/// điền giá trị (D-262). `cargo xtask check` chặn câu chưa dịch và câu ghép sẵn.
///
/// ```
/// use autoxide_kernel::error::FieldError;
///
/// let error = FieldError::new("title", "length", "must be at most {max} characters long")
///     .with_param("max", 200);
/// assert_eq!(error.message, "must be at most 200 characters long");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FieldError {
    /// Đường dẫn tới trường: `name`, `address.city`, `items[2].quantity`.
    pub field: String,
    /// Mã ổn định của loại lỗi: `required`, `length`, `range`, `email`…
    pub code: String,
    /// Câu tiếng Anh đã điền giá trị: nói rõ giá trị phải như thế nào.
    pub message: String,
    /// Câu mẫu (khoá dịch) và giá trị điền vào nó.
    template: Cow<'static, str>,
    params: Vec<(Cow<'static, str>, String)>,
}

impl FieldError {
    /// Lỗi của trường `field`, mã `code`, câu mẫu `message` (có thể chứa `{tên}`).
    pub fn new(
        field: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        let template = message.into();
        FieldError {
            field: field.into(),
            code: code.into(),
            message: template.to_string(),
            template,
            params: Vec::new(),
        }
    }

    /// Điền `{name}` của câu mẫu bằng `value`.
    #[must_use]
    pub fn with_param(mut self, name: impl Into<Cow<'static, str>>, value: impl ToString) -> Self {
        self.params.push((name.into(), value.to_string()));
        // Điền một lượt (D-150): giá trị người dùng gõ có `{x}` không bị thay tiếp. Còn chỗ chưa
        // có giá trị (đang gán dở) thì giữ câu mẫu.
        let params: Vec<(&str, &str)> = self.params().collect();
        self.message = crate::i18n::fill_all(&self.template, &params)
            .unwrap_or_else(|| self.template.to_string());
        self
    }

    /// Câu mẫu tiếng Anh, chưa điền (khoá dịch).
    pub fn template(&self) -> &str {
        &self.template
    }

    /// Các giá trị điền vào câu mẫu, theo thứ tự đã gán.
    pub fn params(&self) -> impl Iterator<Item = (&str, &str)> {
        self.params.iter().map(|(name, value)| (name.as_ref(), value.as_str()))
    }

    /// Lỗi `required` nếu `value` rỗng (hoặc chỉ có khoảng trắng). Dùng trong service khi
    /// gom mọi lỗi của một đầu vào để báo một lần.
    pub fn blank(field: impl Into<String>, value: &str) -> Option<Self> {
        value.trim().is_empty().then(|| FieldError::new(field, "required", "must not be empty"))
    }
}

/// Nguyên nhân gốc của một lỗi, giữ cho log.
type Source = Box<dyn std::error::Error + Send + Sync>;

/// Lỗi chung của Autoxide. Xem tài liệu đầu file.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    // `Cow<'static, str>`: thường là chuỗi viết sẵn trong code (không tốn bộ nhớ
    // cấp phát), nhưng vẫn nhận được chuỗi dựng lúc chạy khi cần.
    code: Cow<'static, str>,
    /// Câu tiếng Anh mặc định, có thể chứa chỗ điền `{tên}`.
    template: Cow<'static, str>,
    params: Vec<(Cow<'static, str>, String)>,
    fields: Vec<FieldError>,
    /// Phần ít khi dùng (nguyên nhân gốc, `Retry-After`, `reason`), để trong hộp: giữ `Error` đủ nhỏ để trả về
    /// trong `Result` mà không phải chép nhiều (clippy `result_large_err`).
    extra: Option<Box<Extra>>,
}

/// Phần ít khi dùng của [`Error`].
#[derive(Debug, Default)]
struct Extra {
    /// Nguyên nhân gốc, chỉ cho log.
    source: Option<Source>,
    /// Số giây client nên chờ trước khi thử lại (header `Retry-After`).
    retry_after: Option<NonZeroU32>,
    /// Lý do ổn định cho máy, chi tiết hơn `code` (vd `expired` / `used` của token): trường mở
    /// rộng `reason` của Problem (RFC 9457 cho phép trường mở rộng).
    reason: Option<&'static str>,
}

/// Mã và thông báo của lỗi nội bộ. Client chỉ thấy đúng hai thứ này.
const INTERNAL_CODE: &str = "internal_error";
const INTERNAL_MESSAGE: &str =
    "An internal error occurred. Retry later; if it persists, report the request id.";

impl Error {
    /// Tạo lỗi. `code` là mã ổn định cho máy và là i18n key: `snake_case`, lỗi của
    /// module mang tiền tố `<module>.` (vd `notes.title_taken`). `message` là câu
    /// tiếng Anh nói rõ phải sửa gì; chỗ điền viết `{tên}` và gán bằng
    /// [`with_param`](Self::with_param).
    pub fn new(
        kind: ErrorKind,
        code: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Error {
            kind,
            code: code.into(),
            template: message.into(),
            params: Vec::new(),
            fields: Vec::new(),
            extra: None,
        }
    }

    pub fn bad_request(
        code: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Error::new(ErrorKind::BadRequest, code, message)
    }

    pub fn unauthenticated(
        code: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Error::new(ErrorKind::Unauthenticated, code, message)
    }

    pub fn forbidden(
        code: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Error::new(ErrorKind::Forbidden, code, message)
    }

    pub fn not_found(
        code: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Error::new(ErrorKind::NotFound, code, message)
    }

    pub fn conflict(
        code: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Error::new(ErrorKind::Conflict, code, message)
    }

    pub fn too_many_requests(
        code: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Error::new(ErrorKind::TooManyRequests, code, message)
    }

    pub fn unavailable(
        code: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Error::new(ErrorKind::Unavailable, code, message)
    }

    /// Dữ liệu đầu vào không qua kiểm tra; `fields` nói rõ từng trường sai gì.
    pub fn validation(fields: Vec<FieldError>) -> Self {
        let mut error = Error::new(
            ErrorKind::Validation,
            "validation_failed",
            "The request contains invalid values. See `errors` for each field.",
        );
        error.fields = fields;
        error
    }

    /// Lỗi phía máy chủ. `source` là nguyên nhân gốc: ghi vào log, không gửi cho
    /// client — client chỉ nhận mã `internal_error` và một câu chung.
    pub fn internal(source: impl Into<Source>) -> Self {
        Error::new(ErrorKind::Internal, INTERNAL_CODE, INTERNAL_MESSAGE).with_source(source)
    }

    /// Gán giá trị cho chỗ điền `{name}` trong thông báo.
    pub fn with_param(mut self, name: impl Into<Cow<'static, str>>, value: impl ToString) -> Self {
        self.params.push((name.into(), value.to_string()));
        self
    }

    /// Thêm lỗi của một trường.
    pub fn with_field(mut self, field: FieldError) -> Self {
        self.fields.push(field);
        self
    }

    /// Gắn nguyên nhân gốc (cho log).
    pub fn with_source(mut self, source: impl Into<Source>) -> Self {
        self.extra.get_or_insert_default().source = Some(source.into());
        self
    }

    /// Báo client chờ `seconds` giây rồi mới thử lại: response mang header
    /// `Retry-After` (RFC 9110). Dùng cho 429 và 503.
    /// Ít nhất 1 giây.
    pub fn with_retry_after(mut self, seconds: u64) -> Self {
        let seconds = u32::try_from(seconds).unwrap_or(u32::MAX).max(1);
        self.extra.get_or_insert_default().retry_after = NonZeroU32::new(seconds);
        self
    }

    /// Gắn lý do ổn định cho máy (trường `reason` của Problem), vd `expired`.
    #[must_use]
    pub fn with_reason(mut self, reason: &'static str) -> Self {
        self.extra.get_or_insert_default().reason = Some(reason);
        self
    }

    /// Lý do gắn bằng [`with_reason`](Self::with_reason), nếu có.
    pub fn reason(&self) -> Option<&'static str> {
        self.extra.as_ref().and_then(|extra| extra.reason)
    }

    /// Số giây nên chờ trước khi thử lại, nếu lỗi có nêu.
    pub fn retry_after(&self) -> Option<u64> {
        self.extra
            .as_ref()
            .and_then(|extra| extra.retry_after)
            .map(|seconds| u64::from(seconds.get()))
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Mã ổn định cho máy; cũng là i18n key của thông báo.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Câu tiếng Anh mặc định, chưa điền giá trị.
    pub fn template(&self) -> &str {
        &self.template
    }

    /// Các giá trị điền vào thông báo, theo thứ tự đã gán.
    pub fn params(&self) -> impl Iterator<Item = (&str, &str)> {
        self.params.iter().map(|(name, value)| (name.as_ref(), value.as_str()))
    }

    /// Thông báo tiếng Anh đã điền giá trị.
    pub fn message(&self) -> String {
        fill(&self.template, &self.params)
    }

    /// Lỗi theo từng trường (với lỗi đầu vào).
    pub fn fields(&self) -> &[FieldError] {
        &self.fields
    }
}

/// Thay từng `{tên}` trong `template` bằng giá trị tương ứng.
fn fill(template: &str, params: &[(Cow<'static, str>, String)]) -> String {
    let mut text = template.to_owned();
    for (name, value) in params {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

// `Display` là thứ đi vào log: mã, thông báo, và nguyên nhân gốc nếu có.
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message())?;
        if let Some(source) = self.extra.as_ref().and_then(|extra| extra.source.as_ref()) {
            write!(f, " (caused by: {source})")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // Đổi `&Box<dyn Error + Send + Sync>` thành `&dyn Error`.
        let source = self.extra.as_ref().and_then(|extra| extra.source.as_deref());
        source.map(|source| source as &(dyn std::error::Error + 'static))
    }
}

// Nhờ `From`, service viết `?` trên kết quả của SeaORM: lỗi database tự thành lỗi
// nội bộ, nguyên nhân được giữ cho log và không lộ ra client.
impl From<sea_orm::DbErr> for Error {
    fn from(error: sea_orm::DbErr) -> Self {
        Error::internal(error)
    }
}

// Kết quả của `input.validate()` (thư viện validator) thành lỗi đầu vào có danh
// sách trường.
impl From<ValidationErrors> for Error {
    fn from(errors: ValidationErrors) -> Self {
        let mut fields = Vec::new();
        collect_fields("", &errors, &mut fields);
        // validator giữ lỗi trong HashMap (thứ tự ngẫu nhiên); sắp lại để cùng một
        // đầu vào luôn cho cùng một đầu ra.
        fields.sort_by(|a, b| (&a.field, &a.code).cmp(&(&b.field, &b.code)));
        Error::validation(fields)
    }
}

/// Trải cây lỗi lồng nhau của validator thành danh sách phẳng có đường dẫn trường.
fn collect_fields(prefix: &str, errors: &ValidationErrors, fields: &mut Vec<FieldError>) {
    for (name, kind) in errors.errors() {
        let path = if prefix.is_empty() { name.to_string() } else { format!("{prefix}.{name}") };
        match kind {
            ValidationErrorsKind::Field(problems) => {
                for problem in problems {
                    fields.push(field_message(path.clone(), problem));
                }
            }
            ValidationErrorsKind::Struct(nested) => collect_fields(&path, nested, fields),
            ValidationErrorsKind::List(items) => {
                for (index, nested) in items {
                    collect_fields(&format!("{path}[{index}]"), nested, fields);
                }
            }
        }
    }
}

/// Lỗi của trường `path` từ một lỗi của validator: câu mẫu tiếng Anh dựng từ mã và tham số của
/// nó (dịch được như mọi câu mẫu).
fn field_message(path: String, problem: &validator::ValidationError) -> FieldError {
    let code = problem.code.to_string();
    if let Some(message) = &problem.message {
        // Câu riêng khai ở `#[validate(… message = "…")]`: chưa có bản dịch (không module nào dùng).
        return FieldError::new(path, code, "{message}").with_param("message", message);
    }
    // Tham số là JSON; chuỗi thì bỏ dấu nháy bao quanh.
    let param = |name: &str| {
        problem.params.get(name).map(|value| match value {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        })
    };
    let (min, max, equal) = (param("min"), param("max"), param("equal"));
    match problem.code.as_ref() {
        "required" => FieldError::new(path.clone(), code.clone(), "is required"),
        "email" => FieldError::new(path.clone(), code.clone(), "must be a valid email address"),
        "url" => FieldError::new(path.clone(), code.clone(), "must be a valid URL"),
        "length" => match (min, max, equal) {
            (_, _, Some(equal)) => FieldError::new(
                path.clone(),
                code.clone(),
                "must be exactly {equal} characters long",
            )
            .with_param("equal", equal),
            (Some(min), Some(max), _) => FieldError::new(
                path.clone(),
                code.clone(),
                "must be between {min} and {max} characters long",
            )
            .with_param("min", min)
            .with_param("max", max),
            (Some(min), None, _) => FieldError::new(
                path.clone(),
                code.clone(),
                "must be at least {min} characters long",
            )
            .with_param("min", min),
            (None, Some(max), _) => {
                FieldError::new(path.clone(), code.clone(), "must be at most {max} characters long")
                    .with_param("max", max)
            }
            (None, None, None) => {
                FieldError::new(path.clone(), code.clone(), "has an invalid length")
            }
        },
        "range" => match (min, max) {
            (Some(min), Some(max)) => {
                FieldError::new(path.clone(), code.clone(), "must be between {min} and {max}")
                    .with_param("min", min)
                    .with_param("max", max)
            }
            (Some(min), None) => {
                FieldError::new(path.clone(), code.clone(), "must be at least {min}")
                    .with_param("min", min)
            }
            (None, Some(max)) => {
                FieldError::new(path.clone(), code.clone(), "must be at most {max}")
                    .with_param("max", max)
            }
            (None, None) => FieldError::new(path.clone(), code.clone(), "is out of range"),
        },
        "must_match" => match param("other") {
            Some(other) => FieldError::new(path.clone(), code.clone(), "must match `{other}`")
                .with_param("other", other),
            None => FieldError::new(path.clone(), code.clone(), "does not match"),
        },
        _ => FieldError::new(path.clone(), code.clone(), "is not valid"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;
    use validator::Validate;

    #[test]
    fn moi_loai_loi_co_mot_ma_trang_thai() {
        let cases = [
            (Error::bad_request("c", "m"), 400),
            (Error::unauthenticated("c", "m"), 401),
            (Error::forbidden("c", "m"), 403),
            (Error::not_found("c", "m"), 404),
            (Error::conflict("c", "m"), 409),
            (Error::new(ErrorKind::PayloadTooLarge, "c", "m"), 413),
            (Error::new(ErrorKind::UnsupportedMediaType, "c", "m"), 415),
            (Error::validation(Vec::new()), 422),
            (Error::too_many_requests("c", "m"), 429),
            (Error::internal("boom"), 500),
            (Error::unavailable("c", "m"), 503),
        ];
        for (error, status) in cases {
            assert_eq!(error.kind().status(), status, "{error}");
        }
    }

    #[test]
    fn tham_so_duoc_dien_vao_thong_bao() {
        let error =
            Error::conflict("notes.title_taken", "A note titled `{title}` exists ({count}).")
                .with_param("title", "Hello")
                .with_param("count", 2);

        assert_eq!(error.code(), "notes.title_taken");
        assert_eq!(error.template(), "A note titled `{title}` exists ({count}).");
        assert_eq!(error.message(), "A note titled `Hello` exists (2).");
        assert_eq!(error.params().collect::<Vec<_>>(), [("title", "Hello"), ("count", "2")]);
        assert_eq!(error.to_string(), "notes.title_taken: A note titled `Hello` exists (2).");
    }

    #[test]
    fn loi_noi_bo_giu_nguyen_nhan_cho_log_va_khong_dua_vao_thong_bao() {
        let error = Error::internal("connection reset by peer");

        assert_eq!(error.code(), "internal_error");
        // Thông báo cho client không chứa nguyên nhân…
        assert!(!error.message().contains("connection reset"));
        // …còn log (Display) và `source()` thì có.
        assert!(error.to_string().ends_with("(caused by: connection reset by peer)"));
        assert_eq!(error.source().unwrap().to_string(), "connection reset by peer");
    }

    #[test]
    fn loi_database_thanh_loi_noi_bo() {
        let error: Error = sea_orm::DbErr::Custom("syntax error at or near".to_owned()).into();

        assert_eq!(error.kind(), ErrorKind::Internal);
        assert_eq!(error.code(), "internal_error");
        assert!(!error.message().contains("syntax error"));
        assert!(error.to_string().contains("syntax error at or near"));
    }

    #[derive(Validate)]
    struct Address {
        #[validate(length(min = 1))]
        city: String,
    }

    #[derive(Validate)]
    struct Item {
        #[validate(range(min = 1, max = 99))]
        quantity: u32,
    }

    #[derive(Validate)]
    struct Order {
        #[validate(length(min = 3, max = 20))]
        reference: String,
        #[validate(email)]
        email: String,
        #[validate(url)]
        website: String,
        #[validate(length(equal = 2))]
        country: String,
        #[validate(range(min = 0))]
        discount: i64,
        #[validate(nested)]
        address: Address,
        #[validate(nested)]
        items: Vec<Item>,
    }

    fn invalid_order() -> Order {
        Order {
            reference: "ab".to_owned(),
            email: "not-an-email".to_owned(),
            website: "nope".to_owned(),
            country: "VNM".to_owned(),
            discount: -5,
            address: Address { city: String::new() },
            items: vec![Item { quantity: 5 }, Item { quantity: 0 }],
        }
    }

    #[test]
    fn loi_cua_validator_thanh_danh_sach_truong_co_duong_dan() {
        let error: Error = invalid_order().validate().unwrap_err().into();

        assert_eq!(error.kind(), ErrorKind::Validation);
        assert_eq!(error.code(), "validation_failed");
        let fields: Vec<(&str, &str, &str)> = error
            .fields()
            .iter()
            .map(|f| (f.field.as_str(), f.code.as_str(), f.message.as_str()))
            .collect();
        assert_eq!(
            fields,
            [
                ("address.city", "length", "must be at least 1 characters long"),
                ("country", "length", "must be exactly 2 characters long"),
                ("discount", "range", "must be at least 0"),
                ("email", "email", "must be a valid email address"),
                ("items[1].quantity", "range", "must be between 1 and 99"),
                ("reference", "length", "must be between 3 and 20 characters long"),
                ("website", "url", "must be a valid URL"),
            ]
        );
    }

    #[test]
    fn thong_bao_rieng_va_ma_la_cua_validator_deu_duoc_giu() {
        let mut custom = validator::ValidationError::new("taken");
        custom.message = Some("is already used by another account".into());
        assert_eq!(
            field_message(String::new(), &custom).message,
            "is already used by another account"
        );
        assert_eq!(
            field_message(String::new(), &validator::ValidationError::new("exotic")).message,
            "is not valid"
        );
        assert_eq!(
            field_message(String::new(), &validator::ValidationError::new("required")).message,
            "is required"
        );
    }

    #[test]
    fn them_loi_truong_bang_tay() {
        let error = Error::validation(Vec::new()).with_field(FieldError::new(
            "email",
            "taken",
            "is already used",
        ));
        assert_eq!(error.fields(), [FieldError::new("email", "taken", "is already used")]);
    }
}
