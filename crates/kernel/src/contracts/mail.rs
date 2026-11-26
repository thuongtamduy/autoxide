//! Sending mail: a shared trait so any module can send mail without depending on how it is sent.
//!
//! One module (`mail`) publishes `dyn Mailer`; a module that needs to send mail gets it with
//! `context.require::<dyn Mailer>()` (registered AFTER module `mail`). `send` only puts the mail into the
//! queue and returns at once: sending runs in the background, and temporary errors are retried (D-061). When
//! the receiving server flatly rejects the RECIPIENT ADDRESS ITSELF (SMTP 5xx with extended code `5.1.x` /
//! `5.2.x`: no mailbox …) the mail is not retried and the event [`DELIVERY_FAILED`] is emitted (D-121,
//! D-158). Any other permanent error is a sender-side error: it is retried and then goes to the dead-letter
//! queue.

use crate::error::{Error, FieldError};
use crate::i18n::{Language, Translations};
use crate::tenancy::TenantContext;

/// Event emitted when the receiving server flatly rejects a mail. Payload:
/// `{"to": "<address>", "dedupe_key": "<sender key>" | null, "code": "<SMTP code>",
/// "enhanced": "<enhanced status>" | null, "hard": <bool>}`. `hard` is true only when the address itself does
/// not work (enhanced status 5.1.1, 5.1.2, 5.1.3, 5.1.6 or 5.1.10); a mailing-list module (e.g. newsletters)
/// stops sending to the address only then. Events written before `hard` existed lack it: treat a missing
/// `hard` as false (mark that one mail failed, do not stop sending to the address).
pub const DELIVERY_FAILED: &str = "mail.delivery_failed";

/// Extra headers that may be set (compared case-insensitively). Other headers are set by the
/// mail sender itself (`From`, `To`, `Subject`, `Message-ID`…).
pub const ALLOWED_HEADERS: [&str; 4] =
    ["List-Unsubscribe", "List-Unsubscribe-Post", "List-Id", "Auto-Submitted"];

/// A mail to send. Built with [`Mail::new`] followed by the setters for each field.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Mail {
    /// Recipient address (`ada@example.com` or `Ada <ada@example.com>`).
    pub to: String,
    pub subject: String,
    /// Plain-text body (always present: every mail app can read it).
    pub text: String,
    /// HTML body, if any.
    pub html: Option<String>,
    /// Replies go to this address (e.g. the form submitter) instead of the sending address.
    pub reply_to: Option<String>,
    /// Extra headers, only those in [`ALLOWED_HEADERS`] (e.g. `List-Unsubscribe` of a newsletter).
    pub headers: Vec<(String, String)>,
    /// Dedupe key: a later mail with the same key as one already queued is dropped (e.g. the ID of a
    /// newsletter send, so a rerun does not send twice). Included in the [`DELIVERY_FAILED`] event.
    pub dedupe_key: Option<String>,
}

impl Mail {
    /// A plain-text mail to `to`, without HTML, reply address, extra headers or dedupe key.
    pub fn new(to: impl Into<String>, subject: impl Into<String>, text: impl Into<String>) -> Self {
        Mail {
            to: to.into(),
            subject: subject.into(),
            text: text.into(),
            html: None,
            reply_to: None,
            headers: Vec::new(),
            dedupe_key: None,
        }
    }

    /// Sets the HTML body.
    pub fn html(mut self, html: String) -> Self {
        self.html = Some(html);
        self
    }

    /// Sets the address replies go to.
    pub fn reply_to(mut self, reply_to: String) -> Self {
        self.reply_to = Some(reply_to);
        self
    }

    /// Adds one extra header; only names in [`ALLOWED_HEADERS`] are sent.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Sets the key that keeps the same mail from being queued twice.
    pub fn dedupe_key(mut self, dedupe_key: String) -> Self {
        self.dedupe_key = Some(dedupe_key);
        self
    }

    /// Errors of extra headers: a name outside [`ALLOWED_HEADERS`], a value containing a line break
    /// (header injection) or longer than 998 characters (the RFC 5322 single-line limit).
    pub fn header_problems(&self) -> Vec<FieldError> {
        let mut problems = Vec::new();
        for (index, (name, value)) in self.headers.iter().enumerate() {
            if !ALLOWED_HEADERS.iter().any(|allowed| allowed.eq_ignore_ascii_case(name)) {
                problems.push(
                    FieldError::new(
                        format!("headers[{index}].name"),
                        "not_allowed",
                        "must be one of {allowed}",
                    )
                    .with_param("allowed", ALLOWED_HEADERS.join(", ")),
                );
            }
            if value.chars().any(|c| c == '\r' || c == '\n' || c == '\0') || value.len() > 998 {
                problems.push(FieldError::new(
                    format!("headers[{index}].value"),
                    "invalid",
                    "must be one line of at most 998 characters",
                ));
            }
        }
        problems
    }
}

/// The program's mail sender.
#[async_trait::async_trait]
pub trait Mailer: Send + Sync {
    /// Queues `mail` of tenant `tenant` for sending (the mail server is chosen by the tenant's configuration).
    /// Errors when the recipient address is malformed (`validation_failed`, field `to`) or the
    /// queue cannot be written.
    async fn send(&self, tenant: &TenantContext, mail: Mail) -> Result<(), Error>;
}

/// Mail template: the English subject and body live in code (with `{name}` placeholders), the
/// translations live in the crate's `locales/<language>.json` under the keys `<key>.subject` and
/// `<key>.text` (`cargo xtask check` verifies they are complete).
///
/// ```
/// use autoxide_kernel::mail::MailTemplate;
///
/// const WELCOME: MailTemplate = MailTemplate::new(
///     "notes.mail.welcome",
///     "Welcome, {name}",
///     "Hello {name}, your notebook is ready.",
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailTemplate {
    key: &'static str,
    subject: &'static str,
    text: &'static str,
}

/// Subject and body after filling in, in one language.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Rendered {
    pub subject: String,
    pub text: String,
}

impl MailTemplate {
    /// `key`: `<module>.mail.<name>`.
    pub const fn new(key: &'static str, subject: &'static str, text: &'static str) -> Self {
        MailTemplate { key, subject, text }
    }

    pub fn key(&self) -> &'static str {
        self.key
    }

    /// Fills `params` into the template using `language`. Any part without a translation (or whose translation
    /// uses a placeholder with no value) falls back to English.
    pub fn render(
        &self,
        translations: &Translations,
        language: Language,
        params: &[(&str, &str)],
    ) -> Rendered {
        let pick = |part: &str, english: &str| {
            let key = format!("{}.{part}", self.key);
            translations
                .text(&key, language)
                .and_then(|template| crate::i18n::fill_all(template, params))
                .or_else(|| crate::i18n::fill_all(english, params))
                .unwrap_or_else(|| english.to_owned())
        };
        Rendered { subject: pick("subject", self.subject), text: pick("text", self.text) }
    }

    /// Builds a [`Mail`] addressed to `to`.
    pub fn mail(
        &self,
        to: &str,
        translations: &Translations,
        language: Language,
        params: &[(&str, &str)],
    ) -> Mail {
        let rendered = self.render(translations, language, params);
        Mail::new(to, rendered.subject, rendered.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_rieng_chi_trong_danh_sach_va_mot_dong() {
        let mut mail = Mail { to: "a@example.com".to_owned(), ..Mail::default() };
        mail.headers = vec![
            ("list-unsubscribe".to_owned(), "<https://x/u>".to_owned()),
            ("Bcc".to_owned(), "eve@example.com".to_owned()),
            ("List-Id".to_owned(), "News\r\nBcc: eve@example.com".to_owned()),
        ];
        let problems: Vec<String> = mail.header_problems().into_iter().map(|p| p.field).collect();
        assert_eq!(problems, ["headers[1].name", "headers[2].value"]);
    }

    const RESET: MailTemplate = MailTemplate::new(
        "auth.mail.reset",
        "Reset your password",
        "Open {link} within {minutes} minutes.",
    );

    fn translations() -> Translations {
        Translations::new(vec![(
            "auth",
            vec![(
                "vi",
                r#"{"auth.mail.reset.subject": "Đặt lại mật khẩu",
                    "auth.mail.reset.text": "Mở {link} trong {minutes} phút."}"#,
            )],
        )])
        .unwrap()
    }

    #[test]
    fn dien_mau_theo_ngon_ngu() {
        let params = [("link", "https://x/reset?t=1"), ("minutes", "30")];

        let vi = RESET.render(&translations(), Language::Vi, &params);
        let en = RESET.render(&translations(), Language::En, &params);

        assert_eq!(vi.subject, "Đặt lại mật khẩu");
        assert_eq!(vi.text, "Mở https://x/reset?t=1 trong 30 phút.");
        assert_eq!(en.subject, "Reset your password");
        assert_eq!(en.text, "Open https://x/reset?t=1 within 30 minutes.");
        let mail = RESET.mail("ada@example.com", &translations(), Language::Vi, &params);
        assert_eq!(mail.to, "ada@example.com");
    }

    #[test]
    fn thieu_ban_dich_hay_thieu_gia_tri_thi_dung_tieng_anh() {
        let empty = Translations::new(Vec::new()).unwrap();
        let only_link = [("link", "L")];

        assert_eq!(
            RESET.render(&empty, Language::Vi, &[("link", "L"), ("minutes", "5")]).subject,
            "Reset your password"
        );
        // Thiếu giá trị `minutes`: bản dịch không dùng được, bản tiếng Anh cũng thiếu —
        // trả nguyên mẫu tiếng Anh thay vì bịa.
        assert_eq!(
            RESET.render(&translations(), Language::Vi, &only_link).text,
            "Open {link} within {minutes} minutes."
        );
    }
}
