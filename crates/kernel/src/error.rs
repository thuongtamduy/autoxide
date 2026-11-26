//! Common error type for every layer.
//!
//! Services, REST handlers and MCP tools all return this [`Error`]. It carries enough information
//! for two different readers:
//!
//! - **client** (a person or an AI agent): a stable `code` for machines to branch on, a message
//!   stating what to fix, and a per-field error list for input errors;
//! - **operator**: the root cause (`source`) - written to the log only, NEVER
//!   sent to the client.
//!
//! `code` is a contract with clients: changing a code is a breaking change. `code` is also
//! the i18n key of the message; the English sentence written where the error is created is the default
//! text, and translations are looked up by `code` (docs/DECISIONS.md D-039).
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

use crate::time::Timestamp;
use std::borrow::Cow;
use std::fmt;
use std::num::NonZeroU32;

use validator::{ValidationErrors, ValidationErrorsKind};

/// Kind of error; determines the HTTP status code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The request cannot be read (broken JSON, missing parameter, wrong type). HTTP 400.
    BadRequest,
    /// Not authenticated, or the credentials are invalid. HTTP 401.
    Unauthenticated,
    /// Authenticated but lacking the permission. HTTP 403.
    Forbidden,
    /// The resource does not exist. HTTP 404.
    NotFound,
    /// Conflicts with the current state (duplicate unique value, editing a stale version). HTTP 409.
    Conflict,
    /// The request's precondition no longer holds (stale `If-Match`: the record was modified). HTTP 412.
    PreconditionFailed,
    /// The request body is larger than the limit. HTTP 413.
    PayloadTooLarge,
    /// The request's content type is not supported. HTTP 415.
    UnsupportedMediaType,
    /// The requested byte range (`Range`) lies outside the content. HTTP 416.
    RangeNotSatisfiable,
    /// The request is readable but a value fails validation. HTTP 422.
    Validation,
    /// The requested thing is gone and will not come back (sync cursor too old). HTTP 410.
    Gone,
    /// The endpoint requires a precondition (`If-Match`) and the request did not send one. HTTP 428.
    PreconditionRequired,
    /// Too many calls. HTTP 429.
    TooManyRequests,
    /// Temporarily hidden for legal reasons (e.g. a violation case under review). HTTP 451 (RFC 7725).
    UnavailableForLegalReasons,
    /// Server-side error; details appear only in the log. HTTP 500.
    Internal,
    /// Temporarily unable to serve (database not ready, shutting down). HTTP 503.
    Unavailable,
}

impl ErrorKind {
    /// The corresponding HTTP status code.
    pub fn status(self) -> u16 {
        match self {
            ErrorKind::BadRequest => 400,
            ErrorKind::Unauthenticated => 401,
            ErrorKind::Forbidden => 403,
            ErrorKind::NotFound => 404,
            ErrorKind::Conflict => 409,
            ErrorKind::Gone => 410,
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

/// An error on one field of the input data.
///
/// The message is written as an English **template**, with `{name}` placeholders filled in by
/// [`with_param`](Self::with_param) - not pre-assembled with `format!`: translations are looked up by the
/// template itself (key `field:<template>` in `locales/vi.json`, like gettext using the source sentence
/// as the key), then the values are filled in (D-262). `cargo xtask check` rejects untranslated and
/// pre-assembled sentences.
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
    /// Path to the field: `name`, `address.city`, `items[2].quantity`.
    pub field: String,
    /// Stable code of the error kind: `required`, `length`, `range`, `email`...
    pub code: String,
    /// The English sentence with values filled in: states what the value must look like.
    pub message: String,
    /// The template (translation key) and the values filled into it.
    template: Cow<'static, str>,
    params: Vec<(Cow<'static, str>, String)>,
}

impl FieldError {
    /// An error for field `field`, code `code`, template `message` (may contain `{name}`).
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

    /// Fills `{name}` of the template with `value`.
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

    /// The English template, not yet filled in (translation key).
    pub fn template(&self) -> &str {
        &self.template
    }

    /// The values filled into the template, in the order they were assigned.
    pub fn params(&self) -> impl Iterator<Item = (&str, &str)> {
        self.params.iter().map(|(name, value)| (name.as_ref(), value.as_str()))
    }

    /// A `required` error if `value` is empty (or only whitespace). Used in services when
    /// collecting every error of an input to report them at once.
    pub fn blank(field: impl Into<String>, value: &str) -> Option<Self> {
        value.trim().is_empty().then(|| FieldError::new(field, "required", "must not be empty"))
    }
}

/// Root cause of an error, kept for the log.
type Source = Box<dyn std::error::Error + Send + Sync>;

/// Common Autoxide error. See the module documentation at the top of the file.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    // `Cow<'static, str>`: thường là chuỗi viết sẵn trong code (không tốn bộ nhớ
    // cấp phát), nhưng vẫn nhận được chuỗi dựng lúc chạy khi cần.
    code: Cow<'static, str>,
    /// The default English sentence, which may contain `{name}` placeholders.
    template: Cow<'static, str>,
    params: Vec<(Cow<'static, str>, String)>,
    fields: Vec<FieldError>,
    /// The rarely used parts (root cause, `Retry-After`, `reason`), boxed: keeps `Error` small enough to return
    /// in a `Result` without copying much (clippy `result_large_err`).
    extra: Option<Box<Extra>>,
}

/// The rarely used parts of [`Error`].
#[derive(Debug, Default)]
struct Extra {
    /// Root cause, for the log only.
    source: Option<Source>,
    /// Number of seconds the client should wait before retrying (`Retry-After` header).
    retry_after: Option<NonZeroU32>,
    /// Stable machine-readable reason, more detailed than `code` (e.g. `expired` / `used` for a token): the
    /// Problem's `reason` extension field (RFC 9457 allows extension fields).
    reason: Option<&'static str>,
    /// The current record on the server, with its version (the 412 error of a conditional write, step 69 P2):
    /// the Problem's `current` extension field and the `ETag` header.
    current: Option<(serde_json::Value, Timestamp)>,
    /// Value of the 401 `WWW-Authenticate` header in place of a bare `Bearer` (e.g. the re-authentication
    /// challenge of RFC 9470).
    challenge: Option<String>,
}

/// Code and message of an internal error. The client sees exactly these two things.
const INTERNAL_CODE: &str = "internal_error";
const INTERNAL_MESSAGE: &str =
    "An internal error occurred. Retry later; if it persists, report the request id.";

impl Error {
    /// A copy of this error for answering again (e.g. the remembered failure of authentication within one
    /// request); the root cause, kept only for the log, is not copied.
    pub(crate) fn replay(&self) -> Error {
        Error {
            kind: self.kind,
            code: self.code.clone(),
            template: self.template.clone(),
            params: self.params.clone(),
            fields: self.fields.clone(),
            extra: self.extra.as_ref().map(|extra| {
                Box::new(Extra {
                    source: None,
                    retry_after: extra.retry_after,
                    reason: extra.reason,
                    current: extra.current.clone(),
                    challenge: extra.challenge.clone(),
                })
            }),
        }
    }
}

impl Error {
    /// Creates an error. `code` is the stable machine code and the i18n key: `snake_case`, a module's errors
    /// carry the prefix `<module>.` (e.g. `notes.title_taken`). `message` is the English
    /// sentence stating what to fix; write placeholders as `{name}` and assign them with
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

    /// Input data fails validation; `fields` states what is wrong with each field.
    pub fn validation(fields: Vec<FieldError>) -> Self {
        let mut error = Error::new(
            ErrorKind::Validation,
            "validation_failed",
            "The request contains invalid values. See `errors` for each field.",
        );
        error.fields = fields;
        error
    }

    /// Server-side error. `source` is the root cause: written to the log, not sent to
    /// the client - the client receives only the code `internal_error` and a generic sentence.
    pub fn internal(source: impl Into<Source>) -> Self {
        Error::new(ErrorKind::Internal, INTERNAL_CODE, INTERNAL_MESSAGE).with_source(source)
    }

    /// Assigns a value to the `{name}` placeholder in the message.
    pub fn with_param(mut self, name: impl Into<Cow<'static, str>>, value: impl ToString) -> Self {
        self.params.push((name.into(), value.to_string()));
        self
    }

    /// Adds an error for one field.
    pub fn with_field(mut self, field: FieldError) -> Self {
        self.fields.push(field);
        self
    }

    /// Attaches the root cause (for the log).
    pub fn with_source(mut self, source: impl Into<Source>) -> Self {
        self.extra.get_or_insert_default().source = Some(source.into());
        self
    }

    /// Tells the client to wait `seconds` seconds before retrying: the response carries the `Retry-After`
    /// header (RFC 9110). Used for 429 and 503. At least 1 second.
    pub fn with_retry_after(mut self, seconds: u64) -> Self {
        let seconds = u32::try_from(seconds).unwrap_or(u32::MAX).max(1);
        self.extra.get_or_insert_default().retry_after = NonZeroU32::new(seconds);
        self
    }

    /// Attaches a stable machine-readable reason (the Problem's `reason` field), e.g. `expired`.
    #[must_use]
    pub fn with_reason(mut self, reason: &'static str) -> Self {
        self.extra.get_or_insert_default().reason = Some(reason);
        self
    }

    /// Attaches the current record `record` (in the form it is read) and its version `version`: the client
    /// sees the server's copy right away to merge by itself, without re-reading.
    #[must_use]
    pub fn with_current(mut self, record: serde_json::Value, version: Timestamp) -> Self {
        self.extra.get_or_insert_default().current = Some((record, version));
        self
    }

    /// The 401 error's own `WWW-Authenticate` header (e.g. `Bearer error="insufficient_user_authentication",
    /// max_age=900` of RFC 9470).
    #[must_use]
    pub fn with_challenge(mut self, challenge: impl Into<String>) -> Self {
        self.extra.get_or_insert_default().challenge = Some(challenge.into());
        self
    }

    /// The `WWW-Authenticate` header attached with [`with_challenge`](Self::with_challenge), if any.
    pub fn challenge(&self) -> Option<&str> {
        self.extra.as_ref().and_then(|extra| extra.challenge.as_deref())
    }

    /// The current record attached with [`with_current`](Self::with_current), if any.
    pub fn current(&self) -> Option<&(serde_json::Value, Timestamp)> {
        self.extra.as_ref().and_then(|extra| extra.current.as_ref())
    }

    /// The reason attached with [`with_reason`](Self::with_reason), if any.
    pub fn reason(&self) -> Option<&'static str> {
        self.extra.as_ref().and_then(|extra| extra.reason)
    }

    /// Number of seconds to wait before retrying, if the error specifies it.
    pub fn retry_after(&self) -> Option<u64> {
        self.extra
            .as_ref()
            .and_then(|extra| extra.retry_after)
            .map(|seconds| u64::from(seconds.get()))
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Stable machine code; also the i18n key of the message.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// The default English sentence, not yet filled in.
    pub fn template(&self) -> &str {
        &self.template
    }

    /// The values filled into the message, in the order they were assigned.
    pub fn params(&self) -> impl Iterator<Item = (&str, &str)> {
        self.params.iter().map(|(name, value)| (name.as_ref(), value.as_str()))
    }

    /// The English message with values filled in.
    pub fn message(&self) -> String {
        fill(&self.template, &self.params)
    }

    /// Per-field errors (for input errors).
    pub fn fields(&self) -> &[FieldError] {
        &self.fields
    }
}

/// Replaces each `{name}` in `template` with the corresponding value.
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

/// Flattens the validator's nested error tree into a flat list with field paths.
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

/// The error for field `path` from one validator error: an English template built from its code and
/// parameters (translatable like any template).
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
