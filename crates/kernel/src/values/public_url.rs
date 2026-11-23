//! The API's public URL (`AUTOXIDE_HTTP_PUBLIC_URL`, D-124): needed for absolute links that point back to the
//! API and that the recipient opens from elsewhere — e.g. the newsletter's `List-Unsubscribe` header (RFC
//! 8058 requires an absolute `https` URL). Modules obtain it with `context.require::<PublicUrl>()`.

use crate::config::{ConfigReader, Invalid, Setting};
use crate::error::Error;

pub(crate) const PUBLIC_URL: Setting = Setting::new(
    "AUTOXIDE_HTTP_PUBLIC_URL",
    "Address at which the internet reaches this API, without a trailing slash, e.g. \
     `https://api.example.com`. Needed for absolute links back to the API, such as the \
     one-click unsubscribe link in newsletters; features that need it say so when it is unset.",
)
.with_example("https://api.example.com");

/// The API's public URL, if configured.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PublicUrl(Option<String>);

impl PublicUrl {
    pub fn new(url: Option<String>) -> Self {
        PublicUrl(url)
    }

    /// The base URL (no trailing `/`), or an error that says which variable to set.
    pub fn base(&self) -> Result<&str, Error> {
        self.0.as_deref().ok_or_else(|| {
            Error::unavailable(
                "public_url_missing",
                "This needs the public address of the API: set AUTOXIDE_HTTP_PUBLIC_URL (for \
                 example `https://api.example.com`) and restart.",
            )
        })
    }

    /// Absolute link to `path` (starting with `/`, e.g. `/api/v1/newsletter/unsubscribe`).
    pub fn link(&self, path: &str) -> Result<String, Error> {
        Ok(format!("{}{path}", self.base()?))
    }
}

pub(crate) fn read_setting(reader: &mut ConfigReader<'_>) -> Result<PublicUrl, Invalid> {
    let url = reader.optional_with(&PUBLIC_URL, |raw| {
        let url = raw.trim().trim_end_matches('/');
        let host = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
        match host {
            Some(rest) if !rest.is_empty() && !rest.contains(['?', '#', ' ']) && url.is_ascii() => {
                Ok(url.to_owned())
            }
            _ => Err(
                "must be an absolute `https://` (or `http://`) address without query or fragment, \
                      like `https://api.example.com`"
                    .to_owned(),
            ),
        }
    })?;
    Ok(PublicUrl(url))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigSource;

    fn read(value: Option<&str>) -> Result<PublicUrl, String> {
        let pairs: Vec<(&str, &str)> =
            value.map(|value| ("AUTOXIDE_HTTP_PUBLIC_URL", value)).into_iter().collect();
        let source = ConfigSource::from_pairs(pairs);
        let mut reader = ConfigReader::new(&source);
        let url = read_setting(&mut reader);
        reader.finish(url).map(|(url, _)| url).map_err(|errors| errors.to_string())
    }

    #[test]
    fn doc_kiem_va_ghep_link() {
        let url = read(Some("https://api.example.com/")).unwrap();
        assert_eq!(url.link("/api/v1/x").unwrap(), "https://api.example.com/api/v1/x");
        let unset = read(None).unwrap();
        assert_eq!(unset.link("/x").unwrap_err().code(), "public_url_missing");
        let bad = read(Some("api.example.com")).unwrap_err();
        assert!(bad.contains("AUTOXIDE_HTTP_PUBLIC_URL: must be an absolute"), "{bad}");
        assert!(read(Some("https://x.example?a=1")).is_err());
    }
}
