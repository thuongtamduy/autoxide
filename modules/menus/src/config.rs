//! Cấu hình của module `menus`, tiền tố `AUTOXIDE_MENUS_`. Mọi biến có mặc định chạy được ngay.
//! Bước 34 chuyển ngôn ngữ sang cài đặt của từng website (D-238).

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const LOCALES: Setting = Setting::new(
    "AUTOXIDE_MENUS_LOCALES",
    "Languages a website menu may be written in, comma-separated BCP 47 codes (e.g. `en,vi`).",
)
.with_default("en,vi");

pub(crate) const DEFAULT_LOCALE: Setting = Setting::new(
    "AUTOXIDE_MENUS_DEFAULT_LOCALE",
    "Language of the menu served when a menu is missing in the language asked for. Must be one \
     of AUTOXIDE_MENUS_LOCALES.",
)
.with_default("en");

pub(crate) const PUBLIC_CACHE_SECONDS: Setting = Setting::new(
    "AUTOXIDE_MENUS_PUBLIC_CACHE_SECONDS",
    "How long a public menu is kept ready in memory and may be cached by browsers, in seconds \
     (0 to 3600; 0 turns caching off). A renamed or moved target shows within this time.",
)
.with_default("60");

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MenusConfig {
    pub locales: Vec<String>,
    pub default_locale: String,
    pub public_cache_seconds: u32,
}

impl Default for MenusConfig {
    fn default() -> Self {
        MenusConfig {
            locales: vec!["en".to_owned(), "vi".to_owned()],
            default_locale: "en".to_owned(),
            public_cache_seconds: 60,
        }
    }
}

impl MenusConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let locales = reader.get_with(&LOCALES, |raw| {
            let mut locales = Vec::new();
            for item in raw.split(',').map(str::trim).filter(|item| !item.is_empty()) {
                let locale = canonical_locale(item).ok_or_else(|| {
                    format!("`{item}` is not a language code like `en` or `pt-BR`")
                })?;
                if !locales.contains(&locale) {
                    locales.push(locale);
                }
            }
            if locales.is_empty() {
                Err("must list at least one language".to_owned())
            } else {
                Ok(locales)
            }
        });
        let default_locale = reader.get_with(&DEFAULT_LOCALE, |raw| {
            canonical_locale(raw.trim()).ok_or_else(|| "is not a language code".to_owned())
        });
        let public_cache_seconds =
            reader.get_with(&PUBLIC_CACHE_SECONDS, |raw| parse_in_range(raw, 0u32, 3600));
        let (locales, default_locale) = (locales?, default_locale?);
        if !locales.contains(&default_locale) {
            return reader.reject(
                &DEFAULT_LOCALE,
                format!("must be one of AUTOXIDE_MENUS_LOCALES ({})", locales.join(", ")),
            );
        }
        Ok(MenusConfig { locales, default_locale, public_cache_seconds: public_cache_seconds? })
    }
}

/// Mã ngôn ngữ BCP 47 dạng chuẩn (`vi`, `pt-BR`, `zh-Hant-TW`); `None` nếu sai dạng. Cùng luật
/// với `content`, để menu nhập từ `content` giữ nguyên mã.
pub(crate) fn canonical_locale(raw: &str) -> Option<String> {
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
    use autoxide_kernel::config::ConfigSource;

    fn read(pairs: &[(&str, &str)]) -> Result<MenusConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = MenusConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|error| error.to_string())
    }

    #[test]
    fn mac_dinh_va_kiem() {
        assert_eq!(read(&[]).unwrap(), MenusConfig::default());
        let config = read(&[
            ("AUTOXIDE_MENUS_LOCALES", "vi, PT_br"),
            ("AUTOXIDE_MENUS_DEFAULT_LOCALE", "vi"),
            ("AUTOXIDE_MENUS_PUBLIC_CACHE_SECONDS", "0"),
        ])
        .unwrap();
        assert_eq!(config.locales, ["vi", "pt-BR"]);
        assert_eq!(config.public_cache_seconds, 0);
        assert!(read(&[("AUTOXIDE_MENUS_DEFAULT_LOCALE", "fr")]).is_err());
        assert!(read(&[("AUTOXIDE_MENUS_LOCALES", "x1")]).is_err());
        assert!(read(&[("AUTOXIDE_MENUS_PUBLIC_CACHE_SECONDS", "3601")]).is_err());
    }
}
