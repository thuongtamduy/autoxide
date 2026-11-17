//! Kiểm một response thật có khớp hợp đồng `docs/openapi.json` của chính endpoint đó không (đặc tả
//! 31 B4, D-241): mã trạng thái có khai, media type đúng, thân JSON đúng schema.
//!
//! [`TestRequest::send`](crate::TestRequest::send) gọi bộ kiểm này cho mọi request: mọi test HTTP
//! thành test hợp đồng, tài liệu không hứa điều code không làm.
//!
//! Bộ kiểm JSON Schema tự viết cho đúng tập con mà utoipa sinh ra (`type` / nullable kiểu danh
//! sách, `properties`, `required`, `additionalProperties`, `items`, `enum`, `const`, `oneOf`,
//! `anyOf`, `allOf`, `$ref`, giới hạn số và độ dài, `format` `uuid` / `date-time` / `date`) — không
//! thêm thư viện cho vài trăm dòng. `pattern` không kiểm (cần thư viện regex).

use serde_json::Value;

/// Tìm operation của `method` + `path` trong tài liệu: đường dẫn mẫu `{…}` khớp một đoạn bất kỳ;
/// nhiều mẫu khớp thì mẫu có nhiều đoạn cố định hơn thắng (`/public/{key}` trước `/{id}`).
fn operation<'a>(document: &'a Value, method: &str, path: &str) -> Option<(&'a str, &'a Value)> {
    let paths = document.get("paths")?.as_object()?;
    let segments: Vec<&str> = path.split('/').collect();
    let mut best: Option<(usize, &str, &Value)> = None;
    for (template, item) in paths {
        let parts: Vec<&str> = template.split('/').collect();
        if parts.len() != segments.len() {
            continue;
        }
        let mut literal = 0;
        let matches = parts.iter().zip(&segments).all(|(part, segment)| {
            if part.starts_with('{') && part.ends_with('}') {
                !segment.is_empty()
            } else {
                literal += 1;
                part == segment
            }
        });
        if !matches {
            continue;
        }
        let Some(operation) = item.get(method.to_ascii_lowercase()) else {
            continue;
        };
        if best.is_none_or(|(score, _, _)| literal > score) {
            best = Some((literal, template, operation));
        }
    }
    best.map(|(_, template, operation)| (template, operation))
}

/// Kiểm một response. `Ok(())` cả khi đường dẫn không có trong tài liệu (route lạ → 404 của
/// router): không có hợp đồng để so.
pub fn check(
    document: &Value,
    method: &str,
    path: &str,
    status: u16,
    content_type: Option<&str>,
    body: &[u8],
) -> Result<(), String> {
    let Some((template, operation)) = operation(document, method, path) else {
        return Ok(());
    };
    let label = format!("{} {template} {status}", method.to_ascii_uppercase());
    let responses = operation.get("responses").and_then(Value::as_object);
    let response = responses
        .and_then(|responses| {
            responses.get(&status.to_string()).or_else(|| responses.get("default"))
        })
        .ok_or_else(|| format!("{label}: this status is not documented"))?;
    let response = deref(document, response);
    if body.is_empty() || status == 204 || status == 304 || method.eq_ignore_ascii_case("HEAD") {
        return Ok(());
    }
    let Some(content) = response.get("content").and_then(Value::as_object) else {
        return Err(format!("{label}: the response has a body but the document declares none"));
    };
    let media = content_type
        .map(|value| value.split(';').next().unwrap_or_default().trim().to_ascii_lowercase())
        .unwrap_or_default();
    let declared = content.get(&media).or_else(|| {
        // Nội dung nhị phân (tệp của media, CSV…) khai `application/octet-stream`: nhận mọi loại
        // không phải JSON.
        (!media.ends_with("json")).then(|| content.get("application/octet-stream")).flatten()
    });
    let Some(declared) = declared else {
        let known: Vec<&str> = content.keys().map(String::as_str).collect();
        return Err(format!(
            "{label}: Content-Type `{media}` is not documented (the document lists {})",
            known.join(", ")
        ));
    };
    // JSON Lines (`application/x-ndjson`) là nhiều văn bản JSON, không phải một: không kiểm schema.
    if !media.ends_with("json") || media.ends_with("ndjson") {
        return Ok(());
    }
    let Some(schema) = declared.get("schema") else {
        return Ok(());
    };
    let value: Value = serde_json::from_slice(body)
        .map_err(|error| format!("{label}: the body is not JSON: {error}"))?;
    let mut problems = Vec::new();
    validate(document, schema, &value, "$", &mut problems, 0);
    match problems.first() {
        None => Ok(()),
        Some(first) => Err(format!(
            "{label}: {first}{}",
            if problems.len() > 1 {
                format!(" (and {} more)", problems.len() - 1)
            } else {
                String::new()
            }
        )),
    }
}

fn deref<'a>(document: &'a Value, value: &'a Value) -> &'a Value {
    match value.get("$ref").and_then(Value::as_str) {
        Some(reference) => resolve(document, reference).unwrap_or(value),
        None => value,
    }
}

fn resolve<'a>(document: &'a Value, reference: &str) -> Option<&'a Value> {
    document.pointer(reference.strip_prefix('#')?)
}

/// Thêm vào `problems` mọi chỗ `value` lệch `schema` (vị trí kiểu `$.data.items[0].id`).
pub fn validate(
    document: &Value,
    schema: &Value,
    value: &Value,
    at: &str,
    problems: &mut Vec<String>,
    depth: usize,
) {
    if depth > 64 {
        return;
    }
    let Some(object) = schema.as_object() else {
        return; // `true` / schema rỗng: mọi giá trị.
    };
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        match resolve(document, reference) {
            Some(target) => validate(document, target, value, at, problems, depth + 1),
            None => problems.push(format!("{at}: `$ref` {reference} does not resolve")),
        }
    }
    if let Some(parts) = object.get("allOf").and_then(Value::as_array) {
        for part in parts {
            validate(document, part, value, at, problems, depth + 1);
        }
    }
    // `oneOf` của utoipa có thể chồng nhau (vd hai biến thể đều là object): kiểm "khớp ít nhất một"
    // như `anyOf`, để không báo oan.
    for key in ["oneOf", "anyOf"] {
        if let Some(parts) = object.get(key).and_then(Value::as_array) {
            let matched = parts.iter().any(|part| {
                let mut inner = Vec::new();
                validate(document, part, value, at, &mut inner, depth + 1);
                inner.is_empty()
            });
            if !matched {
                problems.push(format!("{at}: matches none of the `{key}` variants"));
            }
        }
    }
    if let Some(kind) = object.get("type") {
        let kinds: Vec<&str> = match kind {
            Value::String(kind) => vec![kind.as_str()],
            Value::Array(kinds) => kinds.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if !kinds.is_empty() && !kinds.iter().any(|kind| has_type(value, kind)) {
            let format =
                object.get("format").and_then(Value::as_str).map(|format| format!(" ({format})"));
            problems.push(format!(
                "{at}: expected {}{}, got {}",
                kinds.join(" | "),
                format.unwrap_or_default(),
                describe(value)
            ));
            return;
        }
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array)
        && !values.contains(value)
    {
        problems.push(format!("{at}: {value} is not one of {}", Value::Array(values.clone())));
    }
    if let Some(expected) = object.get("const")
        && expected != value
    {
        problems.push(format!("{at}: expected {expected}, got {value}"));
    }
    match value {
        Value::String(text) => {
            if let Some(format) = object.get("format").and_then(Value::as_str)
                && !format_ok(format, text)
            {
                problems.push(format!("{at}: `{text}` is not a valid {format}"));
            }
            let length = text.chars().count() as f64;
            limit(object, "minLength", "maxLength", length, at, "length", problems);
        }
        Value::Number(number) => {
            if let Some(number) = number.as_f64() {
                limit(object, "minimum", "maximum", number, at, "value", problems);
            }
        }
        Value::Array(items) => {
            limit(object, "minItems", "maxItems", items.len() as f64, at, "item count", problems);
            if let Some(item_schema) = object.get("items") {
                for (index, item) in items.iter().enumerate() {
                    validate(
                        document,
                        item_schema,
                        item,
                        &format!("{at}[{index}]"),
                        problems,
                        depth + 1,
                    );
                }
            }
        }
        Value::Object(fields) => {
            if let Some(required) = object.get("required").and_then(Value::as_array) {
                for name in required.iter().filter_map(Value::as_str) {
                    if !fields.contains_key(name) {
                        problems.push(format!("{at}.{name}: required but missing"));
                    }
                }
            }
            let properties = object.get("properties").and_then(Value::as_object);
            for (name, field) in fields {
                let here = format!("{at}.{name}");
                match properties.and_then(|properties| properties.get(name)) {
                    Some(field_schema) => {
                        validate(document, field_schema, field, &here, problems, depth + 1)
                    }
                    None => match object.get("additionalProperties") {
                        Some(Value::Bool(false)) => {
                            problems.push(format!("{here}: not in the documented fields"));
                        }
                        Some(extra @ Value::Object(_)) => {
                            validate(document, extra, field, &here, problems, depth + 1);
                        }
                        _ => {}
                    },
                }
            }
        }
        _ => {}
    }
}

fn has_type(value: &Value, kind: &str) -> bool {
    match kind {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "number" => value.is_number(),
        "integer" => {
            value.is_i64() || value.is_u64() || value.as_f64().is_some_and(|n| n.fract() == 0.0)
        }
        _ => true,
    }
}

fn describe(value: &Value) -> String {
    let text = value.to_string();
    let short: String = text.chars().take(60).collect();
    if short.len() < text.len() { format!("{short}…") } else { short }
}

fn limit(
    object: &serde_json::Map<String, Value>,
    min_key: &str,
    max_key: &str,
    actual: f64,
    at: &str,
    what: &str,
    problems: &mut Vec<String>,
) {
    if let Some(min) = object.get(min_key).and_then(Value::as_f64)
        && actual < min
    {
        problems.push(format!("{at}: {what} {actual} is below {min_key} {min}"));
    }
    if let Some(max) = object.get(max_key).and_then(Value::as_f64)
        && actual > max
    {
        problems.push(format!("{at}: {what} {actual} is above {max_key} {max}"));
    }
}

fn format_ok(format: &str, text: &str) -> bool {
    match format {
        "uuid" => uuid::Uuid::parse_str(text).is_ok(),
        "date-time" => chrono::DateTime::parse_from_rfc3339(text).is_ok(),
        "date" => chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn document() -> Value {
        json!({
            "paths": {
                "/api/v1/x/{id}": {"get": {"responses": {
                    "200": {"description": "ok", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Item"}}}},
                    "default": {"description": "error", "content": {"application/problem+json": {"schema": {"type": "object", "required": ["code"]}}}}
                }}},
                "/api/v1/x/public": {"get": {"responses": {"204": {"description": "none"}}}},
                "/api/v1/file": {"get": {"responses": {"200": {"description": "file", "content": {"application/octet-stream": {}}}}}}
            },
            "components": {"schemas": {"Item": {
                "type": "object",
                "required": ["id", "tags"],
                "properties": {
                    "id": {"type": "string", "format": "uuid"},
                    "name": {"type": ["string", "null"], "maxLength": 3},
                    "tags": {"type": "array", "items": {"type": "string", "enum": ["a", "b"]}},
                    "parent": {"oneOf": [{"$ref": "#/components/schemas/Item"}, {"type": "null"}]}
                }
            }}}
        })
    }

    fn run(path: &str, status: u16, media: &str, body: Value) -> Result<(), String> {
        check(&document(), "GET", path, status, Some(media), body.to_string().as_bytes())
    }

    #[test]
    fn khop_hop_dong_thi_qua() {
        let id = uuid::Uuid::nil().to_string();
        let body = json!({"id": id, "name": null, "tags": ["a"], "parent": {"id": id, "tags": []}});
        run("/api/v1/x/1", 200, "application/json; charset=utf-8", body).unwrap();
        run("/api/v1/x/1", 404, "application/problem+json", json!({"code": "x"})).unwrap();
        run("/api/v1/nowhere", 200, "text/plain", json!(1)).unwrap();
        check(&document(), "GET", "/api/v1/file", 200, Some("image/png"), b"\x89PNG").unwrap();
        check(&document(), "GET", "/api/v1/x/public", 204, None, b"").unwrap();
    }

    #[test]
    fn lech_hop_dong_thi_bao_dung_cho() {
        let id = uuid::Uuid::nil().to_string();
        let error = run("/api/v1/x/1", 200, "application/json", json!({"id": "nope", "tags": []}))
            .unwrap_err();
        assert_eq!(error, "GET /api/v1/x/{id} 200: $.id: `nope` is not a valid uuid");
        let error = run("/api/v1/x/1", 200, "application/json", json!({"id": id})).unwrap_err();
        assert!(error.ends_with("$.tags: required but missing"), "{error}");
        let error = run(
            "/api/v1/x/1",
            200,
            "application/json",
            json!({"id": id, "tags": ["c"], "name": "long"}),
        )
        .unwrap_err();
        assert!(error.contains("$.name: length 4 is above maxLength 3"), "{error}");
        assert!(error.contains("(and 1 more)"), "{error}");
        let error =
            run("/api/v1/x/1", 200, "application/json", json!({"id": id, "tags": [], "parent": 5}))
                .unwrap_err();
        assert!(error.contains("$.parent: matches none of the `oneOf` variants"), "{error}");
        let error = run("/api/v1/x/1", 404, "application/json", json!({"code": "x"})).unwrap_err();
        assert!(error.contains("Content-Type `application/json` is not documented"), "{error}");
        let error = run("/api/v1/x/public", 200, "application/json", json!({})).unwrap_err();
        assert!(error.contains("this status is not documented"), "{error}");
        let error = check(&document(), "GET", "/api/v1/file", 200, Some("application/json"), b"{}")
            .unwrap_err();
        assert!(error.contains("not documented"), "{error}");
    }

    #[test]
    fn mau_co_dinh_thang_mau_tham_so() {
        let error = run("/api/v1/x/public", 200, "application/json", json!({})).unwrap_err();
        assert!(error.starts_with("GET /api/v1/x/public 200"), "{error}");
    }
}
