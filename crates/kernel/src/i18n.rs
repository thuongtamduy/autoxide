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

use crate::error::{Error, FieldError};

/// Một ngôn ngữ được hỗ trợ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
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

/// Tiền tố khoá bản dịch của câu mẫu lỗi từng ô (D-262): `field:must not be empty`.
pub const FIELD_PREFIX: &str = "field:";

/// Tiền tố khoá bản dịch của câu mô tả cho người dùng (mô tả permission, event — D-264):
/// `text:<câu tiếng Anh>`.
pub const TEXT_PREFIX: &str = "text:";

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
                    // Câu mẫu của lỗi từng ô (`field:<câu tiếng Anh>`, D-262) là của chung: hai module
                    // cùng câu thì phải cùng bản dịch.
                    if code.starts_with(FIELD_PREFIX) || code.starts_with(TEXT_PREFIX) {
                        if let Some(known) = vi.get(&code)
                            && known != &text
                        {
                            problems.push(format!(
                                "module `{module}` translates `{code}` differently from another \
                                 module: `{text}` / `{known}`"
                            ));
                        }
                        vi.insert(code, text);
                        continue;
                    }
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
        let params: Vec<(&str, &str)> = error.params().collect();
        fill_all(template, &params).unwrap_or_else(|| error.message())
    }

    /// Câu mô tả `english` (mô tả permission, event…) bằng `language`: bản dịch của khoá
    /// `text:<english>`; chưa có thì nguyên câu tiếng Anh.
    pub fn describe(&self, english: &str, language: Language) -> String {
        match language {
            Language::En => english.to_owned(),
            Language::Vi => self
                .vi
                .get(&format!("{TEXT_PREFIX}{english}"))
                .cloned()
                .unwrap_or_else(|| english.to_owned()),
        }
    }

    /// Như [`describe`](Self::describe), cho câu mẫu `template` có chỗ điền `{tên}`: dịch câu
    /// mẫu (khoá `text:<câu mẫu>`) rồi điền `params` một lượt; chưa dịch thì câu tiếng Anh đã điền.
    pub fn describe_with(
        &self,
        template: &str,
        params: &[(&str, &str)],
        language: Language,
    ) -> String {
        let english = || fill_all(template, params).unwrap_or_else(|| template.to_owned());
        match language {
            Language::En => english(),
            Language::Vi => match self.vi.get(&format!("{TEXT_PREFIX}{template}")) {
                Some(translated) => fill_all(translated, params).unwrap_or_else(english),
                None => english(),
            },
        }
    }

    /// Câu của lỗi từng ô `field` bằng `language`: bản dịch của câu mẫu (khoá `field:<câu mẫu>`),
    /// điền giá trị; chưa có bản dịch thì câu tiếng Anh.
    pub fn field_message(&self, field: &FieldError, language: Language) -> String {
        let template = match language {
            Language::En => None,
            Language::Vi => self.vi.get(&format!("{FIELD_PREFIX}{}", field.template())),
        };
        let Some(template) = template else {
            return field.message.clone();
        };
        let params: Vec<(&str, &str)> = field.params().collect();
        fill_all(template, &params).unwrap_or_else(|| field.message.clone())
    }

    /// Câu đã dịch của khoá `key` bằng `language`, nếu có (tiếng Anh: luôn `None`, câu
    /// nằm trong code).
    pub fn text(&self, key: &str, language: Language) -> Option<&str> {
        match language {
            Language::En => None,
            Language::Vi => self.vi.get(key).map(String::as_str),
        }
    }

    /// Có bản dịch tiếng Việt cho `code` không.
    pub fn has(&self, code: &str) -> bool {
        self.vi.contains_key(code)
    }
}

/// Điền mọi `{tên}` của `template` bằng `params`; `None` nếu còn chỗ điền không có giá trị.
///
/// Một lượt, từ trái sang phải: giá trị đã điền không bị đọc lại, nên chữ người dùng gõ (vd câu
/// trả lời của form chứa `{x}` hay `{id}`) không làm hỏng mẫu hay bị thay tiếp (D-150).
pub(crate) fn fill_all(template: &str, params: &[(&str, &str)]) -> Option<String> {
    let mut text = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        text.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let name = after.split_once('}').map(|(name, _)| name).filter(|name| {
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
        match name {
            Some(name) => {
                let (_, value) = params.iter().find(|(key, _)| *key == name)?;
                text.push_str(value);
                rest = &after[name.len() + 1..];
            }
            None => {
                text.push('{');
                rest = after;
            }
        }
    }
    text.push_str(rest);
    Some(text)
}

/// Mã ngôn ngữ BCP 47 dạng chuẩn: phần đầu chữ thường (2–3 chữ cái), phần vùng 2 chữ hoa hoặc 3
/// số, phần chữ viết 4 chữ cái viết hoa chữ đầu (`vi`, `pt-BR`, `zh-Hant-TW`). `None` nếu sai dạng.
/// Một luật cho mọi module (`content`, `menus`, ngôn ngữ của website ở `organizations`).
pub fn canonical_locale(raw: &str) -> Option<String> {
    let mut parts = raw.split(['-', '_']);
    let language = parts.next()?.to_ascii_lowercase();
    if !(2..=3).contains(&language.len()) || !language.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut out = vec![language];
    for part in parts {
        let part = if part.len() == 4 && part.chars().all(|c| c.is_ascii_alphabetic()) {
            let mut chars = part.chars();
            let first = chars.next()?.to_ascii_uppercase();
            format!("{first}{}", chars.as_str().to_ascii_lowercase())
        } else if part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()) {
            part.to_ascii_uppercase()
        } else if part.len() == 3 && part.chars().all(|c| c.is_ascii_digit()) {
            part.to_owned()
        } else {
            return None;
        };
        out.push(part);
    }
    if out.len() > 3 {
        return None;
    }
    Some(out.join("-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dien_mot_luot_khong_doc_lai_gia_tri() {
        let filled = fill_all(
            "Hi {name}: {answers} ({id})",
            &[("name", "Ada"), ("answers", "message: {x} {id} {"), ("id", "42")],
        );
        assert_eq!(filled.as_deref(), Some("Hi Ada: message: {x} {id} { (42)"));
        assert_eq!(fill_all("{missing}", &[]), None);
        assert_eq!(
            fill_all("JSON {\"a\": 1} and { spaced }", &[]).as_deref(),
            Some("JSON {\"a\": 1} and { spaced }")
        );
    }

    #[test]
    fn cau_loi_tung_o_dich_theo_cau_mau_va_dien_gia_tri() {
        let translations = Translations::new(vec![
            (
                "notes",
                vec![("vi", r#"{"field:must be at most {max} colours": "tối đa {max} màu"}"#)],
            ),
            // Module khác cùng câu mẫu, cùng bản dịch: được.
            (
                "tags",
                vec![("vi", r#"{"field:must be at most {max} colours": "tối đa {max} màu"}"#)],
            ),
        ])
        .unwrap();
        let field = FieldError::new("title", "length", "must be at most {max} colours")
            .with_param("max", 200);
        assert_eq!(field.message, "must be at most 200 colours");
        assert_eq!(translations.field_message(&field, Language::Vi), "tối đa 200 màu");
        assert_eq!(translations.field_message(&field, Language::En), field.message);
        // Câu chưa dịch: tiếng Anh.
        let other = FieldError::new("body", "required", "is not a colour");
        assert_eq!(translations.field_message(&other, Language::Vi), "is not a colour");

        // Hai module dịch cùng câu khác nhau: lỗi khởi động.
        let clash = Translations::new(vec![
            ("notes", vec![("vi", r#"{"field:is a colour": "là màu"}"#)]),
            ("tags", vec![("vi", r#"{"field:is a colour": "là một màu"}"#)]),
        ]);
        assert!(clash.unwrap_err()[0].contains("translates `field:is a colour` differently"));
    }

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
