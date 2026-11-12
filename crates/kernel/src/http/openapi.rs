//! Tài liệu OpenAPI: hoàn thiện phần chung, phục vụ qua HTTP, và kiểm chất lượng.
//!
//! Tài liệu sinh từ chính khai báo route (`routes!`), nên đường dẫn và method
//! không thể lệch với code. File này thêm những thứ giống nhau ở mọi endpoint để
//! người viết module không phải lặp lại:
//!
//! - tag = tên module, để tài liệu gom theo module;
//! - `operationId` mang tiền tố module, để không trùng giữa các module (công cụ
//!   sinh SDK dùng nó làm tên hàm);
//! - response `default` = Problem Details: mọi endpoint đều có thể trả lỗi dạng đó.

use std::collections::BTreeSet;

use utoipa::ToSchema;
use utoipa::openapi::path::{Operation, PathItem};
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::openapi::{
    ContentBuilder, InfoBuilder, OpenApi, OpenApiBuilder, Ref, RefOr, ResponseBuilder,
};

use crate::response::{PROBLEM_CONTENT_TYPE, Problem};

/// Đường dẫn phục vụ tài liệu OpenAPI dạng JSON.
pub const OPENAPI_PATH: &str = "/api/v1/openapi.json";
/// Đường dẫn trang xem tài liệu API (khi được bật).
pub const DOCS_PATH: &str = "/api/docs";

/// Tên cách xác thực "bearer token" trong tài liệu. Handler cần đăng nhập khai
/// `security((BEARER_AUTH = []))` — viết chữ `"bearer"` trong `#[utoipa::path]`.
pub const BEARER_AUTH: &str = "bearer";

const DESCRIPTION: &str = "\
REST API of Autoxide.

## Conventions

Every endpoint follows the same rules, so learning one endpoint teaches all of them.

- **Success**: a single result is `{\"data\": …}`; a list is \
`{\"data\": […], \"pagination\": {\"limit\", \"next_cursor\"}}`.
- **Errors**: always RFC 9457 Problem Details (`application/problem+json`). Branch on the \
stable `code` field, never on `detail`. Validation problems list every invalid field in `errors`.
- **Lists**: accept `limit` (1–200, default 50), `order` (`desc` or `asc`, by creation time) and \
`cursor`. To get the next page, pass `pagination.next_cursor` as `cursor`; `null` means last page.
- **Authentication**: endpoints that need it list the `bearer` security scheme. Send \
`Authorization: Bearer <access token>`. A `401` response carries `WWW-Authenticate: Bearer`; \
its `code` says whether the token is missing, invalid or expired.
- **Retries**: send `Idempotency-Key: <a random UUID>` with `POST`, `PUT`, `PATCH` and \
`DELETE`. Repeating the same request with the same key within 24 hours returns the first result \
(marked `Idempotent-Replayed: true`) instead of doing it twice; a different request with a used \
key gets `422`, and a repeat while the first is still running gets `409`.
- **Deprecation**: an endpoint being retired is marked `deprecated` here and answers with \
`Deprecation` (RFC 9745), `Sunset` (RFC 8594, the removal date) and `Link: <…>; \
rel=\"successor-version\"` headers. Move to the successor before the sunset date.
- **Language**: send `Accept-Language` (e.g. `vi`) to get the `detail` of errors in that \
language; English (`en`) by default and for unsupported languages. Responses carry \
`Content-Language`. Codes never change with the language.
- **Rate limits**: too many requests from one address are answered with `429` (code \
`rate_limited`) and a `Retry-After` header giving the seconds to wait.
- **Request id**: every response carries `x-request-id`. Send your own to correlate calls; quote \
it when reporting a problem.
- **IDs** are UUIDv7 strings; **timestamps** are UTC in RFC 3339; **money** is an integer amount \
in the smallest currency unit plus a currency code.";

/// Phần đầu của tài liệu: tên, phiên bản, và các quy ước chung của API.
pub(super) fn base_document() -> OpenApi {
    OpenApiBuilder::new()
        .info(
            InfoBuilder::new()
                .title("Autoxide API")
                .version(env!("CARGO_PKG_VERSION"))
                .description(Some(DESCRIPTION))
                .build(),
        )
        .build()
}

/// Gắn nhãn cho tài liệu của MỘT module trước khi ghép vào tài liệu chung.
pub(super) fn label_module(module: &str, openapi: &mut OpenApi) {
    for item in openapi.paths.paths.values_mut() {
        for (_, operation) in operations_mut(item) {
            // Module tự đặt tag thì giữ; không thì tag là tên module.
            if operation.tags.as_ref().is_none_or(Vec::is_empty) {
                operation.tags = Some(vec![module.to_owned()]);
            }
            if let Some(id) = &operation.operation_id {
                operation.operation_id = Some(format!("{module}_{id}"));
            }
        }
    }
}

/// Hoàn thiện tài liệu chung sau khi đã ghép mọi module.
pub(super) fn finish(openapi: &mut OpenApi) {
    // Schema `Problem` (và các schema nó dùng) phải có mặt để `$ref` bên dưới trỏ tới.
    let mut schemas =
        vec![(Problem::name().into_owned(), <Problem as utoipa::PartialSchema>::schema())];
    Problem::schemas(&mut schemas);
    let components = openapi.components.get_or_insert_default();
    for (name, schema) in schemas {
        components.schemas.entry(name).or_insert(schema);
    }
    components.security_schemes.entry(BEARER_AUTH.to_owned()).or_insert_with(|| {
        RefOr::T(SecurityScheme::Http(
            HttpBuilder::new()
                .scheme(HttpAuthScheme::Bearer)
                .bearer_format("JWT")
                .description(Some(
                    "A short-lived access token, sent as `Authorization: Bearer <token>`.",
                ))
                .build(),
        ))
    });

    let problem = ResponseBuilder::new()
        .description(
            "Any error, as RFC 9457 Problem Details. The `code` field identifies the problem.",
        )
        .content(
            PROBLEM_CONTENT_TYPE,
            ContentBuilder::new()
                .schema(Some(RefOr::Ref(Ref::from_schema_name(Problem::name()))))
                .build(),
        )
        .build();
    for item in openapi.paths.paths.values_mut() {
        for (_, operation) in operations_mut(item) {
            operation
                .responses
                .responses
                .entry("default".to_owned())
                .or_insert_with(|| RefOr::T(problem.clone()));
        }
    }
}

/// Kiểm tài liệu có đủ để gọi đúng API mà không cần đọc mã nguồn. Trả về danh sách
/// vấn đề; rỗng nghĩa là đạt. Gate chạy hàm này trên tài liệu của cả chương trình.
pub fn lint(openapi: &OpenApi) -> Vec<String> {
    let mut problems = Vec::new();
    let mut operation_ids = BTreeSet::new();
    for (path, item) in &openapi.paths.paths {
        for (method, operation) in operations(item) {
            let endpoint = format!("{} {path}", method.to_uppercase());
            if operation.summary.as_deref().is_none_or(|summary| summary.trim().is_empty()) {
                problems.push(format!(
                    "{endpoint}: no summary — add a `///` doc comment to the handler"
                ));
            }
            if operation.tags.as_ref().is_none_or(Vec::is_empty) {
                problems.push(format!("{endpoint}: no tag"));
            }
            match &operation.operation_id {
                None => problems.push(format!("{endpoint}: no operationId")),
                Some(id) => {
                    if !operation_ids.insert(id.clone()) {
                        problems.push(format!("{endpoint}: operationId `{id}` is used twice"));
                    }
                }
            }
            let responses = &operation.responses.responses;
            if !responses.keys().any(|status| status.starts_with('2') || status.starts_with('3')) {
                problems.push(format!(
                    "{endpoint}: no success response — declare what the endpoint returns"
                ));
            }
            for (status, response) in responses {
                if let RefOr::T(response) = response
                    && response.description.trim().is_empty()
                {
                    problems.push(format!("{endpoint}: response {status} has no description"));
                }
            }
        }
    }
    match serde_json::to_value(openapi) {
        Ok(document) => {
            problems.extend(dangling_refs(&document));
            problems.extend(untranslated_text(&document, "OpenAPI"));
        }
        Err(error) => problems.push(format!("the document cannot be serialized: {error}")),
    }
    problems
}

/// Mọi `$ref` phải trỏ tới một mục có thật trong chính tài liệu: thiếu một mục là tài
/// liệu không hợp lệ — trình kiểm chuẩn và công cụ sinh SDK từ chối cả file.
fn dangling_refs(document: &serde_json::Value) -> Vec<String> {
    let mut refs = BTreeSet::new();
    collect_refs(document, &mut refs);
    refs.into_iter()
        .filter(|reference| {
            let target = reference
                .strip_prefix("#/")
                .map(|path| path.split('/').try_fold(document, |node, key| node.get(key)));
            !matches!(target, Some(Some(_)))
        })
        .map(|reference| {
            format!(
                "`$ref` `{reference}` points to nothing in the document — the type is used but \
                 not registered (derive `ToSchema` and list it, or mark the field `inline`)"
            )
        })
        .collect()
}

fn collect_refs(node: &serde_json::Value, refs: &mut BTreeSet<String>) {
    match node {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                match value {
                    serde_json::Value::String(reference) if key == "$ref" => {
                        refs.insert(reference.clone());
                    }
                    _ => collect_refs(value, refs),
                }
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|item| collect_refs(item, refs)),
        _ => {}
    }
}

/// Mô tả gửi ra ngoài (OpenAPI, tool MCP) phải là tiếng Anh: `///` trên kiểu có
/// `ToSchema` / `IntoParams` được chép thẳng vào đó. Báo mọi `summary` / `description`
/// có chữ tiếng Việt; `what` là tên tài liệu trong thông báo.
pub(crate) fn untranslated_text(document: &serde_json::Value, what: &str) -> Vec<String> {
    let mut problems = Vec::new();
    collect_untranslated(document, what, &mut problems);
    problems
}

fn collect_untranslated(node: &serde_json::Value, what: &str, problems: &mut Vec<String>) {
    match node {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                match value {
                    serde_json::Value::String(text)
                        if matches!(key.as_str(), "summary" | "description" | "title")
                            && looks_vietnamese(text) =>
                    {
                        problems.push(format!(
                            "{what}: `{text}` is not English — `///` comments on handlers and on \
                             types with `ToSchema`/`IntoParams` are published; write them in \
                             English and keep notes for readers of the code in `//` comments"
                        ));
                    }
                    _ => collect_untranslated(value, what, problems),
                }
            }
        }
        serde_json::Value::Array(items) => {
            items.iter().for_each(|item| collect_untranslated(item, what, problems));
        }
        _ => {}
    }
}

/// Có chữ cái riêng của tiếng Việt (ă â đ ê ô ơ ư, hoặc nguyên âm mang dấu thanh).
pub(crate) fn looks_vietnamese(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c, 'ă' | 'Ă' | 'đ' | 'Đ' | 'ơ' | 'Ơ' | 'ư' | 'Ư')
            || ('\u{1EA0}'..='\u{1EF9}').contains(&c)
    })
}

/// Các operation của một đường dẫn, kèm tên method.
fn operations(item: &PathItem) -> impl Iterator<Item = (&'static str, &Operation)> {
    [
        ("get", &item.get),
        ("put", &item.put),
        ("post", &item.post),
        ("delete", &item.delete),
        ("options", &item.options),
        ("head", &item.head),
        ("patch", &item.patch),
        ("trace", &item.trace),
    ]
    .into_iter()
    .filter_map(|(method, operation)| operation.as_ref().map(|operation| (method, operation)))
}

fn operations_mut(item: &mut PathItem) -> impl Iterator<Item = (&'static str, &mut Operation)> {
    [
        ("get", &mut item.get),
        ("put", &mut item.put),
        ("post", &mut item.post),
        ("delete", &mut item.delete),
        ("options", &mut item.options),
        ("head", &mut item.head),
        ("patch", &mut item.patch),
        ("trace", &mut item.trace),
    ]
    .into_iter()
    .filter_map(|(method, operation)| operation.as_mut().map(|operation| (method, operation)))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn bat_ref_tro_vao_khoang_khong() {
        let document = json!({
            "paths": {"/x": {"get": {"parameters": [
                {"schema": {"$ref": "#/components/schemas/Order"}},
                {"schema": {"$ref": "#/components/schemas/Page"}},
            ]}}},
            "components": {"schemas": {"Page": {"type": "object"}}},
        });
        let problems = dangling_refs(&document);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("#/components/schemas/Order"), "{problems:?}");
    }

    #[test]
    fn bat_mo_ta_tieng_viet() {
        let document = json!({"description": "Đầu vào của tool", "properties": {
            "a": {"description": "Maximum number of items, café"},
            "b": {"title": "Tên người dùng"},
            "description": {"type": "string"},
        }});
        let problems = untranslated_text(&document, "tool `x`");
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(!looks_vietnamese("Resume naïve café über"));
    }
}
