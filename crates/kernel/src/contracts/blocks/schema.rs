//! Kiểm props của khối theo JSON Schema (bước 67, D-20261122-block-registry).
//!
//! Chỉ nhận một phần nhỏ của JSON Schema (draft 2020-12) — đủ để mô tả props của khối, đủ để
//! giao diện dựng bảng thuộc tính và để mô hình AI hiểu: `type`, `properties`, `required`,
//! `additionalProperties`, `items`, `enum`, `const`, `minLength` / `maxLength`, `minimum` /
//! `maximum`, `minItems` / `maxItems`, `format` (`uuid`, `date-time`), `title`, `description`,
//! `default`, `examples`, cùng ba từ khoá riêng của Autoxide:
//!
//! - `x-responsive: true` — giá trị là một giá trị đơn, hoặc `{base, md?, lg?}` theo cỡ màn hình;
//! - `x-media: true` — chuỗi là ID ảnh / video của module `media` (UUID), không phải URL;
//! - `x-link: true` — link theo luật link của thân bài (D-102): `http(s)`, `mailto:`, `tel:`, hoặc
//!   đường dẫn trong site (`/…`, không `//`); không bao giờ `javascript:`.
//!
//! Từ khoá ngoài danh sách là lỗi khai báo (báo lúc khởi động): kiểm "từ chối mặc định" chỉ đúng
//! khi mọi điều schema nói đều được kiểm thật. Object nào cũng phải đóng
//! (`additionalProperties: false`). Mọi chuỗi trong props là chữ thuần: chuỗi có dạng thẻ HTML
//! (`<b`, `</`, `<!`) bị từ chối — giao diện hiện chữ, không chèn HTML.

use serde_json::{Map, Value};

use crate::error::FieldError;

const KEYWORDS: [&str; 22] = [
    "$schema",
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "enum",
    "const",
    "minLength",
    "maxLength",
    "minimum",
    "maximum",
    "minItems",
    "maxItems",
    "format",
    "title",
    "description",
    "default",
    "examples",
    "x-responsive",
    "x-media",
    "x-link",
];
const TYPES: [&str; 7] = ["object", "array", "string", "integer", "number", "boolean", "null"];
const FORMATS: [&str; 2] = ["uuid", "date-time"];
/// Cỡ màn hình của giá trị `x-responsive`: `base` (mọi cỡ, bắt buộc), `md`, `lg`.
pub const BREAKPOINTS: [&str; 3] = ["base", "md", "lg"];

/// Lỗi khai báo của schema `schema` (đã đọc từ JSON) của khối `block`. Rỗng: dùng được.
pub(crate) fn declaration_problems(block: &str, schema: &Value) -> Vec<String> {
    let mut found = Vec::new();
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        found.push(format!("props schema of block `{block}` must be an object schema"));
    }
    declared(block, schema, "$", &mut found);
    found
}

fn declared(block: &str, schema: &Value, at: &str, found: &mut Vec<String>) {
    let Some(object) = schema.as_object() else {
        found.push(format!("props schema of block `{block}`: {at} must be an object"));
        return;
    };
    for key in object.keys() {
        if !KEYWORDS.contains(&key.as_str()) {
            found.push(format!(
                "props schema of block `{block}`: {at} uses `{key}`, which Autoxide does not check"
            ));
        }
    }
    let types = type_names(schema);
    if types.is_empty() && !object.contains_key("enum") && !object.contains_key("const") {
        found.push(format!("props schema of block `{block}`: {at} needs a `type`"));
    }
    for name in &types {
        if !TYPES.contains(name) {
            found.push(format!("props schema of block `{block}`: {at} has unknown type `{name}`"));
        }
    }
    if let Some(format) = object.get("format")
        && !format.as_str().is_some_and(|format| FORMATS.contains(&format))
    {
        found.push(format!(
            "props schema of block `{block}`: {at} has a format other than `uuid` / `date-time`"
        ));
    }
    if types.contains(&"object") {
        if object.get("additionalProperties") != Some(&Value::Bool(false)) {
            found.push(format!(
                "props schema of block `{block}`: object {at} must set `additionalProperties: false`"
            ));
        }
        let properties = object.get("properties").and_then(Value::as_object);
        match properties {
            Some(properties) => {
                for (name, inner) in properties {
                    declared(block, inner, &format!("{at}.{name}"), found);
                }
            }
            None => found.push(format!(
                "props schema of block `{block}`: object {at} must list its `properties`"
            )),
        }
        if let Some(required) = object.get("required") {
            let known = required.as_array().is_some_and(|required| {
                required.iter().all(|name| {
                    name.as_str().is_some_and(|name| {
                        properties.is_some_and(|properties| properties.contains_key(name))
                    })
                })
            });
            if !known {
                found.push(format!(
                    "props schema of block `{block}`: {at}.required must name its properties"
                ));
            }
        }
    }
    if types.contains(&"array") {
        match object.get("items") {
            Some(items) => declared(block, items, &format!("{at}[]"), found),
            None => {
                found.push(format!("props schema of block `{block}`: array {at} needs `items`"))
            }
        }
    }
    for key in ["minLength", "maxLength", "minItems", "maxItems"] {
        if object.get(key).is_some_and(|value| value.as_u64().is_none()) {
            found.push(format!(
                "props schema of block `{block}`: {at}.{key} must be a non-negative integer"
            ));
        }
    }
    for key in ["minimum", "maximum"] {
        if object.get(key).is_some_and(|value| !value.is_number()) {
            found.push(format!("props schema of block `{block}`: {at}.{key} must be a number"));
        }
    }
    for key in ["x-responsive", "x-media", "x-link"] {
        if object.get(key).is_some_and(|value| !value.is_boolean()) {
            found
                .push(format!("props schema of block `{block}`: {at}.{key} must be true or false"));
        }
    }
    if flag(object, "x-media") && object.get("format").and_then(Value::as_str) != Some("uuid") {
        found.push(format!(
            "props schema of block `{block}`: {at} is `x-media`, so it must be a string of format `uuid`"
        ));
    }
    if flag(object, "x-link") && !types.contains(&"string") {
        found.push(format!(
            "props schema of block `{block}`: {at} is `x-link`, so it must be a string"
        ));
    }
}

fn type_names(schema: &Value) -> Vec<&str> {
    schema.as_object().map(object_types).unwrap_or_default()
}

fn object_types(schema: &Map<String, Value>) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(one)) => vec![one.as_str()],
        Some(Value::Array(many)) => many.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn flag(object: &Map<String, Value>, key: &str) -> bool {
    object.get(key) == Some(&Value::Bool(true))
}

/// Kiểm `value` theo `schema` (đã qua [`declaration_problems`]); mọi chỗ sai vào `found`, mỗi chỗ
/// một [`FieldError`] có đường dẫn bắt đầu bằng `at` (vd `sections[0].props.title`).
pub(crate) fn check(schema: &Value, value: &Value, at: &str, found: &mut Vec<FieldError>) {
    let Some(object) = schema.as_object() else {
        return;
    };
    if flag(object, "x-responsive")
        && let Some(sizes) = value.as_object()
        && sizes.contains_key("base")
        && !type_names(schema).contains(&"object")
    {
        for (size, inner) in sizes {
            if BREAKPOINTS.contains(&size.as_str()) {
                plain(object, inner, &format!("{at}.{size}"), found);
            } else {
                found.push(FieldError::new(
                    format!("{at}.{size}"),
                    "unknown",
                    "is not a screen size; use `base`, `md` or `lg`",
                ));
            }
        }
        return;
    }
    plain(object, value, at, found);
}

fn plain(schema: &Map<String, Value>, value: &Value, at: &str, found: &mut Vec<FieldError>) {
    let types = object_types(schema);
    if !types.is_empty() && !types.iter().any(|expected| is_type(value, expected)) {
        found.push(
            FieldError::new(at, "type", "must be of type {expected}")
                .with_param("expected", types.join(" or ")),
        );
        return;
    }
    if let Some(Value::Array(options)) = schema.get("enum")
        && !options.contains(value)
    {
        let allowed: Vec<String> = options.iter().map(Value::to_string).collect();
        found.push(
            FieldError::new(at, "enum", "must be one of {allowed}")
                .with_param("allowed", allowed.join(", ")),
        );
        return;
    }
    if let Some(expected) = schema.get("const")
        && expected != value
    {
        found
            .push(FieldError::new(at, "enum", "must be {allowed}").with_param("allowed", expected));
        return;
    }
    match value {
        Value::String(text) => string(schema, text, at, found),
        Value::Number(number) => {
            let number = number.as_f64().unwrap_or_default();
            let below =
                schema.get("minimum").and_then(Value::as_f64).is_some_and(|min| number < min);
            let above =
                schema.get("maximum").and_then(Value::as_f64).is_some_and(|max| number > max);
            if below || above {
                found.push(range(schema, at));
            }
        }
        Value::Array(items) => {
            let count = items.len() as u64;
            let few = schema.get("minItems").and_then(Value::as_u64).is_some_and(|min| count < min);
            let many =
                schema.get("maxItems").and_then(Value::as_u64).is_some_and(|max| count > max);
            if few || many {
                found.push(
                    FieldError::new(at, "length", "must have {min} to {max} items")
                        .with_param(
                            "min",
                            schema.get("minItems").and_then(Value::as_u64).unwrap_or(0),
                        )
                        .with_param("max", bound(schema.get("maxItems"))),
                );
            }
            if let Some(inner) = schema.get("items") {
                for (index, item) in items.iter().enumerate() {
                    check(inner, item, &format!("{at}[{index}]"), found);
                }
            }
        }
        Value::Object(fields) => {
            if let Some(Value::Array(required)) = schema.get("required") {
                for name in required.iter().filter_map(Value::as_str) {
                    if !fields.contains_key(name) {
                        found.push(FieldError::new(
                            format!("{at}.{name}"),
                            "required",
                            "is required",
                        ));
                    }
                }
            }
            let properties = schema.get("properties").and_then(Value::as_object);
            for (name, item) in fields {
                match properties.and_then(|properties| properties.get(name)) {
                    Some(inner) => check(inner, item, &format!("{at}.{name}"), found),
                    None => found.push(FieldError::new(
                        format!("{at}.{name}"),
                        "unknown",
                        "is not a property of this block",
                    )),
                }
            }
        }
        Value::Bool(_) | Value::Null => {}
    }
}

fn string(schema: &Map<String, Value>, text: &str, at: &str, found: &mut Vec<FieldError>) {
    let count = text.chars().count() as u64;
    let short = schema.get("minLength").and_then(Value::as_u64).is_some_and(|min| count < min);
    let long = schema.get("maxLength").and_then(Value::as_u64).is_some_and(|max| count > max);
    if short || long {
        found.push(
            FieldError::new(at, "length", "must be {min} to {max} characters long")
                .with_param("min", schema.get("minLength").and_then(Value::as_u64).unwrap_or(0))
                .with_param("max", bound(schema.get("maxLength"))),
        );
    }
    if looks_like_html(text) {
        found.push(FieldError::new(at, "html", "must be plain text, without HTML tags"));
    }
    match schema.get("format").and_then(Value::as_str) {
        Some("uuid") if uuid::Uuid::parse_str(text).is_err() => {
            found.push(FieldError::new(at, "format", "must be an id (UUID)"));
        }
        Some("date-time") if text.parse::<jiff::Timestamp>().is_err() => {
            found.push(FieldError::new(
                at,
                "format",
                "must be a time in RFC 3339 form, e.g. 2026-12-31T17:00:00Z",
            ));
        }
        _ => {}
    }
    if flag(schema, "x-link") && !safe_link(text) {
        found.push(FieldError::new(
            at,
            "link",
            "must be an http(s), mailto: or tel: link, or a path on the site starting with /",
        ));
    }
}

fn range(schema: &Map<String, Value>, at: &str) -> FieldError {
    FieldError::new(at, "range", "must be between {min} and {max}")
        .with_param("min", schema.get("minimum").map_or_else(|| "-∞".to_owned(), Value::to_string))
        .with_param("max", schema.get("maximum").map_or_else(|| "∞".to_owned(), Value::to_string))
}

fn bound(value: Option<&Value>) -> String {
    value.map_or_else(|| "∞".to_owned(), Value::to_string)
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

/// Chuỗi có dạng thẻ HTML: `<` ngay trước chữ cái, `/`, `!` hay `?`.
fn looks_like_html(text: &str) -> bool {
    text.as_bytes().windows(2).any(|pair| {
        pair[0] == b'<' && (pair[1].is_ascii_alphabetic() || matches!(pair[1], b'/' | b'!' | b'?'))
    })
}

/// Link an toàn (luật link của thân bài, D-102): `http`, `https`, `mailto`, `tel`, hoặc đường dẫn
/// trong site (`/…`, không `//`).
pub(crate) fn safe_link(href: &str) -> bool {
    let lower = href.trim().to_ascii_lowercase();
    // `\` thành `/` trong trình duyệt: `/\evil.com` là `//evil.com` — link ra ngoài trá hình
    // đường dẫn trong site.
    if href.chars().any(|c| c.is_control() || c == '\\') || href.len() > 2048 {
        return false;
    }
    lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
        || lower.starts_with("tel:")
        || (lower.starts_with('/') && !lower.starts_with("//"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["title"],
            "properties": {
                "title": {"type": "string", "minLength": 1, "maxLength": 10},
                "columns": {"type": "integer", "minimum": 1, "maximum": 4, "x-responsive": true},
                "image": {"type": "string", "format": "uuid", "x-media": true},
                "link": {"type": "string", "x-link": true},
                "align": {"enum": ["left", "center"]},
                "items": {"type": "array", "maxItems": 2, "items": {"type": "string"}}
            }
        })
    }

    fn codes(value: Value) -> Vec<(String, String)> {
        let mut found = Vec::new();
        check(&schema(), &value, "props", &mut found);
        found.into_iter().map(|error| (error.field, error.code)).collect()
    }

    #[test]
    fn schema_hop_le_khong_co_loi_khai_bao() {
        assert!(declaration_problems("x.y", &schema()).is_empty());
    }

    #[test]
    fn schema_sai_bi_bao_luc_khai() {
        let open =
            json!({"type": "object", "properties": {"a": {"type": "string", "pattern": "x"}}});
        let problems = declaration_problems("x.y", &open);
        assert!(
            problems.iter().any(|problem| problem.contains("additionalProperties")),
            "{problems:?}"
        );
        assert!(problems.iter().any(|problem| problem.contains("`pattern`")), "{problems:?}");
        let media = json!({"type": "object", "additionalProperties": false,
            "properties": {"a": {"type": "string", "x-media": true}}});
        assert_eq!(declaration_problems("x.y", &media).len(), 1);
        assert_eq!(declaration_problems("x.y", &json!({"type": "string"})).len(), 1);
    }

    #[test]
    fn props_dung_thi_qua() {
        let ok = json!({
            "title": "Sale",
            "columns": {"base": 1, "lg": 4},
            "image": "0190a1b2-0000-7000-8000-000000000001",
            "link": "/san-pham",
            "align": "left",
            "items": ["a"]
        });
        assert!(codes(ok).is_empty());
        assert!(codes(json!({"title": "x", "columns": 2})).is_empty());
    }

    #[test]
    fn props_sai_bao_moi_cho_mot_lan() {
        let found = codes(json!({
            "columns": {"base": 9, "xl": 2},
            "image": "https://example.com/a.png",
            "link": "javascript:alert(1)",
            "align": "right",
            "items": ["a", "b", "c"],
            "style": "color: red"
        }));
        let expected = [
            ("props.title", "required"),
            ("props.align", "enum"),
            ("props.columns.base", "range"),
            ("props.columns.xl", "unknown"),
            ("props.image", "format"),
            ("props.items", "length"),
            ("props.link", "link"),
            ("props.style", "unknown"),
        ];
        for (field, code) in expected {
            assert!(
                found.iter().any(|(f, c)| f == field && c == code),
                "{field} {code} missing in {found:?}"
            );
        }
    }

    #[test]
    fn chu_co_the_html_bi_tu_choi() {
        assert_eq!(
            codes(json!({"title": "<b>x</b>"})),
            [("props.title".to_owned(), "html".to_owned())]
        );
        assert!(codes(json!({"title": "a < b"})).is_empty());
    }

    #[test]
    fn link_an_toan() {
        for good in ["https://a.vn", "/a", "mailto:a@b.vn", "tel:+84901234567"] {
            assert!(safe_link(good), "{good}");
        }
        for bad in ["javascript:alert(1)", "//evil.com", "/\\evil.com", "data:text/html,x"] {
            assert!(!safe_link(bad), "{bad}");
        }
    }
}
