//! Dựng mail: mail xác nhận đăng ký và mail chiến dịch.
//!
//! - Mail gửi cho người lạ (xác nhận) **không chứa chữ do người đó gõ** — kể cả tên — để API
//!   không thành công cụ gửi thư rác tới hộp thư người khác (hợp đồng chung §5).
//! - Mỗi mail chiến dịch có `List-Unsubscribe: <https://…/unsubscribe?token=…>` và
//!   `List-Unsubscribe-Post: List-Unsubscribe=One-Click` (RFC 8058), chân thư có link tới trang
//!   lựa chọn của website và địa chỉ người gửi; thư quảng cáo có nhãn đầu tiêu đề.

use autoxide_kernel::error::Error;
use autoxide_kernel::mail::{Mail, MailTemplate};

use crate::entity::campaign;
use crate::lists::List;
use crate::service::NewsletterService;
use crate::text::{Locale, token_link};

pub(crate) const CONFIRM_MAIL: MailTemplate = MailTemplate::new(
    "newsletter.mail.confirm",
    "Confirm your subscription",
    "Someone, probably you, asked to receive these newsletters at this address:\n\n{lists}\n\nTo \
     confirm, open this link within {hours} hours:\n{link}\n\nIf you did not ask for this, \
     ignore this mail: nothing will be sent to you.\n",
);

pub(crate) const FOOTER: MailTemplate = MailTemplate::new(
    "newsletter.mail.footer",
    "Newsletter",
    "You receive this mail because you subscribed to \"{list}\".\nUnsubscribe or choose your \
     newsletters: {link}\n{address}",
);

pub(crate) const NEW_POST: MailTemplate =
    MailTemplate::new("newsletter.mail.new_post", "{title}", "{summary}\n\nRead it here: {url}\n");

/// Đường dẫn (từ gốc API) của endpoint huỷ một chạm.
pub(crate) const UNSUBSCRIBE_PATH: &str = "/api/v1/newsletter/public/unsubscribe";

impl NewsletterService {
    /// URL trang lựa chọn của website (có `{token}`), hoặc lỗi nói rõ phải đặt biến nào.
    pub(crate) fn preferences_url(&self) -> Result<&str, Error> {
        self.config.preferences_url.as_deref().ok_or_else(|| {
            Error::unavailable(
                "newsletter.preferences_url_missing",
                "Campaign mails link to your website's newsletter preferences page: set \
                 AUTOXIDE_NEWSLETTER_PREFERENCES_URL (for example \
                 `https://www.example.com/newsletter/preferences?token={token}`) and restart.",
            )
        })
    }

    /// URL trang xác nhận của website (có `{token}`), hoặc lỗi nói rõ phải đặt biến nào.
    pub(crate) fn confirm_url(&self) -> Result<&str, Error> {
        self.config.confirm_url.as_deref().ok_or_else(|| {
            Error::unavailable(
                "newsletter.confirm_url_missing",
                "Lists with double opt-in send a confirmation link to your website: set \
                 AUTOXIDE_NEWSLETTER_CONFIRM_URL (for example \
                 `https://www.example.com/newsletter/confirm?token={token}`) and restart.",
            )
        })
    }

    /// Mail xác nhận gửi tới `to`: tên các danh sách (do quản trị viên đặt) và link — không có
    /// chữ nào do người đăng ký gõ.
    pub(crate) fn confirmation_mail(
        &self,
        to: &str,
        locale: Locale,
        lists: &[&List],
        token: &str,
        token_id: uuid::Uuid,
    ) -> Result<Mail, Error> {
        let link = token_link(self.confirm_url()?, token);
        let names: Vec<String> =
            lists.iter().map(|list| format!("- {}", list.name.get(locale))).collect();
        let names = names.join("\n");
        let hours = self.config.confirm_hours.to_string();
        let params =
            [("lists", names.as_str()), ("hours", hours.as_str()), ("link", link.as_str())];
        let mut mail = CONFIRM_MAIL.mail(to, &self.translations, locale.language(), &params);
        mail.dedupe_key = Some(format!("newsletter.confirm:{token_id}"));
        Ok(mail)
    }

    /// Mail chiến dịch `row` của danh sách `list` gửi tới `to`, với token huỷ đăng ký `token`.
    /// `delivery_id`: khoá chống trùng (`None` cho mail thử).
    pub(crate) fn campaign_mail(
        &self,
        row: &campaign::Model,
        list: &List,
        to: &str,
        token: &str,
        delivery_id: Option<uuid::Uuid>,
    ) -> Result<Mail, Error> {
        self.check_ready(row)?;
        let unsubscribe = self.public_url.link(&format!("{UNSUBSCRIBE_PATH}?token={token}"))?;
        let preferences = token_link(self.preferences_url()?, token);
        let locale = Locale::stored(&row.locale);
        let address = self.config.postal_address.clone().unwrap_or_default();
        let params = [
            ("list", list.name.get(locale)),
            ("link", preferences.as_str()),
            ("address", address.as_str()),
        ];
        let footer = FOOTER.render(&self.translations, locale.language(), &params).text;
        let footer = footer.trim_end();
        let label = &self.config.advertising_label;
        let subject = if row.advertising && !label.is_empty() {
            format!("{label} {}", row.subject)
        } else {
            row.subject.clone()
        };
        let text = format!("{}\n\n-- \n{footer}\n", row.body_text.trim_end());
        let html = row.body_html.as_ref().map(|body| {
            let preheader = row.preheader.as_deref().map_or_else(String::new, |preheader| {
                format!(
                    "<div style=\"display:none;max-height:0;overflow:hidden\">{}</div>",
                    escape(preheader)
                )
            });
            let escaped_link = escape(&preferences);
            let footer = escape(footer)
                .replace(&escaped_link, &format!("<a href=\"{escaped_link}\">{escaped_link}</a>"))
                .replace('\n', "<br>");
            format!(
                "{preheader}{body}\n<hr>\n<p style=\"font-size:12px;color:#666666\">{footer}</p>\n"
            )
        });
        let mut mail = Mail::new(to, subject, text)
            .header("List-Unsubscribe", format!("<{unsubscribe}>"))
            .header("List-Unsubscribe-Post", "List-Unsubscribe=One-Click");
        if let Some(html) = html {
            mail = mail.html(html);
        }
        if let Some(id) = delivery_id {
            mail = mail.dedupe_key(delivery_key(id));
        }
        Ok(mail)
    }
}

/// Khoá chống trùng của lượt gửi `id` (có trong event `mail.delivery_failed`).
pub(crate) fn delivery_key(id: uuid::Uuid) -> String {
    format!("newsletter.delivery:{id}")
}

/// Thoát ký tự đặc biệt của HTML.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(c),
        }
    }
    escaped
}
