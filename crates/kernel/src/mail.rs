//! Gửi mail: trait chung để module nào cũng gửi được mà không phụ thuộc cách gửi.
//!
//! Một module (`mail`) công bố `dyn Mailer`; module cần gửi mail lấy nó bằng
//! `context.require::<dyn Mailer>()` (đăng ký SAU module `mail`). `send` chỉ đưa mail vào
//! hàng đợi rồi trả về ngay: việc gửi chạy nền, lỗi thì thử lại (D-061).

use crate::error::Error;
use crate::i18n::{Language, Translations};

/// Một mail cần gửi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    /// Địa chỉ người nhận (`ada@example.com` hoặc `Ada <ada@example.com>`).
    pub to: String,
    pub subject: String,
    /// Nội dung dạng chữ (luôn có: mọi ứng dụng mail đọc được).
    pub text: String,
    /// Nội dung HTML, nếu có.
    pub html: Option<String>,
}

/// Bên gửi mail của chương trình.
#[async_trait::async_trait]
pub trait Mailer: Send + Sync {
    /// Đưa `mail` vào hàng đợi gửi. Lỗi khi địa chỉ người nhận sai dạng
    /// (`validation_failed`, trường `to`) hay không ghi được hàng đợi.
    async fn send(&self, mail: Mail) -> Result<(), Error>;
}

/// Mẫu mail: tiêu đề và nội dung tiếng Anh nằm trong code (có chỗ điền `{tên}`), bản
/// dịch nằm trong `locales/<ngôn ngữ>.json` của crate với khoá `<key>.subject` và
/// `<key>.text` (`cargo xtask check` kiểm đủ).
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

/// Tiêu đề và nội dung đã điền, bằng một ngôn ngữ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub subject: String,
    pub text: String,
}

impl MailTemplate {
    /// `key`: `<module>.mail.<tên>`.
    pub const fn new(key: &'static str, subject: &'static str, text: &'static str) -> Self {
        MailTemplate { key, subject, text }
    }

    pub fn key(&self) -> &'static str {
        self.key
    }

    /// Điền `params` vào mẫu bằng `language`. Phần nào chưa có bản dịch (hay bản dịch
    /// dùng chỗ điền không có giá trị) thì dùng tiếng Anh.
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

    /// Dựng [`Mail`] gửi tới `to`.
    pub fn mail(
        &self,
        to: &str,
        translations: &Translations,
        language: Language,
        params: &[(&str, &str)],
    ) -> Mail {
        let rendered = self.render(translations, language, params);
        Mail { to: to.to_owned(), subject: rendered.subject, text: rendered.text, html: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
