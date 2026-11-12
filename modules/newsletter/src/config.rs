//! Cấu hình của module `newsletter`, tiền tố `AUTOXIDE_NEWSLETTER_`. Mọi biến có mặc định chạy
//! được ngay, hoặc là tuỳ chọn: thiếu thì chỉ chức năng cần nó báo lỗi rõ khi được dùng (vd gửi
//! chiến dịch cần URL trang lựa chọn), chương trình vẫn khởi động. Nhãn và giới hạn thư quảng cáo
//! là cấu hình (không cài cứng) vì yêu cầu pháp lý còn chưa chắc: người cài tự đối chiếu văn bản
//! gốc cho trường hợp của mình.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const CONFIRM_URL: Setting = Setting::new(
    "AUTOXIDE_NEWSLETTER_CONFIRM_URL",
    "Page of your website that confirms a newsletter subscription, with `{token}` where the \
     token goes; the page sends it to `POST /api/v1/newsletter/public/confirm`. Needed to \
     subscribe to a list with double opt-in; without it, sign-ups that must be confirmed on \
     other lists (held by the anti-spam checks, or from people who unsubscribed) get no mail.",
)
.with_example("https://www.example.com/newsletter/confirm?token={token}");

pub(crate) const PREFERENCES_URL: Setting = Setting::new(
    "AUTOXIDE_NEWSLETTER_PREFERENCES_URL",
    "Page of your website where a subscriber chooses lists or unsubscribes, with `{token}` \
     where the token goes; the page uses `/api/v1/newsletter/public/preferences`. Every \
     campaign mail links there, so sending needs it.",
)
.with_example("https://www.example.com/newsletter/preferences?token={token}");

pub(crate) const SITE_URL: Setting = Setting::new(
    "AUTOXIDE_NEWSLETTER_SITE_URL",
    "Address of your website, without a trailing slash, put in front of the path of a new post \
     in new-post campaigns. When unset, new-post campaigns are only created as drafts.",
)
.with_example("https://www.example.com");

pub(crate) const CONFIRM_HOURS: Setting = Setting::new(
    "AUTOXIDE_NEWSLETTER_CONFIRM_HOURS",
    "Hours a confirmation link stays valid (1 to 720).",
)
.with_default("168");

pub(crate) const CONFIRM_MAILS_PER_DAY: Setting = Setting::new(
    "AUTOXIDE_NEWSLETTER_CONFIRM_MAILS_PER_DAY",
    "Sign-up requests per address per day that may send a confirmation mail (1 to 100); every \
     request for the address counts, and later ones are answered as usual but send nothing.",
)
.with_default("3");

pub(crate) const UNCONFIRMED_DAYS: Setting = Setting::new(
    "AUTOXIDE_NEWSLETTER_UNCONFIRMED_DAYS",
    "Days an unconfirmed subscription is kept before a daily job deletes it (1 to 365).",
)
.with_default("30");

pub(crate) const PER_IP_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_NEWSLETTER_SUBSCRIBE_PER_IP_PER_HOUR",
    "Public subscribe requests accepted from one network block (IPv4 /24, IPv6 /64) per hour \
     (1 to 100000); more are dropped silently.",
)
.with_default("10");

pub(crate) const PER_EMAIL_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_NEWSLETTER_SUBSCRIBE_PER_EMAIL_PER_HOUR",
    "Public subscribe requests accepted for one email address per hour (1 to 100000); more are \
     dropped silently.",
)
.with_default("5");

pub(crate) const PER_USER_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_NEWSLETTER_SUBSCRIBE_PER_USER_PER_HOUR",
    "Subscribe requests per hour from one caller with the permission `newsletter.subscribe`, \
     e.g. a website server with an API key subscribing its visitors (1 to 1000000); more are \
     refused with 429.",
)
.with_default("1000");

pub(crate) const SEND_RATE_PER_MINUTE: Setting = Setting::new(
    "AUTOXIDE_NEWSLETTER_SEND_RATE_PER_MINUTE",
    "Campaign mails handed to the mail queue per minute at most (1 to 100000). Keep it under \
     what your mail relay accepts.",
)
.with_default("300");

pub(crate) const BATCH_SIZE: Setting = Setting::new(
    "AUTOXIDE_NEWSLETTER_BATCH_SIZE",
    "Campaign mails per sending job (1 to 1000); the jobs are spaced to keep the send rate.",
)
.with_default("100");

pub(crate) const SOFT_BOUNCE_LIMIT: Setting = Setting::new(
    "AUTOXIDE_NEWSLETTER_SOFT_BOUNCE_LIMIT",
    "Soft bounces of one address before it is suppressed (1 to 100). A hard bounce or a \
     complaint suppresses at once.",
)
.with_default("3");

pub(crate) const ADVERTISING_LABEL: Setting = Setting::new(
    "AUTOXIDE_NEWSLETTER_ADVERTISING_LABEL",
    "Label put at the start of the subject of advertising campaigns (at most 20 characters; \
     `none` for no label). The default follows a reading of Vietnam's Decree 91/2020 that was \
     not checked against the original text: check the law that applies to you.",
)
.with_default("[QC]");

pub(crate) const ADVERTISING_MAX_PER_DAY: Setting = Setting::new(
    "AUTOXIDE_NEWSLETTER_ADVERTISING_MAX_PER_DAY",
    "Advertising campaign mails one address may receive in 24 hours (0 to 100; 0: no limit); \
     beyond it the mail is skipped. The default follows a reading of Vietnam's Decree \
     91/2020 that was not checked against the original text.",
)
.with_default("3");

pub(crate) const POSTAL_ADDRESS: Setting = Setting::new(
    "AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS",
    "Name and postal address of the sender, put at the foot of every campaign mail (at most 300 \
     characters). Needed to send advertising campaigns.",
)
.with_example("Example Ltd, 1 Example Street, Hanoi, Vietnam");

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewsletterConfig {
    pub confirm_url: Option<String>,
    pub preferences_url: Option<String>,
    pub site_url: Option<String>,
    pub confirm_hours: u32,
    pub confirm_mails_per_day: u32,
    pub unconfirmed_days: u32,
    pub per_ip_per_hour: u32,
    pub per_email_per_hour: u32,
    pub per_user_per_hour: u32,
    pub send_rate_per_minute: u32,
    pub batch_size: u32,
    pub soft_bounce_limit: u32,
    /// Rỗng: không gắn nhãn.
    pub advertising_label: String,
    /// 0: không giới hạn.
    pub advertising_max_per_day: u32,
    pub postal_address: Option<String>,
}

impl Default for NewsletterConfig {
    fn default() -> Self {
        NewsletterConfig {
            confirm_url: None,
            preferences_url: None,
            site_url: None,
            confirm_hours: 168,
            confirm_mails_per_day: 3,
            unconfirmed_days: 30,
            per_ip_per_hour: 10,
            per_email_per_hour: 5,
            per_user_per_hour: 1000,
            send_rate_per_minute: 300,
            batch_size: 100,
            soft_bounce_limit: 3,
            advertising_label: "[QC]".to_owned(),
            advertising_max_per_day: 3,
            postal_address: None,
        }
    }
}

impl NewsletterConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let confirm_url = reader.optional_with(&CONFIRM_URL, token_url);
        let preferences_url = reader.optional_with(&PREFERENCES_URL, token_url);
        let site_url = reader.optional_with(&SITE_URL, site_url);
        let confirm_hours = reader.get_with(&CONFIRM_HOURS, |raw| parse_in_range(raw, 1u32, 720));
        let confirm_mails =
            reader.get_with(&CONFIRM_MAILS_PER_DAY, |raw| parse_in_range(raw, 1u32, 100));
        let unconfirmed = reader.get_with(&UNCONFIRMED_DAYS, |raw| parse_in_range(raw, 1u32, 365));
        let per_ip = reader.get_with(&PER_IP_PER_HOUR, |raw| parse_in_range(raw, 1u32, 100_000));
        let per_email =
            reader.get_with(&PER_EMAIL_PER_HOUR, |raw| parse_in_range(raw, 1u32, 100_000));
        let per_user =
            reader.get_with(&PER_USER_PER_HOUR, |raw| parse_in_range(raw, 1u32, 1_000_000));
        let rate = reader.get_with(&SEND_RATE_PER_MINUTE, |raw| parse_in_range(raw, 1u32, 100_000));
        let batch = reader.get_with(&BATCH_SIZE, |raw| parse_in_range(raw, 1u32, 1000));
        let soft = reader.get_with(&SOFT_BOUNCE_LIMIT, |raw| parse_in_range(raw, 1u32, 100));
        let label = reader.get_with(&ADVERTISING_LABEL, |raw| {
            let label = raw.trim();
            if label.eq_ignore_ascii_case("none") {
                Ok(String::new())
            } else if label.chars().count() <= 20 && !label.chars().any(char::is_control) {
                Ok(label.to_owned())
            } else {
                Err("must be at most 20 characters on one line, or `none`".to_owned())
            }
        });
        let ads = reader.get_with(&ADVERTISING_MAX_PER_DAY, |raw| parse_in_range(raw, 0u32, 100));
        let address = reader.optional_with(&POSTAL_ADDRESS, |raw| {
            let address = raw.trim();
            if !address.is_empty()
                && address.chars().count() <= 300
                && !address.chars().any(|c| c.is_control() && c != '\n')
            {
                Ok(address.to_owned())
            } else {
                Err("must be 1 to 300 characters".to_owned())
            }
        });
        Ok(NewsletterConfig {
            confirm_url: confirm_url?,
            preferences_url: preferences_url?,
            site_url: site_url?,
            confirm_hours: confirm_hours?,
            confirm_mails_per_day: confirm_mails?,
            unconfirmed_days: unconfirmed?,
            per_ip_per_hour: per_ip?,
            per_email_per_hour: per_email?,
            per_user_per_hour: per_user?,
            send_rate_per_minute: rate?,
            batch_size: batch?,
            soft_bounce_limit: soft?,
            advertising_label: label?,
            advertising_max_per_day: ads?,
            postal_address: address?,
        })
    }
}

/// Địa chỉ http(s) có chỗ điền `{token}`.
fn token_url(raw: &str) -> Result<String, String> {
    let url = raw.trim();
    let lower = url.to_ascii_lowercase();
    if (lower.starts_with("https://") || lower.starts_with("http://")) && url.contains("{token}") {
        Ok(url.to_owned())
    } else {
        Err("must be an http(s) URL containing `{token}`".to_owned())
    }
}

/// Địa chỉ http(s) của website, bỏ `/` cuối.
fn site_url(raw: &str) -> Result<String, String> {
    let url = raw.trim().trim_end_matches('/');
    let lower = url.to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"));
    match rest {
        Some(rest) if !rest.is_empty() && !rest.contains(['?', '#', ' ']) => Ok(url.to_owned()),
        _ => Err("must be an absolute `https://` (or `http://`) address like \
                  `https://www.example.com`"
            .to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::ConfigSource;

    fn read(pairs: &[(&str, &str)]) -> Result<NewsletterConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = NewsletterConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mac_dinh_chay_duoc_va_giong_default() {
        assert_eq!(read(&[]).unwrap(), NewsletterConfig::default());
    }

    #[test]
    fn gia_tri_sai_bi_bao_cung_luc() {
        let error = read(&[
            ("AUTOXIDE_NEWSLETTER_CONFIRM_URL", "https://example.com/confirm"),
            ("AUTOXIDE_NEWSLETTER_BATCH_SIZE", "5000"),
            ("AUTOXIDE_NEWSLETTER_SITE_URL", "example.com"),
        ])
        .unwrap_err();
        for name in ["CONFIRM_URL", "BATCH_SIZE", "SITE_URL"] {
            assert!(error.contains(name), "{error}");
        }
        let config = read(&[
            ("AUTOXIDE_NEWSLETTER_ADVERTISING_LABEL", "none"),
            ("AUTOXIDE_NEWSLETTER_SITE_URL", "https://www.example.com/"),
        ])
        .unwrap();
        assert_eq!(config.advertising_label, "");
        assert_eq!(config.site_url.as_deref(), Some("https://www.example.com"));
    }
}
