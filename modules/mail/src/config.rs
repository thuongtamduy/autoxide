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

#[derive(Debug, Clone)]
pub(crate) struct MailConfig {
    /// `None`: ghi mail ra file.
    pub smtp_url: Option<String>,
    pub from: Mailbox,
    pub file_dir: PathBuf,
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
        Ok(MailConfig { smtp_url: smtp_url?, from: from?, file_dir: file_dir? })
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
