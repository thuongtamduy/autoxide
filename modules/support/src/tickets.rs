//! Tạo phiếu (mọi đường: người mua, khách vãng lai, nhân viên ghi hộ, module khác qua
//! `SupportIntake`), danh sách và một phiếu cho nhân viên, đổi trạng thái / ưu tiên / loại / người
//! được giao.

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::numbering;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::secret::{random_token, sha256};
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext, root_of};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Condition;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
    SqlErr,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{link, message, ticket};
use crate::events::{self, Change};
use crate::permissions::{TICKETS_READ, TICKETS_REPLY};
use crate::service::{
    MODULE, SupportService, Ticket, Viewer, finish, hidden, not_found, now, org_exists,
    required_text, target,
};
use crate::status::{self, AuthorKind, Channel, MessageKind, Priority, Resolution, Status};

/// Loại phiếu mặc định khi nút chưa khai loại riêng (bước 53 — agent tự quyết).
pub(crate) const DEFAULT_CATEGORIES: [&str; 5] =
    ["order", "product", "payment", "account", "other"];
/// Độ dài tối đa của tiêu đề và tên người hỏi.
pub(crate) const SUBJECT_MAX: usize = 200;
pub(crate) const NAME_MAX: usize = 200;
/// Số liên kết tối đa của một phiếu.
pub(crate) const LINKS_MAX: usize = 20;

/// Người hỏi của phiếu đang tạo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Who {
    User(Uuid),
    Email(String),
    Phone(String),
}

/// Một phiếu cần tạo (đã chuẩn hoá liên lạc; còn phải kiểm độ dài, loại…).
#[derive(Debug, Clone)]
pub(crate) struct Opening {
    pub org_id: Option<Uuid>,
    pub requester: Who,
    pub requester_name: String,
    /// Liên lạc đã được chứng minh (tài khoản, OTP của module khác).
    pub verified: bool,
    pub category: String,
    pub subject: String,
    pub body: String,
    pub priority: Option<Priority>,
    pub channel: Channel,
    pub media_ids: Vec<Uuid>,
    pub links: Vec<String>,
    pub reference: Option<String>,
    pub follow_up_of: Option<Uuid>,
    pub locale: Option<String>,
}

/// Phiếu vừa tạo, kèm token xem phiếu (chỉ khách vãng lai, chỉ lần tạo đầu).
#[derive(Debug, Clone)]
pub(crate) struct Opened {
    pub ticket: Ticket,
    pub access_token: Option<String>,
    /// `false` khi `reference` trùng và đây là phiếu cũ.
    pub created: bool,
}

/// Bộ lọc danh sách của nhân viên.
#[derive(Debug, Clone, Default)]
pub(crate) struct TicketFilter {
    pub status: Option<Status>,
    /// `Some(None)`: chưa giao ai; `Some(Some(id))`: giao `id`.
    pub assignee: Option<Option<Uuid>>,
    pub team_id: Option<Uuid>,
    pub org_id: Option<Uuid>,
    pub category: Option<String>,
    pub priority: Option<Priority>,
    pub breached: Option<bool>,
    pub link: Option<String>,
    pub q: Option<String>,
}

/// Thay đổi của nhân viên; trường `None` giữ nguyên.
#[derive(Debug, Clone, Default)]
pub(crate) struct TicketChanges {
    pub status: Option<Status>,
    pub resolution: Option<Resolution>,
    pub priority: Option<Priority>,
    pub category: Option<String>,
    /// `Some(None)`: bỏ giao.
    pub assignee_user_id: Option<Option<Uuid>>,
    /// `Some(None)`: bỏ nhóm.
    pub team_id: Option<Option<Uuid>>,
}

impl SupportService {
    /// Tạo phiếu và tin nhắn đầu tiên trong MỘT transaction, cùng event. `reference` trùng thì trả
    /// phiếu cũ (`created = false`). `actor`: ai tạo (người mua, nhân viên, hệ thống).
    pub(crate) async fn open_ticket(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        uploader: Option<Uuid>,
        opening: Opening,
        checked: Option<&autoxide_kernel::antispam::Checked>,
    ) -> Result<Opened, Error> {
        if let Some(reference) = &opening.reference
            && let Some(existing) = self.by_reference(tenant, reference).await?
        {
            return Ok(Opened { ticket: existing, access_token: None, created: false });
        }
        let mut problems = Vec::new();
        let subject = required_text("subject", &opening.subject, SUBJECT_MAX, &mut problems);
        let body = self.check_body("body", &opening.body, &mut problems);
        let name = opening.requester_name.trim().chars().take(NAME_MAX).collect::<String>();
        let category = opening.category.trim().to_owned();
        let org_uuid = opening.org_id.unwrap_or_else(|| root_of(tenant.id()).as_uuid());
        let policy = self.policy(tenant, org_uuid).await?.policy;
        if !policy.categories.contains(&category) {
            problems.push(category_unknown());
        }
        if opening.links.len() > LINKS_MAX {
            problems.push(
                FieldError::new("links", "too_many", "at most {max} links")
                    .with_param("max", LINKS_MAX),
            );
        }
        let org =
            OrgId::from_uuid(opening.org_id.unwrap_or_else(|| root_of(tenant.id()).as_uuid()));
        if !org_exists(tenant, org).await? {
            problems.push(FieldError::new("org_id", "unknown", "is not a unit of this shop"));
        }
        finish(problems)?;
        self.check_attachments(tenant, uploader, &opening.media_ids).await?;
        if let Some(previous) = opening.follow_up_of {
            self.check_follow_up(tenant, &opening.requester, previous).await?;
        }

        let guest = !matches!(opening.requester, Who::User(_));
        let token = if guest { Some(random_token(32)?) } else { None };
        let at = now();
        let (user, email, phone) = match &opening.requester {
            Who::User(id) => (Some(*id), None, None),
            Who::Email(email) => (None, Some(email.clone()), None),
            Who::Phone(phone) => (None, None, Some(phone.clone())),
        };
        let priority = opening.priority.unwrap_or(Priority::Normal);
        let (first_due, resolve_due) = crate::sla::due_dates(&policy, at, priority, 0);
        let id = Uuid::now_v7();
        let transaction = tenant.database().begin_write().await?;
        // Số phiếu cấp ở cuối, ngay trước khi ghi (giữ khoá dãy số ngắn nhất).
        let number = format!("{:06}", numbering::next(&transaction, tenant.id(), NUMBERING).await?);
        let row = ticket::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org.as_uuid()),
            number: Set(number.clone()),
            requester_user_id: Set(user),
            requester_email: Set(email),
            requester_phone: Set(phone),
            requester_name: Set(name),
            locale: Set(opening.locale),
            verified_at: Set(opening.verified.then_some(at)),
            token_hash: Set(token.as_deref().map(sha256)),
            otp_hash: Set(None),
            otp_expires_at: Set(None),
            otp_attempts: Set(0),
            category: Set(category),
            priority: Set(priority.as_str().to_owned()),
            status: Set(Status::Open.as_str().to_owned()),
            resolution: Set(None),
            subject: Set(subject.clone()),
            channel: Set(opening.channel.as_str().to_owned()),
            assignee_user_id: Set(None),
            team_id: Set(None),
            first_response_due_at: Set(first_due),
            resolve_due_at: Set(resolve_due),
            first_responded_at: Set(None),
            sla_paused_at: Set(None),
            sla_paused_minutes: Set(0),
            solved_at: Set(None),
            closed_at: Set(None),
            breached: Set(false),
            case_id: Set(None),
            dispute_id: Set(None),
            follow_up_of: Set(opening.follow_up_of),
            escalated_to_platform: Set(false),
            satisfaction: Set(None),
            satisfaction_note: Set(None),
            auto_reply_paused: Set(false),
            auto_replies_in_row: Set(0),
            last_message_at: Set(at),
            reference: Set(opening.reference.clone()),
            search_key: Set(search_key(&number, &subject)),
            created_at: Set(at),
            updated_at: Set(at),
        };
        let inserted = match row.insert(&transaction).await {
            Ok(inserted) => inserted,
            Err(error)
                if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_)))
                    && opening.reference.is_some() =>
            {
                // Hai lời gọi cùng `reference` chạy cùng lúc: bên thua trả phiếu của bên thắng.
                transaction.rollback().await?;
                let reference = opening.reference.as_deref().unwrap_or_default();
                let existing =
                    self.by_reference(tenant, reference).await?.ok_or_else(|| not_found(id))?;
                return Ok(Opened { ticket: existing, access_token: None, created: false });
            }
            Err(error) => return Err(error.into()),
        };
        if let Some(checked) = checked {
            // Pha hai của chống spam: token form đã dùng → trả phiếu của lần gửi đầu.
            let claim = checked.claim(&transaction, tenant, MODULE, Some(&id.to_string())).await?;
            if !matches!(claim, autoxide_kernel::antispam::Claim::New) {
                transaction.rollback().await?;
                let existing = self.replayed(tenant, claim).await?.ok_or_else(|| not_found(id))?;
                return Ok(Opened { ticket: existing, access_token: None, created: false });
            }
        }
        let author = match opening.channel {
            Channel::Staff => None,
            _ => user,
        };
        let first = message::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            ticket_id: Set(id),
            kind: Set(MessageKind::Public.as_str().to_owned()),
            author_kind: Set(AuthorKind::Customer.as_str().to_owned()),
            author_user_id: Set(author),
            body: Set(body),
            media_ids: Set(json!(opening.media_ids).to_string()),
            mentions: Set("[]".to_owned()),
            hidden_at: Set(None),
            hidden_by: Set(None),
            hidden_reason: Set(None),
            ai_assisted: Set(false),
            auto_reply: Set(false),
            created_at: Set(at),
        }
        .insert(&transaction)
        .await?;
        for target in dedupe(opening.links) {
            link::ActiveModel {
                tenant_id: Set(tenant.id().as_uuid()),
                ticket_id: Set(id),
                target: Set(target),
                created_at: Set(at),
            }
            .insert(&transaction)
            .await?;
        }
        if policy.auto_reply && opening.channel != Channel::Staff {
            self.enqueue_auto_reply(&transaction, tenant, id, first.id).await?;
        }
        let outbox = &self.parts.outbox;
        let subject = events::ticket_subject(id);
        let created =
            events::TicketCreated(Change { before: None, after: Some(snapshot(&inserted)) });
        outbox.record(&transaction, tenant, actor, subject.clone(), &created).await?;
        let added =
            events::MessageAdded(Change { before: None, after: Some(message_snapshot(&first)) });
        outbox.record(&transaction, tenant, actor, subject, &added).await?;
        transaction.commit().await?;
        tracing::info!(ticket_id = %id, channel = opening.channel.as_str(), "support ticket opened");
        // Tự phân công theo quy tắc của nút (ngoài transaction tạo: hỏi quyền người nhận). Lỗi chỉ
        // ghi log — phiếu đã có, nhân viên giao tay được.
        let ticket = match self.auto_assign(tenant, &inserted).await {
            Ok(()) => ticket::Entity::find_by_id(id)
                .scoped(tenant)
                .one(tenant.database().read())
                .await?
                .unwrap_or(inserted),
            Err(error) => {
                tracing::warn!(ticket_id = %id, code = error.code(), "support ticket not routed");
                inserted
            }
        };
        Ok(Opened { ticket, access_token: token, created: true })
    }

    /// Nhân viên mở phiếu thay người mua (ghi hộ cuộc gọi): cần `support.tickets.reply` ở nút của
    /// phiếu; nút đang khoá thì `org_frozen`.
    pub(crate) async fn open_by_staff(
        &self,
        tenant: &TenantContext,
        principal: &autoxide_kernel::access::Principal,
        opening: Opening,
    ) -> Result<Opened, Error> {
        let org =
            OrgId::from_uuid(opening.org_id.unwrap_or_else(|| root_of(tenant.id()).as_uuid()));
        let target = autoxide_kernel::access::OrgTarget::new(org, None);
        let read = tenant.database().read();
        principal.require_at(read, &TICKETS_REPLY, target).await?;
        OrgTree::ensure_writable_unlocked(read, tenant.id(), org).await?;
        self.open_ticket(tenant, Actor::from(principal), None, opening, None).await
    }

    async fn by_reference(
        &self,
        tenant: &TenantContext,
        reference: &str,
    ) -> Result<Option<Ticket>, Error> {
        Ok(ticket::Entity::find()
            .filter(ticket::Column::Reference.eq(reference))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?)
    }

    /// Phiếu `follow_up_of` phải là phiếu của cùng người hỏi.
    async fn check_follow_up(
        &self,
        tenant: &TenantContext,
        requester: &Who,
        previous: Uuid,
    ) -> Result<(), Error> {
        let row = ticket::Entity::find_by_id(previous)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        let same = row.is_some_and(|row| match requester {
            Who::User(id) => row.requester_user_id == Some(*id),
            Who::Email(email) => row.requester_email.as_deref() == Some(email),
            Who::Phone(phone) => row.requester_phone.as_deref() == Some(phone),
        });
        if same {
            Ok(())
        } else {
            Err(Error::validation(vec![FieldError::new(
                "follow_up_of",
                "unknown",
                "is not one of your tickets",
            )]))
        }
    }

    /// Kiểm nội dung tin: bỏ khoảng trắng hai đầu, 1 tới `AUTOXIDE_SUPPORT_MAX_BODY_CHARS` ký tự.
    pub(crate) fn check_body(
        &self,
        field: &'static str,
        body: &str,
        problems: &mut Vec<FieldError>,
    ) -> String {
        required_text(field, body, self.config.max_body_chars, problems)
    }

    /// Loại phiếu có trong danh sách của chính sách đang hiệu lực ở nút `org`.
    pub(crate) async fn category_known(
        &self,
        tenant: &TenantContext,
        org: Uuid,
        category: &str,
    ) -> Result<bool, Error> {
        Ok(self.policy(tenant, org).await?.policy.categories.iter().any(|known| known == category))
    }

    /// Một trang phiếu trong phạm vi `support.tickets.read` của nhân viên, mới trước (mặc định).
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        principal: &autoxide_kernel::access::Principal,
        filter: &TicketFilter,
        page: &PageRequest,
    ) -> Result<Page<Ticket>, Error> {
        let scope = principal.scope_condition(
            &TICKETS_READ,
            ticket::Column::OrgId,
            Some(ticket::Column::AssigneeUserId),
        )?;
        let mut condition = Condition::all().add(scope);
        if let Some(status) = filter.status {
            condition = condition.add(ticket::Column::Status.eq(status.as_str()));
        }
        match filter.assignee {
            Some(None) => condition = condition.add(ticket::Column::AssigneeUserId.is_null()),
            Some(Some(id)) => condition = condition.add(ticket::Column::AssigneeUserId.eq(id)),
            None => {}
        }
        if let Some(team) = filter.team_id {
            condition = condition.add(ticket::Column::TeamId.eq(team));
        }
        if let Some(org) = filter.org_id {
            condition = condition.add(ticket::Column::OrgId.eq(org));
        }
        if let Some(category) = &filter.category {
            condition = condition.add(ticket::Column::Category.eq(category.as_str()));
        }
        if let Some(priority) = filter.priority {
            condition = condition.add(ticket::Column::Priority.eq(priority.as_str()));
        }
        if let Some(breached) = filter.breached {
            condition = condition.add(ticket::Column::Breached.eq(breached));
        }
        if let Some(target) = &filter.link {
            let linked = sea_orm::sea_query::Query::select()
                .column(link::Column::TicketId)
                .from(link::Entity)
                .and_where(link::Column::TenantId.eq(tenant.id().as_uuid()))
                .and_where(link::Column::Target.eq(target.as_str()))
                .to_owned();
            condition = condition.add(ticket::Column::Id.in_subquery(linked));
        }
        if let Some(q) = filter.q.as_deref().filter(|q| !q.trim().is_empty()) {
            condition =
                condition.add(autoxide_kernel::search::contains(ticket::Column::SearchKey, q));
        }
        let select = ticket::Entity::find().filter(condition).scoped(tenant);
        fetch_page(select, ticket::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Một trang phiếu của người mua `user`, mới trước (mặc định).
    pub(crate) async fn list_mine(
        &self,
        tenant: &TenantContext,
        user: Uuid,
        page: &PageRequest,
    ) -> Result<Page<Ticket>, Error> {
        let select =
            ticket::Entity::find().filter(ticket::Column::RequesterUserId.eq(user)).scoped(tenant);
        fetch_page(select, ticket::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Phiếu `id` cho người xem `viewer`; ngoài phạm vi / không phải của mình → 404.
    pub(crate) async fn get(
        &self,
        tenant: &TenantContext,
        viewer: Viewer<'_>,
        id: Uuid,
    ) -> Result<Ticket, Error> {
        let read = tenant.database().read();
        let row = ticket::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| not_found(id))?;
        match viewer {
            Viewer::Staff(principal) => {
                hidden(principal.require_at(read, &TICKETS_READ, target(&row)).await, id)?;
            }
            Viewer::Customer(user) if row.requester_user_id == Some(user) => {}
            Viewer::Customer(_) => return Err(not_found(id)),
            // Token đã kiểm ở `guest::by_token`.
            Viewer::Guest | Viewer::Internal => {}
        }
        Ok(row)
    }

    /// Đổi trạng thái / ưu tiên / loại / người được giao của phiếu `id` (nhân viên,
    /// `support.tickets.reply` ở nút của phiếu). `expected`: `If-Match` — người khác vừa đổi thì
    /// 412, đúng một bên thắng.
    pub(crate) async fn update(
        &self,
        tenant: &TenantContext,
        principal: &autoxide_kernel::access::Principal,
        id: Uuid,
        changes: TicketChanges,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Ticket, Error> {
        let actor = Actor::from(principal);
        let before = self.get(tenant, Viewer::Staff(principal), id).await?;
        let policy = self.policy(tenant, before.org_id).await?.policy;
        if let Some(category) = &changes.category
            && !policy.categories.contains(category)
        {
            return Err(Error::validation(vec![category_unknown()]));
        }
        if let Some(Some(assignee)) = changes.assignee_user_id {
            self.check_assignee(tenant, &before, principal, assignee).await?;
        }
        if let Some(Some(team)) = changes.team_id {
            self.check_team(tenant, &before, team).await?;
        }
        let transaction = tenant.database().begin_write().await?;
        let before = ticket::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(id))?;
        principal.require_write_at(&transaction, &TICKETS_REPLY, target(&before)).await?;
        let at = now();
        let mut row = before.clone().into_active_model();
        let from = parse_status(&before.status)?;
        let mut lines = Vec::new();
        if let Some(to) = changes.status {
            status::transition(from, to)?;
            if to != from {
                crate::sla::transition(&mut row, &before, to, changes.resolution, &policy, at);
                lines.push(format!("status:{}", to.as_str()));
            }
        }
        if let Some(priority) = changes.priority
            && priority.as_str() != before.priority
        {
            row.priority = Set(priority.as_str().to_owned());
            // Đổi ưu tiên: tính lại hạn từ lúc tạo, giữ phút đã chờ người mua.
            let (first, resolve) = crate::sla::due_dates(
                &policy,
                before.created_at,
                priority,
                before.sla_paused_minutes,
            );
            row.first_response_due_at = Set(first);
            row.resolve_due_at = Set(resolve);
            lines.push(format!("priority:{}", priority.as_str()));
        }
        if let Some(category) = changes.category
            && category != before.category
        {
            row.category = Set(category);
        }
        let assigned = changes.assignee_user_id.is_some_and(|new| new != before.assignee_user_id)
            || changes.team_id.is_some_and(|new| new != before.team_id);
        if let Some(assignee) = changes.assignee_user_id {
            row.assignee_user_id = Set(assignee);
        }
        if let Some(team) = changes.team_id {
            row.team_id = Set(team);
        }
        row.updated_at = Set(at);
        let mut update = ticket::Entity::update(row).validate()?.scoped(tenant);
        if let Some(expected) = expected {
            // Kiểm phiên bản ngay trong câu `UPDATE`: người khác vừa sửa thì 0 dòng (D-084).
            update = update.filter(ticket::Column::UpdatedAt.eq(expected));
        }
        let after = update.exec(&transaction).await.map_err(|error| match error {
            sea_orm::DbErr::RecordNotUpdated => autoxide_kernel::version::changed(),
            error => error.into(),
        })?;
        for line in &lines {
            self.system_line(&transaction, tenant, id, line, at).await?;
        }
        self.record_changes(&transaction, tenant, actor, &before, &after, assigned).await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Ghi các event của một lần đổi phiếu: trạng thái, người được giao, phần còn lại.
    pub(crate) async fn record_changes<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        actor: Actor,
        before: &Ticket,
        after: &Ticket,
        assigned: bool,
    ) -> Result<(), Error> {
        let outbox = &self.parts.outbox;
        let subject = events::ticket_subject(after.id);
        let change = || Change { before: Some(snapshot(before)), after: Some(snapshot(after)) };
        if before.status != after.status {
            let event = events::TicketStatusChanged(change());
            outbox.record(connection, tenant, actor, subject.clone(), &event).await?;
        }
        if assigned {
            let event = events::TicketAssigned(change());
            outbox.record(connection, tenant, actor, subject.clone(), &event).await?;
        }
        if before.priority != after.priority || before.category != after.category {
            let event = events::TicketUpdated(change());
            outbox.record(connection, tenant, actor, subject, &event).await?;
        }
        Ok(())
    }

    /// Dòng hệ thống (i18n key `support.line.<việc>` + giá trị) vào luồng tin của phiếu.
    pub(crate) async fn system_line<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        ticket_id: Uuid,
        line: &str,
        at: DateTime<Utc>,
    ) -> Result<(), Error> {
        message::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            ticket_id: Set(ticket_id),
            kind: Set(MessageKind::System.as_str().to_owned()),
            author_kind: Set(AuthorKind::System.as_str().to_owned()),
            author_user_id: Set(None),
            body: Set(line.to_owned()),
            media_ids: Set("[]".to_owned()),
            mentions: Set("[]".to_owned()),
            hidden_at: Set(None),
            hidden_by: Set(None),
            hidden_reason: Set(None),
            ai_assisted: Set(false),
            auto_reply: Set(false),
            created_at: Set(at),
        }
        .insert(connection)
        .await?;
        Ok(())
    }
}

/// Đặt trạng thái `to` (đã kiểm hợp lệ) cùng các mốc thời gian đi kèm.
pub(crate) fn apply_status(
    row: &mut ticket::ActiveModel,
    before: &Ticket,
    to: Status,
    resolution: Option<Resolution>,
    at: DateTime<Utc>,
) {
    row.status = Set(to.as_str().to_owned());
    match to {
        Status::PendingCustomer => {
            // Chờ người mua: đồng hồ hạn trả lời dừng.
            if before.sla_paused_at.is_none() {
                row.sla_paused_at = Set(Some(at));
            }
        }
        Status::Open | Status::PendingAgent => {
            row.sla_paused_at = Set(None);
            row.solved_at = Set(None);
            row.resolution = Set(None);
        }
        Status::Solved => {
            row.solved_at = Set(Some(at));
            row.resolution =
                Set(Some(resolution.unwrap_or(Resolution::Answered).as_str().to_owned()));
        }
        Status::Closed => {
            row.closed_at = Set(Some(at));
            if before.solved_at.is_none() {
                row.solved_at = Set(Some(at));
            }
            let resolution = resolution
                .map(Resolution::as_str)
                .map(str::to_owned)
                .or_else(|| before.resolution.clone())
                .unwrap_or_else(|| Resolution::Answered.as_str().to_owned());
            row.resolution = Set(Some(resolution));
        }
    }
}

pub(crate) fn parse_status(raw: &str) -> Result<Status, Error> {
    Status::parse(raw).ok_or_else(|| Error::internal(format!("unknown ticket status `{raw}`")))
}

/// Tên của dãy số phiếu (bảng `kernel_sequences`).
const NUMBERING: &str = "support:tickets";

/// Khoá tìm: số phiếu và tiêu đề, không dấu.
pub(crate) fn search_key(number: &str, subject: &str) -> String {
    autoxide_kernel::search::fold(&format!("{number} {subject}"))
}

fn dedupe(links: Vec<String>) -> Vec<String> {
    let mut seen = Vec::new();
    for link in links {
        if !seen.contains(&link) {
            seen.push(link);
        }
    }
    seen
}

pub(crate) fn category_unknown() -> FieldError {
    FieldError::new("category", "unknown", "is not a ticket category of this shop")
}

pub(crate) fn assignee_out_of_scope(assignee: Uuid) -> Error {
    Error::new(
        autoxide_kernel::error::ErrorKind::Validation,
        "support.assignee_out_of_scope",
        "User `{assignee}` does not work on tickets of this unit: assign yourself or a member of a \
         support team of the unit.",
    )
    .with_param("assignee", assignee)
}

/// Những gì audit cần biết về một phiếu — không email, số điện thoại hay token.
pub(crate) fn snapshot(ticket: &Ticket) -> Value {
    json!({
        "number": ticket.number,
        "org_id": ticket.org_id,
        "status": ticket.status,
        "resolution": ticket.resolution,
        "priority": ticket.priority,
        "category": ticket.category,
        "channel": ticket.channel,
        "assignee_user_id": ticket.assignee_user_id,
        "team_id": ticket.team_id,
        "requester_user_id": ticket.requester_user_id,
    })
}

/// Event về tin nhắn: không mang nội dung.
pub(crate) fn message_snapshot(message: &message::Model) -> Value {
    json!({
        "id": message.id,
        "ticket_id": message.ticket_id,
        "kind": message.kind,
        "author_kind": message.author_kind,
        "author_user_id": message.author_user_id,
    })
}
