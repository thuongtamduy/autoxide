//! Hình dạng thống nhất của `/api/v1`, kiểm trên `docs/openapi.json` (D-20261121-api-shape).
//!
//! Học một endpoint là biết các endpoint còn lại (CLAUDE.md, "Thân thiện với AI"). Năm luật:
//!
//! - `api-closed-body`: thân request JSON là object thì đóng (`additionalProperties: false`, tức
//!   kiểu Rust có `#[serde(deny_unknown_fields)]`): trường lạ hay gõ sai tên bị trả 422
//!   `invalid_body` thay vì bị lặng lẽ bỏ qua.
//! - `api-problem`: mọi response 4xx / 5xx là `application/problem+json` với schema `Problem`
//!   (RFC 9457).
//! - `api-unpaged-list`: `GET` trả mảng phải là `Page<T>` (`data` + `pagination`, lấy bằng
//!   `fetch_page`); `Data<Vec<T>>` chỉ cho tập nhỏ có giới hạn, ghi trong [`ALLOW_FILE`].
//! - `api-page-params`: endpoint trả `Page<T>` nhận `limit` và `cursor`.
//! - `api-param-description`: mọi tham số (đường dẫn, query, header) có `description` ở chính tham số — chỗ
//!   Swagger UI / Scalar hiển thị và agent đọc để gọi đúng (D-20261125-list-pages-kernel). Dòng cho phép của luật
//!   này ghi `<đường dẫn>?<tên tham số>`.
//!
//! SCIM 2.0 (`/api/v1/auth/scim/v2/…`) theo RFC 7643 / 7644 — thân lỗi riêng
//! (`application/scim+json`) và client gửi kèm thuộc tính chỉ đọc (`id`, `meta`, `schemas`) mà máy
//! chủ phải bỏ qua — nên đứng ngoài các luật này.
//!
//! Ngoại lệ có chủ đích ghi ở [`ALLOW_FILE`], mỗi dòng `<luật> <METHOD> <đường dẫn> # lý do`.
//! Dòng không còn khớp vi phạm nào chỉ được cảnh báo (nhắc xoá), không làm gate đỏ.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

use crate::boundaries::Violation;

const OPENAPI_FILE: &str = "docs/openapi.json";
pub const ALLOW_FILE: &str = "xtask/api-shape-allow.txt";
const SCIM_PREFIX: &str = "/api/v1/auth/scim/v2/";
const METHODS: &[&str] = &["get", "post", "put", "patch", "delete"];

/// Vi phạm còn lại và cảnh báo (chỉ in).
pub struct Report {
    pub violations: Vec<Violation>,
    pub warnings: Vec<String>,
}

pub fn check(root: &Path) -> Result<Report, String> {
    let text = fs::read_to_string(root.join(OPENAPI_FILE))
        .map_err(|error| format!("{OPENAPI_FILE}: {error}"))?;
    let document: Value =
        serde_json::from_str(&text).map_err(|error| format!("{OPENAPI_FILE}: {error}"))?;
    let allow_text = fs::read_to_string(root.join(ALLOW_FILE)).unwrap_or_default();
    let allowed = parse_allow(&allow_text)?;
    let found = find(&document);
    let mut used = BTreeSet::new();
    let mut violations = Vec::new();
    for (key, message) in found {
        if allowed.contains(&key) {
            used.insert(key);
            continue;
        }
        let (rule, method, path) = key;
        violations.push(Violation {
            rule,
            location: format!("{OPENAPI_FILE} {method} {path}"),
            message,
        });
    }
    let warnings = allowed
        .difference(&used)
        .map(|(rule, method, path)| {
            format!(
                "warning: {ALLOW_FILE}: `{rule} {method} {path}` no longer matches a violation; \
                 remove the line"
            )
        })
        .collect();
    Ok(Report { violations, warnings })
}

type Key = (&'static str, String, String);

const RULES: &[&str] = &[
    "api-closed-body",
    "api-problem",
    "api-unpaged-list",
    "api-page-params",
    "api-param-description",
];

fn parse_allow(text: &str) -> Result<BTreeSet<Key>, String> {
    let mut entries = BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        let content = line.split('#').next().unwrap_or("").trim();
        if content.is_empty() {
            continue;
        }
        let fields: Vec<&str> = content.split_whitespace().collect();
        let [rule, method, path] = fields.as_slice() else {
            return Err(format!(
                "{ALLOW_FILE}:{}: expected `<rule> <METHOD> <path> # reason`",
                index + 1
            ));
        };
        let Some(rule) = RULES.iter().find(|known| *known == rule) else {
            return Err(format!("{ALLOW_FILE}:{}: unknown rule `{rule}`", index + 1));
        };
        if !line.contains('#') {
            return Err(format!("{ALLOW_FILE}:{}: give the reason after `#`", index + 1));
        }
        entries.insert((*rule, (*method).to_owned(), (*path).to_owned()));
    }
    Ok(entries)
}

/// Mọi vi phạm của tài liệu, theo thứ tự đường dẫn.
fn find(document: &Value) -> Vec<(Key, String)> {
    let empty = Map::new();
    let schemas =
        document.pointer("/components/schemas").and_then(Value::as_object).unwrap_or(&empty);
    let Some(paths) = document.get("paths").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (path, operations) in paths {
        if !path.starts_with("/api/v1/") || path.starts_with(SCIM_PREFIX) {
            continue;
        }
        for method in METHODS {
            let Some(operation) = operations.get(*method) else {
                continue;
            };
            let upper = method.to_uppercase();
            let key = |rule: &'static str| (rule, upper.clone(), path.clone());
            if let Some(message) = open_body(operation, schemas) {
                found.push((key("api-closed-body"), message));
            }
            if let Some(message) = non_problem_error(operation) {
                found.push((key("api-problem"), message));
            }
            for name in undescribed_parameters(operation) {
                found.push((
                    ("api-param-description", upper.clone(), format!("{path}?{name}")),
                    format!(
                        "parameter `{name}` has no description; write a `///` line on its field (or \
                         `description` in `#[param(…)]`) saying what it does and its default"
                    ),
                ));
            }
            match list_shape(operation, schemas) {
                Some(ListShape::Page) => {
                    let names = parameter_names(operation);
                    let missing: Vec<&str> = ["limit", "cursor"]
                        .into_iter()
                        .filter(|name| !names.contains(name))
                        .collect();
                    if !missing.is_empty() {
                        found.push((
                            key("api-page-params"),
                            format!(
                                "returns `Page<T>` but has no `{}` query parameter; take \
                                 `PageRequest` and list with `fetch_page`",
                                missing.join("`, `")
                            ),
                        ));
                    }
                }
                Some(ListShape::Unpaged) if *method == "get" => found.push((
                    key("api-unpaged-list"),
                    "returns a bare list (`Data<Vec<T>>` or an array) instead of `Page<T>`; \
                     list with `fetch_page`, or add the endpoint to xtask/api-shape-allow.txt with \
                     the reason the list stays small"
                        .to_owned(),
                )),
                _ => {}
            }
        }
    }
    found
}

/// Câu báo lỗi khi thân request JSON là object mà không đóng.
fn open_body(operation: &Value, schemas: &Map<String, Value>) -> Option<String> {
    let content = operation.pointer("/requestBody/content")?.as_object()?;
    for (media_type, body) in content {
        if !media_type.contains("json") {
            continue;
        }
        let schema = body.get("schema")?;
        let name = schema
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|reference| reference.rsplit('/').next())
            .unwrap_or("inline schema");
        let resolved = resolve(schema, schemas);
        if is_object(resolved) && resolved.get("additionalProperties") != Some(&Value::Bool(false))
        {
            return Some(format!(
                "request body `{name}` accepts unknown fields; add `#[serde(deny_unknown_fields)]` \
                 to the Rust type so a misspelt field is answered with 422 instead of being \
                 ignored"
            ));
        }
    }
    None
}

/// Câu báo lỗi khi một response 4xx / 5xx không phải `Problem`.
fn non_problem_error(operation: &Value) -> Option<String> {
    let responses = operation.get("responses")?.as_object()?;
    for (status, response) in responses {
        if !(status.starts_with('4') || status.starts_with('5')) {
            continue;
        }
        let Some(content) = response.get("content").and_then(Value::as_object) else {
            continue;
        };
        for (media_type, body) in content {
            let reference = body.pointer("/schema/$ref").and_then(Value::as_str).unwrap_or("");
            if media_type != "application/problem+json" || !reference.ends_with("/Problem") {
                return Some(format!(
                    "response {status} is `{media_type}` `{reference}`; errors are \
                     `application/problem+json` with the `Problem` schema (return \
                     `autoxide_kernel::error::Error`)"
                ));
            }
        }
    }
    None
}

enum ListShape {
    Page,
    Unpaged,
}

/// Hình dạng của response 200 JSON khi nó là một danh sách.
fn list_shape(operation: &Value, schemas: &Map<String, Value>) -> Option<ListShape> {
    let content = operation.pointer("/responses/200/content")?.as_object()?;
    for (media_type, body) in content {
        if !media_type.contains("json") {
            continue;
        }
        let resolved = resolve(body.get("schema")?, schemas);
        if resolved.get("type").and_then(Value::as_str) == Some("array") {
            return Some(ListShape::Unpaged);
        }
        let properties = resolved.get("properties").and_then(Value::as_object)?;
        if properties.contains_key("pagination") {
            return Some(ListShape::Page);
        }
        let data = resolve(properties.get("data")?, schemas);
        if data.get("type").and_then(Value::as_str) == Some("array") {
            return Some(ListShape::Unpaged);
        }
    }
    None
}

/// Tên các tham số không có `description` (hay để trống) ở chính tham số.
fn undescribed_parameters(operation: &Value) -> Vec<&str> {
    operation
        .get("parameters")
        .and_then(Value::as_array)
        .map(|parameters| {
            parameters
                .iter()
                .filter(|parameter| {
                    parameter
                        .get("description")
                        .and_then(Value::as_str)
                        .is_none_or(|text| text.trim().is_empty())
                })
                .filter_map(|parameter| parameter.get("name")?.as_str())
                .collect()
        })
        .unwrap_or_default()
}

fn parameter_names(operation: &Value) -> Vec<&str> {
    operation
        .get("parameters")
        .and_then(Value::as_array)
        .map(|parameters| {
            parameters.iter().filter_map(|parameter| parameter.get("name")?.as_str()).collect()
        })
        .unwrap_or_default()
}

/// Theo `$ref` (và `allOf` chỉ một phần, cách utoipa gắn mô tả vào một tham chiếu) tới schema thật.
fn resolve<'a>(schema: &'a Value, schemas: &'a Map<String, Value>) -> &'a Value {
    let mut current = schema;
    for _ in 0..16 {
        if let Some(reference) = current.get("$ref").and_then(Value::as_str) {
            match reference.rsplit('/').next().and_then(|name| schemas.get(name)) {
                Some(next) => current = next,
                None => return current,
            }
            continue;
        }
        match current.get("allOf").and_then(Value::as_array).map(Vec::as_slice) {
            Some([only]) if current.get("additionalProperties").is_none() => current = only,
            _ => return current,
        }
    }
    current
}

fn is_object(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("object")
        || schema.get("properties").is_some()
        || schema.get("allOf").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document(operation: Value) -> Value {
        json!({
            "paths": {"/api/v1/x": {"post": operation.clone(), "get": operation}},
            "components": {"schemas": {
                "Closed": {"type": "object", "properties": {"a": {}}, "additionalProperties": false},
                "Open": {"type": "object", "properties": {"a": {}}},
                "Problem": {"type": "object"},
                "PageX": {"type": "object", "properties": {"data": {"type": "array"}, "pagination": {}}},
                "DataVec": {"type": "object", "properties": {"data": {"type": "array"}}}
            }}
        })
    }

    fn rules(document: &Value) -> Vec<(&'static str, String)> {
        find(document).into_iter().map(|((rule, method, _), _)| (rule, method)).collect()
    }

    #[test]
    fn than_dong_va_loi_problem_thi_khong_bao() {
        let doc = document(json!({
            "requestBody": {"content": {"application/json": {"schema": {"allOf": [{"$ref": "#/components/schemas/Closed"}]}}}},
            "responses": {"422": {"content": {"application/problem+json": {"schema": {"$ref": "#/components/schemas/Problem"}}}}}
        }));
        assert!(rules(&doc).is_empty());
    }

    #[test]
    fn than_mo_bi_bao_than_khong_phai_object_thi_khong() {
        let open = document(json!({
            "requestBody": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/Open"}}}}
        }));
        assert_eq!(
            rules(&open),
            vec![("api-closed-body", "GET".to_owned()), ("api-closed-body", "POST".to_owned())]
        );
        let bytes = document(json!({
            "requestBody": {"content": {"application/json": {"schema": {"type": "string", "format": "binary"}}}}
        }));
        assert!(rules(&bytes).is_empty());
    }

    #[test]
    fn loi_khong_phai_problem_bi_bao() {
        let doc = document(json!({
            "responses": {"400": {"content": {"application/json": {"schema": {"type": "string"}}}}}
        }));
        assert!(rules(&doc).iter().all(|(rule, _)| *rule == "api-problem"));
        assert_eq!(rules(&doc).len(), 2);
    }

    #[test]
    fn danh_sach_tran_va_trang_thieu_tham_so() {
        let unpaged = document(json!({
            "responses": {"200": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/DataVec"}}}}}
        }));
        // Chỉ GET mới là danh sách; POST trả mảng (vd kết quả hàng loạt) thì không.
        assert_eq!(rules(&unpaged), vec![("api-unpaged-list", "GET".to_owned())]);
        let page = document(json!({
            "parameters": [{"name": "limit", "description": "Page size."}],
            "responses": {"200": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/PageX"}}}}}
        }));
        let found = find(&page);
        assert_eq!(found.len(), 2);
        assert!(found[0].1.contains("`cursor`"));
    }

    #[test]
    fn tham_so_thieu_mo_ta_bi_bao_theo_ten() {
        let doc = document(json!({
            "parameters": [
                {"name": "q", "description": "Search text."},
                {"name": "status"},
                {"name": "kind", "description": "  "}
            ]
        }));
        let found: Vec<(&str, String, String)> =
            find(&doc).into_iter().map(|((rule, method, path), _)| (rule, method, path)).collect();
        let get =
            |name: &str| ("api-param-description", "GET".to_owned(), format!("/api/v1/x?{name}"));
        assert!(found.contains(&get("status")));
        assert!(found.contains(&get("kind")));
        assert!(!found.contains(&get("q")));
        assert_eq!(found.len(), 4, "GET and POST, two parameters each");
    }

    #[test]
    fn scim_dung_ngoai() {
        let mut doc = document(json!({
            "requestBody": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/Open"}}}}
        }));
        let operation = doc["paths"]["/api/v1/x"].clone();
        doc["paths"] = json!({ "/api/v1/auth/scim/v2/Users": operation });
        assert!(rules(&doc).is_empty());
    }

    #[test]
    fn file_ngoai_le_can_ly_do_va_luat_dung() {
        assert!(parse_allow("api-problem GET /api/v1/x # vì sao").is_ok());
        assert!(parse_allow("api-problem GET /api/v1/x").is_err());
        assert!(parse_allow("api-nope GET /api/v1/x # x").is_err());
        assert!(parse_allow("api-problem /api/v1/x # x").is_err());
    }
}
