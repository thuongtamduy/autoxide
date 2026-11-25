//! Bước 53 mục 4: gắn phiếu với bản ghi khác (sổ đối tượng, D-123), câu trả lời mẫu có biến, chuyển
//! phiếu thành khiếu nại người tiêu dùng của `compliance` hay lên sàn, mức hài lòng sau khi xử lý.

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, SortValue, fetch_page_by};
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{link, saved_reply, ticket};
use crate::events::{self, Change};
use crate::permissions::{SETTINGS_MANAGE, TICKETS_MANAGE, TICKETS_READ, TICKETS_REPLY};
use crate::service::{
    SupportService, Ticket, Viewer, finish, not_found, now, required_text, target,
};
use crate::status::{Resolution, Status};
use crate::tickets::{parse_status, snapshot};

pub(crate) type SavedReply = saved_reply::Model;

/// Where to hand a ticket over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportEscalation)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Escalation {
    /// A consumer complaint with legal deadlines, handled in module `compliance`; the ticket closes
    /// as `escalated`.
    ConsumerComplaint,
    /// The marketplace operator (for tickets of a seller's store); the ticket stays open.
    Platform,
}

/// Một câu trả lời mẫu cần ghi.
#[derive(Debug, Clone)]
pub(crate) struct SavedReplyInput {
    pub org_id: Uuid,
    pub name: String,
    /// Nội dung theo ngôn ngữ (`vi`, `en`…), có biến `{customer_name}`, `{ticket_number}`,
    /// `{subject}`.
    pub bodies: std::collections::BTreeMap<String, String>,
}

impl SupportService {
    /// Liên kết của phiếu (`<kind>:<id>`), theo thứ tự gắn.
    pub(crate) async fn links(
        &self,
        tenant: &TenantContext,
        ticket: Uuid,
    ) -> Result<Vec<String>, Error> {
        Ok(link::Entity::find()
            .filter(link::Column::TicketId.eq(ticket))
            .scoped(tenant)
            .order_by_asc(link::Column::CreatedAt)
            .all(tenant.database().read())
            .await?
            .into_iter()
            .map(|row| row.target)
            .collect())
    }

    /// Gắn phiếu với bản ghi `target` (`<module>.<loại>:<id>`), kiểm qua sổ đối tượng: loại phải
    /// có, bản ghi phải tồn tại. Gắn lại không đổi gì.
    pub(crate) async fn add_link(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        target_ref: &str,
    ) -> Result<Vec<String>, Error> {
        let target_ref = target_ref.trim();
        let Some((kind, record)) = target_ref.split_once(':').filter(|(kind, record)| {
            !kind.is_empty() && !record.is_empty() && target_ref.len() <= 300
        }) else {
            return Err(link_not_found(target_ref));
        };
        let Some(resources) = &self.parts.resources else {
            return Err(link_not_found(target_ref));
        };
        if !resources.knows(kind) || resources.resolve(tenant, kind, record).await?.is_none() {
            return Err(link_not_found(target_ref));
        }
        self.get(tenant, Viewer::Staff(principal), id).await?;
        let transaction = tenant.database().begin_write().await?;
        let row = ticket::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(id))?;
        principal.require_write_at(&transaction, &TICKETS_REPLY, target(&row)).await?;
        let exists = link::Entity::find()
            .filter(link::Column::TicketId.eq(id))
            .filter(link::Column::Target.eq(target_ref))
            .scoped(tenant)
            .one(&transaction)
            .await?
            .is_some();
        if !exists {
            link::ActiveModel {
                tenant_id: Set(tenant.id().as_uuid()),
                ticket_id: Set(id),
                target: Set(target_ref.to_owned()),
                created_at: Set(now()),
            }
            .insert(&transaction)
            .await?;
            let event = events::TicketLinked(Change {
                before: None,
                after: Some(json!({"ticket_id": id, "target": target_ref})),
            });
            let actor = Actor::from(principal);
            self.parts
                .outbox
                .record(&transaction, tenant, actor, events::ticket_subject(id), &event)
                .await?;
        }
        transaction.commit().await?;
        self.links(tenant, id).await
    }

    /// Gỡ liên kết `target` khỏi phiếu.
    pub(crate) async fn remove_link(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        target_ref: &str,
    ) -> Result<Vec<String>, Error> {
        self.get(tenant, Viewer::Staff(principal), id).await?;
        let transaction = tenant.database().begin_write().await?;
        let row = ticket::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(id))?;
        principal.require_write_at(&transaction, &TICKETS_REPLY, target(&row)).await?;
        let removed = link::Entity::delete_many()
            .filter(link::Column::TicketId.eq(id))
            .filter(link::Column::Target.eq(target_ref))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if removed.rows_affected > 0 {
            let event = events::TicketLinked(Change {
                before: Some(json!({"ticket_id": id, "target": target_ref})),
                after: None,
            });
            let actor = Actor::from(principal);
            self.parts
                .outbox
                .record(&transaction, tenant, actor, events::ticket_subject(id), &event)
                .await?;
        }
        transaction.commit().await?;
        self.links(tenant, id).await
    }

    /// Chuyển phiếu: thành khiếu nại người tiêu dùng (gọi `compliance` NGOÀI transaction, rồi ghi
    /// `case_id` và đóng phiếu `escalated`), hay lên sàn (cờ `escalated_to_platform`).
    pub(crate) async fn escalate(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        to: Escalation,
        note: Option<String>,
    ) -> Result<Ticket, Error> {
        let before = self.get(tenant, Viewer::Staff(principal), id).await?;
        principal.require_at(tenant.database().read(), &TICKETS_MANAGE, target(&before)).await?;
        if parse_status(&before.status)? == Status::Closed && before.case_id.is_none() {
            return Err(crate::status::ticket_closed());
        }
        let case_id = match to {
            Escalation::Platform if before.escalated_to_platform => return Err(already_escalated()),
            Escalation::Platform => None,
            Escalation::ConsumerComplaint => {
                let Some(intake) = &self.parts.complaints else {
                    return Err(complaints_unavailable());
                };
                let claim = match note.as_deref().map(str::trim).filter(|note| !note.is_empty()) {
                    Some(note) => note.to_owned(),
                    None => self.first_message(tenant, id).await?,
                };
                let complaint = autoxide_compliance::api::ComplaintFromTicket {
                    source_ref: format!("support.ticket:{id}"),
                    org_id: before.org_id,
                    reporter_user_id: before.requester_user_id,
                    name: Some(before.requester_name.clone())
                        .filter(|name| !name.trim().is_empty())
                        .or_else(|| Some("Customer".to_owned())),
                    email: before.requester_email.clone(),
                    phone: before.requester_phone.clone(),
                    claim,
                };
                // Gọi lại cùng phiếu: `compliance` trả vụ việc cũ, không tạo vụ thứ hai.
                Some(intake.open(tenant, complaint).await?.id)
            }
        };
        let transaction = tenant.database().begin_write().await?;
        let current = ticket::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(id))?;
        principal.require_write_at(&transaction, &TICKETS_MANAGE, target(&current)).await?;
        if let Some(case) = case_id
            && current.case_id.is_some()
        {
            // Đã chuyển (lần gọi trước đã ghi): trả phiếu như đang có.
            transaction.rollback().await?;
            let _ = case;
            return Ok(current);
        }
        let at = now();
        let mut row = current.clone().into_active_model();
        let line = match case_id {
            Some(case) => {
                let policy = self.policy(tenant, current.org_id).await?.policy;
                row.case_id = Set(Some(case));
                crate::sla::transition(
                    &mut row,
                    &current,
                    Status::Closed,
                    Some(Resolution::Escalated),
                    &policy,
                    at,
                );
                "escalated:consumer_complaint"
            }
            None => {
                row.escalated_to_platform = Set(true);
                "escalated:platform"
            }
        };
        row.updated_at = Set(at);
        let after =
            ticket::Entity::update(row).validate()?.scoped(tenant).exec(&transaction).await?;
        self.system_line(&transaction, tenant, id, line, at).await?;
        let actor = Actor::from(principal);
        let event = events::TicketEscalated(Change {
            before: Some(snapshot(&current)),
            after: Some(snapshot(&after)),
        });
        self.parts
            .outbox
            .record(&transaction, tenant, actor, events::ticket_subject(id), &event)
            .await?;
        self.record_changes(&transaction, tenant, actor, &current, &after, false).await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Nội dung tin đầu tiên của người mua (nội dung khiếu nại mặc định).
    async fn first_message(&self, tenant: &TenantContext, id: Uuid) -> Result<String, Error> {
        use crate::entity::message;
        let first = message::Entity::find()
            .filter(message::Column::TicketId.eq(id))
            .filter(message::Column::AuthorKind.eq("customer"))
            .scoped(tenant)
            .order_by_asc(message::Column::Id)
            .one(tenant.database().read())
            .await?;
        Ok(first.map(|row| row.body).unwrap_or_default())
    }

    /// Người mua chấm mức hài lòng (1–5) cho phiếu đã xử lý hay đã đóng; chấm lại thì thay.
    pub(crate) async fn rate(
        &self,
        tenant: &TenantContext,
        viewer: Viewer<'_>,
        id: Uuid,
        rating: i16,
        note: Option<String>,
    ) -> Result<Ticket, Error> {
        let mut problems = Vec::new();
        if !(1..=5).contains(&rating) {
            problems.push(FieldError::new("rating", "range", "must be 1 to 5"));
        }
        let note = note.map(|note| note.trim().to_owned()).filter(|note| !note.is_empty());
        if note.as_ref().is_some_and(|note| note.chars().count() > 1000) {
            problems.push(FieldError::new("note", "length", "must be at most 1000 characters"));
        }
        finish(problems)?;
        let before = self.get(tenant, viewer, id).await?;
        if !matches!(parse_status(&before.status)?, Status::Solved | Status::Closed) {
            return Err(Error::conflict(
                "support.not_solved",
                "Rate the support once the ticket is solved.",
            ));
        }
        let transaction = tenant.database().begin_write().await?;
        let mut row = before.clone().into_active_model();
        row.satisfaction = Set(Some(rating));
        row.satisfaction_note = Set(note);
        let after =
            ticket::Entity::update(row).validate()?.scoped(tenant).exec(&transaction).await?;
        let actor = match viewer {
            Viewer::Customer(user) => Actor::user(user),
            _ => Actor::public(),
        };
        let event = events::SatisfactionRecorded(Change {
            before: None,
            after: Some(json!({"ticket_id": id, "satisfaction": rating})),
        });
        self.parts
            .outbox
            .record(&transaction, tenant, actor, events::ticket_subject(id), &event)
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Câu trả lời mẫu dùng được ở nút `org`: của nút và mọi nút trên nó, theo tên rồi ID, từng trang.
    pub(crate) async fn saved_replies(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
        page: &PageRequest,
    ) -> Result<Page<SavedReply>, Error> {
        let node = OrgId::from_uuid(org);
        let read = tenant.database().read();
        principal.require_at(read, &TICKETS_READ, OrgTarget::new(node, None)).await?;
        let select = self.replies_select(tenant, org).await?;
        // Thứ tự theo tên do database quyết (collation); khoá phụ ID giữ thứ tự ổn định, không trùng / sót khi
        // duyệt (D-20261125-list-pages).
        fetch_page_by(
            select,
            saved_reply::Column::Name,
            |row| SortValue::Text(row.name.clone()),
            saved_reply::Column::Id,
            |row| row.id,
            page,
            read,
        )
        .await
    }

    /// Câu trả lời mẫu `id` dùng được ở nút `org` (của nút hay một nút trên nó), không kiểm quyền.
    pub(crate) async fn saved_reply_at(
        &self,
        tenant: &TenantContext,
        org: Uuid,
        id: Uuid,
    ) -> Result<Option<SavedReply>, Error> {
        let select = self.replies_select(tenant, org).await?;
        Ok(select.filter(saved_reply::Column::Id.eq(id)).one(tenant.database().read()).await?)
    }

    /// `first` câu trả lời mẫu đầu tiên theo tên của nút `org` và các nút trên nó (gợi ý cho AI: chỉ cần vài
    /// câu, không phải danh sách để duyệt).
    pub(crate) async fn first_replies_at(
        &self,
        tenant: &TenantContext,
        org: Uuid,
        first: u64,
    ) -> Result<Vec<SavedReply>, Error> {
        use sea_orm::QuerySelect;
        let select = self.replies_select(tenant, org).await?;
        Ok(select
            .order_by_asc(saved_reply::Column::Name)
            .order_by_asc(saved_reply::Column::Id)
            .limit(first)
            .all(tenant.database().read())
            .await?)
    }

    /// Câu trả lời mẫu của nút `org` và mọi nút trên nó (chưa sắp, chưa giới hạn).
    async fn replies_select(
        &self,
        tenant: &TenantContext,
        org: Uuid,
    ) -> Result<sea_orm::Select<saved_reply::Entity>, Error> {
        let node = OrgId::from_uuid(org);
        let mut lineage = OrgTree::ancestors(tenant.database().read(), tenant.id(), node).await?;
        if lineage.is_empty() {
            lineage.push(node);
        }
        Ok(saved_reply::Entity::find()
            .filter(saved_reply::Column::OrgId.is_in(lineage.iter().map(OrgId::as_uuid)))
            .scoped(tenant))
    }

    /// Tạo hay sửa (`id`) câu trả lời mẫu (`support.settings.manage` ở nút của nó). Sửa nội dung thì
    /// bỏ dấu duyệt trả lời tự động (phải duyệt lại).
    pub(crate) async fn save_reply(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Option<Uuid>,
        input: SavedReplyInput,
    ) -> Result<SavedReply, Error> {
        let mut problems = Vec::new();
        let name = required_text("name", &input.name, 100, &mut problems);
        let bodies_ok = !input.bodies.is_empty()
            && input.bodies.len() <= 20
            && input.bodies.iter().all(|(locale, body)| {
                (2..=16).contains(&locale.len())
                    && (1..=self.config.max_body_chars).contains(&body.trim().chars().count())
            });
        if !bodies_ok {
            problems.push(
                FieldError::new(
                    "bodies",
                    "invalid",
                    "give the text for 1 to 20 languages (`vi`, `en`…), each 1 to {max} characters",
                )
                .with_param("max", self.config.max_body_chars),
            );
        }
        finish(problems)?;
        let bodies = serde_json::to_string(&input.bodies).map_err(Error::internal)?;
        let transaction = tenant.database().begin_write().await?;
        let org = OrgId::from_uuid(input.org_id);
        principal
            .require_write_at(&transaction, &SETTINGS_MANAGE, OrgTarget::new(org, None))
            .await?;
        let at = now();
        let saved = match id {
            None => {
                saved_reply::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    org_id: Set(input.org_id),
                    name: Set(name),
                    bodies: Set(bodies),
                    auto_reply: Set(false),
                    questions: Set("{}".to_owned()),
                    approved_by: Set(None),
                    approved_at: Set(None),
                    review_due_on: Set(None),
                    created_by: Set(Some(principal.user_id())),
                    created_at: Set(at),
                    updated_at: Set(at),
                }
                .insert(&transaction)
                .await?
            }
            Some(id) => {
                let current = saved_reply::Entity::find_by_id(id)
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                    .ok_or_else(|| saved_reply_not_found(id))?;
                let target = OrgTarget::new(OrgId::from_uuid(current.org_id), None);
                principal.require_write_at(&transaction, &SETTINGS_MANAGE, target).await?;
                let changed = current.bodies != bodies;
                let mut row = current.into_active_model();
                row.org_id = Set(input.org_id);
                row.name = Set(name);
                row.bodies = Set(bodies);
                if changed {
                    row.auto_reply = Set(false);
                    row.approved_by = Set(None);
                    row.approved_at = Set(None);
                }
                row.updated_at = Set(at);
                saved_reply::Entity::update(row)
                    .validate()?
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?
            }
        };
        self.settings_event(&transaction, tenant, principal, json!({"saved_reply": saved.id}))
            .await?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Xoá câu trả lời mẫu.
    pub(crate) async fn delete_reply(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let current = saved_reply::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| saved_reply_not_found(id))?;
        let target = OrgTarget::new(OrgId::from_uuid(current.org_id), None);
        principal.require_write_at(&transaction, &SETTINGS_MANAGE, target).await?;
        saved_reply::Entity::delete_by_id(id).scoped(tenant).exec(&transaction).await?;
        self.settings_event(
            &transaction,
            tenant,
            principal,
            json!({"saved_reply": id, "deleted": true}),
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Điền câu trả lời mẫu `reply` cho phiếu `ticket`: ngôn ngữ của người mua (không có thì `en`,
    /// rồi ngôn ngữ đầu tiên), biến `{customer_name}`, `{ticket_number}`, `{subject}`.
    pub(crate) async fn render_reply(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        ticket_id: Uuid,
        reply: Uuid,
    ) -> Result<String, Error> {
        let ticket = self.get(tenant, Viewer::Staff(principal), ticket_id).await?;
        let node = OrgId::from_uuid(ticket.org_id);
        principal
            .require_at(tenant.database().read(), &TICKETS_READ, OrgTarget::new(node, None))
            .await?;
        let reply = self
            .saved_reply_at(tenant, ticket.org_id, reply)
            .await?
            .ok_or_else(|| saved_reply_not_found(reply))?;
        Ok(fill(&reply, &ticket))
    }
}

/// Chọn ngôn ngữ và điền biến.
pub(crate) fn fill(reply: &SavedReply, ticket: &Ticket) -> String {
    let locale = ticket.locale.clone().unwrap_or_else(|| "en".to_owned());
    fill_for(reply, &locale, &ticket.requester_name, &ticket.number, &ticket.subject)
}

/// Như [`fill`] khi không có phiếu (xem thử, chat): ngôn ngữ và giá trị biến cho sẵn.
pub(crate) fn fill_for(
    reply: &SavedReply,
    locale: &str,
    name: &str,
    number: &str,
    subject: &str,
) -> String {
    let bodies: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&reply.bodies).unwrap_or_default();
    let text = bodies
        .get(locale)
        .or_else(|| bodies.get("en"))
        .or_else(|| bodies.values().next())
        .cloned()
        .unwrap_or_default();
    let name = if name.trim().is_empty() { "Customer" } else { name.trim() };
    text.replace("{customer_name}", name)
        .replace("{ticket_number}", number)
        .replace("{subject}", subject)
}

pub(crate) fn link_not_found(target: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "support.link_not_found",
        "`{target}` is not a record you can link: write it as `<module>.<type>:<id>` of an existing record.",
    )
    .with_param("target", target)
}

pub(crate) fn saved_reply_not_found(id: Uuid) -> Error {
    Error::not_found("support.saved_reply_not_found", "There is no saved reply `{id}` you can use.")
        .with_param("id", id)
}

fn already_escalated() -> Error {
    Error::conflict("support.already_escalated", "This ticket was already handed over.")
}

fn complaints_unavailable() -> Error {
    Error::unavailable(
        "support.complaints_unavailable",
        "Consumer complaints cannot be opened on this server (module `compliance` is not installed).",
    )
}
