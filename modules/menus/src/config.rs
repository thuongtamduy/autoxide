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

/// Ngôn ngữ menu theo khách (mục 9 của đợt cấu hình theo khách), như `content.locales`.
pub(crate) const TENANT_LOCALES: autoxide_kernel::tenancy::TenantSetting =
    autoxide_kernel::tenancy::TenantSetting::new(
        "menus.locales",
        "Languages this tenant's website menus may be written in, comma-separated BCP 47 codes \
         (e.g. `vi,en`), instead of the server's AUTOXIDE_MENUS_LOCALES. Menus already written in \
         a language left out stay as they are.",
    )
    .checked(|raw| tenant_locales(raw).map(|_| ()));

/// Ngôn ngữ mặc định của menu theo khách.
pub(crate) const TENANT_DEFAULT_LOCALE: autoxide_kernel::tenancy::TenantSetting =
    autoxide_kernel::tenancy::TenantSetting::new(
        "menus.default_locale",
        "Language of a menu when none is given, and of the menu served when one is missing in the \
         language asked for, instead of the server's AUTOXIDE_MENUS_DEFAULT_LOCALE. When it is not \
         one of the tenant's languages, the server's default is used if it is one of them, otherwise \
         the first of them; `GET /api/v1/kernel/client-config` shows the language in use.",
    )
    .checked(|raw| {
        canonical_locale(raw.trim()).map(|_| ()).ok_or_else(|| {
            autoxide_kernel::error::FieldError::new(
                "value",
                "invalid",
                "must be a language code like `en` or `pt-BR`",
            )
        })
    });

/// Danh sách ngôn ngữ (`menus.locales`): ít nhất một mã, bỏ trùng, giữ thứ tự.
pub(crate) fn tenant_locales(raw: &str) -> Result<Vec<String>, autoxide_kernel::error::FieldError> {
    let mut locales = Vec::new();
    for item in raw.split(',').map(str::trim).filter(|item| !item.is_empty()) {
        let Some(locale) = canonical_locale(item) else {
            return Err(autoxide_kernel::error::FieldError::new(
                "value",
                "invalid",
                "must be comma-separated language codes like `en` or `pt-BR`",
            ));
        };
        if !locales.contains(&locale) {
            locales.push(locale);
        }
    }
    if locales.is_empty() {
        return Err(autoxide_kernel::error::FieldError::new(
            "value",
            "invalid",
            "must be comma-separated language codes like `en` or `pt-BR`",
        ));
    }
    Ok(locales)
}

/// Mã ngôn ngữ BCP 47 dạng chuẩn (`vi`, `pt-BR`, `zh-Hant-TW`); `None` nếu sai dạng. Cùng luật
/// với `content`, để menu nhập từ `content` giữ nguyên mã.
pub(crate) fn canonical_locale(raw: &str) -> Option<String> {
    autoxide_kernel::i18n::canonical_locale(raw)
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
