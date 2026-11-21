//! Chuẩn hoá giá trị theo kiểu dữ liệu của trường và tách giá trị để đánh chỉ mục.

use super::*;
use crate::time::{Date, Timestamp};

/// Giá trị đã kiểm theo kiểu của `field`.
pub(super) fn normalize(field: &CustomFieldView, value: &Value) -> Result<Value, FieldError> {
    let text_bounds = |text: &str| -> Result<(), FieldError> {
        let length = <i64 as TryFrom<usize>>::try_from(text.chars().count()).unwrap_or(i64::MAX);
        let min = field.min.as_deref().and_then(|min| min.parse::<i64>().ok()).unwrap_or(0);
        let max = field.max.as_deref().and_then(|max| max.parse::<i64>().ok()).unwrap_or(10_000);
        if (min..=max).contains(&length) {
            Ok(())
        } else {
            Err(FieldError::new("", "invalid", "has a length out of the field's range"))
        }
    };
    let number_bounds = |number: f64| -> Result<(), FieldError> {
        let below = field
            .min
            .as_deref()
            .and_then(|min| min.parse::<f64>().ok())
            .is_some_and(|min| number < min);
        let above = field
            .max
            .as_deref()
            .and_then(|max| max.parse::<f64>().ok())
            .is_some_and(|max| number > max);
        if below || above {
            Err(FieldError::new("", "invalid", "is out of the field's range"))
        } else {
            Ok(())
        }
    };
    let text = || {
        value.as_str().map(str::trim).ok_or_else(|| FieldError::new("", "invalid", "must be text"))
    };
    match field.data_type.as_str() {
        "text" | "long_text" | "phone" => {
            let text = text()?;
            if field.data_type == "text" && text.contains('\n') {
                return Err(FieldError::new("", "invalid", "must be one line"));
            }
            text_bounds(text)?;
            Ok(Value::String(text.to_owned()))
        }
        "integer" => {
            let number = value
                .as_i64()
                .ok_or_else(|| FieldError::new("", "invalid", "must be a whole number"))?;
            #[allow(clippy::cast_precision_loss)]
            number_bounds(number as f64)?;
            Ok(Value::from(number))
        }
        "decimal" => {
            // Lưu dạng chữ cố định số chữ số sau dấu chấm: không mất chính xác như `f64`.
            let raw = match value {
                Value::String(text) => text.trim().to_owned(),
                Value::Number(number) => number.to_string(),
                _ => return Err(FieldError::new("", "invalid", "must be a number")),
            };
            let parsed: f64 =
                raw.parse().map_err(|_| FieldError::new("", "invalid", "must be a number"))?;
            number_bounds(parsed)?;
            let scale = usize::try_from(field.scale.unwrap_or(2)).unwrap_or(2);
            let decimals = raw.split_once('.').map_or(0, |(_, fraction)| fraction.len());
            if decimals > scale {
                return Err(FieldError::new(
                    "",
                    "invalid",
                    "has too many digits after the decimal point",
                ));
            }
            Ok(Value::String(format!("{parsed:.scale$}")))
        }
        "boolean" => value
            .as_bool()
            .map(Value::Bool)
            .ok_or_else(|| FieldError::new("", "invalid", "must be `true` or `false`")),
        "date" => {
            let text = text()?;
            Date::parse_from_str(text, "%Y-%m-%d")
                .map_err(|_| FieldError::new("", "invalid", "must be a date `YYYY-MM-DD`"))?;
            Ok(Value::String(text.to_owned()))
        }
        "datetime" => {
            let at = Timestamp::parse_rfc3339(text()?)
                .map_err(|_| FieldError::new("", "invalid", "must be an RFC 3339 time"))?;
            Ok(Value::String(at.to_rfc3339()))
        }
        "option" => {
            let text = text()?;
            if field.options.iter().any(|option| option == text) {
                Ok(Value::String(text.to_owned()))
            } else {
                Err(FieldError::new("", "invalid", "is not one of the field's options"))
            }
        }
        "options" => {
            let list = value
                .as_array()
                .ok_or_else(|| FieldError::new("", "invalid", "must be a list of options"))?;
            let mut chosen = Vec::new();
            for item in list {
                match item.as_str() {
                    Some(option) if field.options.iter().any(|known| known == option) => {
                        chosen.push(option.to_owned())
                    }
                    _ => {
                        return Err(FieldError::new(
                            "",
                            "invalid",
                            "holds a value that is not one of the field's options",
                        ));
                    }
                }
            }
            chosen.sort();
            chosen.dedup();
            Ok(Value::from(chosen))
        }
        "url" => {
            let text = text()?;
            if !(text.starts_with("https://") || text.starts_with("http://")) || text.len() > 2000 {
                return Err(FieldError::new("", "invalid", "must be an `http(s)://` address"));
            }
            Ok(Value::String(text.to_owned()))
        }
        "email" => {
            let text = text()?;
            if !crate::email::is_valid(text) {
                return Err(FieldError::new("", "invalid", "must be a mail address"));
            }
            Ok(Value::String(crate::email::normalize(text)))
        }
        "media" => {
            let id = text()?
                .parse::<Uuid>()
                .map_err(|_| FieldError::new("", "invalid", "must be a media item id"))?;
            Ok(Value::String(id.to_string()))
        }
        "reference" => {
            let id = text()?;
            if id.is_empty() || id.len() > 200 {
                return Err(FieldError::new("", "invalid", "must be the id of a record"));
            }
            Ok(Value::String(id.to_owned()))
        }
        _ => Err(FieldError::new("", "invalid", "has an unknown type")),
    }
}

/// Số thực đã làm tròn thành số nguyên (giá trị thập phân đã nhân theo `scale`, luôn trong khoảng
/// của `i64` với trường hợp thường gặp; vượt khoảng thì bão hoà).
#[allow(clippy::cast_possible_truncation)]
pub(super) fn whole(number: f64) -> i64 {
    number as i64
}

/// Dòng index của một giá trị: (chữ thường bỏ dấu, số nguyên). Ngày, số, boolean về `value_int`.
pub(super) fn index_values(
    field: &CustomFieldView,
    value: &Value,
) -> Vec<(Option<String>, Option<i64>)> {
    match field.data_type.as_str() {
        "integer" => vec![(None, value.as_i64())],
        "decimal" => {
            let scale = field.scale.unwrap_or(2);
            let scaled = value
                .as_str()
                .and_then(|text| text.parse::<f64>().ok())
                .map(|number| (number * 10f64.powi(scale)).round());
            vec![(None, scaled.map(whole))]
        }
        "boolean" => vec![(None, value.as_bool().map(i64::from))],
        "date" => vec![(
            None,
            value
                .as_str()
                .and_then(|text| Date::parse_from_str(text, "%Y-%m-%d").ok())
                .map(|date| date.start_of_day_utc().as_unix_seconds()),
        )],
        "datetime" => vec![(
            None,
            value
                .as_str()
                .and_then(|text| Timestamp::parse_rfc3339(text).ok())
                .map(|at| at.as_unix_seconds()),
        )],
        "options" => value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|item| (Some(crate::search::fold(item)), None))
                    .collect()
            })
            .unwrap_or_default(),
        _ => vec![(value.as_str().map(crate::search::fold), None)],
    }
}
