//! Validates block props against a JSON Schema (step 67, D-20261122-block-registry).
//!
//! Accepts only a small subset of JSON Schema (draft 2020-12) — enough to describe a block's props, to let
//! the UI build a property panel and to let AI models understand: `type`, `properties`, `required`,
//! `additionalProperties`, `items`, `enum`, `const`, `minLength` / `maxLength`, `minimum` /
//! `maximum`, `minItems` / `maxItems`, `format` (`uuid`, `date-time`), `title`, `description`,
//! `default`, `examples`, plus three Autoxide-specific keywords:
//!
//! - `x-responsive: true` — the value is a single value, or `{base, md?, lg?}` per screen size;
//! - `x-media: true` — the string is an image / video ID of the `media` module (a UUID), not a URL;
//! - `x-link: true` — a link following the post-body link rule (D-102): `http(s)`, `mailto:`, `tel:`, or
//!   a path inside the site (`/…`, not `//`); never `javascript:`.
//!
//! A keyword outside the list is a declaration error (reported at startup): "reject by default" validation is
//! only correct when everything the schema says is really checked. Every object must be closed
//! (`additionalProperties: false`). Every string in props is plain text: a string shaped like an HTML tag
//! (`<b`, `</`, `<!`) is rejected — the UI renders text and never injects HTML.

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
/// Screen size of an `x-responsive` value: `base` (all sizes, required), `md`, `lg`.
pub const BREAKPOINTS: [&str; 3] = ["base", "md", "lg"];

/// Declaration errors of schema `schema` (read from JSON) of block `block`. Empty: usable.
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

/// Validates `value` against `schema` (already passed through [`declaration_problems`]); every mismatch goes
/// into `found`, each one a [`FieldError`] whose path starts with `at` (e.g. `sections[0].props.title`).
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

/// A string shaped like an HTML tag: `<` right before a letter, `/`, `!` or `?`.
fn looks_like_html(text: &str) -> bool {
    text.as_bytes().windows(2).any(|pair| {
        pair[0] == b'<' && (pair[1].is_ascii_alphabetic() || matches!(pair[1], b'/' | b'!' | b'?'))
    })
}

/// A safe link (post-body link rule, D-102): `http`, `https`, `mailto`, `tel`, or a path
/// inside the site (`/…`, not `//`).
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

// Thử phá luật link an toàn (D-20261123-property-fuzzing): so với bộ phân tích URL chuẩn WHATWG (cách trình
// duyệt hiểu link) — link được nhận thì trình duyệt mở đúng giao thức cho phép, và link "trong site" thì
// không bao giờ ra host khác.
#[cfg(test)]
mod properties {
    use super::*;
    use crate::net::Url;
    use proptest::prelude::*;

    fn href() -> impl Strategy<Value = String> {
        let prefix = prop_oneof![
            Just(""),
            Just("/"),
            Just("//"),
            Just("/\\"),
            Just("\\/"),
            Just("/%2f"),
            Just("http:"),
            Just("https://"),
            Just("HTTPS://"),
            Just("javascript:"),
            Just("data:"),
            Just("mailto:"),
            Just("tel:"),
            Just(" "),
            Just("\u{3000}"),
            Just("\t"),
        ];
        (prefix.clone(), prefix, "\\PC{0,30}|[a-z./:@\\\\%]{0,20}")
            .prop_map(|(first, second, rest)| format!("{first}{second}{rest}"))
    }

    proptest! {
        #[test]
        fn link_duoc_nhan_trinh_duyet_mo_dung_cho(href in href()) {
            if !safe_link(&href) {
                return Ok(());
            }
            let site = Url::parse("https://site.example/blog/post").unwrap();
            // Link sai dạng thì trình duyệt không mở gì cả: an toàn.
            let Ok(target) = site.join(&href) else {
                return Ok(());
            };
            prop_assert!(
                matches!(target.scheme(), "http" | "https" | "mailto" | "tel"),
                "{href:?} → {target}"
            );
            if href.trim().starts_with('/') {
                prop_assert_eq!(target.host_str(), Some("site.example"), "{:?} → {}", href, target);
            }
        }
    }
}
