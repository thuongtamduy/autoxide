//! i18n: error messages in the client's language (D-057).
//!
//! - The error `code` is the i18n key (D-039). The English sentence lives where the error is created; other
//!   languages live in each crate's `locales/<language>.json` (`{"code": "sentence"}`), with the same
//!   `{name}` placeholders as the English sentence.
//! - The client picks a language with the `Accept-Language` header; if none is given, or the language is
//!   not available, it gets English. The response carries `Content-Language`.
//! - `cargo xtask check` guarantees every `code` in the code has a translation.

use std::collections::HashMap;
use std::sync::Arc;

use crate::error::{Error, FieldError};

/// A supported language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Language {
    /// English: the default, the sentence is right in the code.
    En,
    /// Vietnamese.
    Vi,
}

impl Language {
    /// The language tag per BCP 47, as in `Content-Language`.
    pub fn tag(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::Vi => "vi",
        }
    }

    /// The language with a translation matching the tag `tag` (only the primary part is considered:
    /// `vi-VN` → `vi`).
    pub fn from_tag(tag: &str) -> Option<Self> {
        // Chỉ xét phần ngôn ngữ chính: `vi-VN` → `vi`.
        let primary = tag.split(['-', '_']).next().unwrap_or_default();
        match primary.to_ascii_lowercase().as_str() {
            "en" => Some(Language::En),
            "vi" => Some(Language::Vi),
            _ => None,
        }
    }

    /// The language the client prefers most among the supported ones, per the `Accept-Language` header (RFC
    /// 9110, section 12.5.4). No header, or no language matches: English.
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

/// The `q` of `Accept-Language` in thousandths: `0.8` → 800. A malformed value gives 0 (ignored).
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

/// The translation-key prefix for the template sentence of a per-field error (D-262):
/// `field:must not be empty`.
pub const FIELD_PREFIX: &str = "field:";

/// The translation-key prefix for a user-facing description (permission and event descriptions - D-264):
/// `text:<English sentence>`.
pub const TEXT_PREFIX: &str = "text:";

/// All translations of the program: the kernel and the modules.
#[derive(Debug, Clone, Default)]
pub struct Translations {
    // `Arc`: dựng một lần lúc khởi động, mọi request dùng chung.
    vi: Arc<HashMap<String, String>>,
}

impl Translations {
    /// The kernel's translations, plus those of the modules (`(module name, [(language, JSON)])`). A
    /// malformed file, an unsupported language, or a module translating a `code` that is not its own is a
    /// startup error.
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

    /// The message of `error` in `language`. If there is no translation, or the translation uses a
    /// placeholder that the error has no value for, the English sentence is used.
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

    /// The description `english` (a permission or event description...) in `language`: the translation
    /// under key `text:<english>`; if none, the English sentence unchanged.
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

    /// Like [`describe`](Self::describe), for a template sentence `template` with `{name}` placeholders:
    /// translates the template (key `text:<template>`) then fills `params` in one pass; if untranslated,
    /// the filled English sentence.
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

    /// The sentence of a per-field error `field` in `language`: the translation of the template (key
    /// `field:<template>`), with values filled in; if there is no translation, the English sentence.
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

    /// The translated sentence for key `key` in `language`, if any (English: always `None`, the sentence is
    /// in the code).
    pub fn text(&self, key: &str, language: Language) -> Option<&str> {
        match language {
            Language::En => None,
            Language::Vi => self.vi.get(key).map(String::as_str),
        }
    }

    /// Whether there is a Vietnamese translation for `code`.
    pub fn has(&self, code: &str) -> bool {
        self.vi.contains_key(code)
    }
}

/// Fills every `{name}` of `template` with `params`; `None` if any placeholder has no value.
///
/// One pass, left to right: an already-filled value is not read again, so text typed by a user (e.g. a
/// form's reply containing `{x}` or `{id}`) cannot break the template or be substituted again (D-150).
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

/// A BCP 47 language tag in canonical form: a lowercase primary part (2-3 letters), a region part of 2
/// uppercase letters or 3 digits, a script part of 4 letters with the first capitalized (`vi`, `pt-BR`,
/// `zh-Hant-TW`). `None` if malformed. One rule for every module (`content`, `menus`, the website language
/// in `organizations`).
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
