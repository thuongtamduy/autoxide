//! Cấu hình của module `auth`. Mọi biến mang tiền tố `AUTOXIDE_AUTH_`.

use std::time::Duration;

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const TOKEN_SECRET: Setting = Setting::new(
    "AUTOXIDE_AUTH_TOKEN_SECRET",
    "Secret key that signs access tokens: at least 32 characters, random. Generate one with \
     `openssl rand -base64 48`. Every server of one deployment must use the same value. When \
     unset, a random key is generated at startup: access tokens then stop working after a \
     restart (clients refresh them), and several servers cannot share them.",
)
.secret()
.with_example("change-me-to-a-long-random-value-from-openssl-rand");

pub(crate) const ACCESS_TOKEN_SECONDS: Setting = Setting::new(
    "AUTOXIDE_AUTH_ACCESS_TOKEN_SECONDS",
    "How long an access token stays valid, in seconds (60 to 86400). Short is safer: a stolen \
     token works only until it expires.",
)
.with_default("900");

pub(crate) const REFRESH_TOKEN_DAYS: Setting = Setting::new(
    "AUTOXIDE_AUTH_REFRESH_TOKEN_DAYS",
    "How long a refresh token stays valid, in days (1 to 365). Each refresh issues a new one \
     with a fresh lifetime; a session left unused this long must sign in again.",
)
.with_default("30");

/// Độ dài tối thiểu của khoá ký token: 256 bit, bằng độ dài đầu ra của SHA-256.
const MIN_SECRET_LENGTH: usize = 32;

#[derive(Debug, Clone)]
pub(crate) struct AuthConfig {
    /// `None`: chưa cấu hình, sinh khoá ngẫu nhiên lúc khởi động.
    pub token_secret: Option<Vec<u8>>,
    pub access_token_ttl: Duration,
    pub refresh_token_ttl: Duration,
}

impl AuthConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let token_secret = reader.optional_with(&TOKEN_SECRET, |raw| {
            if raw.len() < MIN_SECRET_LENGTH {
                Err(format!(
                    "must be at least {MIN_SECRET_LENGTH} characters long; generate one with \
                     `openssl rand -base64 48`"
                ))
            } else {
                Ok(raw.as_bytes().to_vec())
            }
        });
        let access =
            reader.get_with(&ACCESS_TOKEN_SECONDS, |raw| parse_in_range(raw, 60u64, 86_400));
        let refresh = reader.get_with(&REFRESH_TOKEN_DAYS, |raw| parse_in_range(raw, 1u64, 365));
        Ok(AuthConfig {
            token_secret: token_secret?,
            access_token_ttl: Duration::from_secs(access?),
            refresh_token_ttl: Duration::from_secs(refresh? * 24 * 60 * 60),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::ConfigSource;

    fn read(pairs: &[(&str, &str)]) -> Result<AuthConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = AuthConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mac_dinh_hop_ly() {
        let config = read(&[]).unwrap();

        assert_eq!(config.token_secret, None);
        assert_eq!(config.access_token_ttl, Duration::from_secs(15 * 60));
        assert_eq!(config.refresh_token_ttl, Duration::from_secs(30 * 24 * 60 * 60));
    }

    #[test]
    fn khoa_ngan_bi_tu_choi_va_khong_bi_in_ra() {
        let error = read(&[("AUTOXIDE_AUTH_TOKEN_SECRET", "too-short-s3cr3t")]).unwrap_err();

        assert!(error.contains("AUTOXIDE_AUTH_TOKEN_SECRET: must be at least 32"), "{error}");
        assert!(!error.contains("s3cr3t"), "{error}");
    }

    #[test]
    fn thoi_han_ngoai_khoang_bi_tu_choi() {
        let error = read(&[
            ("AUTOXIDE_AUTH_ACCESS_TOKEN_SECONDS", "5"),
            ("AUTOXIDE_AUTH_REFRESH_TOKEN_DAYS", "0"),
        ])
        .unwrap_err();

        assert!(error.contains("(2 problems)"), "{error}");
    }
}
