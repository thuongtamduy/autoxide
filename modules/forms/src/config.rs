//! Cấu hình của module `forms`, tiền tố `AUTOXIDE_FORMS_`. Mọi biến có mặc định chạy được
//! ngay. Thời hạn giữ dữ liệu là cấu hình (không cài cứng) vì yêu cầu pháp lý còn chưa chắc:
//! người cài tự đối chiếu văn bản gốc cho trường hợp của mình.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};
use autoxide_kernel::i18n::Language;

pub(crate) const MAX_SUBMISSION_BYTES: Setting = Setting::new(
    "AUTOXIDE_FORMS_MAX_SUBMISSION_BYTES",
    "Largest request body of a public form submission, in bytes (1024 to 1048576).",
)
.with_default("65536");

pub(crate) const MAX_FIELDS: Setting =
    Setting::new("AUTOXIDE_FORMS_MAX_FIELDS", "Most fields one form may have (1 to 200).")
        .with_default("50");

pub(crate) const PER_IP_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_FORMS_SUBMISSIONS_PER_IP_PER_HOUR",
    "Public submissions accepted per form from one network block (IPv4 /24, IPv6 /64) per \
     hour (1 to 100000); more are dropped silently. A form may set its own limit.",
)
.with_default("10");

pub(crate) const PER_EMAIL_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_FORMS_SUBMISSIONS_PER_EMAIL_PER_HOUR",
    "Public submissions accepted per form from one email address per hour (1 to 100000); \
     more are dropped silently. A form may set its own limit.",
)
.with_default("5");

pub(crate) const PER_USER_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_FORMS_SUBMISSIONS_PER_USER_PER_HOUR",
    "Submissions per hour from one caller with the permission `forms.submit`, e.g. a website \
     server sending forms with its API key for all its visitors (1 to 1000000); more are \
     refused with 429.",
)
.with_default("1000");

pub(crate) const RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_FORMS_RETENTION_DAYS",
    "Days a submission is kept before a daily job deletes it (0 to 36500; 0 keeps them until \
     deleted by hand). A form may set its own. Check what the law that applies to you asks.",
)
.with_default("0");

pub(crate) const SPAM_RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_FORMS_SPAM_RETENTION_DAYS",
    "Days a submission marked as spam is kept before the daily job deletes it (1 to 3650).",
)
.with_default("30");

pub(crate) const IP_RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_FORMS_IP_RETENTION_DAYS",
    "Days the hashed network address of a submission is kept before the daily job clears it \
     (1 to 3650). Raw addresses are never stored.",
)
.with_default("90");

pub(crate) const NOTIFY_LANGUAGE: Setting = Setting::new(
    "AUTOXIDE_FORMS_NOTIFY_LANGUAGE",
    "Language of the mail sent to a form's notify addresses: `en` or `vi`.",
)
.with_default("en");

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FormsConfig {
    pub max_submission_bytes: usize,
    pub max_fields: usize,
    pub per_ip_per_hour: u32,
    pub per_email_per_hour: u32,
    pub per_user_per_hour: u32,
    /// 0: giữ mãi.
    pub retention_days: u32,
    pub spam_retention_days: u32,
    pub ip_retention_days: u32,
    pub notify_language: Language,
}

impl Default for FormsConfig {
    fn default() -> Self {
        FormsConfig {
            max_submission_bytes: 65_536,
            max_fields: 50,
            per_ip_per_hour: 10,
            per_email_per_hour: 5,
            per_user_per_hour: 1000,
            retention_days: 0,
            spam_retention_days: 30,
            ip_retention_days: 90,
            notify_language: Language::En,
        }
    }
}

impl FormsConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let max_submission_bytes = reader
            .get_with(&MAX_SUBMISSION_BYTES, |raw| parse_in_range(raw, 1024usize, 1024 * 1024));
        let max_fields = reader.get_with(&MAX_FIELDS, |raw| parse_in_range(raw, 1usize, 200));
        let per_ip = reader.get_with(&PER_IP_PER_HOUR, |raw| parse_in_range(raw, 1u32, 100_000));
        let per_email =
            reader.get_with(&PER_EMAIL_PER_HOUR, |raw| parse_in_range(raw, 1u32, 100_000));
        let per_user =
            reader.get_with(&PER_USER_PER_HOUR, |raw| parse_in_range(raw, 1u32, 1_000_000));
        let retention = reader.get_with(&RETENTION_DAYS, |raw| parse_in_range(raw, 0u32, 36_500));
        let spam = reader.get_with(&SPAM_RETENTION_DAYS, |raw| parse_in_range(raw, 1u32, 3650));
        let ip = reader.get_with(&IP_RETENTION_DAYS, |raw| parse_in_range(raw, 1u32, 3650));
        let language = reader.get_with(&NOTIFY_LANGUAGE, |raw| match raw.trim() {
            "en" => Ok(Language::En),
            "vi" => Ok(Language::Vi),
            _ => Err("must be `en` or `vi`".to_owned()),
        });
        Ok(FormsConfig {
            max_submission_bytes: max_submission_bytes?,
            max_fields: max_fields?,
            per_ip_per_hour: per_ip?,
            per_email_per_hour: per_email?,
            per_user_per_hour: per_user?,
            retention_days: retention?,
            spam_retention_days: spam?,
            ip_retention_days: ip?,
            notify_language: language?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::ConfigSource;

    fn read(pairs: &[(&str, &str)]) -> Result<FormsConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = FormsConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mac_dinh_chay_duoc_va_giong_default() {
        assert_eq!(read(&[]).unwrap(), FormsConfig::default());
    }

    #[test]
    fn gia_tri_sai_bi_bao_cung_luc() {
        let error = read(&[
            ("AUTOXIDE_FORMS_SPAM_RETENTION_DAYS", "0"),
            ("AUTOXIDE_FORMS_NOTIFY_LANGUAGE", "fr"),
        ])
        .unwrap_err();
        assert!(
            error.contains("SPAM_RETENTION_DAYS") && error.contains("NOTIFY_LANGUAGE"),
            "{error}"
        );
        let config = read(&[("AUTOXIDE_FORMS_RETENTION_DAYS", "365")]).unwrap();
        assert_eq!(config.retention_days, 365);
    }
}
