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

pub(crate) const TOKEN_SECRET_PREVIOUS: Setting = Setting::new(
    "AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS",
    "Old values of AUTOXIDE_AUTH_TOKEN_SECRET, separated by commas, while you change it: access \
     tokens are signed with the new key and still accepted when signed with one of these, so \
     nobody is signed out. Remove them once AUTOXIDE_AUTH_ACCESS_TOKEN_SECONDS has passed. Do not \
     list a leaked key here.",
)
.secret();

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
    "Failed sign-ins allowed for one email from one client address within \
     AUTOXIDE_AUTH_LOGIN_LOCK_MINUTES (1 to 100); ten times as many for the email from all \
     addresses together. One more locks sign-in for that email from that address (or from \
     everywhere) for AUTOXIDE_AUTH_LOGIN_LOCK_MINUTES, even with the right password. Shared by \
     every server, since it is counted in the database.",
)
.with_default("5");

pub(crate) const LOGIN_LOCK_MINUTES: Setting = Setting::new(
    "AUTOXIDE_AUTH_LOGIN_LOCK_MINUTES",
    "How long failed sign-ins are remembered, and how long sign-in stays locked after too many \
     (1 to 1440).",
)
.with_default("15");

pub(crate) const LOGIN_LINK: Setting = Setting::new(
    "AUTOXIDE_AUTH_LOGIN_LINK",
    "Let people sign in with a link sent by email, without a password (`true` or `false`). \
     Two-step sign-in still applies.",
)
.with_default("false");

pub(crate) const OIDC: Setting = Setting::new(
    "AUTOXIDE_AUTH_OIDC",
    "Let people sign in with an outside provider — Google, Microsoft, GitHub or any OpenID \
     Connect issuer — that each tenant adds with `POST /api/v1/auth/oidc-providers` (`true` or \
     `false`). Two-step sign-in still applies. Off: providers can still be managed, but signing in \
     with them answers `auth.oidc_disabled`.",
)
.with_default("false");

pub(crate) const OIDC_ALLOW_PRIVATE_NETWORKS: Setting = Setting::new(
    "AUTOXIDE_AUTH_OIDC_ALLOW_PRIVATE_NETWORKS",
    "Let sign-in providers live on private or local network addresses, and over plain `http://` \
     (`true` or `false`). Tenant administrators choose the provider address, so leave it `false` \
     in production: otherwise they can make the server call internal services. For tests and a \
     provider on your own network only.",
)
.with_default("false");

pub(crate) const LOGIN_LINK_URL: Setting = Setting::new(
    "AUTOXIDE_AUTH_LOGIN_LINK_URL",
    "Address of the interface's page that signs in with a link, with `{token}` where the token \
     goes, e.g. `https://app.example.com/sign-in/link?token={token}`. Unset: the mail carries the \
     token for `POST /api/v1/auth/login/link/confirm`.",
);

pub(crate) const LOGIN_LINK_MINUTES: Setting = Setting::new(
    "AUTOXIDE_AUTH_LOGIN_LINK_MINUTES",
    "How long a sign-in link works (5 to 60 minutes). It works once.",
)
.with_default("15");

pub(crate) const INVITATION_URL: Setting = Setting::new(
    "AUTOXIDE_AUTH_INVITATION_URL",
    "Address of the interface's page that accepts an invitation, with `{token}` where the token \
     goes, e.g. `https://app.example.com/invitation?token={token}`. Unset: the mail carries the \
     token for `POST /api/v1/auth/invitations/accept`.",
);

pub(crate) const INVITATION_DAYS: Setting = Setting::new(
    "AUTOXIDE_AUTH_INVITATION_DAYS",
    "How long an invitation can be accepted (1 to 30 days); resending starts it again.",
)
.with_default("7");

pub(crate) const SUPPORT_ACCESS_MAX_HOURS: Setting = Setting::new(
    "AUTOXIDE_AUTH_SUPPORT_ACCESS_MAX_HOURS",
    "Longest support access to a tenant's data that can be granted to platform staff (1 to 72 \
     hours).",
)
.with_default("8");

pub(crate) const API_KEY_MAX_DAYS: Setting = Setting::new(
    "AUTOXIDE_AUTH_API_KEY_MAX_DAYS",
    "Longest lifetime of an API key (1 to 3650 days). Every key expires: a key created without \
     `expires_at` gets this lifetime.",
)
.with_default("365");

pub(crate) const STEP_UP_MINUTES: Setting = Setting::new(
    "AUTOXIDE_AUTH_STEP_UP_MINUTES",
    "How long after signing in or confirming with `POST /api/v1/auth/me/reauth` a session may do \
     sensitive actions (deleting for good, money), 1 to 120 \
     minutes. Tenants can turn this off with the setting `auth.step_up`.",
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
     create accounts. A tenant may decide for itself with the setting \
     `auth.staff_registration` at its root unit.",
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
    /// Khoá ký cũ, chỉ để kiểm token trong lúc đổi khoá (bước 39).
    pub token_secret_previous: Vec<Vec<u8>>,
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
    /// Đăng nhập bằng link qua email (D-311): bật không, trang của giao diện, hạn (phút).
    pub login_link: bool,
    pub login_link_url: Option<String>,
    pub login_link_minutes: i64,
    /// Đăng nhập qua nhà cung cấp danh tính (bước 40 P3) và cho nhà cung cấp ở mạng riêng.
    pub oidc: bool,
    pub oidc_allow_private_networks: bool,
    /// Lời mời (D-312): trang của giao diện, hạn (ngày).
    pub invitation_url: Option<String>,
    pub invitation_days: i64,
    /// Trần giờ của quyền vào hỗ trợ (D-211, D-313).
    pub support_max_hours: u32,
    /// Hạn dài nhất của API key, theo ngày (bước 54).
    pub api_key_max_days: i64,
    /// Cửa sổ xác nhận lại cho việc nhạy cảm, theo phút (bước 54 P5).
    pub step_up_minutes: i64,
}

/// Cấu hình passkey (WebAuthn).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PasskeyConfig {
    /// RP ID: tên miền mà passkey thuộc về.
    pub rp_id: String,
    /// Origin được phép, dạng `https://host[:port]`.
    pub origins: Vec<String>,
}

/// Giá trị mẫu của `.env.example` (`change-me…`): ai đọc repo cũng biết, dùng nó làm key ký token là
/// ai cũng ký được token cho mọi tài khoản. Từ chối lúc khởi động như `AUTOXIDE_SECRET_KEY`
/// (D-20261121-hardening-batch mục 7; rà soát bảo mật 2026-11).
fn is_placeholder(raw: &str) -> bool {
    raw.trim().to_ascii_lowercase().starts_with("change-me")
}

const PLACEHOLDER_MESSAGE: &str =
    "is still the example value; generate a key with `openssl rand -base64 48`";

impl AuthConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let token_secret = reader.optional_with(&TOKEN_SECRET, |raw| {
            if is_placeholder(raw) {
                Err(PLACEHOLDER_MESSAGE.to_owned())
            } else if raw.len() < MIN_SECRET_LENGTH {
                Err(format!(
                    "must be at least {MIN_SECRET_LENGTH} characters long; generate one with \
                     `openssl rand -base64 48`"
                ))
            } else {
                Ok(raw.as_bytes().to_vec())
            }
        });
        let token_secret_previous = reader.optional_with(&TOKEN_SECRET_PREVIOUS, |raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .map(|key| {
                    if is_placeholder(key) {
                        Err(PLACEHOLDER_MESSAGE.to_owned())
                    } else if key.len() < MIN_SECRET_LENGTH {
                        Err(format!(
                            "every key must be at least {MIN_SECRET_LENGTH} characters long, like \
                             AUTOXIDE_AUTH_TOKEN_SECRET"
                        ))
                    } else {
                        Ok(key.as_bytes().to_vec())
                    }
                })
                .collect::<Result<Vec<_>, _>>()
        });
        let access =
            reader.get_with(&ACCESS_TOKEN_SECONDS, |raw| parse_in_range(raw, 60u64, 86_400));
        let refresh = reader.get_with(&REFRESH_TOKEN_DAYS, |raw| parse_in_range(raw, 1u64, 365));
        let max_failures =
            reader.get_with(&LOGIN_MAX_FAILURES, |raw| parse_in_range(raw, 1u32, 100));
        let lock = reader.get_with(&LOGIN_LOCK_MINUTES, |raw| parse_in_range(raw, 1u64, 1440));
        let reauth = reader.get_with(&REAUTH_MINUTES, |raw| parse_in_range(raw, 1u64, 60));
        let login_link = reader.get_with(&LOGIN_LINK, |raw| match raw {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("must be `true` or `false`".to_owned()),
        });
        let login_link_url = reader.optional_with(&LOGIN_LINK_URL, token_url);
        let oidc = reader.get_with(&OIDC, boolean);
        let oidc_allow_private_networks = reader.get_with(&OIDC_ALLOW_PRIVATE_NETWORKS, boolean);
        let invitation_url = reader.optional_with(&INVITATION_URL, token_url);
        let support_max_hours =
            reader.get_with(&SUPPORT_ACCESS_MAX_HOURS, |raw| parse_in_range(raw, 1u32, 72));
        let step_up_minutes =
            reader.get_with(&STEP_UP_MINUTES, |raw| parse_in_range(raw, 1i64, 120));
        let api_key_max_days =
            reader.get_with(&API_KEY_MAX_DAYS, |raw| parse_in_range(raw, 1i64, 3650));
        let invitation_days =
            reader.get_with(&INVITATION_DAYS, |raw| parse_in_range(raw, 1i64, 30));
        let login_link_minutes =
            reader.get_with(&LOGIN_LINK_MINUTES, |raw| parse_in_range(raw, 5i64, 60));
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
            token_secret_previous: token_secret_previous?.unwrap_or_default(),
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
            login_link: login_link?,
            login_link_url: login_link_url?,
            login_link_minutes: login_link_minutes?,
            oidc: oidc?,
            oidc_allow_private_networks: oidc_allow_private_networks?,
            invitation_url: invitation_url?,
            invitation_days: invitation_days?,
            support_max_hours: support_max_hours?,
            api_key_max_days: api_key_max_days?,
            step_up_minutes: step_up_minutes?,
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

fn boolean(raw: &str) -> Result<bool, String> {
    match raw {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("must be `true` or `false`".to_owned()),
    }
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
    fn tu_choi_gia_tri_mau_cua_key_ky_token() {
        let sample = "change-me-to-a-long-random-value-from-openssl-rand";
        let error = read(&[("AUTOXIDE_AUTH_TOKEN_SECRET", sample)]).unwrap_err();
        assert!(error.contains("example value"), "{error}");
        let real = "x".repeat(MIN_SECRET_LENGTH);
        let error = read(&[
            ("AUTOXIDE_AUTH_TOKEN_SECRET", real.as_str()),
            ("AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS", sample),
        ])
        .unwrap_err();
        assert!(error.contains("example value"), "{error}");
        assert!(read(&[("AUTOXIDE_AUTH_TOKEN_SECRET", real.as_str())]).is_ok());
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
        assert!(!config.oidc && !config.oidc_allow_private_networks);
        assert_eq!(config.totp_issuer, "Autoxide");
        assert_eq!(config.password_reset_url, None);
        assert!(read(&[("AUTOXIDE_AUTH_PASSWORD_RESET_URL", "https://app/reset")]).is_err());
        assert!(read(&[("AUTOXIDE_AUTH_EMAIL_VERIFICATION_URL", "ftp://x/{token}")]).is_err());
        assert_eq!(config.passkey, None);
        assert!(read(&[("AUTOXIDE_AUTH_TOTP_ISSUER", "A:B")]).is_err());
        assert!(read(&[("AUTOXIDE_AUTH_SELF_REGISTRATION", "true")]).unwrap().self_registration);
    }

    #[test]
    fn khoa_cu_doc_thanh_danh_sach_va_khoa_ngan_bi_tu_choi() {
        let old_a = "old-key-old-key-old-key-old-key-a";
        let old_b = "old-key-old-key-old-key-old-key-b";
        let config =
            read(&[("AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS", &format!("{old_a}, {old_b},"))])
                .unwrap();
        assert_eq!(config.token_secret_previous, [old_a.as_bytes(), old_b.as_bytes()]);
        assert!(read(&[]).unwrap().token_secret_previous.is_empty());
        let error =
            read(&[("AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS", "short-old-s3cr3t")]).unwrap_err();
        assert!(error.contains("must be at least 32"), "{error}");
        assert!(!error.contains("s3cr3t"), "{error}");
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
