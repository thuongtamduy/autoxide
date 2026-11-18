//! Điền mẫu vào tham số của hành động, bằng bộ dựng mẫu của kernel (D-206).
//!
//! - Chuỗi chỉ gồm MỘT đường dẫn (`"{{ event.order.id }}"`) được thay bằng nguyên giá trị ở đó, giữ
//!   kiểu (số vẫn là số, mảng vẫn là mảng) — tham số của tool có kiểu.
//! - Chuỗi khác có `{{` / `{%` được dựng như mẫu chữ (`"Đơn {{ event.code }} từ {{ event.city }}"`).
//!
//! Biến được dùng: `event` (dữ liệu kích hoạt), `trigger` (tên event, lúc xảy ra…), `steps` (kết quả
//! các bước trước, theo thứ tự).

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::template::{Output, TemplateEngine, TemplateSchema, Variable};
use serde_json::Value;

use super::condition::{lookup, valid_path};

/// Biến mẫu được dùng trong tham số và câu thông báo.
pub(crate) fn variables() -> TemplateSchema {
    TemplateSchema::new(vec![
        Variable::optional(
            "event",
            "The data of the trigger: the event payload, the metric reading or the manual input.",
        ),
        Variable::optional(
            "trigger",
            "What started the run: `type`, `name`, `event_id`, `subject`, `occurred_at`.",
        ),
        Variable::optional("steps", "Results of the earlier steps, in order: `steps.0.result`."),
    ])
}

/// Chuỗi có phần mẫu không.
pub(crate) fn is_template(text: &str) -> bool {
    text.contains("{{") || text.contains("{%")
}

/// Đường dẫn khi chuỗi chỉ là `{{ <đường dẫn> }}`.
fn whole_path(text: &str) -> Option<&str> {
    let inner = text.trim().strip_prefix("{{")?.strip_suffix("}}")?.trim();
    valid_path(inner).then_some(inner)
}

/// Kiểm mọi mẫu trong `value` (cú pháp, chỉ dùng biến được phép); lỗi ghi vào `errors` theo trường.
pub(crate) fn check(
    engine: &TemplateEngine,
    value: &Value,
    field: &str,
    errors: &mut Vec<FieldError>,
) {
    match value {
        Value::String(text) if is_template(text) => {
            if whole_path(text).is_some() {
                return;
            }
            if let Err(error) = engine.validate(field, text, &variables()) {
                if error.fields().is_empty() {
                    errors.push(FieldError::new(field, "invalid", "is not a valid template"));
                } else {
                    errors.extend(error.fields().iter().cloned());
                }
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                check(engine, item, &format!("{field}.{index}"), errors);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                check(engine, item, &format!("{field}.{key}"), errors);
            }
        }
        _ => {}
    }
}

/// Điền mẫu trong `value` bằng dữ liệu `context`. Lỗi của mẫu (sai kiểu lúc chạy, vượt giới hạn) là
/// lỗi 422 `template.*` của kernel.
pub(crate) fn fill(
    engine: &TemplateEngine,
    value: &Value,
    context: &Value,
) -> Result<Value, Error> {
    Ok(match value {
        Value::String(text) if is_template(text) => match whole_path(text) {
            Some(path) => lookup(context, path).cloned().unwrap_or(Value::Null),
            None => Value::String(engine.render(text, Output::Text, "en", context)?),
        },
        Value::Array(items) => Value::Array(
            items.iter().map(|item| fill(engine, item, context)).collect::<Result<_, _>>()?,
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| Ok((key.clone(), fill(engine, item, context)?)))
                .collect::<Result<_, Error>>()?,
        ),
        other => other.clone(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn duong_dan_giu_kieu_mau_chu_duoc_dung() {
        let engine = TemplateEngine::default();
        let context = json!({"event": {"id": 7, "tags": ["a"], "code": "DH1"}});
        let filled = fill(
            &engine,
            &json!({"id": "{{ event.id }}", "tags": "{{event.tags}}", "title": "Order {{ event.code }}", "missing": "{{ event.none }}", "n": 1}),
            &context,
        )
        .unwrap();
        assert_eq!(
            filled,
            json!({"id": 7, "tags": ["a"], "title": "Order DH1", "missing": null, "n": 1})
        );
    }

    #[test]
    fn mau_dung_bien_la_bi_bao() {
        let engine = TemplateEngine::default();
        let mut errors = Vec::new();
        check(
            &engine,
            &json!({"a": "{{ secret.key }}", "b": ["{{ event.x"]}),
            "arguments",
            &mut errors,
        );
        let fields: Vec<&str> = errors.iter().map(|error| error.field.as_str()).collect();
        assert_eq!(fields, vec!["arguments.a", "arguments.b.0"]);
    }
}
