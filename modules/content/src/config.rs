//! Cấu hình của module `content`, tiền tố `AUTOXIDE_CONTENT_`. Mọi biến có mặc định chạy
//! được ngay, trừ secret ký link xem trước (không có thì tính năng xem trước tắt).

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};
use autoxide_kernel::error::FieldError;
use autoxide_kernel::tenancy::TenantSetting;

pub(crate) const LOCALES: Setting = Setting::new(
    "AUTOXIDE_CONTENT_LOCALES",
    "Languages content may be written in, comma-separated BCP 47 codes (e.g. `en,vi,pt-BR`).",
)
.with_default("en,vi");

pub(crate) const DEFAULT_LOCALE: Setting = Setting::new(
    "AUTOXIDE_CONTENT_DEFAULT_LOCALE",
    "Language used when a request names none, and for category, tag and menu names missing in \
     a language. Must be one of AUTOXIDE_CONTENT_LOCALES.",
)
.with_default("en");

pub(crate) const POST_PATH_PREFIX: Setting = Setting::new(
    "AUTOXIDE_CONTENT_POST_PATH_PREFIX",
    "First segment of the address of every post: `blog` gives `/blog/<slug>`. Lower-case \
     letters, digits and `-`.",
)
.with_default("blog");

pub(crate) const MAX_REVISIONS: Setting = Setting::new(
    "AUTOXIDE_CONTENT_MAX_REVISIONS",
    "Revisions kept per post or page (10 to 1000); older ones are removed by a daily job, \
     except those still used by the published version or a pending schedule.",
)
.with_default("50");

pub(crate) const MAX_BODY_BYTES: Setting = Setting::new(
    "AUTOXIDE_CONTENT_MAX_BODY_BYTES",
    "Largest body of a post or page, in bytes of JSON (1024 to 4194304).",
)
.with_default("524288");

pub(crate) const WORDS_PER_MINUTE: Setting = Setting::new(
    "AUTOXIDE_CONTENT_WORDS_PER_MINUTE",
    "Reading speed used for `reading_time_minutes` (50 to 1000). The default, 238, is measured \
     for English; there is no published figure for Vietnamese.",
)
.with_default("238");

pub(crate) const EMBED_HOSTS: Setting = Setting::new(
    "AUTOXIDE_CONTENT_EMBED_HOSTS",
    "Hosts whose addresses may be embedded in a body (`embed` blocks), comma-separated. A host \
     also allows its subdomains.",
)
.with_default("youtube.com,youtu.be,youtube-nocookie.com,vimeo.com");

pub(crate) const PREVIEW_SECRET: Setting = Setting::new(
    "AUTOXIDE_CONTENT_PREVIEW_SECRET",
    "Key that signs draft preview links, at least 32 characters. Unset: previews are off.",
)
.secret()
.with_example("a-long-random-string-of-at-least-32-characters");

pub(crate) const PREVIEW_TTL_SECONDS: Setting = Setting::new(
    "AUTOXIDE_CONTENT_PREVIEW_TTL_SECONDS",
    "How long a draft preview link works, in seconds (60 to 604800).",
)
.with_default("600");

pub(crate) const TRANSLATE_BATCH_TOKENS: Setting = Setting::new(
    "AUTOXIDE_CONTENT_TRANSLATE_BATCH_TOKENS",
    "AI translation sends a long entry in parts of about this many tokens each (500 to \
     100000); smaller parts suit smaller models.",
)
.with_default("8000");

pub(crate) const PUBLIC_SEMANTIC_SEARCH: Setting = Setting::new(
    "AUTOXIDE_CONTENT_PUBLIC_SEMANTIC_SEARCH",
    "Serve search by meaning to anonymous readers at `/api/v1/content/public/search` (`true` \
     or `false`). Every search costs one embeddings request, so it is off by default.",
)
.with_default("false");

pub(crate) const THEME_VARIANTS_MAX: Setting = Setting::new(
    "AUTOXIDE_CONTENT_THEME_VARIANTS_MAX",
    "Most scheduled theme variants (seasons, holidays, hours) one website may have (1 to 500).",
)
.with_default("50");

pub(crate) const PUBLIC_SEARCH_PER_MINUTE: Setting = Setting::new(
    "AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_MINUTE",
    "Public searches by meaning per minute (1 to 10000), one budget shared by every reader and \
     counted per server process; past it, public searches match words only until the next \
     minute. It bounds the embeddings cost; the general rate limit per address still applies.",
)
.with_default("30");

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContentConfig {
    pub locales: Vec<String>,
    pub default_locale: String,
    pub post_path_prefix: String,
    pub max_revisions: u32,
    pub max_body_bytes: usize,
    pub words_per_minute: u32,
    pub embed_hosts: Vec<String>,
    pub preview_secret: Option<String>,
    pub preview_ttl_seconds: u32,
    pub translate_batch_tokens: u32,
    pub public_semantic_search: bool,
    pub public_search_per_minute: u32,
    pub theme_variants_max: u32,
}

impl ContentConfig {
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
        let post_path_prefix = reader.get_with(&POST_PATH_PREFIX, |raw| {
            let prefix = raw.trim().trim_matches('/');
            if autoxide_kernel::slug::is_valid(prefix) {
                Ok(prefix.to_owned())
            } else {
                Err("must be lower-case letters, digits and `-`, like `blog`".to_owned())
            }
        });
        let max_revisions = reader.get_with(&MAX_REVISIONS, |raw| parse_in_range(raw, 10u32, 1000));
        let max_body_bytes =
            reader.get_with(&MAX_BODY_BYTES, |raw| parse_in_range(raw, 1024usize, 4 * 1024 * 1024));
        let words_per_minute =
            reader.get_with(&WORDS_PER_MINUTE, |raw| parse_in_range(raw, 50u32, 1000));
        let embed_hosts = reader.get_with(&EMBED_HOSTS, |raw| {
            Ok(raw
                .split(',')
                .map(|host| host.trim().trim_start_matches("www.").to_ascii_lowercase())
                .filter(|host| !host.is_empty())
                .collect::<Vec<_>>())
        });
        let preview_secret = reader.optional_with(&PREVIEW_SECRET, |raw| {
            if raw.chars().count() >= 32 {
                Ok(raw.to_owned())
            } else {
                Err("must be at least 32 characters".to_owned())
            }
        });
        let preview_ttl_seconds =
            reader.get_with(&PREVIEW_TTL_SECONDS, |raw| parse_in_range(raw, 60u32, 604_800));
        let translate_batch_tokens =
            reader.get_with(&TRANSLATE_BATCH_TOKENS, |raw| parse_in_range(raw, 500u32, 100_000));
        let public_semantic_search =
            reader.get_with(&PUBLIC_SEMANTIC_SEARCH, |raw| match raw.trim() {
                "true" => Ok(true),
                "false" => Ok(false),
                _ => Err("must be `true` or `false`".to_owned()),
            });
        let public_search_per_minute =
            reader.get_with(&PUBLIC_SEARCH_PER_MINUTE, |raw| parse_in_range(raw, 1u32, 10_000));
        let theme_variants_max =
            reader.get_with(&THEME_VARIANTS_MAX, |raw| parse_in_range(raw, 1u32, 500));
        let (locales, default_locale) = (locales?, default_locale?);
        if !locales.contains(&default_locale) {
            return reader.reject(
                &DEFAULT_LOCALE,
                format!("must be one of AUTOXIDE_CONTENT_LOCALES ({})", locales.join(", ")),
            );
        }
        Ok(ContentConfig {
            locales,
            default_locale,
            post_path_prefix: post_path_prefix?,
            max_revisions: max_revisions?,
            max_body_bytes: max_body_bytes?,
            words_per_minute: words_per_minute?,
            embed_hosts: embed_hosts?,
            preview_secret: preview_secret?,
            preview_ttl_seconds: preview_ttl_seconds?,
            translate_batch_tokens: translate_batch_tokens?,
            public_semantic_search: public_semantic_search?,
            public_search_per_minute: public_search_per_minute?,
            theme_variants_max: theme_variants_max?,
        })
    }
}

// --- Ngôn ngữ theo khách (đối chiếu thiết kế 2, A-b7) ----------------------------------------

pub(crate) const TENANT_LOCALES: TenantSetting = TenantSetting::new(
    "content.locales",
    "Languages this tenant's content may be written in, comma-separated BCP 47 codes (e.g. \
     `en,vi`), instead of the server's AUTOXIDE_CONTENT_LOCALES. Entries already written in a \
     language left out stay as they are.",
)
.checked(|raw| tenant_locales(raw).map(|_| ()));

pub(crate) const TENANT_DEFAULT_LOCALE: TenantSetting = TenantSetting::new(
    "content.default_locale",
    "Language used when a request names none, instead of the server's \
     AUTOXIDE_CONTENT_DEFAULT_LOCALE. When it is not one of the tenant's languages, the server's \
     default is used if it is one of them, otherwise the first of them.",
)
.checked(|raw| tenant_default_locale(raw).map(|_| ()));

/// Danh sách ngôn ngữ (`content.locales`): ít nhất một mã, bỏ trùng, giữ thứ tự.
pub(crate) fn tenant_locales(raw: &str) -> Result<Vec<String>, FieldError> {
    let mut locales = Vec::new();
    for item in raw.split(',').map(str::trim).filter(|item| !item.is_empty()) {
        let Some(locale) = canonical_locale(item) else {
            return Err(FieldError::new(
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
        return Err(FieldError::new(
            "value",
            "invalid",
            "must be comma-separated language codes like `en` or `pt-BR`",
        ));
    }
    Ok(locales)
}

/// Ngôn ngữ mặc định (`content.default_locale`).
pub(crate) fn tenant_default_locale(raw: &str) -> Result<String, FieldError> {
    canonical_locale(raw.trim()).ok_or_else(|| {
        FieldError::new("value", "invalid", "must be a language code like `en` or `pt-BR`")
    })
}

/// Ngôn ngữ đang áp cho một khách.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Languages {
    /// Ngôn ngữ được viết, theo thứ tự đã khai.
    pub locales: Vec<String>,
    /// Luôn nằm trong `locales`.
    pub default: String,
}

impl Languages {
    /// Ghép cấu hình của khách (`None`: chưa đặt) với của máy chủ. Ngôn ngữ mặc định của khách chỉ
    /// dùng khi nằm trong danh sách; không thì mặc định của máy chủ nếu nằm trong danh sách; không
    /// nữa thì ngôn ngữ đầu tiên.
    pub(crate) fn resolve(
        config: &ContentConfig,
        locales: Option<Vec<String>>,
        default: Option<String>,
    ) -> Self {
        let locales = locales.unwrap_or_else(|| config.locales.clone());
        let default = [default, Some(config.default_locale.clone())]
            .into_iter()
            .flatten()
            .find(|locale| locales.contains(locale))
            .or_else(|| locales.first().cloned())
            .unwrap_or_else(|| config.default_locale.clone());
        Languages { locales, default }
    }
}

/// Mã ngôn ngữ BCP 47 dạng chuẩn: phần đầu chữ thường (2–3 chữ cái), phần vùng 2 chữ hoa
/// hoặc 3 số, phần chữ viết 4 chữ cái viết hoa chữ đầu (`zh-Hant-TW`). `None` nếu sai dạng.
pub(crate) fn canonical_locale(raw: &str) -> Option<String> {
    autoxide_kernel::i18n::canonical_locale(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::ConfigSource;

    #[test]
    fn ngon_ngu_theo_khach_ghep_voi_may_chu() {
        let config = read(&[]).unwrap();
        let both = |locales: Option<&str>, default: Option<&str>| {
            let locales = locales.map(|raw| tenant_locales(raw).unwrap());
            let default = default.map(|raw| tenant_default_locale(raw).unwrap());
            let languages = Languages::resolve(&config, locales, default);
            (languages.locales.join(","), languages.default)
        };
        assert_eq!(both(None, None), ("en,vi".to_owned(), "en".to_owned()));
        assert_eq!(both(Some("vi, FR,vi"), None), ("vi,fr".to_owned(), "vi".to_owned()));
        assert_eq!(both(Some("vi,en"), Some("vi")), ("vi,en".to_owned(), "vi".to_owned()));
        assert_eq!(both(None, Some("vi")), ("en,vi".to_owned(), "vi".to_owned()));
        // Mặc định của khách không nằm trong danh sách: mặc định của máy chủ, rồi ngôn ngữ đầu.
        assert_eq!(both(None, Some("fr")), ("en,vi".to_owned(), "en".to_owned()));
        assert_eq!(both(Some("fr,de"), Some("ja")), ("fr,de".to_owned(), "fr".to_owned()));
        for bad in ["", " , ", "english", "en,xx-yyyyyy"] {
            assert_eq!(tenant_locales(bad).unwrap_err().field, "value", "{bad}");
        }
        assert!(tenant_default_locale("english").is_err());
    }

    fn read(pairs: &[(&str, &str)]) -> Result<ContentConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = ContentConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mac_dinh_chay_duoc() {
        let config = read(&[]).unwrap();
        assert_eq!(config.locales, ["en", "vi"]);
        assert_eq!(config.default_locale, "en");
        assert_eq!(config.post_path_prefix, "blog");
        assert_eq!((config.max_revisions, config.words_per_minute), (50, 238));
        assert_eq!(config.preview_secret, None);
        assert!(config.embed_hosts.contains(&"youtube.com".to_owned()));
    }

    #[test]
    fn ma_ngon_ngu_va_gia_tri_sai() {
        assert_eq!(canonical_locale("pt_br").as_deref(), Some("pt-BR"));
        assert_eq!(canonical_locale("zh-hant-tw").as_deref(), Some("zh-Hant-TW"));
        assert_eq!(canonical_locale("english"), None);
        let config = read(&[
            ("AUTOXIDE_CONTENT_LOCALES", "vi, EN"),
            ("AUTOXIDE_CONTENT_DEFAULT_LOCALE", "vi"),
        ])
        .unwrap();
        assert_eq!(config.locales, ["vi", "en"]);
        let error = read(&[("AUTOXIDE_CONTENT_DEFAULT_LOCALE", "fr")]).unwrap_err();
        assert!(error.contains("must be one of"), "{error}");
        let error = read(&[
            ("AUTOXIDE_CONTENT_POST_PATH_PREFIX", "My Blog"),
            ("AUTOXIDE_CONTENT_PREVIEW_SECRET", "short"),
        ])
        .unwrap_err();
        assert!(error.contains("POST_PATH_PREFIX") && error.contains("PREVIEW_SECRET"), "{error}");
    }
}
