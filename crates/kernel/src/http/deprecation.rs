//! Endpoint deprecated: module khai, kernel đánh dấu trong OpenAPI và gắn header chuẩn
//! vào mỗi response — `Deprecation` (RFC 9745), `Sunset` (RFC 8594), `Link` tới endpoint
//! thay thế (`rel="successor-version"`). Client (và AI agent) biết sớm để chuyển.
//!
//! Tool MCP deprecated khai cùng chỗ, bằng [`Deprecation::tool`] (D-240): lớp HTTP bỏ qua chúng;
//! `Modules::tools` gắn chúng vào tool, `tools/list` và kết quả của tool báo cho agent.

use crate::time::Date;
use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{MatchedPath, Request};
use axum::http::{HeaderName, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::Response;
use utoipa::openapi::OpenApi;
use utoipa::openapi::path::Operation;

use super::API_PREFIX;

/// Một endpoint sắp bỏ, do module khai (`Module::deprecations`).
///
/// ```
/// use autoxide_kernel::http::Deprecation;
/// use axum::http::Method;
///
/// let deprecation = Deprecation::new(Method::GET, "/{id}", "2026-11-01")
///     .sunset("2027-05-01")
///     .successor("/api/v2/notes/{id}");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Deprecation {
    pub method: Method,
    /// Đường dẫn trong module, như khi khai route (`/`, `/{id}`).
    pub path: &'static str,
    /// Ngày bắt đầu deprecated, `YYYY-MM-DD` (UTC).
    pub since: &'static str,
    /// Ngày endpoint bị bỏ hẳn, `YYYY-MM-DD` (UTC), nếu đã định.
    pub sunset: Option<&'static str>,
    /// Endpoint dùng thay, nếu có (đường dẫn hoặc URL). Với tool: tên ngắn của tool dùng thay.
    pub successor: Option<&'static str>,
    /// Tên ngắn của tool MCP bị deprecated (khai bằng [`Deprecation::tool`]); khi có thì `method`
    /// và `path` không dùng.
    pub tool: Option<&'static str>,
}

impl Deprecation {
    pub fn new(method: Method, path: &'static str, since: &'static str) -> Self {
        Deprecation { method, path, since, sunset: None, successor: None, tool: None }
    }

    /// Tool MCP `name` (tên ngắn, như trong `Module::tools`) deprecated từ ngày `since`. Thay đổi
    /// phá vỡ của tool thì làm tool mới (vd `create_draft_v2`), tool cũ chạy như cũ tới ngày
    /// sunset:
    ///
    /// ```
    /// use autoxide_kernel::http::Deprecation;
    ///
    /// let deprecation = Deprecation::tool("create_draft", "2026-12-01")
    ///     .sunset("2027-06-01")
    ///     .successor("create_draft_v2");
    /// ```
    pub fn tool(name: &'static str, since: &'static str) -> Self {
        Deprecation {
            method: Method::POST,
            path: "",
            since,
            sunset: None,
            successor: None,
            tool: Some(name),
        }
    }

    /// Khai báo này cho một tool MCP (không phải endpoint).
    pub const fn is_tool(&self) -> bool {
        self.tool.is_some()
    }

    pub fn sunset(mut self, date: &'static str) -> Self {
        self.sunset = Some(date);
        self
    }

    pub fn successor(mut self, path: &'static str) -> Self {
        self.successor = Some(path);
        self
    }

    /// Các vấn đề của khai báo này (ngày sai dạng, sunset trước since…).
    pub fn problems(&self, module: &str) -> Vec<String> {
        let mut problems = Vec::new();
        let endpoint = match self.tool {
            Some(tool) => format!("tool `{tool}` of module `{module}`"),
            None => format!("{} {} of module `{module}`", self.method, self.path),
        };
        let since = parse_date(self.since);
        if since.is_none() {
            problems.push(format!("deprecation of {endpoint}: `since` must be a YYYY-MM-DD date"));
        }
        if let Some(sunset) = self.sunset {
            match (since, parse_date(sunset)) {
                (_, None) => problems
                    .push(format!("deprecation of {endpoint}: `sunset` must be a YYYY-MM-DD date")),
                (Some(since), Some(sunset)) if sunset <= since => problems
                    .push(format!("deprecation of {endpoint}: `sunset` must come after `since`")),
                _ => {}
            }
        }
        if self.tool.is_none() && !self.path.starts_with('/') {
            problems.push(format!("deprecation of {endpoint}: the path must start with `/`"));
        }
        problems
    }
}

fn parse_date(text: &str) -> Option<Date> {
    Date::parse_from_str(text, "%Y-%m-%d").ok()
}

/// Header của một endpoint deprecated, dựng sẵn một lần.
struct Headers {
    deprecation: HeaderValue,
    sunset: Option<HeaderValue>,
    link: Option<HeaderValue>,
}

/// Bảng tra (method, mẫu đường dẫn đầy đủ) → header.
pub(super) struct Table(HashMap<(Method, String), Headers>);

/// Đánh dấu `deprecated` trong tài liệu và dựng bảng tra header. Khai báo không khớp
/// endpoint nào thì ghi log lỗi (test của module nên bắt được trước).
pub(super) fn apply(openapi: &mut OpenApi, deprecations: &[(&'static str, Deprecation)]) -> Table {
    let mut table = HashMap::new();
    for (module, deprecation) in
        deprecations.iter().filter(|(_, deprecation)| !deprecation.is_tool())
    {
        let suffix = if deprecation.path == "/" { "" } else { deprecation.path };
        let path = format!("{API_PREFIX}/{module}{suffix}");
        let (Some(since), sunset) =
            (parse_date(deprecation.since), deprecation.sunset.and_then(parse_date))
        else {
            continue; // `Modules::new` đã từ chối khai báo sai.
        };
        let Some(operation) = operation_mut(openapi, &path, &deprecation.method) else {
            tracing::error!(
                module,
                method = %deprecation.method,
                path,
                "a deprecation names an endpoint that does not exist"
            );
            continue;
        };
        operation.deprecated = Some(utoipa::openapi::Deprecated::True);
        let mut note = format!("\n\n**Deprecated** since {since}.");
        if let Some(sunset) = sunset {
            note.push_str(&format!(" It will be removed on {sunset}."));
        }
        if let Some(successor) = deprecation.successor {
            note.push_str(&format!(" Use `{successor}` instead."));
        }
        operation.description =
            Some(format!("{}{note}", operation.description.clone().unwrap_or_default()));
        // Ngày gỡ đọc được bằng máy, cho `cargo xtask api-diff` (đặc tả 31 A1). Một khoá duy nhất:
        // `Extensions` của utoipa là `HashMap`, nhiều khoá thì thứ tự trong `openapi.json` đổi
        // giữa các lần sinh.
        if let Some(sunset) = sunset {
            operation
                .extensions
                .get_or_insert_with(Default::default)
                .insert("x-sunset".to_owned(), serde_json::Value::String(sunset.to_string()));
        }

        let midnight = |date: Date| date.start_of_day_utc();
        let headers = Headers {
            deprecation: header(&format!("@{}", midnight(since).as_unix_seconds())),
            sunset: sunset.map(|date| {
                header(&midnight(date).format("%a, %d %b %Y %H:%M:%S GMT").to_string())
            }),
            link: deprecation
                .successor
                .map(|successor| header(&format!("<{successor}>; rel=\"successor-version\""))),
        };
        table.insert((deprecation.method.clone(), path), headers);
    }
    Table(table)
}

fn header(text: &str) -> HeaderValue {
    HeaderValue::from_str(text).unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// Câu gắn vào mô tả của mọi operation và tool MCP thuộc module thử nghiệm.
pub(crate) const EXPERIMENTAL_NOTE: &str = "**Experimental**: this belongs to an experimental \
     module; it may change between releases, breaking changes included (see the changelog).";

/// Đánh dấu mọi operation của các module thử nghiệm `modules` (bước 70 P3): extension
/// `x-stability: experimental` cho máy, một câu trong mô tả cho người. Operation không có
/// `x-stability` thuộc module chính thức.
pub(super) fn mark_experimental(openapi: &mut OpenApi, modules: &[&'static str]) {
    for (path, item) in &mut openapi.paths.paths {
        let owner = path
            .strip_prefix(super::API_PREFIX)
            .and_then(|rest| rest.strip_prefix('/'))
            .and_then(|rest| rest.split('/').next());
        if !owner.is_some_and(|owner| modules.contains(&owner)) {
            continue;
        }
        let operations =
            [&mut item.get, &mut item.post, &mut item.put, &mut item.patch, &mut item.delete];
        for operation in operations.into_iter().flatten() {
            operation
                .extensions
                .get_or_insert_with(Default::default)
                .insert("x-stability".to_owned(), serde_json::Value::String("experimental".into()));
            let description = operation.description.take().unwrap_or_default();
            operation.description = Some(if description.is_empty() {
                EXPERIMENTAL_NOTE.to_owned()
            } else {
                format!("{description}\n\n{EXPERIMENTAL_NOTE}")
            });
        }
    }
}

fn operation_mut<'a>(
    openapi: &'a mut OpenApi,
    path: &str,
    method: &Method,
) -> Option<&'a mut Operation> {
    let item = openapi.paths.paths.get_mut(path)?;
    match *method {
        Method::GET => item.get.as_mut(),
        Method::POST => item.post.as_mut(),
        Method::PUT => item.put.as_mut(),
        Method::PATCH => item.patch.as_mut(),
        Method::DELETE => item.delete.as_mut(),
        _ => None,
    }
}

/// Lớp HTTP: gắn header vào response của endpoint deprecated, và đếm số lần gọi.
pub(super) async fn mark(table: Arc<Table>, request: Request, next: Next) -> Response {
    let key = request
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| (request.method().clone(), matched.as_str().to_owned()));
    let mut response = next.run(request).await;
    if let Some(key) = key
        && let Some(headers) = table.0.get(&key)
    {
        metrics::counter!("http_deprecated_requests_total", "route" => key.1.clone()).increment(1);
        let values = response.headers_mut();
        values.insert(HeaderName::from_static("deprecation"), headers.deprecation.clone());
        if let Some(sunset) = &headers.sunset {
            values.insert(HeaderName::from_static("sunset"), sunset.clone());
        }
        if let Some(link) = &headers.link {
            values.append(axum::http::header::LINK, link.clone());
        }
    }
    response
}
