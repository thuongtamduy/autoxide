//! Email trả lời vào lại phiếu (tắt mặc định, bật theo nút bằng `inbound_email` của chính sách).
//! Bộ nhận thư thật (webhook của nhà cung cấp email) là module riêng, gọi `dyn InboundMail` của
//! `support` (D-191). Địa chỉ nhận mang khoá của phiếu: `support+<id phiếu>.<chữ ký>@…`, chữ ký là
//! HMAC theo khách nên không đoán được. Thư không khớp phiếu nào, từ người khác người hỏi, hay vào
//! phiếu đã đóng thì bỏ — không bao giờ tạo phiếu mới từ thư (chống spam).

use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::secret::sha256;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::EntityTrait;
use uuid::Uuid;

use crate::api::{InboundEmail, InboundMail, InboundOutcome};
use crate::entity::ticket;
use crate::messages::NewMessage;
use crate::service::{SupportService, Viewer};

impl SupportService {
    /// Khoá trả lời của phiếu `id`: phần sau `support+` của địa chỉ nhận.
    pub(crate) async fn reply_key(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<String, Error> {
        let pepper = self.parts.secret.derive_for(tenant.id(), "support.inbound").await?;
        let pepper: String = pepper.iter().map(|byte| format!("{byte:02x}")).collect();
        let signature = sha256(&format!("{pepper}:{id}"));
        Ok(format!("{id}.{}", &signature[..24]))
    }

    async fn ticket_of(
        &self,
        tenant: &TenantContext,
        recipient: &str,
    ) -> Result<Option<Uuid>, Error> {
        let local = recipient.trim().trim_matches(['<', '>']).split('@').next().unwrap_or_default();
        let Some(key) = local.strip_prefix("support+") else {
            return Ok(None);
        };
        let Some(id) = key.split('.').next().and_then(|raw| raw.parse::<Uuid>().ok()) else {
            return Ok(None);
        };
        Ok((self.reply_key(tenant, id).await? == key).then_some(id))
    }
}

#[async_trait::async_trait]
impl InboundMail for SupportService {
    async fn reply_address_key(
        &self,
        tenant: &TenantContext,
        ticket_id: Uuid,
    ) -> Result<String, Error> {
        self.reply_key(tenant, ticket_id).await
    }

    async fn receive(
        &self,
        tenant: &TenantContext,
        mail: InboundEmail,
    ) -> Result<InboundOutcome, Error> {
        let Some(id) = self.ticket_of(tenant, &mail.recipient).await? else {
            return Ok(InboundOutcome::Dropped);
        };
        let Some(row) =
            ticket::Entity::find_by_id(id).scoped(tenant).one(tenant.database().read()).await?
        else {
            return Ok(InboundOutcome::Dropped);
        };
        if !self.policy(tenant, row.org_id).await?.policy.inbound_email {
            return Ok(InboundOutcome::Dropped);
        }
        // Chỉ người hỏi (email của phiếu) trả lời được; phiếu của tài khoản không có email lưu thì
        // không nhận qua thư.
        let sender = autoxide_kernel::email::normalize(mail.sender.trim().trim_matches(['<', '>']));
        if row.requester_email.as_deref() != Some(sender.as_str()) {
            return Ok(InboundOutcome::Dropped);
        }
        let body = strip_quoted(&mail.text);
        if body.is_empty() {
            return Ok(InboundOutcome::Dropped);
        }
        let viewer = match row.requester_user_id {
            Some(user) => Viewer::Customer(user),
            None => Viewer::Guest,
        };
        let new = NewMessage { body, ..NewMessage::default() };
        match self.add_message(tenant, viewer, id, new).await {
            Ok(_) => Ok(InboundOutcome::Added { ticket_id: id }),
            // Phiếu đã đóng, chưa xác thực…: bỏ thư, không báo lỗi cho người gửi lạ.
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::Conflict
                        | ErrorKind::Forbidden
                        | ErrorKind::NotFound
                        | ErrorKind::Validation
                ) =>
            {
                Ok(InboundOutcome::Dropped)
            }
            Err(error) => Err(error),
        }
    }
}

/// Cắt phần trích dẫn thư cũ: bỏ dòng bắt đầu bằng `>`, và mọi thứ từ dòng "On … wrote:" /
/// "Vào … đã viết:" / "-----Original Message-----" trở đi.
pub(crate) fn strip_quoted(text: &str) -> String {
    let mut kept = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();
        if (lower.starts_with("on ") && lower.ends_with("wrote:"))
            || (lower.starts_with("vào ") && lower.ends_with("đã viết:"))
            || lower.starts_with("-----original message-----")
        {
            break;
        }
        if trimmed.starts_with('>') {
            continue;
        }
        kept.push(line);
    }
    kept.join("\n").trim().to_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn cat_trich_dan_thu_cu() {
        let mail = "Cảm ơn, tôi đã nhận.\n\nVào Thứ Hai, Shop <a@b.co> đã viết:\n> Đơn đã giao";
        assert_eq!(super::strip_quoted(mail), "Cảm ơn, tôi đã nhận.");
        assert_eq!(super::strip_quoted("> chỉ trích dẫn"), "");
    }
}
