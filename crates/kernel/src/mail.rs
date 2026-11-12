//! Gửi mail: trait chung để module nào cũng gửi được mà không phụ thuộc cách gửi.
//!
//! Một module (`mail`) công bố `dyn Mailer`; module cần gửi mail lấy nó bằng
//! `context.require::<dyn Mailer>()` (đăng ký SAU module `mail`). `send` chỉ đưa mail vào
//! hàng đợi rồi trả về ngay: việc gửi chạy nền, lỗi tạm thời thì thử lại (D-061). Máy chủ nhận
//! từ chối hẳn CHÍNH ĐỊA CHỈ NGƯỜI NHẬN (SMTP 5xx với mã mở rộng `5.1.x` / `5.2.x`: không có hộp
//! thư…) thì không thử lại mà phát event [`DELIVERY_FAILED`] (D-121, D-158). Lỗi vĩnh viễn khác
//! là lỗi bên gửi: thử lại rồi vào dead-letter queue.

use crate::error::{Error, FieldError};
use crate::i18n::{Language, Translations};

/// Event phát khi một mail bị máy chủ nhận từ chối hẳn. Payload:
/// `{"to": "<địa chỉ>", "dedupe_key": "<khoá của bên gửi>" | null, "code": "<mã SMTP>"}`. Module
/// quản lý danh sách gửi (vd bản tin) nghe nó để ngừng gửi tới địa chỉ đó.
pub const DELIVERY_FAILED: &str = "mail.delivery_failed";

/// Header riêng được phép đặt (so sánh không phân biệt hoa thường). Header khác do bên gửi
/// mail tự đặt (`From`, `To`, `Subject`, `Message-ID`…).
pub const ALLOWED_HEADERS: [&str; 4] =
    ["List-Unsubscribe", "List-Unsubscribe-Post", "List-Id", "Auto-Submitted"];

/// Một mail cần gửi. Dựng bằng `Mail { to, subject, text, ..Mail::default() }`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Mail {
    /// Địa chỉ người nhận (`ada@example.com` hoặc `Ada <ada@example.com>`).
    pub to: String,
    pub subject: String,
    /// Nội dung dạng chữ (luôn có: mọi ứng dụng mail đọc được).
    pub text: String,
    /// Nội dung HTML, nếu có.
    pub html: Option<String>,
    /// Trả lời thì tới địa chỉ này (vd người gửi form), thay vì địa chỉ gửi.
    pub reply_to: Option<String>,
    /// Header riêng, chỉ trong [`ALLOWED_HEADERS`] (vd `List-Unsubscribe` của bản tin).
    pub headers: Vec<(String, String)>,
    /// Khoá chống gửi trùng: mail cùng khoá đã vào hàng đợi thì mail sau bị bỏ qua (vd ID của
    /// lượt gửi bản tin, để chạy lại không gửi hai lần). Có trong event [`DELIVERY_FAILED`].
    pub dedupe_key: Option<String>,
}

impl Mail {
    /// Lỗi của các header riêng: tên ngoài [`ALLOWED_HEADERS`], giá trị có ký tự xuống dòng
    /// (chèn header giả) hay dài quá 998 ký tự (giới hạn một dòng của RFC 5322).
    pub fn header_problems(&self) -> Vec<FieldError> {
        let mut problems = Vec::new();
        for (index, (name, value)) in self.headers.iter().enumerate() {
            if !ALLOWED_HEADERS.iter().any(|allowed| allowed.eq_ignore_ascii_case(name)) {
                problems.push(FieldError::new(
                    format!("headers[{index}].name"),
                    "not_allowed",
                    format!("must be one of {}", ALLOWED_HEADERS.join(", ")),
                ));
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
        Mail {
            to: to.to_owned(),
            subject: rendered.subject,
            text: rendered.text,
            ..Mail::default()
        }
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
