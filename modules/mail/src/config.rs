//! Cấu hình của module `mail`. Mọi biến mang tiền tố `AUTOXIDE_MAIL_`.

use std::path::PathBuf;

use autoxide_kernel::config::{ConfigReader, Invalid, Setting};
use lettre::message::Mailbox;
use lettre::{AsyncSmtpTransport, Tokio1Executor};

pub(crate) const SMTP_URL: Setting = Setting::new(
    "AUTOXIDE_MAIL_SMTP_URL",
    "SMTP server that sends mail: `smtps://user:password@host:465` (TLS), or \
     `smtp://user:password@host:587?tls=required` (STARTTLS). Percent-encode special characters \
     in the user and password. When unset, mail is written as `.eml` files to \
     AUTOXIDE_MAIL_FILE_DIR instead of being sent — fine for development, not for production.",
)
.secret()
.with_example("smtps://user:password@smtp.example.com:465");

pub(crate) const FROM: Setting = Setting::new(
    "AUTOXIDE_MAIL_FROM",
    "Sender of every mail, as `Name <address>` or `address`. Use an address your SMTP server \
     may send from.",
)
.with_default("Autoxide <noreply@localhost>");

pub(crate) const FILE_DIR: Setting = Setting::new(
    "AUTOXIDE_MAIL_FILE_DIR",
    "Directory where mail is written when AUTOXIDE_MAIL_SMTP_URL is unset; created when \
     missing.",
)
.with_default("data/mail");

pub(crate) const ALLOW_PRIVATE_NETWORKS: Setting = Setting::new(
    "AUTOXIDE_MAIL_ALLOW_PRIVATE_NETWORKS",
    "Allow a tenant's own SMTP server (tenant setting `mail.connection.smtp.url`) in private or \
     local networks (`localhost`, `10.x`, `192.168.x`, …). Off by default: tenants could otherwise \
     make this server connect to internal services. Turn it on only when every tenant is trusted. \
     AUTOXIDE_MAIL_SMTP_URL (set by the operator) is never restricted.",
)
.with_default("false");

pub(crate) const HARD_BOUNCE_LIMIT: Setting = Setting::new(
    "AUTOXIDE_MAIL_HARD_BOUNCE_LIMIT",
    "Hard bounces after which mail to an address is suppressed for that tenant, 1 to 20. Counted: \
     the receiving server refuses the recipient address itself with enhanced status 5.1.1, 5.1.2, \
     5.1.3, 5.1.6 or 5.1.10, at most once per 24 hours (a burst of failures counts once). Not \
     counted: sender-side errors (5.1.7, 5.1.8), full mailbox, message too big, replies without an \
     enhanced status. A tenant's administrator lists and releases suppressed addresses at \
     `/api/v1/mail/bounces`.",
)
.with_default("3");

pub(crate) const BOUNCE_RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_MAIL_BOUNCE_RETENTION_DAYS",
    "Days a bounce record (an email address and why mail to it failed) is kept after its last \
     bounce or skipped mail, 30 to 3650; a suppressed address is mailed again once its record is \
     gone.",
)
.with_default("365");

#[derive(Debug, Clone)]
pub(crate) struct MailConfig {
    /// `None`: ghi mail ra file.
    pub smtp_url: Option<String>,
    pub from: Mailbox,
    pub file_dir: PathBuf,
    /// `AUTOXIDE_MAIL_ALLOW_PRIVATE_NETWORKS`.
    pub allow_private_networks: bool,
    /// `AUTOXIDE_MAIL_HARD_BOUNCE_LIMIT`.
    pub hard_bounce_limit: u32,
    /// `AUTOXIDE_MAIL_BOUNCE_RETENTION_DAYS`.
    pub bounce_retention_days: u32,
}

impl MailConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let smtp_url = reader.optional_with(&SMTP_URL, |raw| {
            // Kiểm ngay lúc khởi động: URL sai thì báo trước, đừng đợi tới mail đầu tiên.
            AsyncSmtpTransport::<Tokio1Executor>::from_url(raw).map(|_| raw.to_owned()).map_err(
                |_| {
                    "must be an SMTP URL like `smtps://user:password@host:465` or \
                     `smtp://user:password@host:587?tls=required`"
                        .to_owned()
                },
            )
        });
        let from = reader.get_with(&FROM, |raw| {
            raw.parse::<Mailbox>()
                .map_err(|_| "must be a mail address, like `Name <noreply@example.com>`".to_owned())
        });
        let file_dir = reader.get_with(&FILE_DIR, |raw| Ok(PathBuf::from(raw)));
        let allow_private_networks = reader.get::<bool>(&ALLOW_PRIVATE_NETWORKS);
        let hard_bounce_limit = reader.get_with(&HARD_BOUNCE_LIMIT, |raw| {
            autoxide_kernel::config::parse_in_range(raw, 1u32, 20)
        });
        let bounce_retention_days = reader.get_with(&BOUNCE_RETENTION_DAYS, |raw| {
            autoxide_kernel::config::parse_in_range(raw, 30u32, 3650)
        });
        Ok(MailConfig {
            smtp_url: smtp_url?,
            from: from?,
            file_dir: file_dir?,
            allow_private_networks: allow_private_networks?,
            hard_bounce_limit: hard_bounce_limit?,
            bounce_retention_days: bounce_retention_days?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::ConfigSource;

    fn read(pairs: &[(&str, &str)]) -> Result<MailConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = MailConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mac_dinh_ghi_ra_file() {
        let config = read(&[]).unwrap();

        assert_eq!(config.smtp_url, None);
        assert_eq!(config.from.to_string(), "Autoxide <noreply@localhost>");
        assert_eq!(config.file_dir, PathBuf::from("data/mail"));
        assert!(!config.allow_private_networks);
    }

    #[test]
    fn gia_tri_sai_bi_bao_va_khong_lo_mat_khau() {
        let error = read(&[
            ("AUTOXIDE_MAIL_SMTP_URL", "ftp://user:s3cr3t@host"),
            ("AUTOXIDE_MAIL_FROM", "not an address"),
        ])
        .unwrap_err();

        assert!(error.contains("AUTOXIDE_MAIL_SMTP_URL: must be an SMTP URL"), "{error}");
        assert!(error.contains("AUTOXIDE_MAIL_FROM: must be a mail address"), "{error}");
        assert!(!error.contains("s3cr3t"), "{error}");
        assert!(read(&[("AUTOXIDE_MAIL_SMTP_URL", "smtps://u:p@smtp.example.com")]).is_ok());
    }
}
