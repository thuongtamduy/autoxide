//! Dựng mail: mail xác nhận đăng ký và mail chiến dịch.
//!
//! - Mail gửi cho người lạ (xác nhận) **không chứa chữ do người đó gõ** — kể cả tên — để API
//!   không thành công cụ gửi thư rác tới hộp thư người khác (hợp đồng chung §5).
//! - Mỗi mail chiến dịch có `List-Unsubscribe: <https://…/unsubscribe?token=…>` và
//!   `List-Unsubscribe-Post: List-Unsubscribe=One-Click` (RFC 8058), chân thư có link tới trang
//!   lựa chọn của website và địa chỉ người gửi; thư quảng cáo có nhãn đầu tiêu đề.

use autoxide_kernel::error::Error;
use autoxide_kernel::mail::{Mail, MailTemplate};

use crate::audience::lists::List;
use crate::campaigns::CampaignFormat;
use crate::entity::campaign;
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
    /// Mail xác nhận gửi tới `to`: tên các danh sách (do quản trị viên đặt) và link — không có
    /// chữ nào do người đăng ký gõ.
    pub(crate) async fn confirmation_mail(
        &self,
        tenant: &autoxide_kernel::tenancy::TenantContext,
        to: &str,
        locale: Locale,
        lists: &[&List],
        token: &str,
        token_id: uuid::Uuid,
    ) -> Result<Mail, Error> {
        let link = token_link(&self.confirm_page(tenant).await?, token);
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

    /// Mail chiến dịch `row` của danh sách `list` gửi tới `to` (email, tên), với token huỷ đăng ký
    /// `token`. `delivery_id`: khoá chống trùng (`None` cho mail thử). Chiến dịch `template`: dựng
    /// riêng cho người nhận này bằng bộ dựng mẫu của kernel.
    pub(crate) async fn campaign_mail(
        &self,
        row: &campaign::Model,
        list: &List,
        (to, name): (&str, Option<&str>),
        pages: &crate::tenant_config::CampaignPages,
        token: &str,
        delivery_id: Option<uuid::Uuid>,
    ) -> Result<Mail, Error> {
        let unsubscribe = self.public_url.link(&format!("{UNSUBSCRIBE_PATH}?token={token}"))?;
        let preferences = token_link(&pages.preferences, token);
        let locale = Locale::stored(&row.locale);
        let address = pages.address.as_str();
        let params =
            [("list", list.name.get(locale)), ("link", preferences.as_str()), ("address", address)];
        let footer = FOOTER.render(&self.translations, locale.language(), &params).text;
        let footer = footer.trim_end();
        let (subject, body_text, body_html) = match CampaignFormat::stored(&row.format) {
            CampaignFormat::Static => {
                (row.subject.clone(), row.body_text.clone(), row.body_html.clone())
            }
            CampaignFormat::Template => {
                use autoxide_kernel::template::Output;
                let context = serde_json::json!({
                    "subscriber": {"name": name.unwrap_or_default(), "email": mask_email(to)},
                    "list": {"name": list.name.get(locale)},
                    "unsubscribe_url": preferences,
                });
                let engine = &self.templates;
                let tag = locale.as_str();
                let html = match &row.body_html {
                    Some(source) => Some(engine.render(source, Output::Html, tag, &context)?),
                    None => None,
                };
                (
                    engine.render(&row.subject, Output::Text, tag, &context)?,
                    engine.render(&row.body_text, Output::Text, tag, &context)?,
                    html,
                )
            }
        };
        let label = &self.config.advertising_label;
        let subject = if row.advertising && !label.is_empty() {
            format!("{label} {subject}")
        } else {
            subject
        };
        let text = format!("{}\n\n-- \n{footer}\n", body_text.trim_end());
        let html = body_html.as_ref().map(|body| {
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

/// Email đã che bớt cho mẫu: `a***@example.com`.
pub(crate) fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => {
            let first: String = local.chars().take(1).collect();
            format!("{first}***@{domain}")
        }
        None => "***".to_owned(),
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

/// Thư của một chiến dịch đúng như người nhận thấy (tiêu đề có nhãn, chân thư).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Preview {
    pub subject: String,
    pub text: String,
    pub html: Option<String>,
}

/// Cấu hình đang dùng của module, cho trang cài đặt (không có secret).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Settings {
    pub advertising_label: String,
    pub advertising_max_per_day: u32,
    pub postal_address: Option<String>,
    pub public_url_set: bool,
    pub preferences_url_set: bool,
    pub confirm_url_set: bool,
    pub send_rate_per_minute: u32,
}

/// Token giữ chỗ trong link của bản xem trước: không phải token thật, link không làm gì.
const PREVIEW_TOKEN: &str = "preview";

impl NewsletterService {
    /// Thư của chiến dịch `id` như khi gửi (cùng chân thư, cùng nhãn quảng cáo), với token giữ chỗ
    /// trong link. Cần `newsletter.read`. Chưa đủ cấu hình để gửi thì báo đúng lỗi như khi gửi.
    pub(crate) async fn preview(
        &self,
        tenant: &autoxide_kernel::tenancy::TenantContext,
        principal: &autoxide_kernel::access::Principal,
        id: uuid::Uuid,
    ) -> Result<Preview, Error> {
        principal.require(&crate::permissions::NEWSLETTER_READ)?;
        let campaign = self.find_campaign(tenant, id).await?;
        let list = self.find_list(tenant, campaign.row.list_id).await?;
        let pages = self.campaign_pages(tenant, &campaign.row).await?;
        let mail = self
            .campaign_mail(
                &campaign.row,
                &list,
                ("subscriber@example.invalid", Some("Subscriber")),
                &pages,
                PREVIEW_TOKEN,
                None,
            )
            .await?;
        Ok(Preview { subject: mail.subject, text: mail.text, html: mail.html })
    }

    /// Cấu hình đang dùng ở khách `tenant` (cấu hình khách thay biến chung). Cần `newsletter.read`.
    pub(crate) async fn settings(
        &self,
        tenant: &autoxide_kernel::tenancy::TenantContext,
        principal: &autoxide_kernel::access::Principal,
    ) -> Result<Settings, Error> {
        principal.require(&crate::permissions::NEWSLETTER_READ)?;
        Ok(Settings {
            advertising_label: self.config.advertising_label.clone(),
            advertising_max_per_day: self.config.advertising_max_per_day,
            postal_address: self.postal_address(tenant).await?,
            public_url_set: self.public_url.base().is_ok(),
            preferences_url_set: is_set(self.preferences_page(tenant).await)?,
            confirm_url_set: is_set(self.confirm_page(tenant).await)?,
            send_rate_per_minute: self.config.send_rate_per_minute,
        })
    }
}

/// Trang đã đặt chưa: lỗi "chưa đặt" (503) là `false`; lỗi khác (database…) truyền lên, không đọc thành "chưa đặt".
fn is_set(page: Result<String, Error>) -> Result<bool, Error> {
    match page {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == autoxide_kernel::error::ErrorKind::Unavailable => Ok(false),
        Err(error) => Err(error),
    }
}
