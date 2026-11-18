//! Hành động của luật: danh sách tuần tự. Mỗi bước là một tool MCP đã có (tham số điền từ dữ liệu
//! kích hoạt bằng mẫu `{{ event.order.id }}`), hay một hành động sẵn có của module này:
//!
//! ```json
//! [
//!   {"tool": "notes.create_note", "arguments": {"title": "Order {{ event.code }}", "body": "…"}},
//!   {"wait": {"minutes": 60}},
//!   {"stop_if": {"field": "steps.0.result.data.archived", "op": "eq", "value": true}},
//!   {"notify": {"title": "Big order {{ event.code }}", "body": "…"}}
//! ]
//! ```
//!
//! `notify` gửi thông báo (hộp thư trong ứng dụng, email) cho người tạo luật, qua module
//! `notifications` — mặc định của cảnh báo số liệu.

use autoxide_kernel::error::FieldError;
use serde_json::{Map, Value};

use super::condition::Condition;

/// Số bước tối đa của một luật.
pub(crate) const MAX_ACTIONS: usize = 20;
/// `wait` dài nhất: 30 ngày.
pub(crate) const MAX_WAIT_MINUTES: u64 = 30 * 24 * 60;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Action {
    Tool { tool: String, arguments: Map<String, Value> },
    Wait { minutes: u64 },
    StopIf(Condition),
    Notify { title: String, body: String },
}

impl Action {
    /// Tên loại bước, ghi vào nhật ký lần chạy.
    pub(crate) fn kind(&self) -> &str {
        match self {
            Action::Tool { tool, .. } => tool,
            Action::Wait { .. } => "wait",
            Action::StopIf(_) => "stop_if",
            Action::Notify { .. } => "notify",
        }
    }

    /// Đọc danh sách hành động ở `field`.
    pub(crate) fn parse_all(
        value: &Value,
        field: &str,
        errors: &mut Vec<FieldError>,
    ) -> Option<Vec<Action>> {
        let Some(items) = value.as_array().filter(|items| !items.is_empty()) else {
            errors.push(FieldError::new(field, "invalid", "must be a non-empty list of actions"));
            return None;
        };
        if items.len() > MAX_ACTIONS {
            errors.push(
                FieldError::new(field, "too_large", "must have at most {max} actions")
                    .with_param("max", MAX_ACTIONS),
            );
            return None;
        }
        let parsed: Vec<Option<Action>> = items
            .iter()
            .enumerate()
            .map(|(index, item)| parse_one(item, &format!("{field}.{index}"), errors))
            .collect();
        parsed.into_iter().collect()
    }
}

fn parse_one(value: &Value, field: &str, errors: &mut Vec<FieldError>) -> Option<Action> {
    let invalid = || {
        FieldError::new(
            field,
            "invalid",
            "must be an object with `tool` (and `arguments`), `wait`, `stop_if` or `notify`",
        )
    };
    let Some(object) = value.as_object() else {
        errors.push(invalid());
        return None;
    };
    if let Some(tool) = object.get("tool") {
        if object.keys().any(|key| key != "tool" && key != "arguments") {
            errors.push(invalid());
            return None;
        }
        let Some(tool) = tool.as_str().filter(|tool| tool.contains('.')) else {
            errors.push(FieldError::new(
                format!("{field}.tool"),
                "invalid",
                "must be the full name of an MCP tool, such as `notes.create_note`",
            ));
            return None;
        };
        let arguments = match object.get("arguments") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(arguments)) => arguments.clone(),
            Some(_) => {
                errors.push(FieldError::new(
                    format!("{field}.arguments"),
                    "invalid",
                    "must be an object",
                ));
                return None;
            }
        };
        return Some(Action::Tool { tool: tool.to_owned(), arguments });
    }
    if object.len() != 1 {
        errors.push(invalid());
        return None;
    }
    if let Some(wait) = object.get("wait") {
        let minutes = wait
            .as_object()
            .filter(|wait| wait.len() == 1)
            .and_then(|wait| wait.get("minutes"))
            .and_then(Value::as_u64)
            .filter(|minutes| (1..=MAX_WAIT_MINUTES).contains(minutes));
        return match minutes {
            Some(minutes) => Some(Action::Wait { minutes }),
            None => {
                errors.push(
                    FieldError::new(
                        format!("{field}.wait.minutes"),
                        "range",
                        "must be from 1 to {max} (30 days)",
                    )
                    .with_param("max", MAX_WAIT_MINUTES),
                );
                None
            }
        };
    }
    if let Some(condition) = object.get("stop_if") {
        if condition.is_null() {
            errors.push(FieldError::new(format!("{field}.stop_if"), "required", "is required"));
            return None;
        }
        return Condition::parse(condition, &format!("{field}.stop_if"), errors)
            .map(Action::StopIf);
    }
    if let Some(notify) = object.get("notify") {
        let text = |key: &str, max: usize, errors: &mut Vec<FieldError>| match notify
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
        {
            Some(text) if text.chars().count() <= max => Some(text.to_owned()),
            _ => {
                errors.push(
                    FieldError::new(
                        format!("{field}.notify.{key}"),
                        "length",
                        "must have from 1 to {max} characters",
                    )
                    .with_param("max", max),
                );
                None
            }
        };
        let title = text("title", 200, errors);
        let body = text("body", 2000, errors);
        return Some(Action::Notify { title: title?, body: body? });
    }
    errors.push(invalid());
    None
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn doc_hanh_dong() {
        let mut errors = Vec::new();
        let actions = Action::parse_all(
            &json!([
                {"tool": "notes.create_note", "arguments": {"title": "x"}},
                {"wait": {"minutes": 60}},
                {"stop_if": {"field": "event.x", "op": "exists"}},
                {"notify": {"title": "a", "body": "b"}}
            ]),
            "actions",
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");
        let kinds: Vec<String> =
            actions.unwrap().iter().map(|action| action.kind().to_owned()).collect();
        assert_eq!(kinds, vec!["notes.create_note", "wait", "stop_if", "notify"]);
    }

    #[test]
    fn hanh_dong_sai_bao_tung_cho() {
        let mut errors = Vec::new();
        let actions = Action::parse_all(
            &json!([
                {"tool": "create_note"},
                {"wait": {"minutes": 50_000}},
                {"run": "rm -rf /"},
                {"notify": {"title": ""}}
            ]),
            "actions",
            &mut errors,
        );
        assert!(actions.is_none());
        let fields: Vec<&str> = errors.iter().map(|error| error.field.as_str()).collect();
        assert_eq!(
            fields,
            vec![
                "actions.0.tool",
                "actions.1.wait.minutes",
                "actions.2",
                "actions.3.notify.title",
                "actions.3.notify.body"
            ]
        );
    }
}
