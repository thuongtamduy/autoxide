//! Cấu hình của module `content`, tiền tố `AUTOXIDE_CONTENT_`. Mọi biến có mặc định chạy
//! được ngay, trừ secret ký link xem trước (không có thì tính năng xem trước tắt).

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

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
            if crate::slug::is_valid(prefix) {
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
        })
    }
}

/// Mã ngôn ngữ BCP 47 dạng chuẩn: phần đầu chữ thường (2–3 chữ cái), phần vùng 2 chữ hoa
/// hoặc 3 số, phần chữ viết 4 chữ cái viết hoa chữ đầu (`zh-Hant-TW`). `None` nếu sai dạng.
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
