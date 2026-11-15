//! Phần của `mail` trong cấu hình cho giao diện (D-271): địa chỉ người gửi, mail có thật sự được
//! gửi đi không. Không bao giờ có URL SMTP (chứa mật khẩu).

use autoxide_kernel::error::Error;
use autoxide_kernel::module::ClientSection;
use serde::Serialize;
use utoipa::ToSchema;

use crate::config::MailConfig;

/// Mail settings an interface needs.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = MailClientConfig)]
struct MailClientConfig {
    /// Sender of every mail (`AUTOXIDE_MAIL_FROM`).
    #[schema(example = "Example <no-reply@example.com>")]
    from: String,
    /// `true` when mail goes out through an SMTP server; `false` when it is only written to
    /// files on the server (`AUTOXIDE_MAIL_SMTP_URL` not set) — nobody receives it.
    sends: bool,
}

pub(crate) fn section(config: &MailConfig) -> Result<Option<ClientSection>, Error> {
    let section =
        MailClientConfig { from: config.from.to_string(), sends: config.smtp_url.is_some() };
    ClientSection::new(&section).map(Some)
}
