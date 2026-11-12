//! Kiểu chữ dùng chung: ngôn ngữ, chữ hai thứ tiếng, email, key, chữ một dòng.

use autoxide_kernel::error::FieldError;
use autoxide_kernel::i18n::Language;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A language: of a subscriber's mails, of a campaign's footer, of texts asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
#[schema(as = NewsletterLocale)]
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

    /// Đọc lại giá trị do chính module lưu; lạ thì tiếng Anh.
    pub(crate) fn stored(raw: &str) -> Locale {
        Locale::parse(raw).unwrap_or_default()
    }

    pub(crate) fn language(self) -> Language {
        match self {
            Locale::En => Language::En,
            Locale::Vi => Language::Vi,
        }
    }
}

/// Một tham số ngôn ngữ tuỳ chọn (`en` / `vi`); sai thì thêm lỗi `field`.
pub(crate) fn locale_param(
    raw: Option<&str>,
    field: &str,
    problems: &mut Vec<FieldError>,
) -> Option<Locale> {
    let raw = raw.map(str::trim).filter(|raw| !raw.is_empty())?;
    let parsed = Locale::parse(raw);
    if parsed.is_none() {
        problems.push(FieldError::new(field, "invalid", "must be `en` or `vi`"));
    }
    parsed
}

/// A text in English and Vietnamese; a missing language falls back to the other one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = NewsletterText)]
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
    pub(crate) fn normalized(
        &self,
        field: &str,
        max: usize,
        problems: &mut Vec<FieldError>,
    ) -> Localized {
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

/// Email chuẩn hoá: bỏ khoảng trắng hai đầu, viết thường.
pub(crate) fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// Email có dạng `phần-trước@tên.miền` hợp lý (không đầy đủ RFC 5322 — đủ để chặn chữ rác).
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
    let domain_ok = text.len() <= 320
        && labels.len() >= 2
        && labels.iter().all(|label| {
            (1..=63).contains(&label.chars().count())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_alphanumeric() || c == '-')
        });
    local_ok && domain_ok
}

/// Email đã chuẩn hoá của `raw`; sai dạng thì thêm lỗi `field` và trả `None`.
pub(crate) fn checked_email(
    raw: &str,
    field: &str,
    problems: &mut Vec<FieldError>,
) -> Option<String> {
    let email = normalize_email(raw);
    if is_email(&email) {
        Some(email)
    } else {
        problems.push(FieldError::new(
            field,
            "email",
            "must be an email address like `name@example.com`",
        ));
        None
    }
}

/// `key` của danh sách: chữ thường và số, nhóm nối bằng `-`, tối đa 50 ký tự.
pub(crate) fn is_valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 50
        && key.split('-').all(|part| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

/// Chữ một dòng, đã bỏ khoảng trắng hai đầu; rỗng thành `None`. Dài quá `max` hay có ký tự điều
/// khiển thì thêm lỗi `field`.
pub(crate) fn line(
    raw: Option<&str>,
    field: &str,
    max: usize,
    problems: &mut Vec<FieldError>,
) -> Option<String> {
    let value = raw.map(str::trim).filter(|value| !value.is_empty())?;
    if value.chars().count() > max || value.chars().any(char::is_control) {
        problems.push(FieldError::new(
            field,
            "length",
            format!("must be at most {max} characters, on one line"),
        ));
    }
    Some(value.to_owned())
}

/// Chữ nhiều dòng (văn bản đồng ý…): bỏ khoảng trắng hai đầu; rỗng thành `None`.
pub(crate) fn text(
    raw: Option<&str>,
    field: &str,
    max: usize,
    problems: &mut Vec<FieldError>,
) -> Option<String> {
    let value = raw.map(str::trim).filter(|value| !value.is_empty())?;
    if value.chars().count() > max
        || value.chars().any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
    {
        problems.push(FieldError::new(
            field,
            "length",
            format!("must be at most {max} characters, without control characters"),
        ));
    }
    Some(value.to_owned())
}

/// Thay `{token}` trong URL của website.
pub(crate) fn token_link(template: &str, token: &str) -> String {
    template.replace("{token}", token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_va_key() {
        assert!(is_email("ada@example.com"));
        for bad in
            ["", "ada", "ada@", "@example.com", "a b@example.com", "ada@example", "a..b@x.io"]
        {
            assert!(!is_email(bad), "{bad}");
        }
        assert_eq!(normalize_email("  Ada@Example.COM "), "ada@example.com");
        assert!(is_valid_key("weekly-news-2"));
        for bad in ["", "Weekly", "a--b", "-a", "a_b"] {
            assert!(!is_valid_key(bad), "{bad}");
        }
    }
}
