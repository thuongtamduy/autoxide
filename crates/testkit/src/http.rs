//! HTTP vocabulary of the test kit: [`StatusCode`] and [`Method`]. A module's tests use these instead of the
//! types of the `http` crate, so they do not depend on a library the kernel wraps (D-20261128-library-layer-plan,
//! item 4): a new major version of `http` or `axum` changes the test kit, not the tests.
//!
//! ```
//! use autoxide_testkit::http::{Method, StatusCode};
//!
//! assert_eq!(StatusCode::CREATED.as_u16(), 201);
//! assert!(StatusCode::NO_CONTENT.is_success());
//! assert_eq!(Method::PATCH.as_str(), "PATCH");
//! ```

// Ghi chú cho chủ dự án: hai kiểu này là kiểu của testkit, không phải của thư viện `http`. Trong lúc các module
// chuyển, testkit vẫn cho so sánh / đổi qua lại với kiểu của `http` (impl `PartialEq`, `From` ở cuối file) để test
// cũ còn viết `axum::http::StatusCode::OK` vẫn biên dịch; phần đó là đường cũ, gỡ khi không module nào còn dùng.

use std::borrow::Cow;
use std::fmt;

/// An HTTP status code (RFC 9110), as a response of [`TestApp`](crate::TestApp) reports it.
///
/// Compare with the constants (`StatusCode::OK`) or a number (`status == 201`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StatusCode(u16);

macro_rules! status_codes {
    ($($name:ident = $code:literal, $reason:literal;)*) => {
        impl StatusCode {
            $(
                #[doc = concat!("`", stringify!($code), " ", $reason, "`.")]
                pub const $name: StatusCode = StatusCode($code);
            )*

            /// The reason phrase of a registered code (`"Not Found"`), if the test kit knows it.
            pub fn canonical_reason(self) -> Option<&'static str> {
                match self.0 {
                    $($code => Some($reason),)*
                    _ => None,
                }
            }
        }
    };
}

status_codes! {
    CONTINUE = 100, "Continue";
    SWITCHING_PROTOCOLS = 101, "Switching Protocols";
    PROCESSING = 102, "Processing";
    EARLY_HINTS = 103, "Early Hints";
    OK = 200, "OK";
    CREATED = 201, "Created";
    ACCEPTED = 202, "Accepted";
    NON_AUTHORITATIVE_INFORMATION = 203, "Non Authoritative Information";
    NO_CONTENT = 204, "No Content";
    RESET_CONTENT = 205, "Reset Content";
    PARTIAL_CONTENT = 206, "Partial Content";
    MULTI_STATUS = 207, "Multi-Status";
    ALREADY_REPORTED = 208, "Already Reported";
    IM_USED = 226, "IM Used";
    MULTIPLE_CHOICES = 300, "Multiple Choices";
    MOVED_PERMANENTLY = 301, "Moved Permanently";
    FOUND = 302, "Found";
    SEE_OTHER = 303, "See Other";
    NOT_MODIFIED = 304, "Not Modified";
    USE_PROXY = 305, "Use Proxy";
    TEMPORARY_REDIRECT = 307, "Temporary Redirect";
    PERMANENT_REDIRECT = 308, "Permanent Redirect";
    BAD_REQUEST = 400, "Bad Request";
    UNAUTHORIZED = 401, "Unauthorized";
    PAYMENT_REQUIRED = 402, "Payment Required";
    FORBIDDEN = 403, "Forbidden";
    NOT_FOUND = 404, "Not Found";
    METHOD_NOT_ALLOWED = 405, "Method Not Allowed";
    NOT_ACCEPTABLE = 406, "Not Acceptable";
    REQUEST_TIMEOUT = 408, "Request Timeout";
    CONFLICT = 409, "Conflict";
    GONE = 410, "Gone";
    LENGTH_REQUIRED = 411, "Length Required";
    PRECONDITION_FAILED = 412, "Precondition Failed";
    PAYLOAD_TOO_LARGE = 413, "Payload Too Large";
    URI_TOO_LONG = 414, "URI Too Long";
    UNSUPPORTED_MEDIA_TYPE = 415, "Unsupported Media Type";
    RANGE_NOT_SATISFIABLE = 416, "Range Not Satisfiable";
    EXPECTATION_FAILED = 417, "Expectation Failed";
    IM_A_TEAPOT = 418, "I'm a teapot";
    MISDIRECTED_REQUEST = 421, "Misdirected Request";
    UNPROCESSABLE_ENTITY = 422, "Unprocessable Entity";
    LOCKED = 423, "Locked";
    FAILED_DEPENDENCY = 424, "Failed Dependency";
    TOO_EARLY = 425, "Too Early";
    UPGRADE_REQUIRED = 426, "Upgrade Required";
    PRECONDITION_REQUIRED = 428, "Precondition Required";
    TOO_MANY_REQUESTS = 429, "Too Many Requests";
    REQUEST_HEADER_FIELDS_TOO_LARGE = 431, "Request Header Fields Too Large";
    UNAVAILABLE_FOR_LEGAL_REASONS = 451, "Unavailable For Legal Reasons";
    INTERNAL_SERVER_ERROR = 500, "Internal Server Error";
    NOT_IMPLEMENTED = 501, "Not Implemented";
    BAD_GATEWAY = 502, "Bad Gateway";
    SERVICE_UNAVAILABLE = 503, "Service Unavailable";
    GATEWAY_TIMEOUT = 504, "Gateway Timeout";
    HTTP_VERSION_NOT_SUPPORTED = 505, "HTTP Version Not Supported";
    VARIANT_ALSO_NEGOTIATES = 506, "Variant Also Negotiates";
    INSUFFICIENT_STORAGE = 507, "Insufficient Storage";
    LOOP_DETECTED = 508, "Loop Detected";
    NOT_EXTENDED = 510, "Not Extended";
    NETWORK_AUTHENTICATION_REQUIRED = 511, "Network Authentication Required";
}

impl StatusCode {
    /// The code for a number from 100 to 999; `None` outside that range.
    pub fn from_u16(code: u16) -> Option<StatusCode> {
        (100..1000).contains(&code).then_some(StatusCode(code))
    }

    /// The number (`404`).
    pub fn as_u16(self) -> u16 {
        self.0
    }

    /// `1xx`.
    pub fn is_informational(self) -> bool {
        (100..200).contains(&self.0)
    }

    /// `2xx`.
    pub fn is_success(self) -> bool {
        (200..300).contains(&self.0)
    }

    /// `3xx`.
    pub fn is_redirection(self) -> bool {
        (300..400).contains(&self.0)
    }

    /// `4xx`.
    pub fn is_client_error(self) -> bool {
        (400..500).contains(&self.0)
    }

    /// `5xx`.
    pub fn is_server_error(self) -> bool {
        (500..600).contains(&self.0)
    }
}

// Như `http::StatusCode`: Debug in số, Display in số kèm lý do — thông báo của `assert_eq!` đọc như cũ.
impl fmt::Debug for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.0, self.canonical_reason().unwrap_or("<unknown status code>"))
    }
}

impl PartialEq<u16> for StatusCode {
    fn eq(&self, other: &u16) -> bool {
        self.0 == *other
    }
}

impl PartialEq<StatusCode> for u16 {
    fn eq(&self, other: &StatusCode) -> bool {
        *self == other.0
    }
}

impl From<StatusCode> for u16 {
    fn from(status: StatusCode) -> u16 {
        status.0
    }
}

/// An HTTP request method, for [`TestApp::request`](crate::TestApp::request).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Method(Cow<'static, str>);

impl Method {
    /// `GET`.
    pub const GET: Method = Method(Cow::Borrowed("GET"));
    /// `HEAD`.
    pub const HEAD: Method = Method(Cow::Borrowed("HEAD"));
    /// `POST`.
    pub const POST: Method = Method(Cow::Borrowed("POST"));
    /// `PUT`.
    pub const PUT: Method = Method(Cow::Borrowed("PUT"));
    /// `PATCH`.
    pub const PATCH: Method = Method(Cow::Borrowed("PATCH"));
    /// `DELETE`.
    pub const DELETE: Method = Method(Cow::Borrowed("DELETE"));
    /// `OPTIONS`.
    pub const OPTIONS: Method = Method(Cow::Borrowed("OPTIONS"));

    /// A method by name (`"PATCH"`); `None` when the name is not an HTTP token (RFC 9110 §5.6.2) — empty, a
    /// space, a separator. Case is kept: methods are case-sensitive.
    pub fn from_name(name: &str) -> Option<Method> {
        let token = !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte));
        token.then(|| Method(Cow::Owned(name.to_owned())))
    }

    /// The name (`"GET"`).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// --- Đường cũ: đổi qua lại với kiểu của `http` cho test chưa chuyển (D-20261128-library-layer-plan mục 4). ---
// Ngoài hợp đồng; gỡ khi không module nào còn khai `axum` / `http` ở `[dev-dependencies]`.

impl From<axum::http::StatusCode> for StatusCode {
    fn from(status: axum::http::StatusCode) -> StatusCode {
        StatusCode(status.as_u16())
    }
}

impl PartialEq<axum::http::StatusCode> for StatusCode {
    fn eq(&self, other: &axum::http::StatusCode) -> bool {
        self.0 == other.as_u16()
    }
}

impl PartialEq<StatusCode> for axum::http::StatusCode {
    fn eq(&self, other: &StatusCode) -> bool {
        self.as_u16() == other.0
    }
}

impl From<axum::http::Method> for Method {
    fn from(method: axum::http::Method) -> Method {
        match method.as_str() {
            "GET" => Method::GET,
            "HEAD" => Method::HEAD,
            "POST" => Method::POST,
            "PUT" => Method::PUT,
            "PATCH" => Method::PATCH,
            "DELETE" => Method::DELETE,
            "OPTIONS" => Method::OPTIONS,
            other => Method(Cow::Owned(other.to_owned())),
        }
    }
}

impl Method {
    // Kiểu của `http` cho bộ dựng request bên trong testkit.
    pub(crate) fn to_http(&self) -> Result<axum::http::Method, String> {
        axum::http::Method::from_bytes(self.0.as_bytes()).map_err(|error| error.to_string())
    }
}

/// Sends requests straight to a router built by the kernel (`autoxide_kernel::http::build(…).router`), without
/// [`TestApp`](crate::TestApp): no contract check, no test user — for benchmarks that measure the kernel alone.
/// Tests use `TestApp`.
#[derive(Clone)]
pub struct Client {
    router: axum::Router,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client").finish_non_exhaustive()
    }
}

impl Client {
    /// A client of `router` (the `router` field of what `autoxide_kernel::http::build` returns).
    pub fn new(router: axum::Router) -> Client {
        Client { router }
    }

    /// Sends one request and reads the whole answer: its status and body.
    pub async fn send(
        &self,
        method: &Method,
        uri: &str,
        headers: &[(&str, &str)],
        body: Option<Vec<u8>>,
    ) -> Result<(StatusCode, Vec<u8>), String> {
        use tower::ServiceExt as _;
        let mut builder = axum::http::Request::builder().method(method.to_http()?).uri(uri);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let request = builder
            .body(body.map_or_else(axum::body::Body::empty, axum::body::Body::from))
            .map_err(|error| error.to_string())?;
        // Lỗi của `Router` là `Infallible` (không bao giờ xảy ra): `let Ok(…) = …` không cần nhánh lỗi.
        let Ok(response) = self.router.clone().oneshot(request).await;
        let status = StatusCode::from(response.status());
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .map_err(|error| error.to_string())?;
        Ok((status, bytes.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn so_sanh_voi_so_va_kieu_cu() {
        assert_eq!(StatusCode::OK, 200);
        assert_eq!(StatusCode::NOT_FOUND, axum::http::StatusCode::NOT_FOUND);
        assert_eq!(axum::http::StatusCode::CREATED, StatusCode::CREATED);
        assert_eq!(StatusCode::from(axum::http::StatusCode::IM_A_TEAPOT).as_u16(), 418);
        assert_eq!(StatusCode::UNPROCESSABLE_ENTITY.to_string(), "422 Unprocessable Entity");
        assert_eq!(
            StatusCode::IM_A_TEAPOT.to_string(),
            axum::http::StatusCode::IM_A_TEAPOT.to_string()
        );
        assert_eq!(format!("{:?}", StatusCode::CONFLICT), "409");
        assert_eq!(StatusCode::from_u16(99), None);
        assert!(StatusCode::TOO_MANY_REQUESTS.is_client_error());
    }

    #[test]
    fn method_theo_ten() {
        assert_eq!(Method::from_name("PATCH"), Some(Method::PATCH));
        assert_eq!(Method::from_name("PROPFIND").map(|m| m.to_string()), Some("PROPFIND".into()));
        assert_eq!(Method::from_name("BAD METHOD"), None);
        assert_eq!(Method::from_name(""), None);
        assert_eq!(Method::from(axum::http::Method::DELETE), Method::DELETE);
        assert_eq!(Method::PUT.to_http().unwrap(), axum::http::Method::PUT);
    }
}
