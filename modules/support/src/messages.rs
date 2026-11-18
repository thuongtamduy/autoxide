//! Tin nhắn của phiếu: chỉ thêm, không sửa. Sửa nhầm thì nhân viên ẩn tin kèm lý do (audit). Ghi
//! chú nội bộ (`internal`) chỉ nhân viên thấy; người được nhắc tên nhận thông báo (qua event).

use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{message, ticket};
use crate::events::{self, Change};
use crate::permissions::{TICKETS_MANAGE, TICKETS_REPLY};
use crate::service::{Message, SupportService, Ticket, Viewer, finish, not_found, now, target};
use crate::status::{self, AuthorKind, MessageKind, Status};
use crate::tickets::{apply_status, message_snapshot, parse_status};

/// Số tệp đính kèm tối đa của một tin.
pub(crate) const ATTACHMENTS_MAX: usize = 5;
/// Số người được nhắc tên tối đa trong một ghi chú.
pub(crate) const MENTIONS_MAX: usize = 20;

/// Một tin cần thêm.
#[derive(Debug, Clone, Default)]
pub(crate) struct NewMessage {
    pub body: String,
    /// `true`: ghi chú nội bộ (chỉ nhân viên).
    pub internal: bool,
    pub media_ids: Vec<Uuid>,
    /// Đồng nghiệp được nhắc tên trong ghi chú nội bộ.
    pub mentions: Vec<Uuid>,
    /// Nhân viên gửi bản nháp do AI gợi ý (bước 37 gắn nhãn, D-109).
    pub ai_assisted: bool,
}

impl SupportService {
    /// Thêm tin vào phiếu `ticket_id`. Người mua / khách vãng lai chỉ viết tin công khai; khách vãng
    /// lai chưa xác thực OTP không viết được tin thứ hai (`support.verification_required`). Tin công
    /// khai đổi trạng thái theo máy trạng thái; phiếu đã đóng thì `support.ticket_closed`.
    pub(crate) async fn add_message(
        &self,
        tenant: &TenantContext,
        viewer: Viewer<'_>,
        ticket_id: Uuid,
        input: NewMessage,
    ) -> Result<(Ticket, Message), Error> {
        let (actor, author, uploader) = match viewer {
            Viewer::Staff(principal) => (Actor::from(principal), AuthorKind::Agent, None),
            Viewer::Customer(user) => (Actor::user(user), AuthorKind::Customer, Some(user)),
            Viewer::Guest => (Actor::public(), AuthorKind::Customer, None),
        };
        let mut problems = Vec::new();
        let body = self.check_body("body", &input.body, &mut problems);
        if input.internal && !viewer.is_staff() {
            problems.push(FieldError::new(
                "internal",
                "staff_only",
                "only staff write internal notes",
            ));
        }
        if input.mentions.len() > MENTIONS_MAX || (!input.internal && !input.mentions.is_empty()) {
            problems.push(
                FieldError::new(
                    "mentions",
                    "invalid",
                    "at most {max} people, in internal notes only",
                )
                .with_param("max", MENTIONS_MAX),
            );
        }
        finish(problems)?;
        let owner = if viewer.is_staff() { None } else { uploader };
        self.check_attachments_for(tenant, viewer, owner, &input.media_ids).await?;
        // Phiếu có thật và người gọi thấy được (404 nếu không).
        self.get(tenant, viewer, ticket_id).await?;

        let at = now();
        let transaction = tenant.database().begin_write().await?;
        let before = ticket::Entity::find_by_id(ticket_id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(ticket_id))?;
        if let Viewer::Staff(principal) = viewer {
            principal.require_write_at(&transaction, &TICKETS_REPLY, target(&before)).await?;
        }
        let from = parse_status(&before.status)?;
        if from == Status::Closed {
            return Err(status::ticket_closed());
        }
        if matches!(viewer, Viewer::Guest) && before.verified_at.is_none() {
            return Err(verification_required());
        }
        let kind = if input.internal { MessageKind::Internal } else { MessageKind::Public };
        let mut row = before.clone().into_active_model();
        if kind == MessageKind::Public {
            let to = status::after_message(from, author);
            if to != from {
                apply_status(&mut row, &before, to, None, at);
            }
            if author == AuthorKind::Agent && before.first_responded_at.is_none() {
                row.first_responded_at = Set(Some(at));
            }
            if author == AuthorKind::Customer {
                // Người mua viết: lượt trả lời tự động liên tiếp đếm lại từ đầu (D-230).
                row.auto_replies_in_row = Set(0);
            }
            row.last_message_at = Set(at);
        }
        row.updated_at = Set(at);
        let after =
            ticket::Entity::update(row).validate()?.scoped(tenant).exec(&transaction).await?;
        let author_user = match viewer {
            Viewer::Staff(principal) => Some(principal.user_id()),
            Viewer::Customer(user) => Some(user),
            Viewer::Guest => None,
        };
        let message = message::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            ticket_id: Set(ticket_id),
            kind: Set(kind.as_str().to_owned()),
            author_kind: Set(author.as_str().to_owned()),
            author_user_id: Set(author_user),
            body: Set(body),
            media_ids: Set(json!(input.media_ids).to_string()),
            mentions: Set(json!(input.mentions).to_string()),
            hidden_at: Set(None),
            hidden_by: Set(None),
            hidden_reason: Set(None),
            ai_assisted: Set(input.ai_assisted && viewer.is_staff()),
            auto_reply: Set(false),
            created_at: Set(at),
        }
        .insert(&transaction)
        .await?;
        let subject = events::ticket_subject(ticket_id);
        let mut added = message_snapshot(&message);
        added["mentions"] = json!(input.mentions);
        let event = events::MessageAdded(Change { before: None, after: Some(added) });
        self.parts.outbox.record(&transaction, tenant, actor, subject, &event).await?;
        self.record_changes(&transaction, tenant, actor, &before, &after, false).await?;
        transaction.commit().await?;
        Ok((after, message))
    }

    /// Tin của phiếu theo thứ tự viết. Người mua không thấy ghi chú nội bộ; tin bị ẩn chỉ còn dấu
    /// "đã ẩn" (nhân viên vẫn thấy nội dung và lý do).
    pub(crate) async fn messages(
        &self,
        tenant: &TenantContext,
        viewer: Viewer<'_>,
        ticket: &Ticket,
    ) -> Result<Vec<Message>, Error> {
        let mut select = message::Entity::find()
            .filter(message::Column::TicketId.eq(ticket.id))
            .scoped(tenant)
            .order_by_asc(message::Column::Id);
        if !viewer.is_staff() {
            select = select.filter(message::Column::Kind.ne(MessageKind::Internal.as_str()));
        }
        let mut rows = select.all(tenant.database().read()).await?;
        if !viewer.is_staff() {
            for row in &mut rows {
                if row.hidden_at.is_some() {
                    row.body = String::new();
                    row.media_ids = "[]".to_owned();
                    row.hidden_reason = None;
                    row.hidden_by = None;
                }
            }
        }
        Ok(rows)
    }

    /// Ẩn tin `message_id` (không xoá, không sửa): `support.tickets.manage` ở nút của phiếu, lý do
    /// bắt buộc. Ẩn lại tin đã ẩn thì giữ lý do cũ.
    pub(crate) async fn hide_message(
        &self,
        tenant: &TenantContext,
        principal: &autoxide_kernel::access::Principal,
        ticket_id: Uuid,
        message_id: Uuid,
        reason: &str,
    ) -> Result<Message, Error> {
        let mut problems = Vec::new();
        let reason = crate::service::required_text("reason", reason, 500, &mut problems);
        finish(problems)?;
        self.get(tenant, Viewer::Staff(principal), ticket_id).await?;
        let transaction = tenant.database().begin_write().await?;
        let ticket = ticket::Entity::find_by_id(ticket_id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(ticket_id))?;
        principal.require_write_at(&transaction, &TICKETS_MANAGE, target(&ticket)).await?;
        let before = message::Entity::find_by_id(message_id)
            .filter(message::Column::TicketId.eq(ticket_id))
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| message_not_found(message_id))?;
        if before.hidden_at.is_some() {
            transaction.rollback().await?;
            return Ok(before);
        }
        let mut row = before.clone().into_active_model();
        row.hidden_at = Set(Some(now()));
        row.hidden_by = Set(Some(principal.user_id()));
        row.hidden_reason = Set(Some(reason.clone()));
        let after =
            message::Entity::update(row).validate()?.scoped(tenant).exec(&transaction).await?;
        let mut snapshot = message_snapshot(&after);
        snapshot["hidden_reason"] = json!(reason);
        let event = events::MessageHidden(Change {
            before: Some(message_snapshot(&before)),
            after: Some(snapshot),
        });
        let actor = Actor::from(principal);
        let subject = events::ticket_subject(ticket_id);
        self.parts.outbox.record(&transaction, tenant, actor, subject, &event).await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Kiểm tệp đính kèm: tối đa [`ATTACHMENTS_MAX`], item có thật trong khách, và — với người
    /// mua — do chính họ tải lên (D-202). Khách vãng lai không gửi tệp (không có tài khoản để tải).
    pub(crate) async fn check_attachments_for(
        &self,
        tenant: &TenantContext,
        viewer: Viewer<'_>,
        owner: Option<Uuid>,
        ids: &[Uuid],
    ) -> Result<(), Error> {
        if !ids.is_empty() && matches!(viewer, Viewer::Guest) {
            return Err(attachment_not_owned());
        }
        self.check_attachments(tenant, owner, ids).await
    }

    /// Như [`check_attachments_for`](Self::check_attachments_for); `owner`: người phải là người
    /// tải lên (`None`: nhân viên — mọi item của khách).
    pub(crate) async fn check_attachments(
        &self,
        tenant: &TenantContext,
        owner: Option<Uuid>,
        ids: &[Uuid],
    ) -> Result<(), Error> {
        if ids.is_empty() {
            return Ok(());
        }
        if ids.len() > ATTACHMENTS_MAX {
            return Err(Error::validation(vec![
                FieldError::new("media_ids", "too_many", "at most {max} files per message")
                    .with_param("max", ATTACHMENTS_MAX),
            ]));
        }
        let Some(media) = &self.parts.media else {
            return Err(attachment_not_owned());
        };
        let found = media.uploaders(tenant, ids).await?;
        let owned = ids.iter().all(|id| {
            found.iter().any(|(known, uploader)| {
                known == id && owner.is_none_or(|owner| *uploader == Some(owner))
            })
        });
        if owned { Ok(()) } else { Err(attachment_not_owned()) }
    }
}

pub(crate) fn verification_required() -> Error {
    Error::new(
        ErrorKind::Forbidden,
        "support.verification_required",
        "Confirm your email or phone number with the code we sent before writing again: \
         `POST /api/v1/support/public/tickets/{id}/verify`.",
    )
}

pub(crate) fn attachment_not_owned() -> Error {
    Error::new(
        ErrorKind::Validation,
        "support.attachment_not_owned",
        "Attach only media items you uploaded yourself; guests cannot attach files.",
    )
    .with_field(FieldError::new("media_ids", "not_owned", "attach only files you uploaded"))
}

pub(crate) fn message_not_found(id: Uuid) -> Error {
    Error::not_found("support.message_not_found", "This ticket has no message `{id}`.")
        .with_param("id", id)
}
