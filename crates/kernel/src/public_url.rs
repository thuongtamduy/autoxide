//! URL công khai của API (`AUTOXIDE_HTTP_PUBLIC_URL`, D-118): cần cho link tuyệt đối trỏ về API
//! mà người nhận mở từ nơi khác — vd header `List-Unsubscribe` của bản tin (RFC 8058 đòi URL
//! `https` tuyệt đối). Module lấy bằng `context.require::<PublicUrl>()`.

use crate::config::{ConfigReader, Invalid, Setting};
use crate::error::Error;

pub(crate) const PUBLIC_URL: Setting = Setting::new(
    "AUTOXIDE_HTTP_PUBLIC_URL",
    "Address at which the internet reaches this API, without a trailing slash, e.g. \
     `https://api.example.com`. Needed for absolute links back to the API, such as the \
     one-click unsubscribe link in newsletters; features that need it say so when it is unset.",
)
.with_example("https://api.example.com");

/// URL công khai của API, nếu đã cấu hình.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PublicUrl(Option<String>);

impl PublicUrl {
    pub fn new(url: Option<String>) -> Self {
        PublicUrl(url)
    }

    /// URL gốc (không có `/` cuối), hoặc lỗi nói rõ phải đặt biến nào.
    pub fn base(&self) -> Result<&str, Error> {
        self.0.as_deref().ok_or_else(|| {
            Error::unavailable(
                "public_url_missing",
                "This needs the public address of the API: set AUTOXIDE_HTTP_PUBLIC_URL (for \
                 example `https://api.example.com`) and restart.",
            )
        })
    }

    /// Link tuyệt đối tới `path` (bắt đầu bằng `/`, vd `/api/v1/newsletter/unsubscribe`).
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
