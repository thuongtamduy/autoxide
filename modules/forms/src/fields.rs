//! Trường của form: danh sách kiểu ĐÓNG, kiểm định nghĩa (khi quản trị viên tạo / sửa form) và
//! kiểm câu trả lời (khi người lạ gửi). Cả hai gom MỌI lỗi để báo một lần (D-083), đường dẫn
//! lỗi theo `key` của trường (`data.email`, `fields[2].options`).
//!
//! Kiểu này vừa là dạng lưu trong cột JSON `fields` (và ảnh chụp ở `forms_form_versions`), vừa
//! là dạng của API: đổi tên trường ở đây là đổi hợp đồng — chỉ thêm, không đổi.

use std::collections::HashSet;

use autoxide_kernel::error::FieldError;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use utoipa::ToSchema;

/// The kind of a field. The list is closed: anything else is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FieldType {
    /// One line of text.
    Text,
    /// An email address.
    Email,
    /// A phone number: digits, spaces, `+`, `-`, `(`, `)`, `.`; 6 to 15 digits.
    Phone,
    /// Several lines of text.
    Textarea,
    /// A number (integer or decimal), optionally within `min`..`max`.
    Number,
    /// One value from `options`, as a drop-down list.
    Select,
    /// One value from `options`, as radio buttons.
    Radio,
    /// Any number of values from `options` (an array of values).
    Checkbox,
    /// A calendar date, `YYYY-MM-DD`.
    Date,
    /// A value the page fills in itself (e.g. a campaign tag), not shown to people.
    Hidden,
    /// A consent box: its label is the text shown; the answer is `true`. Never ticked in
    /// advance. The text, the form version and the time are kept with the submission.
    Consent,
}

impl FieldType {
    pub(crate) fn has_options(self) -> bool {
        matches!(self, FieldType::Select | FieldType::Radio | FieldType::Checkbox)
    }

    /// Độ dài mặc định và tối đa được khai của trường chữ; `None` nếu kiểu không nhận
    /// `min_length` / `max_length`.
    fn length_limits(self) -> Option<(u32, u32)> {
        match self {
            FieldType::Text => Some((500, 10_000)),
            FieldType::Textarea => Some((5000, 20_000)),
            FieldType::Hidden => Some((1000, 10_000)),
            _ => None,
        }
    }
}

/// A language of form texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Locale {
    /// English (the default).
    #[default]
    En,
    /// Vietnamese.
    Vi,
}

impl Locale {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::Vi => "vi",
        }
    }

    pub(crate) fn parse(raw: &str) -> Option<Locale> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "en" => Some(Locale::En),
            "vi" => Some(Locale::Vi),
            _ => None,
        }
    }
}

/// A text in English and Vietnamese; at least one is needed where the text is required. A
/// missing language falls back to the other one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Localized {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub en: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vi: Option<String>,
}

impl Localized {
    /// Chữ bằng `locale`, thiếu thì bằng ngôn ngữ kia; `""` nếu không có gì.
    pub(crate) fn get(&self, locale: Locale) -> &str {
        let (first, second) = match locale {
            Locale::En => (&self.en, &self.vi),
            Locale::Vi => (&self.vi, &self.en),
        };
        first.as_deref().filter(|text| !text.is_empty()).or(second.as_deref()).unwrap_or("")
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.en.as_deref().is_none_or(str::is_empty) && self.vi.as_deref().is_none_or(str::is_empty)
    }

    /// Bỏ khoảng trắng hai đầu, chuỗi rỗng thành `None`; kiểm độ dài từng bản.
    pub(crate) fn normalized(&self, field: &str, max: usize, problems: &mut Vec<FieldError>) -> Localized {
        let mut clean = |locale: &str, value: &Option<String>| {
            let value = value.as_deref().map(str::trim).filter(|value| !value.is_empty())?;
            if value.chars().count() > max {
                problems.push(FieldError::new(
                    format!("{field}.{locale}"),
                    "length",
                    format!("must be at most {max} characters long"),
                ));
            }
            Some(value.to_owned())
        };
        Localized { en: clean("en", &self.en), vi: clean("vi", &self.vi) }
    }
}

/// One choice of a `select`, `radio` or `checkbox` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FieldOption {
    /// What is stored and sent: 1 to 200 characters, unique in the field.
    pub value: String,
    /// What people see; omitted: the value.
    #[serde(default, skip_serializing_if = "Localized::is_empty")]
    pub label: Localized,
}

/// A field of a form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Field {
    /// Stable name of the answer: a lower-case letter, then lower-case letters, digits or
    /// `_`; at most 64 characters; unique in the form. Answers are sent and stored under it.
    #[schema(example = "email")]
    pub key: String,
    #[serde(rename = "type")]
    pub kind: FieldType,
    /// What people see next to the field (for `consent`: the whole consent text).
    pub label: Localized,
    /// Help text under the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<Localized>,
    /// Example shown inside an empty field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<Localized>,
    /// An answer is needed (for `consent`: the box must be ticked).
    #[serde(default)]
    pub required: bool,
    /// Choices of `select`, `radio` and `checkbox` fields (1 to 100); not allowed for other
    /// types.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<FieldOption>,
    /// Fewest characters (`text`, `textarea`, `hidden`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u32>,
    /// Most characters (`text`: default 500, up to 10000; `textarea`: default 5000, up to
    /// 20000; `hidden`: default 1000, up to 10000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    /// Smallest value (`number` only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Largest value (`number` only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

const LABEL_MAX: usize = 500;
const CONSENT_LABEL_MAX: usize = 5000;
const OPTIONS_MAX: usize = 100;
const OPTION_VALUE_MAX: usize = 200;
const EMAIL_MAX: usize = 254;
const PHONE_MAX: usize = 32;

/// `key` của trường: chữ thường đầu tiên, rồi chữ thường, số, `_`; tối đa 64 ký tự.
pub(crate) fn is_valid_key(key: &str) -> bool {
    key.len() <= 64
        && key.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Kiểm và chuẩn hoá danh sách trường (định nghĩa của form). Lỗi vào `problems`, đường dẫn
/// `fields[i]…`.
pub(crate) fn check_definition(
    fields: &[Field],
    max_fields: usize,
    problems: &mut Vec<FieldError>,
) -> Vec<Field> {
    if fields.is_empty() || fields.len() > max_fields {
        problems.push(FieldError::new(
            "fields",
            "length",
            format!("must have 1 to {max_fields} fields"),
        ));
    }
    let mut keys = HashSet::new();
    let mut consents = 0;
    let mut clean = Vec::with_capacity(fields.len());
    for (index, field) in fields.iter().enumerate() {
        let at = |name: &str| format!("fields[{index}].{name}");
        let key = field.key.trim().to_owned();
        if !is_valid_key(&key) {
            problems.push(FieldError::new(
                at("key"),
                "invalid",
                "must start with a lower-case letter, then lower-case letters, digits or `_`, at most 64 characters",
            ));
        } else if !keys.insert(key.clone()) {
            problems.push(FieldError::new(at("key"), "duplicate", "is used by another field"));
        }
        let label_max =
            if field.kind == FieldType::Consent { CONSENT_LABEL_MAX } else { LABEL_MAX };
        let label = field.label.normalized(&at("label"), label_max, problems);
        if label.is_empty() {
            problems.push(FieldError::new(at("label"), "required", "needs `en` or `vi`"));
        }
        let help =
            field.help.as_ref().map(|help| help.normalized(&at("help"), LABEL_MAX, problems));
        let placeholder = field
            .placeholder
            .as_ref()
            .map(|text| text.normalized(&at("placeholder"), LABEL_MAX, problems));
        if field.kind == FieldType::Consent {
            consents += 1;
            if consents > 1 {
                problems.push(FieldError::new(
                    at("type"),
                    "too_many",
                    "a form may have only one `consent` field",
                ));
            }
        }
        let options = check_options(field, &at("options"), problems);
        check_lengths(field, &at, problems);
        check_range(field, &at, problems);
        clean.push(Field {
            key,
            kind: field.kind,
            label,
            help: help.filter(|text| !text.is_empty()),
            placeholder: placeholder.filter(|text| !text.is_empty()),
            required: field.required,
            options,
            min_length: field.min_length,
            max_length: field.max_length,
            min: field.min,
            max: field.max,
        });
    }
    clean
}

fn check_options(field: &Field, at: &str, problems: &mut Vec<FieldError>) -> Vec<FieldOption> {
    if !field.kind.has_options() {
        if !field.options.is_empty() {
            problems.push(FieldError::new(
                at,
                "not_allowed",
                "only `select`, `radio` and `checkbox` fields have options",
            ));
        }
        return Vec::new();
    }
    if field.options.is_empty() || field.options.len() > OPTIONS_MAX {
        problems.push(FieldError::new(
            at,
            "length",
            format!("must have 1 to {OPTIONS_MAX} options"),
        ));
    }
    let mut values = HashSet::new();
    let mut clean = Vec::with_capacity(field.options.len());
    for (index, option) in field.options.iter().enumerate() {
        let value = option.value.trim().to_owned();
        if value.is_empty() || value.chars().count() > OPTION_VALUE_MAX {
            problems.push(FieldError::new(
                format!("{at}[{index}].value"),
                "length",
                format!("must be 1 to {OPTION_VALUE_MAX} characters"),
            ));
        } else if !values.insert(value.clone()) {
            problems.push(FieldError::new(
                format!("{at}[{index}].value"),
                "duplicate",
                "is used by another option",
            ));
        }
        let label = option.label.normalized(&format!("{at}[{index}].label"), LABEL_MAX, problems);
        clean.push(FieldOption { value, label });
    }
    clean
}

fn check_lengths(field: &Field, at: &dyn Fn(&str) -> String, problems: &mut Vec<FieldError>) {
    let Some((_, cap)) = field.kind.length_limits() else {
        for (name, value) in [("min_length", field.min_length), ("max_length", field.max_length)] {
            if value.is_some() {
                problems.push(FieldError::new(
                    at(name),
                    "not_allowed",
                    "only `text`, `textarea` and `hidden` fields have a length limit",
                ));
            }
        }
        return;
    };
    if field.max_length.is_some_and(|max| max == 0 || max > cap) {
        problems.push(FieldError::new(at("max_length"), "range", format!("must be 1 to {cap}")));
    }
    if let Some(min) = field.min_length
        && min > field.max_length.unwrap_or(cap)
    {
        problems.push(FieldError::new(
            at("min_length"),
            "range",
            "must not be more than `max_length`",
        ));
    }
}

fn check_range(field: &Field, at: &dyn Fn(&str) -> String, problems: &mut Vec<FieldError>) {
    if field.kind != FieldType::Number {
        for (name, value) in [("min", field.min), ("max", field.max)] {
            if value.is_some() {
                problems.push(FieldError::new(
                    at(name),
                    "not_allowed",
                    "only `number` fields have `min` and `max`",
                ));
            }
        }
        return;
    }
    for (name, value) in [("min", field.min), ("max", field.max)] {
        if value.is_some_and(|value| !value.is_finite()) {
            problems.push(FieldError::new(at(name), "invalid", "must be a finite number"));
        }
    }
    if let (Some(min), Some(max)) = (field.min, field.max)
        && min > max
    {
        problems.push(FieldError::new(at("min"), "range", "must not be more than `max`"));
    }
}

/// Câu trả lời đã kiểm.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Answers {
    /// `{key: value}` đã chuẩn hoá; trường bỏ trống không có mặt.
    pub data: Map<String, Value>,
    /// Mọi câu trả lời dạng chữ nối lại (để tìm và cho bộ phân loại nội dung).
    pub text: String,
}

/// Kiểm câu trả lời `input` theo `fields`: mọi trường, mọi lỗi (đường dẫn `data.<key>`); trường
/// lạ bị từ chối.
pub(crate) fn check_answers(
    fields: &[Field],
    input: &Map<String, Value>,
    problems: &mut Vec<FieldError>,
) -> Answers {
    let mut answers = Answers::default();
    let mut text = Vec::new();
    for key in input.keys() {
        if !fields.iter().any(|field| &field.key == key) {
            problems.push(FieldError::new(
                format!("data.{key}"),
                "unknown",
                "this form has no such field",
            ));
        }
    }
    for field in fields {
        let path = format!("data.{}", field.key);
        let raw = input.get(&field.key).filter(|value| !is_blank(value));
        let Some(raw) = raw else {
            if field.required {
                let (code, message) = if field.kind == FieldType::Consent {
                    ("consent_required", "the box must be ticked")
                } else {
                    ("required", "needs an answer")
                };
                problems.push(FieldError::new(path, code, message));
            }
            continue;
        };
        if let Some(value) = check_value(field, raw, &path, problems) {
            if let Value::String(value) = &value {
                text.push(value.clone());
            } else if let Value::Array(values) = &value {
                text.extend(values.iter().filter_map(Value::as_str).map(str::to_owned));
            }
            answers.data.insert(field.key.clone(), value);
        }
    }
    answers.text = text.join("\n");
    answers
}

/// Trống: `null`, chuỗi chỉ có khoảng trắng, mảng rỗng, `false`.
fn is_blank(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

/// Giá trị đã chuẩn hoá của một câu trả lời không trống; sai thì thêm lỗi, trả `None`.
fn check_value(
    field: &Field,
    raw: &Value,
    path: &str,
    problems: &mut Vec<FieldError>,
) -> Option<Value> {
    let mut fail = |code: &str, message: String| {
        problems.push(FieldError::new(path, code, message));
        None
    };
    match field.kind {
        FieldType::Number => {
            let number = match raw {
                Value::Number(number) => number.as_f64().map(|value| (value, number.clone())),
                Value::String(text) => text
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .and_then(|value| to_number(value).map(|number| (value, number))),
                _ => None,
            };
            let Some((value, number)) = number else {
                return fail("type", "must be a number".to_owned());
            };
            if field.min.is_some_and(|min| value < min) || field.max.is_some_and(|max| value > max)
            {
                let range = match (field.min, field.max) {
                    (Some(min), Some(max)) => format!("from {min} to {max}"),
                    (Some(min), None) => format!("at least {min}"),
                    (None, Some(max)) => format!("at most {max}"),
                    (None, None) => String::new(),
                };
                return fail("range", format!("must be {range}"));
            }
            Some(Value::Number(number))
        }
        FieldType::Consent => match raw {
            Value::Bool(true) => Some(Value::Bool(true)),
            _ => fail("type", "must be `true` (ticked) or `false`".to_owned()),
        },
        FieldType::Checkbox => {
            let Value::Array(items) = raw else {
                return fail("type", "must be an array of option values".to_owned());
            };
            let mut chosen: Vec<Value> = Vec::with_capacity(items.len());
            for item in items {
                let valid = item
                    .as_str()
                    .map(str::trim)
                    .filter(|value| field.options.iter().any(|option| option.value == *value));
                let Some(value) = valid else {
                    return fail(
                        "option",
                        format!("every value must be one of: {}", option_list(field)),
                    );
                };
                if !chosen.iter().any(|seen| seen.as_str() == Some(value)) {
                    chosen.push(Value::String(value.to_owned()));
                }
            }
            Some(Value::Array(chosen))
        }
        kind => {
            let Value::String(text) = raw else {
                return fail("type", "must be a string".to_owned());
            };
            let text = if kind == FieldType::Textarea {
                text.trim().replace("\r\n", "\n")
            } else {
                text.trim().to_owned()
            };
            let control = text.chars().any(|c| {
                c.is_control() && !(kind == FieldType::Textarea && matches!(c, '\n' | '\t'))
            });
            if control {
                return fail("invalid", "must not contain control characters".to_owned());
            }
            let length = text.chars().count();
            match kind {
                FieldType::Email => {
                    if length > EMAIL_MAX || !is_email(&text) {
                        return fail(
                            "email",
                            "must be an email address like `name@example.com`".to_owned(),
                        );
                    }
                }
                FieldType::Phone => {
                    if length > PHONE_MAX || !is_phone(&text) {
                        return fail(
                            "phone",
                            "must be a phone number: 6 to 15 digits, optionally with `+`, spaces, `-`, `(`, `)` or `.`".to_owned(),
                        );
                    }
                }
                FieldType::Date => {
                    if NaiveDate::parse_from_str(&text, "%Y-%m-%d").is_err() || text.len() != 10 {
                        return fail("date", "must be a date written `YYYY-MM-DD`".to_owned());
                    }
                }
                FieldType::Select | FieldType::Radio => {
                    if !field.options.iter().any(|option| option.value == text) {
                        return fail("option", format!("must be one of: {}", option_list(field)));
                    }
                }
                _ => {
                    let (default_max, _) = kind.length_limits().unwrap_or((500, 500));
                    let max = field.max_length.map_or(default_max as usize, |max| max as usize);
                    let min = field.min_length.unwrap_or(0) as usize;
                    if length > max || length < min {
                        return fail("length", format!("must be {min} to {max} characters long"));
                    }
                }
            }
            Some(Value::String(text))
        }
    }
}

/// Số JSON từ `value`: số nguyên thì giữ dạng nguyên.
fn to_number(value: f64) -> Option<Number> {
    // Số nguyên vừa i64: lưu dạng nguyên (`3` thay vì `3.0`).
    // Viết ra rồi đọc lại thay vì ép kiểu `as`: không có chỗ nào cắt ngầm giá trị.
    if value.fract() == 0.0
        && value.abs() < 9.0e15
        && let Ok(integer) = format!("{value:.0}").parse::<i64>()
    {
        return Some(Number::from(integer));
    }
    Number::from_f64(value)
}

fn option_list(field: &Field) -> String {
    field.options.iter().map(|option| format!("`{}`", option.value)).collect::<Vec<_>>().join(", ")
}

/// Địa chỉ email hợp lệ ở mức đủ dùng cho form: một `@`, phần trước 1–64 ký tự không có
/// khoảng trắng hay ký tự đặc biệt của header, tên miền có ít nhất một dấu chấm, mỗi nhãn
/// 1–63 ký tự chữ, số hoặc `-` (không ở đầu, cuối).
pub(crate) fn is_email(text: &str) -> bool {
    let Some((local, domain)) = text.split_once('@') else { return false };
    let local_ok = (1..=64).contains(&local.chars().count())
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local
            .chars()
            .all(|c| !c.is_whitespace() && !c.is_control() && !"<>()[]\\,;:\"@".contains(c));
    let labels: Vec<&str> = domain.split('.').collect();
    let domain_ok = labels.len() >= 2
        && labels.iter().all(|label| {
            (1..=63).contains(&label.chars().count())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_alphanumeric() || c == '-')
        });
    local_ok && domain_ok
}

/// Số điện thoại: chữ số, khoảng trắng, `-`, `(`, `)`, `.`, và `+` chỉ ở đầu; 6–15 chữ số.
fn is_phone(text: &str) -> bool {
    let body = text.strip_prefix('+').unwrap_or(text);
    let digits = body.chars().filter(char::is_ascii_digit).count();
    (6..=15).contains(&digits)
        && body.chars().all(|c| c.is_ascii_digit() || matches!(c, ' ' | '-' | '(' | ')' | '.'))
}

/// Email chuẩn hoá để lưu và tra: bỏ khoảng trắng hai đầu, chữ thường.
pub(crate) fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// Email của người gửi: trường `reply_to_field` nếu có câu trả lời, không thì trường `email`
/// đầu tiên có câu trả lời.
pub(crate) fn sender_email(
    fields: &[Field],
    reply_to_field: Option<&str>,
    data: &Map<String, Value>,
) -> Option<String> {
    let answer = |key: &str| data.get(key).and_then(Value::as_str).map(str::to_owned);
    reply_to_field.and_then(answer).or_else(|| {
        fields
            .iter()
            .filter(|field| field.kind == FieldType::Email)
            .find_map(|field| answer(&field.key))
    })
}

/// Một câu trả lời dạng chữ để hiện (CSV, mail): chuỗi giữ nguyên, số viết ra, mảng nối bằng
/// `; `, ô đồng ý là `true`.
pub(crate) fn display(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(display).collect::<Vec<_>>().join("; "),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields(value: Value) -> Vec<Field> {
        serde_json::from_value(value).unwrap()
    }

    fn sample() -> Vec<Field> {
        fields(json!([
            {"key": "name", "type": "text", "label": {"en": "Name"}, "required": true, "max_length": 20},
            {"key": "email", "type": "email", "label": {"en": "Email"}, "required": true},
            {"key": "phone", "type": "phone", "label": {"en": "Phone"}},
            {"key": "message", "type": "textarea", "label": {"en": "Message"}, "min_length": 5},
            {"key": "guests", "type": "number", "label": {"en": "Guests"}, "min": 1, "max": 10},
            {"key": "topic", "type": "select", "label": {"en": "Topic"}, "options": [{"value": "sales"}, {"value": "support"}]},
            {"key": "size", "type": "radio", "label": {"en": "Size"}, "options": [{"value": "s"}, {"value": "m"}]},
            {"key": "extras", "type": "checkbox", "label": {"en": "Extras"}, "options": [{"value": "a"}, {"value": "b"}]},
            {"key": "day", "type": "date", "label": {"en": "Day"}},
            {"key": "source", "type": "hidden", "label": {"en": "Source"}, "max_length": 10},
            {"key": "agree", "type": "consent", "label": {"en": "I agree", "vi": "Tôi đồng ý"}, "required": true}
        ]))
    }

    fn codes(problems: &[FieldError]) -> Vec<(String, String)> {
        let mut codes: Vec<_> =
            problems.iter().map(|p| (p.field.clone(), p.code.clone())).collect();
        codes.sort();
        codes
    }

    #[test]
    fn moi_kieu_truong_hop_le() {
        let input = json!({
            "name": "  Ada  ", "email": "Ada@Example.com", "phone": "+84 (28) 1234-5678",
            "message": "Xin chào\r\nbạn", "guests": "3", "topic": "sales", "size": "m",
            "extras": ["b", "a", "b"], "day": "2026-02-28", "source": "ads", "agree": true
        });
        let mut problems = Vec::new();
        let answers = check_answers(&sample(), input.as_object().unwrap(), &mut problems);
        assert_eq!(problems, []);
        assert_eq!(answers.data["name"], "Ada");
        assert_eq!(answers.data["message"], "Xin chào\nbạn");
        assert_eq!(answers.data["guests"], json!(3));
        assert_eq!(answers.data["extras"], json!(["b", "a"]));
        assert_eq!(answers.data["agree"], json!(true));
        assert!(answers.text.contains("Xin chào"));
        assert_eq!(
            sender_email(&sample(), None, &answers.data).map(|e| normalize_email(&e)),
            Some("ada@example.com".to_owned())
        );
    }

    #[test]
    fn moi_loi_bao_mot_lan_theo_key() {
        let input = json!({
            "name": "x".repeat(21), "email": "not-an-email", "phone": "12",
            "message": "hey", "guests": 11, "topic": "other", "size": 5,
            "extras": ["c"], "day": "2026-02-30", "source": "a\u{0}b", "unknown": "?",
            "agree": false
        });
        let mut problems = Vec::new();
        check_answers(&sample(), input.as_object().unwrap(), &mut problems);
        assert_eq!(
            codes(&problems),
            [
                ("data.agree", "consent_required"),
                ("data.day", "date"),
                ("data.email", "email"),
                ("data.extras", "option"),
                ("data.guests", "range"),
                ("data.message", "length"),
                ("data.name", "length"),
                ("data.phone", "phone"),
                ("data.size", "type"),
                ("data.source", "invalid"),
                ("data.topic", "option"),
                ("data.unknown", "unknown"),
            ]
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
        );
        // Bỏ trống trường bắt buộc.
        let mut problems = Vec::new();
        check_answers(&sample(), json!({"name": " "}).as_object().unwrap(), &mut problems);
        assert_eq!(
            codes(&problems),
            [
                ("data.agree", "consent_required"),
                ("data.email", "required"),
                ("data.name", "required")
            ]
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
        );
    }

    #[test]
    fn so_va_email() {
        let number = fields(json!([{"key": "n", "type": "number", "label": {"en": "N"}}]));
        for (raw, ok) in
            [(json!(2.5), true), (json!("-1"), true), (json!("abc"), false), (json!(true), false)]
        {
            let mut problems = Vec::new();
            check_answers(&number, json!({"n": raw}).as_object().unwrap(), &mut problems);
            assert_eq!(problems.is_empty(), ok, "{raw}");
        }
        assert!(is_email("a.b+tag@mail.example.vn"));
        for bad in [
            "a@b",
            "a b@example.com",
            "@example.com",
            "a@-x.com",
            "a@@x.com",
            "a..b@x.com",
            "a@x.com\nBcc: y",
        ] {
            assert!(!is_email(bad), "{bad}");
        }
    }

    #[test]
    fn dinh_nghia_sai_bao_du_loi() {
        let bad = fields(json!([
            {"key": "Name", "type": "text", "label": {}},
            {"key": "a", "type": "text", "label": {"en": "A"}, "options": [{"value": "x"}], "min": 1},
            {"key": "a", "type": "select", "label": {"en": "A"}},
            {"key": "c1", "type": "consent", "label": {"vi": "Đồng ý"}},
            {"key": "c2", "type": "consent", "label": {"vi": "Đồng ý"}},
            {"key": "t", "type": "textarea", "label": {"en": "T"}, "min_length": 10, "max_length": 5},
            {"key": "o", "type": "radio", "label": {"en": "O"}, "options": [{"value": "x"}, {"value": " x "}]}
        ]));
        let mut problems = Vec::new();
        check_definition(&bad, 50, &mut problems);
        assert_eq!(
            codes(&problems),
            [
                ("fields[0].key", "invalid"),
                ("fields[0].label", "required"),
                ("fields[1].min", "not_allowed"),
                ("fields[1].options", "not_allowed"),
                ("fields[2].key", "duplicate"),
                ("fields[2].options", "length"),
                ("fields[4].type", "too_many"),
                ("fields[5].min_length", "range"),
                ("fields[6].options[1].value", "duplicate"),
            ]
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
        );
        let mut problems = Vec::new();
        check_definition(&[], 50, &mut problems);
        assert_eq!(codes(&problems), [("fields".to_owned(), "length".to_owned())]);
    }

    #[test]
    fn nhan_theo_ngon_ngu_co_du_phong() {
        let text = Localized { en: None, vi: Some("Xin chào".to_owned()) };
        assert_eq!(text.get(Locale::En), "Xin chào");
        assert_eq!(display(&json!(["a", "b"])), "a; b");
        assert_eq!(display(&json!(3)), "3");
    }
}
