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

pub(crate) const LOGIN_MAX_FAILURES: Setting = Setting::new(
    "AUTOXIDE_AUTH_LOGIN_MAX_FAILURES",
    "Failed sign-ins allowed for one email within AUTOXIDE_AUTH_LOGIN_LOCK_MINUTES (1 to 100). \
     One more locks sign-in for that email for AUTOXIDE_AUTH_LOGIN_LOCK_MINUTES, even with the \
     right password. Shared by every server, since it is counted in the database.",
)
.with_default("5");

pub(crate) const LOGIN_LOCK_MINUTES: Setting = Setting::new(
    "AUTOXIDE_AUTH_LOGIN_LOCK_MINUTES",
    "How long failed sign-ins are remembered, and how long sign-in stays locked after too many \
     (1 to 1440).",
)
.with_default("15");

pub(crate) const SELF_REGISTRATION: Setting = Setting::new(
    "AUTOXIDE_AUTH_SELF_REGISTRATION",
    "Let anyone create an account with `POST /api/v1/auth/register` (`true` or `false`). New \
     accounts get no role. When `false`, only users with the `auth.users.manage` permission \
     create accounts.",
)
.with_default("false");

pub(crate) const TOTP_ISSUER: Setting = Setting::new(
    "AUTOXIDE_AUTH_TOTP_ISSUER",
    "Name shown next to the account in authenticator apps (Google Authenticator, 1Password…) \
     when a user turns on two-step sign-in. Usually your product name; no `:`.",
)
.with_default("Autoxide");

pub(crate) const PASSWORD_RESET_URL: Setting = Setting::new(
    "AUTOXIDE_AUTH_PASSWORD_RESET_URL",
    "Page of your app where people choose a new password, with `{token}` where the reset \
     token goes. Password-reset mail links there. When unset, the mail contains the token and \
     says to call `POST /api/v1/auth/password-reset/confirm`.",
)
.with_example("https://app.example.com/reset-password?token={token}");

/// Độ dài tối thiểu của khoá ký token: 256 bit, bằng độ dài đầu ra của SHA-256.
const MIN_SECRET_LENGTH: usize = 32;

#[derive(Debug, Clone)]
pub(crate) struct AuthConfig {
    /// `None`: chưa cấu hình, sinh khoá ngẫu nhiên lúc khởi động.
    pub token_secret: Option<Vec<u8>>,
    pub access_token_ttl: Duration,
    pub refresh_token_ttl: Duration,
    /// Số lần đăng nhập sai cho phép trong `login_lock`.
    pub login_max_failures: u32,
    /// Thời gian nhớ lần sai, và thời gian khoá đăng nhập sau quá nhiều lần sai.
    pub login_lock: Duration,
    /// Cho người lạ tự đăng ký.
    pub self_registration: bool,
    /// Tên hiện trong ứng dụng authenticator.
    pub totp_issuer: String,
    /// Trang đặt lại mật khẩu của giao diện, có `{token}`.
    pub password_reset_url: Option<String>,
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
        let max_failures =
            reader.get_with(&LOGIN_MAX_FAILURES, |raw| parse_in_range(raw, 1u32, 100));
        let lock = reader.get_with(&LOGIN_LOCK_MINUTES, |raw| parse_in_range(raw, 1u64, 1440));
        let self_registration = reader.get_with(&SELF_REGISTRATION, |raw| match raw {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("must be `true` or `false`".to_owned()),
        });
        let totp_issuer = reader.get_with(&TOTP_ISSUER, |raw| {
            let issuer = raw.trim();
            if issuer.is_empty() || issuer.contains(':') || issuer.chars().count() > 64 {
                Err("must be 1 to 64 characters, without `:`".to_owned())
            } else {
                Ok(issuer.to_owned())
            }
        });
        let password_reset_url = reader.optional_with(&PASSWORD_RESET_URL, |raw| {
            let url = raw.trim();
            if (url.starts_with("https://") || url.starts_with("http://"))
                && url.contains("{token}")
            {
                Ok(url.to_owned())
            } else {
                Err("must be an http(s) URL containing `{token}`".to_owned())
            }
        });
        Ok(AuthConfig {
            token_secret: token_secret?,
            access_token_ttl: Duration::from_secs(access?),
            refresh_token_ttl: Duration::from_secs(refresh? * 24 * 60 * 60),
            login_max_failures: max_failures?,
            login_lock: Duration::from_secs(lock? * 60),
            self_registration: self_registration?,
            totp_issuer: totp_issuer?,
            password_reset_url: password_reset_url?,
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
        assert_eq!(config.login_max_failures, 5);
        assert_eq!(config.login_lock, Duration::from_secs(15 * 60));
        assert!(!config.self_registration);
        assert_eq!(config.totp_issuer, "Autoxide");
        assert_eq!(config.password_reset_url, None);
        assert!(read(&[("AUTOXIDE_AUTH_PASSWORD_RESET_URL", "https://app/reset")]).is_err());
        assert!(read(&[("AUTOXIDE_AUTH_TOTP_ISSUER", "A:B")]).is_err());
        assert!(read(&[("AUTOXIDE_AUTH_SELF_REGISTRATION", "true")]).unwrap().self_registration);
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
