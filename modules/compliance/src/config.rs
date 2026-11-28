//! Cấu hình của module `compliance`.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const KYC_CALLBACK_RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_KYC_CALLBACK_RETENTION_DAYS",
    "Days the raw callbacks of identity providers are kept before they are deleted, 1 to 3650.",
)
.with_default("90");

pub(crate) const OTP_TTL_SECONDS: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_OTP_TTL_SECONDS",
    "Seconds a one-time code sent by SMS (sign-in or phone verification) stays valid, 60 to 600 \
     (NIST SP 800-63B-4: at most 10 minutes).",
)
.with_default("300");

pub(crate) const OTP_MAX_ATTEMPTS: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_OTP_MAX_ATTEMPTS",
    "Wrong entries allowed for one phone verification code before a new code is needed, 1 to 20.",
)
.with_default("5");

pub(crate) const OTP_WRONG_PER_PHONE_PER_DAY: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_OTP_WRONG_PER_PHONE_PER_DAY",
    "Wrong sign-in codes allowed for one phone number over 24 hours from one client (one IPv4 \
     address, or one IPv6 /64 block), across all codes asked from that client and since the last \
     right code; after that no code is sent to or checked for the number from that client until the \
     oldest wrong entry is 24 hours old, 1 to 100.",
)
.with_default("10");

pub(crate) const OTP_WRONG_PER_PHONE_ALL_CLIENTS_PER_DAY: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_OTP_WRONG_PER_PHONE_ALL_CLIENTS_PER_DAY",
    "Wrong sign-in codes allowed for one phone number over 24 hours from all clients together, since \
     the last right code; after that the number is locked for every client that has not signed in \
     with it in the last 90 days. Clients that have keep their own limit. 1 to 100 (NIST SP \
     800-63B-4: at most 100 failed attempts in a row).",
)
.with_default("20");

pub(crate) const OTP_PER_PHONE_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_OTP_PER_PHONE_PER_HOUR",
    "Phone codes sent to one phone number per hour from one client (one IPv4 address, or one IPv6 /64 \
     block), 1 to 100.",
)
.with_default("3");

pub(crate) const OTP_PER_PHONE_ALL_CLIENTS_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_OTP_PER_PHONE_ALL_CLIENTS_PER_HOUR",
    "Sign-in codes sent to one phone number per hour from all clients together, 1 to 1000; a client \
     that signed in with the number in the last 90 days is not counted against it.",
)
.with_default("5");

pub(crate) const OTP_PER_PHONE_PER_DAY: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_OTP_PER_PHONE_PER_DAY",
    "Codes sent to one phone number per day, 1 to 1000, counted separately for sign-in codes and for \
     phone verification codes (one cannot use up the other); sign-in codes asked from a client that \
     signed in with the number in the last 90 days are not counted.",
)
.with_default("20");

pub(crate) const OTP_PER_USER_PER_DAY: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_OTP_PER_USER_PER_DAY",
    "Phone verification codes one account may ask for per day, 1 to 1000.",
)
.with_default("10");

pub(crate) const OTP_PER_IP_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_OTP_PER_IP_PER_HOUR",
    "Phone codes (sign-in and phone verification) asked for per hour from one client: one IPv4 \
     address, or one IPv6 /64 block; 1 to 10000.",
)
.with_default("20");

pub(crate) const COOKIE_CONSENTS_PER_IP_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_COOKIE_CONSENTS_PER_IP_PER_HOUR",
    "Cookie consent records accepted from one network block (IP /24 or /64) per hour, 1 to 100000.",
)
.with_default("60");

pub(crate) const PUBLIC_REPORTS_PER_IP_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_PUBLIC_REPORTS_PER_IP_PER_HOUR",
    "Reports of infringements and consumer complaints accepted from one network block (IPv4 /24, \
     IPv6 /64) and from one email address per hour through the public forms, 1 to 10000.",
)
.with_default("5");

pub(crate) const PURPOSES_TENANT_MAX: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_PURPOSES_TENANT_MAX",
    "Most processing purposes of its own a tenant may add (on top of the defaults of the legal rules), 1 \
     to 10000.",
)
.with_default("200");

pub(crate) const CASE_TRACKING_URL: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_CASE_TRACKING_URL",
    "Address of the interface's page where a reporter follows a report or complaint, with `{case}` \
     and `{token}` where the case id and tracking code go, e.g. \
     `https://shop.example/report-status?case={case}&token={token}`. Unset: the receipt mail \
     carries the API path `GET /api/v1/compliance/public/cases/{id}?token=…`.",
);

/// Origin khách cho phép làm địa chỉ quay về sau khi xác minh danh tính, ngoài tên miền đã xác minh (L-11,
/// D-20261128-return-url-kernel).
pub(crate) const KYC_RETURN_ORIGINS: autoxide_kernel::tenancy::TenantSetting =
    autoxide_kernel::tenancy::TenantSetting::new(
        "compliance.kyc.return_origins",
        "Addresses a person may be sent back to after an identity check at the provider (`return_url` \
         of a KYC session), besides this tenant's verified domains: origins separated by commas, such \
         as `https://shop.example.com, http://localhost:3000`. Only the scheme, host and port count.",
    )
    .checked(check_return_origins);

fn check_return_origins(raw: &str) -> Result<(), autoxide_kernel::error::FieldError> {
    let bad = autoxide_kernel::tenancy::invalid_return_origins(raw);
    if bad.is_empty() {
        return Ok(());
    }
    Err(autoxide_kernel::error::FieldError::new(
        "value",
        "compliance.return_origins_invalid",
        "must be origins such as `https://shop.example.com`, separated by commas (no path, no `*`: \
         list each host); not valid: {items}",
    )
    .with_param("items", bad.join(", ")))
}

/// Trang theo dõi vụ việc theo từng khách (D-20261125-tenant-link-templates).
pub(crate) const CASE_TRACKING_LINK: autoxide_kernel::tenancy::TenantSetting =
    autoxide_kernel::tenancy::TenantSetting::link(
        "compliance.link.case_tracking",
        "Page of this tenant's website where a reporter follows a report or complaint, such as \
         `https://shop.example/khieu-nai?case={case}&token={token}`: `{case}` and `{token}` are \
         filled in. Must be on this tenant's own host (a verified domain of this tenant). When \
         unset, the server's AUTOXIDE_COMPLIANCE_CASE_TRACKING_URL is used.",
        &["{case}", "{token}"],
    );

fn tracking_url(raw: &str) -> Result<String, String> {
    let url = raw.trim();
    if (url.starts_with("https://") || url.starts_with("http://"))
        && url.contains("{case}")
        && url.contains("{token}")
    {
        Ok(url.to_owned())
    } else {
        Err("must be an http(s) URL containing `{case}` and `{token}`".to_owned())
    }
}

/// Giới hạn của mã một lần xác thực số điện thoại.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Otp {
    pub ttl_seconds: i64,
    pub max_attempts: i32,
    /// `AUTOXIDE_COMPLIANCE_OTP_WRONG_PER_PHONE_PER_DAY`.
    pub wrong_per_phone_per_day: i32,
    /// Số mã đăng nhập mỗi giờ của một số, mọi client lạ cộng lại (D-20261126-otp-nist).
    pub per_phone_all_clients_per_hour: u32,
    /// Số mã (đăng nhập và xác thực số) gửi tới một số mỗi ngày (D-20261126-otp-nist).
    pub per_phone_per_day: u32,
    /// Trần chung của một số cho client lạ (D-20261126-otp-nist).
    pub wrong_per_phone_all_clients_per_day: i32,
    pub per_phone_per_hour: u32,
    pub per_user_per_day: u32,
    pub per_ip_per_hour: u32,
}

impl Default for Otp {
    fn default() -> Self {
        Otp {
            ttl_seconds: 300,
            max_attempts: 5,
            wrong_per_phone_per_day: 10,
            wrong_per_phone_all_clients_per_day: 20,
            per_phone_all_clients_per_hour: 5,
            per_phone_per_day: 20,
            per_phone_per_hour: 3,
            per_user_per_day: 10,
            per_ip_per_hour: 20,
        }
    }
}

/// Cấu hình đã đọc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Config {
    pub callback_retention_days: i64,
    pub otp: Otp,
    /// `AUTOXIDE_COMPLIANCE_COOKIE_CONSENTS_PER_IP_PER_HOUR`.
    pub cookie_consents_per_hour: u32,
    /// `AUTOXIDE_COMPLIANCE_PUBLIC_REPORTS_PER_IP_PER_HOUR`.
    pub public_reports_per_hour: u32,
    /// `AUTOXIDE_COMPLIANCE_CASE_TRACKING_URL`.
    pub case_tracking_url: Option<String>,
    /// `AUTOXIDE_COMPLIANCE_PURPOSES_TENANT_MAX`.
    pub purposes_tenant_max: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            callback_retention_days: 90,
            otp: Otp::default(),
            cookie_consents_per_hour: 60,
            public_reports_per_hour: 5,
            case_tracking_url: None,
            purposes_tenant_max: 200,
        }
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Config, Invalid> {
    let callback_retention_days =
        reader.get_with(&KYC_CALLBACK_RETENTION_DAYS, |raw| parse_in_range(raw, 1i64, 3650));
    let ttl_seconds = reader.get_with(&OTP_TTL_SECONDS, |raw| parse_in_range(raw, 60i64, 600));
    let max_attempts = reader.get_with(&OTP_MAX_ATTEMPTS, |raw| parse_in_range(raw, 1i32, 20));
    let wrong_per_phone_per_day =
        reader.get_with(&OTP_WRONG_PER_PHONE_PER_DAY, |raw| parse_in_range(raw, 1i32, 100));
    let wrong_all_clients = reader
        .get_with(&OTP_WRONG_PER_PHONE_ALL_CLIENTS_PER_DAY, |raw| parse_in_range(raw, 1i32, 100));
    let per_phone_per_hour =
        reader.get_with(&OTP_PER_PHONE_PER_HOUR, |raw| parse_in_range(raw, 1u32, 100));
    let per_phone_all =
        reader.get_with(&OTP_PER_PHONE_ALL_CLIENTS_PER_HOUR, |raw| parse_in_range(raw, 1u32, 1000));
    let per_phone_per_day =
        reader.get_with(&OTP_PER_PHONE_PER_DAY, |raw| parse_in_range(raw, 1u32, 1000));
    let per_user_per_day =
        reader.get_with(&OTP_PER_USER_PER_DAY, |raw| parse_in_range(raw, 1u32, 1000));
    let per_ip_per_hour =
        reader.get_with(&OTP_PER_IP_PER_HOUR, |raw| parse_in_range(raw, 1u32, 10_000));
    let cookie_consents_per_hour =
        reader.get_with(&COOKIE_CONSENTS_PER_IP_PER_HOUR, |raw| parse_in_range(raw, 1u32, 100_000));
    let public_reports_per_hour =
        reader.get_with(&PUBLIC_REPORTS_PER_IP_PER_HOUR, |raw| parse_in_range(raw, 1u32, 10_000));
    let case_tracking_url = reader.optional_with(&CASE_TRACKING_URL, tracking_url);
    let purposes_tenant_max =
        reader.get_with(&PURPOSES_TENANT_MAX, |raw| parse_in_range(raw, 1u64, 10_000));
    Ok(Config {
        purposes_tenant_max: purposes_tenant_max?,
        public_reports_per_hour: public_reports_per_hour?,
        case_tracking_url: case_tracking_url?,
        cookie_consents_per_hour: cookie_consents_per_hour?,
        callback_retention_days: callback_retention_days?,
        otp: Otp {
            ttl_seconds: ttl_seconds?,
            max_attempts: max_attempts?,
            wrong_per_phone_per_day: wrong_per_phone_per_day?,
            wrong_per_phone_all_clients_per_day: wrong_all_clients?,
            per_phone_all_clients_per_hour: per_phone_all?,
            per_phone_per_day: per_phone_per_day?,
            per_phone_per_hour: per_phone_per_hour?,
            per_user_per_day: per_user_per_day?,
            per_ip_per_hour: per_ip_per_hour?,
        },
    })
}
