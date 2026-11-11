//! i18n: thông báo lỗi theo ngôn ngữ của client (D-057).
//!
//! - `code` của lỗi là i18n key (D-039). Câu tiếng Anh nằm ở chỗ tạo lỗi; các ngôn ngữ
//!   khác nằm trong `locales/<ngôn ngữ>.json` của từng crate (`{"code": "câu"}`), cùng
//!   chỗ điền `{tên}` như câu tiếng Anh.
//! - Client chọn ngôn ngữ bằng header `Accept-Language`; không nêu, hoặc nêu ngôn ngữ
//!   chưa có, thì nhận tiếng Anh. Response mang `Content-Language`.
//! - `cargo xtask check` bảo đảm mọi `code` trong code đều có bản dịch.

use std::collections::HashMap;
use std::sync::Arc;

use crate::error::Error;

/// Một ngôn ngữ được hỗ trợ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    /// Tiếng Anh: mặc định, câu nằm ngay trong code.
    En,
    /// Tiếng Việt.
    Vi,
}

impl Language {
    /// Mã ngôn ngữ theo BCP 47, như trong `Content-Language`.
    pub fn tag(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::Vi => "vi",
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        // Chỉ xét phần ngôn ngữ chính: `vi-VN` → `vi`.
        let primary = tag.split(['-', '_']).next().unwrap_or_default();
        match primary.to_ascii_lowercase().as_str() {
            "en" => Some(Language::En),
            "vi" => Some(Language::Vi),
            _ => None,
        }
    }

    /// Ngôn ngữ client muốn nhất trong số được hỗ trợ, theo header `Accept-Language`
    /// (RFC 9110, mục 12.5.4). Không có header, hoặc không ngôn ngữ nào khớp: tiếng Anh.
    pub fn negotiate(accept_language: Option<&str>) -> Self {
        let Some(header) = accept_language else {
            return Language::En;
        };
        let mut best: Option<(Language, u16)> = None;
        for entry in header.split(',') {
            let mut parts = entry.split(';');
            let tag = parts.next().unwrap_or_default().trim();
            // q từ 0 tới 1, ba chữ số thập phân: đổi ra số nguyên phần nghìn để so.
            let quality =
                parts.find_map(|part| part.trim().strip_prefix("q=")).map_or(1000, parse_quality);
            let Some(language) = Language::from_tag(tag) else {
                continue;
            };
            if quality > 0 && best.is_none_or(|(_, current)| quality > current) {
                best = Some((language, quality));
            }
        }
        best.map_or(Language::En, |(language, _)| language)
    }
}

/// `q` của `Accept-Language` thành phần nghìn: `0.8` → 800. Sai dạng thì 0 (bỏ qua).
fn parse_quality(raw: &str) -> u16 {
    let (whole, fraction) = raw.trim().split_once('.').unwrap_or((raw.trim(), ""));
    let whole: u16 = match whole {
        "0" => 0,
        "1" => 1000,
        _ => return 0,
    };
    let digits: String = fraction.chars().take(3).collect();
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return 0;
    }
    let padded = format!("{digits:0<3}");
    std::cmp::min(whole + padded.parse::<u16>().unwrap_or(0), 1000)
}

/// Mọi bản dịch của chương trình: kernel và các module.
#[derive(Debug, Clone, Default)]
pub struct Translations {
    // `Arc`: dựng một lần lúc khởi động, mọi request dùng chung.
    vi: Arc<HashMap<String, String>>,
}

impl Translations {
    /// Bản dịch của kernel, cộng của các module (`(tên module, [(ngôn ngữ, JSON)])`).
    /// File sai dạng, ngôn ngữ chưa hỗ trợ, hay module dịch `code` không phải của mình
    /// là lỗi khởi động.
    pub fn new(
        modules: Vec<(&'static str, Vec<(&'static str, &'static str)>)>,
    ) -> Result<Self, Vec<String>> {
        let mut vi = HashMap::new();
        let mut problems = Vec::new();
        let kernel = ("kernel", vec![("vi", include_str!("../locales/vi.json"))]);
        for (module, files) in std::iter::once(kernel).chain(modules) {
            for (language, json) in files {
                if Language::from_tag(language) != Some(Language::Vi) {
                    problems.push(format!(
                        "module `{module}` has translations for `{language}`, which is not \
                         supported (supported: vi)"
                    ));
                    continue;
                }
                let entries: HashMap<String, String> = match serde_json::from_str(json) {
                    Ok(entries) => entries,
                    Err(error) => {
                        problems.push(format!(
                            "module `{module}`: locales/{language}.json is not a JSON object of \
                             strings: {error}"
                        ));
                        continue;
                    }
                };
                for (code, text) in entries {
                    let owned = module == "kernel" || code.starts_with(&format!("{module}."));
                    if !owned {
                        problems.push(format!(
                            "module `{module}` translates code `{code}`, which is not its own \
                             (its codes start with `{module}.`)"
                        ));
                    }
                    vi.insert(code, text);
                }
            }
        }
        if problems.is_empty() { Ok(Translations { vi: Arc::new(vi) }) } else { Err(problems) }
    }

    /// Thông báo của `error` bằng `language`. Chưa có bản dịch, hoặc bản dịch dùng chỗ
    /// điền mà lỗi không có giá trị, thì dùng câu tiếng Anh.
    pub fn message(&self, error: &Error, language: Language) -> String {
        let template = match language {
            Language::En => None,
            Language::Vi => self.vi.get(error.code()),
        };
        let Some(template) = template else {
            return error.message();
        };
        let mut text = template.clone();
        for (name, value) in error.params() {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        if has_placeholder(&text) { error.message() } else { text }
    }

    /// Có bản dịch tiếng Việt cho `code` không.
    pub fn has(&self, code: &str) -> bool {
        self.vi.contains_key(code)
    }
}

/// Còn chỗ điền `{tên}` chưa có giá trị.
fn has_placeholder(text: &str) -> bool {
    text.split('{').skip(1).any(|rest| {
        rest.split_once('}').is_some_and(|(name, _)| {
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chon_ngon_ngu_theo_accept_language() {
        assert_eq!(Language::negotiate(None), Language::En);
        assert_eq!(Language::negotiate(Some("vi")), Language::Vi);
        assert_eq!(Language::negotiate(Some("vi-VN,vi;q=0.9,en;q=0.8")), Language::Vi);
        assert_eq!(Language::negotiate(Some("en-US,vi;q=0.5")), Language::En);
        assert_eq!(Language::negotiate(Some("fr, vi;q=0.3")), Language::Vi);
        assert_eq!(Language::negotiate(Some("fr, de")), Language::En);
        assert_eq!(Language::negotiate(Some("vi;q=0, en;q=0.1")), Language::En);
        assert_eq!(Language::negotiate(Some("*")), Language::En);
        assert_eq!(Language::negotiate(Some("vi;q=abc")), Language::En);
    }

    #[test]
    fn dich_theo_code_va_dien_gia_tri() {
        let translations = Translations::new(vec![(
            "notes",
            vec![("vi", r#"{"notes.note_not_found": "Không có ghi chú {id}."}"#)],
        )])
        .unwrap();
        let error = Error::not_found("notes.note_not_found", "Note {id} does not exist.")
            .with_param("id", 7);

        assert_eq!(translations.message(&error, Language::Vi), "Không có ghi chú 7.");
        assert_eq!(translations.message(&error, Language::En), "Note 7 does not exist.");
        // Chưa có bản dịch thì tiếng Anh.
        let other = Error::conflict("notes.title_taken", "Taken.");
        assert_eq!(translations.message(&other, Language::Vi), "Taken.");
        // Bản dịch của kernel có sẵn.
        assert!(translations.has("validation_failed"));
    }

    #[test]
    fn ban_dich_dung_cho_dien_la_thi_dung_tieng_anh() {
        let translations = Translations::new(vec![(
            "notes",
            vec![("vi", r#"{"notes.note_not_found": "Không có ghi chú {note}."}"#)],
        )])
        .unwrap();
        let error = Error::not_found("notes.note_not_found", "Note {id} does not exist.")
            .with_param("id", 7);

        assert_eq!(translations.message(&error, Language::Vi), "Note 7 does not exist.");
    }

    #[test]
    fn file_ban_dich_sai_bi_tu_choi() {
        let problems = Translations::new(vec![
            ("notes", vec![("vi", "not json")]),
            ("tags", vec![("vi", r#"{"notes.x": "Không."}"#), ("fr", "{}")]),
        ])
        .unwrap_err();

        assert_eq!(problems.len(), 3, "{problems:#?}");
    }
}
