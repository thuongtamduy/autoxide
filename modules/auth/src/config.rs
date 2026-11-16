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

pub(crate) const REAUTH_MINUTES: Setting = Setting::new(
    "AUTOXIDE_AUTH_REAUTH_MINUTES",
    "How long a confirmation token from `POST /api/v1/auth/me/reauth` stays usable (1 to 60 \
     minutes). It is used once, for one sensitive change.",
)
.with_default("10");

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

pub(crate) const EMAIL_VERIFICATION_URL: Setting = Setting::new(
    "AUTOXIDE_AUTH_EMAIL_VERIFICATION_URL",
    "Page of your app that confirms an email address, with `{token}` where the verification \
     token goes. Verification mail links there. When unset, the mail contains the token and \
     says to call `POST /api/v1/auth/email-verification/confirm`.",
)
.with_example("https://app.example.com/verify-email?token={token}");

pub(crate) const EMAIL_CHANGE_REPORT_URL: Setting = Setting::new(
    "AUTOXIDE_AUTH_EMAIL_CHANGE_REPORT_URL",
    "Page of your app where the owner of an account reports an email change they did not make, \
     with `{token}` where the report token goes. The mail sent to the old address after an \
     email change links there. When unset, the mail contains the token and says to call \
     `POST /api/v1/auth/email-change/report`.",
)
.with_example("https://app.example.com/not-me?token={token}");

pub(crate) const COUNTRY_HEADER: Setting = Setting::new(
    "AUTOXIDE_AUTH_COUNTRY_HEADER",
    "Request header in which your reverse proxy or CDN puts the caller's country (ISO 3166-1 \
     two-letter code), e.g. `CF-IPCountry` behind Cloudflare. Each sign-in session keeps that \
     code so people can spot a session they do not know; the IP address itself is never kept. \
     Set it only when a proxy you trust always sets (or removes) this header. When unset, \
     sessions have no country.",
)
.with_example("CF-IPCountry");

pub(crate) const PASSKEY_RP_ID: Setting = Setting::new(
    "AUTOXIDE_AUTH_PASSKEY_RP_ID",
    "Domain that passkeys belong to (the WebAuthn relying party id), usually your site's \
     domain without scheme or port, e.g. `example.com`; passkeys then work on that domain and \
     its subdomains. When unset, passkey sign-in is off. Changing it later makes every \
     registered passkey unusable.",
)
.with_example("example.com");

pub(crate) const PASSKEY_ORIGINS: Setting = Setting::new(
    "AUTOXIDE_AUTH_PASSKEY_ORIGINS",
    "Web origins allowed to use passkeys, comma-separated, e.g. \
     `https://app.example.com,https://example.com`. Each must be on AUTOXIDE_AUTH_PASSKEY_RP_ID \
     or a subdomain of it. Unset: `https://<AUTOXIDE_AUTH_PASSKEY_RP_ID>`.",
)
.with_example("https://app.example.com");

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
    /// Trang xác minh email của giao diện, có `{token}`.
    pub email_verification_url: Option<String>,
    /// Trang báo "không phải tôi" sau khi đổi email, có `{token}`; `None`: mail chứa token.
    pub email_change_report_url: Option<String>,
    /// Header mang quốc gia của người gọi do proxy tin cậy đặt (D-272).
    pub country_header: Option<axum::http::HeaderName>,
    /// `None`: passkey tắt.
    pub passkey: Option<PasskeyConfig>,
    /// Hạn của token xác nhận lại (D-308).
    pub reauth_ttl: Duration,
}

/// Cấu hình passkey (WebAuthn).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PasskeyConfig {
    /// RP ID: tên miền mà passkey thuộc về.
    pub rp_id: String,
    /// Origin được phép, dạng `https://host[:port]`.
    pub origins: Vec<String>,
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
        let reauth = reader.get_with(&REAUTH_MINUTES, |raw| parse_in_range(raw, 1u64, 60));
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
        let password_reset_url = reader.optional_with(&PASSWORD_RESET_URL, token_url);
        let email_verification_url = reader.optional_with(&EMAIL_VERIFICATION_URL, token_url);
        let email_change_report_url = reader.optional_with(&EMAIL_CHANGE_REPORT_URL, token_url);
        let country_header = reader.optional_with(&COUNTRY_HEADER, |raw| {
            axum::http::HeaderName::from_bytes(raw.trim().as_bytes())
                .map_err(|_| "must be an HTTP header name, e.g. `CF-IPCountry`".to_owned())
        });
        let rp_id = reader.optional_with(&PASSKEY_RP_ID, |raw| {
            let domain = raw.trim().to_ascii_lowercase();
            let valid = !domain.is_empty()
                && domain.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                });
            if valid {
                Ok(domain)
            } else {
                Err("must be a domain name like `example.com`, without scheme or port".to_owned())
            }
        });
        let origins = reader.optional_with(&PASSKEY_ORIGINS, |raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|origin| !origin.is_empty())
                .map(|origin| {
                    let origin = origin.trim_end_matches('/');
                    let host = origin
                        .strip_prefix("https://")
                        .or_else(|| origin.strip_prefix("http://localhost").map(|_| "localhost"));
                    match host {
                        Some(host) if !host.is_empty() && !host.contains('/') => {
                            Ok(origin.to_owned())
                        }
                        _ => Err(format!(
                            "`{origin}` must be an origin like `https://app.example.com` (plain \
                             http only for localhost)"
                        )),
                    }
                })
                .collect::<Result<Vec<_>, _>>()
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
            email_verification_url: email_verification_url?,
            email_change_report_url: email_change_report_url?,
            country_header: country_header?,
            passkey: passkey_config(reader, rp_id?, origins?)?,
            reauth_ttl: Duration::from_secs(reauth? * 60),
        })
    }
}

/// Ghép RP ID với origin; origin phải nằm trên RP ID hoặc tên miền con của nó (luật của
/// WebAuthn — trình duyệt cũng từ chối nếu sai, nên báo ngay lúc khởi động).
fn passkey_config(
    reader: &mut ConfigReader<'_>,
    rp_id: Option<String>,
    origins: Option<Vec<String>>,
) -> Result<Option<PasskeyConfig>, Invalid> {
    let Some(rp_id) = rp_id else {
        return Ok(None);
    };
    let origins =
        origins.filter(|list| !list.is_empty()).unwrap_or_else(|| vec![format!("https://{rp_id}")]);
    for origin in &origins {
        let host = origin.split("://").nth(1).unwrap_or_default();
        let host = host.split(':').next().unwrap_or_default();
        let belongs = host == rp_id || host.ends_with(&format!(".{rp_id}")) || host == "localhost";
        if !belongs {
            return reader.reject(
                &PASSKEY_ORIGINS,
                format!(
                    "`{origin}` is not on `{rp_id}` (AUTOXIDE_AUTH_PASSKEY_RP_ID) or a subdomain \
                     of it"
                ),
            );
        }
    }
    Ok(Some(PasskeyConfig { rp_id, origins }))
}

/// Địa chỉ http(s) có chỗ điền `{token}`.
fn token_url(raw: &str) -> Result<String, String> {
    let url = raw.trim();
    if (url.starts_with("https://") || url.starts_with("http://")) && url.contains("{token}") {
        Ok(url.to_owned())
    } else {
        Err("must be an http(s) URL containing `{token}`".to_owned())
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
        assert!(read(&[("AUTOXIDE_AUTH_EMAIL_VERIFICATION_URL", "ftp://x/{token}")]).is_err());
        assert_eq!(config.passkey, None);
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

    #[test]
    fn cau_hinh_passkey() {
        let only_rp = read(&[("AUTOXIDE_AUTH_PASSKEY_RP_ID", "Example.com")]).unwrap();
        let expected = PasskeyConfig {
            rp_id: "example.com".to_owned(),
            origins: vec!["https://example.com".to_owned()],
        };
        assert_eq!(only_rp.passkey, Some(expected));
        let both = read(&[
            ("AUTOXIDE_AUTH_PASSKEY_RP_ID", "example.com"),
            ("AUTOXIDE_AUTH_PASSKEY_ORIGINS", "https://app.example.com/, http://localhost:5173"),
        ])
        .unwrap();
        let origins = both.passkey.unwrap().origins;
        assert_eq!(origins, ["https://app.example.com", "http://localhost:5173"]);

        for (rp, origins) in [
            ("https://example.com", ""),
            ("example.com", "http://app.example.com"),
            ("example.com", "https://evil.com"),
            ("example.com", "https://notexample.com"),
        ] {
            let pairs =
                [("AUTOXIDE_AUTH_PASSKEY_RP_ID", rp), ("AUTOXIDE_AUTH_PASSKEY_ORIGINS", origins)];
            assert!(read(&pairs).is_err(), "{rp} {origins}");
        }
    }
}
