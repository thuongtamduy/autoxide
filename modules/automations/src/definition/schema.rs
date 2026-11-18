//! Kiểm tham số của hành động với JSON Schema đầu vào của tool (sinh từ kiểu Rust của tool, không có
//! `$ref`). Chỉ phần schema mà tool của Autoxide dùng: `type` (một hay nhiều), `oneOf` / `anyOf`,
//! `enum`, `required`, `properties`, `additionalProperties: false`, `items`, `minimum` / `maximum`,
//! `minLength` / `maxLength`, `format: uuid`. Từ khoá khác bỏ qua — tool vẫn tự kiểm lần cuối khi
//! chạy, đây là kiểm sớm lúc lưu luật để báo lỗi một lần (D-083). Vài chục dòng tự viết thay cho
//! một thư viện JSON Schema đầy đủ (CLAUDE.md: không thêm thư viện khi tự viết đủ).
//!
//! Giá trị còn là mẫu (`"{{ event.id }}"`) chưa biết kiểu: bỏ qua, chỉ kiểm có mặt. Chạy thử khô
//! kiểm lại sau khi điền dữ liệu mẫu.

use autoxide_kernel::error::FieldError;
use serde_json::Value;

use super::fill::is_template;

/// Kiểm `value` với `schema`; lỗi ghi vào `errors` với tên trường bắt đầu bằng `field`.
pub(crate) fn check(schema: &Value, value: &Value, field: &str, errors: &mut Vec<FieldError>) {
    if matches!(value, Value::String(text) if is_template(text)) {
        return;
    }
    // `oneOf` / `anyOf` (vd `Option<Enum>`): khớp một nhánh là đủ.
    for keyword in ["oneOf", "anyOf"] {
        if let Some(branches) = schema.get(keyword).and_then(Value::as_array) {
            let fits = branches.iter().any(|branch| {
                let mut scratch = Vec::new();
                check(branch, value, field, &mut scratch);
                scratch.is_empty()
            });
            if !fits {
                errors.push(FieldError::new(field, "invalid", "does not match the expected type"));
            }
            return;
        }
    }
    if let Some(types) = schema.get("type") {
        let allowed: Vec<&str> = match types {
            Value::String(name) => vec![name.as_str()],
            Value::Array(names) => names.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if !allowed.is_empty() && !allowed.iter().any(|name| is_type(value, name)) {
            errors.push(
                FieldError::new(field, "type", "must be of type {type}")
                    .with_param("type", allowed.join(" or ")),
            );
            return;
        }
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array)
        && !options.contains(value)
    {
        let names: Vec<String> = options.iter().map(Value::to_string).collect();
        errors.push(
            FieldError::new(field, "invalid", "must be one of {options}")
                .with_param("options", names.join(", ")),
        );
        return;
    }
    match value {
        Value::Object(map) => {
            let properties = schema.get("properties").and_then(Value::as_object);
            for name in schema.get("required").and_then(Value::as_array).into_iter().flatten() {
                if let Some(name) = name.as_str()
                    && !map.contains_key(name)
                {
                    errors.push(FieldError::new(
                        format!("{field}.{name}"),
                        "required",
                        "is required",
                    ));
                }
            }
            let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
            for (key, item) in map {
                match properties.and_then(|properties| properties.get(key)) {
                    Some(property) => check(property, item, &format!("{field}.{key}"), errors),
                    None if closed => errors.push(FieldError::new(
                        format!("{field}.{key}"),
                        "unknown",
                        "is not a known field",
                    )),
                    None => {}
                }
            }
        }
        Value::Array(items) => {
            if let Some(item_schema) = schema.get("items") {
                for (index, item) in items.iter().enumerate() {
                    check(item_schema, item, &format!("{field}.{index}"), errors);
                }
            }
        }
        Value::String(text) => {
            let length = text.chars().count() as u64;
            let min = schema.get("minLength").and_then(Value::as_u64);
            let max = schema.get("maxLength").and_then(Value::as_u64);
            if min.is_some_and(|min| length < min) || max.is_some_and(|max| length > max) {
                errors.push(
                    FieldError::new(field, "length", "must have from {min} to {max} characters")
                        .with_param("min", min.unwrap_or(0))
                        .with_param(
                            "max",
                            max.map_or_else(|| "any".to_owned(), |max| max.to_string()),
                        ),
                );
            }
            if schema.get("format").and_then(Value::as_str) == Some("uuid")
                && uuid::Uuid::parse_str(text).is_err()
            {
                errors.push(FieldError::new(field, "invalid", "must be a UUID"));
            }
        }
        Value::Number(number) => {
            let number = number.as_f64().unwrap_or_default();
            let min = schema.get("minimum").and_then(Value::as_f64);
            let max = schema.get("maximum").and_then(Value::as_f64);
            if min.is_some_and(|min| number < min) || max.is_some_and(|max| number > max) {
                errors.push(
                    FieldError::new(field, "range", "must be from {min} to {max}")
                        .with_param(
                            "min",
                            min.map_or_else(|| "any".to_owned(), |min| min.to_string()),
                        )
                        .with_param(
                            "max",
                            max.map_or_else(|| "any".to_owned(), |max| max.to_string()),
                        ),
                );
            }
        }
        _ => {}
    }
}

fn is_type(value: &Value, name: &str) -> bool {
    match name {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn errors(schema: &Value, value: &Value) -> Vec<String> {
        let mut errors = Vec::new();
        check(schema, value, "arguments", &mut errors);
        errors.into_iter().map(|error| format!("{} {}", error.field, error.code)).collect()
    }

    #[test]
    fn kiem_theo_schema_cua_tool() {
        let schema = json!({
            "type": "object",
            "required": ["title", "count"],
            "additionalProperties": false,
            "properties": {
                "title": {"type": "string", "minLength": 1, "maxLength": 5},
                "count": {"type": "integer", "minimum": 1},
                "id": {"type": ["string", "null"], "format": "uuid"},
                "kind": {"oneOf": [{"type": "null"}, {"type": "string", "enum": ["a", "b"]}]},
                "tags": {"type": "array", "items": {"type": "string"}}
            }
        });
        assert!(
            errors(&schema, &json!({"title": "abc", "count": 2, "id": null, "kind": "a"}))
                .is_empty()
        );
        assert_eq!(
            errors(
                &schema,
                &json!({"title": "abcdef", "id": "x", "kind": "c", "tags": [1], "extra": 1})
            ),
            vec![
                "arguments.count required",
                "arguments.extra unknown",
                "arguments.id invalid",
                "arguments.kind invalid",
                "arguments.tags.0 type",
                "arguments.title length",
            ]
        );
        // Mẫu chưa điền: chỉ kiểm có mặt.
        assert!(
            errors(&schema, &json!({"title": "{{ event.title }}", "count": "{{ event.n }}"}))
                .is_empty()
        );
    }
}
