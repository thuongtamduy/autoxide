//! Cấu hình của module `compliance`.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const KYC_CALLBACK_RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_KYC_CALLBACK_RETENTION_DAYS",
    "Days the raw callbacks of identity providers are kept before they are deleted, 1 to 3650.",
)
.with_default("90");

pub(crate) const OTP_TTL_SECONDS: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_OTP_TTL_SECONDS",
    "Seconds a one-time code sent by SMS to verify a phone number stays valid, 60 to 3600.",
)
.with_default("300");

pub(crate) const OTP_MAX_ATTEMPTS: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_OTP_MAX_ATTEMPTS",
    "Wrong entries allowed for one phone verification code before a new code is needed, 1 to 20.",
)
.with_default("5");

pub(crate) const OTP_PER_PHONE_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_OTP_PER_PHONE_PER_HOUR",
    "Phone verification codes sent to one phone number per hour, 1 to 100.",
)
.with_default("3");

pub(crate) const OTP_PER_USER_PER_DAY: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_OTP_PER_USER_PER_DAY",
    "Phone verification codes one account may ask for per day, 1 to 1000.",
)
.with_default("10");

pub(crate) const OTP_PER_IP_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_OTP_PER_IP_PER_HOUR",
    "Phone verification codes asked for from one IP address per hour, 1 to 10000.",
)
.with_default("20");

pub(crate) const COOKIE_CONSENTS_PER_IP_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_COOKIE_CONSENTS_PER_IP_PER_HOUR",
    "Cookie consent records accepted from one network block (IP /24 or /64) per hour, 1 to 100000.",
)
.with_default("60");

pub(crate) const PUBLIC_REPORTS_PER_IP_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_PUBLIC_REPORTS_PER_IP_PER_HOUR",
    "Reports of infringements and consumer complaints accepted from one network block (and from \
     one email address) per hour through the public forms, 1 to 10000.",
)
.with_default("5");

pub(crate) const CASE_TRACKING_URL: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_CASE_TRACKING_URL",
    "Address of the interface's page where a reporter follows a report or complaint, with `{case}` \
     and `{token}` where the case id and tracking code go, e.g. \
     `https://shop.example/report-status?case={case}&token={token}`. Unset: the receipt mail \
     carries the API path `GET /api/v1/compliance/public/cases/{id}?token=…`.",
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
    pub per_phone_per_hour: u32,
    pub per_user_per_day: u32,
    pub per_ip_per_hour: u32,
}

impl Default for Otp {
    fn default() -> Self {
        Otp {
            ttl_seconds: 300,
            max_attempts: 5,
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
}

impl Default for Config {
    fn default() -> Self {
        Config {
            callback_retention_days: 90,
            otp: Otp::default(),
            cookie_consents_per_hour: 60,
            public_reports_per_hour: 5,
            case_tracking_url: None,
        }
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Config, Invalid> {
    let callback_retention_days =
        reader.get_with(&KYC_CALLBACK_RETENTION_DAYS, |raw| parse_in_range(raw, 1i64, 3650));
    let ttl_seconds = reader.get_with(&OTP_TTL_SECONDS, |raw| parse_in_range(raw, 60i64, 3600));
    let max_attempts = reader.get_with(&OTP_MAX_ATTEMPTS, |raw| parse_in_range(raw, 1i32, 20));
    let per_phone_per_hour =
        reader.get_with(&OTP_PER_PHONE_PER_HOUR, |raw| parse_in_range(raw, 1u32, 100));
    let per_user_per_day =
        reader.get_with(&OTP_PER_USER_PER_DAY, |raw| parse_in_range(raw, 1u32, 1000));
    let per_ip_per_hour =
        reader.get_with(&OTP_PER_IP_PER_HOUR, |raw| parse_in_range(raw, 1u32, 10_000));
    let cookie_consents_per_hour =
        reader.get_with(&COOKIE_CONSENTS_PER_IP_PER_HOUR, |raw| parse_in_range(raw, 1u32, 100_000));
    let public_reports_per_hour =
        reader.get_with(&PUBLIC_REPORTS_PER_IP_PER_HOUR, |raw| parse_in_range(raw, 1u32, 10_000));
    let case_tracking_url = reader.optional_with(&CASE_TRACKING_URL, tracking_url);
    Ok(Config {
        public_reports_per_hour: public_reports_per_hour?,
        case_tracking_url: case_tracking_url?,
        cookie_consents_per_hour: cookie_consents_per_hour?,
        callback_retention_days: callback_retention_days?,
        otp: Otp {
            ttl_seconds: ttl_seconds?,
            max_attempts: max_attempts?,
            per_phone_per_hour: per_phone_per_hour?,
            per_user_per_day: per_user_per_day?,
            per_ip_per_hour: per_ip_per_hour?,
        },
    })
}
