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
                problems.push(
                    FieldError::new(
                        format!("{field}.{locale}"),
                        "length",
                        "must be at most {max} characters long",
                    )
                    .with_param("max", max),
                );
            }
            Some(value.to_owned())
        };
        Localized { en: clean("en", &self.en), vi: clean("vi", &self.vi) }
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
        problems.push(
            FieldError::new(field, "length", "must be at most {max} characters, on one line")
                .with_param("max", max),
        );
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
        problems.push(
            FieldError::new(
                field,
                "length",
                "must be at most {max} characters, without control characters",
            )
            .with_param("max", max),
        );
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
        assert!(autoxide_kernel::email::is_valid("ada@example.com"));
        for bad in
            ["", "ada", "ada@", "@example.com", "a b@example.com", "ada@example", "a..b@x.io"]
        {
            assert!(!autoxide_kernel::email::is_valid(bad), "{bad}");
        }
        assert_eq!(autoxide_kernel::email::normalize("  Ada@Example.COM "), "ada@example.com");
        // Đếm byte như bên gửi mail: 33 chữ `é` là 66 byte.
        assert!(autoxide_kernel::email::is_valid(&format!("{}@example.com", "é".repeat(32))));
        assert!(!autoxide_kernel::email::is_valid(&format!("{}@example.com", "é".repeat(33))));
        assert!(!autoxide_kernel::email::is_valid(&format!("a@{}.com", "é".repeat(32))));
        // Kiểm sau khi chuẩn hoá: `İ` viết thường dài ra (2 → 3 byte).
        let mut problems = Vec::new();
        assert_eq!(
            autoxide_kernel::email::checked(
                &format!("{}@x.io", "İ".repeat(30)),
                "email",
                &mut problems
            ),
            None
        );
        assert_eq!(problems.len(), 1);
        assert!(is_valid_key("weekly-news-2"));
        for bad in ["", "Weekly", "a--b", "-a", "a_b"] {
            assert!(!is_valid_key(bad), "{bad}");
        }
    }
}
