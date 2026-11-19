//! Cuộc trò chuyện: người mua bắt đầu (hay viết tiếp vào cuộc đang mở với cùng nút và cùng bản ghi),
//! danh sách hộp thư chung của nhân viên, danh sách của người mua, một cuộc, phân công, đóng, đánh
//! dấu đã đọc.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, SortValue, fetch_page_by};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, root_of};
use sea_orm::sea_query::{Condition, Expr};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, Set, SqlErr,
};
use uuid::Uuid;

use crate::entity::{message, read, thread};
use crate::events::{self, Change};
use crate::messages::NewMessage;
use crate::permissions::{MANAGE, READ, REPLY};
use crate::service::{
    AuthorKind, CLOSED, ConversationsService, Message, OPEN, Thread, Viewer, audience, finish,
    hidden, not_found, now, org_exists, snapshot, target,
};

/// Độ dài tối đa của `subject` (`<kind>:<id>`).
pub(crate) const SUBJECT_MAX: usize = 300;

/// Cuộc trò chuyện người mua bắt đầu, kèm tin đầu tiên.
#[derive(Debug, Clone, Default)]
pub(crate) struct NewThread {
    /// Nút nhận; `None`: nút gốc (cửa hàng).
    pub org_id: Option<Uuid>,
    /// Bản ghi được hỏi, `<kind>:<id>` (vd `catalog.product:<id>`).
    pub subject: Option<String>,
    pub message: NewMessage,
    pub locale: Option<String>,
}

/// Kết quả bắt đầu: cuộc trò chuyện và tin vừa gửi; `created = false` khi viết tiếp vào cuộc đang mở.
#[derive(Debug, Clone)]
pub(crate) struct Started {
    pub thread: Thread,
    pub message: Message,
    pub created: bool,
}

/// Bộ lọc hộp thư của nhân viên.
#[derive(Debug, Clone, Default)]
pub(crate) struct ThreadFilter {
    pub status: Option<String>,
    /// `Some(None)`: chưa giao ai; `Some(Some(id))`: giao `id`.
    pub assignee: Option<Option<Uuid>>,
    /// Chỉ cuộc có tin người mua chưa được nhân viên đọc.
    pub unread: Option<bool>,
    /// Chỉ cuộc người mua đã xin gặp nhân viên (hay trả lời tự động chuyển người).
    pub needs_agent: Option<bool>,
    pub org_id: Option<Uuid>,
    pub subject: Option<String>,
}

impl ConversationsService {
    /// Người mua `customer` bắt đầu cuộc trò chuyện. Đã có cuộc đang mở với cùng nút và cùng bản ghi
    /// thì viết tiếp vào đó (`created = false`); gửi lại cùng `client_id` không thêm tin thứ hai.
    pub(crate) async fn start(
        &self,
        tenant: &TenantContext,
        customer: Uuid,
        new: NewThread,
    ) -> Result<Started, Error> {
        let mut problems = Vec::new();
        let subject = match new.subject.as_deref().map(str::trim).filter(|raw| !raw.is_empty()) {
            Some(raw) => {
                self.check_subject(tenant, raw, &mut problems).await?;
                Some(raw.to_owned())
            }
            None => None,
        };
        let org = OrgId::from_uuid(new.org_id.unwrap_or_else(|| root_of(tenant.id()).as_uuid()));
        if !org_exists(tenant, org).await? {
            problems.push(FieldError::new("org_id", "unknown", "is not a unit of this shop"));
        }
        let message = self.check_message(&new.message, &mut problems);
        finish(problems)?;
        let open_key = format!("{customer}:{}:{}", org.as_uuid(), subject.as_deref().unwrap_or(""));
        if let Some(existing) = self.open_thread(tenant, &open_key).await? {
            let (message, _) =
                self.send(tenant, Viewer::Customer(customer), existing.id, new.message).await?;
            let thread = self.reload(tenant, existing.id).await?;
            return Ok(Started { thread, message, created: false });
        }
        self.guard_customer(tenant, customer, org).await?;
        self.check_attachments(tenant, Some(customer), &message.media_ids).await?;
        let policy = self.parts.settings.policy(tenant, org).await?;
        let body = self.prepare_body(&policy, &message.body);
        let at = now();
        let id = Uuid::now_v7();
        let row = thread::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org.as_uuid()),
            customer_user_id: Set(Some(customer)),
            subject: Set(subject),
            status: Set(OPEN.to_owned()),
            assignee_user_id: Set(None),
            open_key: Set(Some(open_key.clone())),
            unread_customer: Set(0),
            unread_staff: Set(0),
            auto_reply_paused: Set(false),
            auto_replies_in_row: Set(0),
            needs_agent: Set(false),
            ticket_id: Set(None),
            draft_reply: Set(None),
            draft_for: Set(None),
            locale: Set(new.locale.clone()),
            last_message_at: Set(at),
            closed_at: Set(None),
            created_at: Set(at),
            updated_at: Set(at),
        };
        let transaction = tenant.database().begin_write().await?;
        let inserted = match row.insert(&transaction).await {
            Ok(inserted) => inserted,
            Err(error) if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) => {
                // Hai lời gọi cùng lúc mở cùng một cuộc: bên thua viết vào cuộc của bên thắng.
                transaction.rollback().await?;
                let existing =
                    self.open_thread(tenant, &open_key).await?.ok_or_else(|| not_found(id))?;
                let (message, _) =
                    self.send(tenant, Viewer::Customer(customer), existing.id, new.message).await?;
                let thread = self.reload(tenant, existing.id).await?;
                return Ok(Started { thread, message, created: false });
            }
            Err(error) => return Err(error.into()),
        };
        let first = self
            .insert_message(
                &transaction,
                tenant,
                &inserted,
                AuthorKind::Customer,
                Some(customer),
                body,
                &message,
                at,
            )
            .await?
            .ok_or_else(|| {
                Error::internal("the first message of a new conversation is a duplicate")
            })?;
        let actor = Actor::user(customer);
        let opened = events::ThreadOpened(Change {
            before: None,
            after: Some(snapshot(&inserted)),
            audience: audience(&inserted),
        });
        let outbox = &self.parts.outbox;
        outbox.record(&transaction, tenant, actor, events::thread_subject(id), &opened).await?;
        self.after_message(&transaction, tenant, actor, &inserted, &first).await?;
        transaction.commit().await?;
        tracing::info!(thread_id = %id, org_id = %org.as_uuid(), "conversation started");
        Ok(Started { thread: inserted, message: first, created: true })
    }

    /// Kiểm `subject` dạng `<kind>:<id>` và bản ghi có thật (sổ đối tượng, D-123).
    async fn check_subject(
        &self,
        tenant: &TenantContext,
        raw: &str,
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        let unknown =
            || FieldError::new("subject", "unknown", "must be `<kind>:<id>` of an existing record");
        let Some((kind, id)) = raw.split_once(':') else {
            problems.push(unknown());
            return Ok(());
        };
        if raw.chars().count() > SUBJECT_MAX || kind.is_empty() || id.is_empty() {
            problems.push(unknown());
            return Ok(());
        }
        if let Some(resources) = &self.parts.resources {
            let found =
                resources.knows(kind) && resources.resolve(tenant, kind, id).await?.is_some();
            if !found {
                problems.push(unknown());
            }
        }
        Ok(())
    }

    async fn open_thread(
        &self,
        tenant: &TenantContext,
        open_key: &str,
    ) -> Result<Option<Thread>, Error> {
        Ok(thread::Entity::find()
            .filter(thread::Column::OpenKey.eq(open_key))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?)
    }

    pub(crate) async fn reload(&self, tenant: &TenantContext, id: Uuid) -> Result<Thread, Error> {
        thread::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| not_found(id))
    }

    /// Cuộc trò chuyện `id` cho người xem `viewer`; ngoài phạm vi / không phải của mình → 404.
    pub(crate) async fn get(
        &self,
        tenant: &TenantContext,
        viewer: Viewer<'_>,
        id: Uuid,
    ) -> Result<Thread, Error> {
        let read = tenant.database().read();
        let row = thread::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| not_found(id))?;
        match viewer {
            Viewer::Staff(principal) => {
                hidden(principal.require_at(read, &READ, target(&row)).await, id)?;
            }
            Viewer::Customer(user) if row.customer_user_id == Some(user) => {}
            Viewer::Customer(_) => return Err(not_found(id)),
        }
        Ok(row)
    }

    /// Hộp thư chung: một trang cuộc trò chuyện trong phạm vi `conversations.read`, tin mới nhất trước
    /// (mặc định).
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &ThreadFilter,
        page: &PageRequest,
    ) -> Result<Page<Thread>, Error> {
        let scope = principal.scope_condition(
            &READ,
            thread::Column::OrgId,
            Some(thread::Column::AssigneeUserId),
        )?;
        let mut condition = Condition::all().add(scope);
        if let Some(status) = &filter.status {
            condition = condition.add(thread::Column::Status.eq(status.as_str()));
        }
        match filter.assignee {
            Some(None) => condition = condition.add(thread::Column::AssigneeUserId.is_null()),
            Some(Some(id)) => condition = condition.add(thread::Column::AssigneeUserId.eq(id)),
            None => {}
        }
        match filter.unread {
            Some(true) => condition = condition.add(thread::Column::UnreadStaff.gt(0)),
            Some(false) => condition = condition.add(thread::Column::UnreadStaff.eq(0)),
            None => {}
        }
        if let Some(needs) = filter.needs_agent {
            condition = condition.add(thread::Column::NeedsAgent.eq(needs));
        }
        if let Some(org) = filter.org_id {
            condition = condition.add(thread::Column::OrgId.eq(org));
        }
        if let Some(subject) = &filter.subject {
            condition = condition.add(thread::Column::Subject.eq(subject.as_str()));
        }
        let select = thread::Entity::find().filter(condition).scoped(tenant);
        self.page_by_activity(tenant, select, page).await
    }

    /// Một trang cuộc trò chuyện của người mua `customer`, tin mới nhất trước (mặc định).
    pub(crate) async fn list_mine(
        &self,
        tenant: &TenantContext,
        customer: Uuid,
        page: &PageRequest,
    ) -> Result<Page<Thread>, Error> {
        let select = thread::Entity::find()
            .filter(thread::Column::CustomerUserId.eq(customer))
            .scoped(tenant);
        self.page_by_activity(tenant, select, page).await
    }

    async fn page_by_activity(
        &self,
        tenant: &TenantContext,
        select: sea_orm::Select<thread::Entity>,
        page: &PageRequest,
    ) -> Result<Page<Thread>, Error> {
        fetch_page_by(
            select,
            thread::Column::LastMessageAt,
            |row| SortValue::Time(row.last_message_at),
            thread::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await
    }

    /// Giao cuộc trò chuyện `id` cho `assignee` (`None`: bỏ giao). Tự nhận (`assignee` = mình) cần
    /// `conversations.reply`; giao người khác hay bỏ giao người khác cần `conversations.manage`.
    pub(crate) async fn assign(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        assignee: Option<Uuid>,
    ) -> Result<Thread, Error> {
        let before = self.get(tenant, Viewer::Staff(principal), id).await?;
        let read = tenant.database().read();
        let own = assignee == Some(principal.user_id())
            || (assignee.is_none() && before.assignee_user_id == Some(principal.user_id()));
        let permission = if own { &REPLY } else { &MANAGE };
        principal.require_at(read, permission, target(&before)).await?;
        if let Some(assignee) = assignee
            && assignee != principal.user_id()
        {
            self.check_assignee(tenant, &before, assignee).await?;
        }
        if before.assignee_user_id == assignee {
            return Ok(before);
        }
        let transaction = tenant.database().begin_write().await?;
        principal.require_write_at(&transaction, permission, target(&before)).await?;
        let mut row = before.clone().into_active_model();
        row.assignee_user_id = Set(assignee);
        row.updated_at = Set(now());
        let after = row.update(&transaction).await?;
        let mut people = audience(&after);
        people.extend(before.assignee_user_id);
        let event = events::ThreadAssigned(Change {
            before: Some(snapshot(&before)),
            after: Some(snapshot(&after)),
            audience: people,
        });
        let subject = events::thread_subject(id);
        self.parts
            .outbox
            .record(&transaction, tenant, Actor::from(principal), subject, &event)
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Người được giao phải làm được cuộc trò chuyện (`conversations.reply` ở nút của nó). Không có
    /// cách hỏi (thiếu `dyn Authenticator`) thì từ chối giao người khác.
    async fn check_assignee(
        &self,
        tenant: &TenantContext,
        thread: &Thread,
        assignee: Uuid,
    ) -> Result<(), Error> {
        let Some(authenticator) = &self.parts.authenticator else {
            return Err(assignee_out_of_scope(assignee));
        };
        let Some(other) = authenticator.principal_of(tenant, assignee).await? else {
            return Err(assignee_out_of_scope(assignee));
        };
        let target = autoxide_kernel::access::OrgTarget::new(
            OrgId::from_uuid(thread.org_id),
            Some(assignee),
        );
        other
            .require_at(tenant.database().read(), &REPLY, target)
            .await
            .map_err(|_| assignee_out_of_scope(assignee))
    }

    /// Đóng cuộc trò chuyện `id` (nhân viên có `conversations.reply`, hay chính người mua). Người mua
    /// viết lần sau thì thành cuộc mới.
    pub(crate) async fn close(
        &self,
        tenant: &TenantContext,
        viewer: Viewer<'_>,
        id: Uuid,
    ) -> Result<Thread, Error> {
        let before = self.get(tenant, viewer, id).await?;
        if let Viewer::Staff(principal) = viewer {
            principal.require_at(tenant.database().read(), &REPLY, target(&before)).await?;
        }
        if before.status == CLOSED {
            return Ok(before);
        }
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        // Chỉ đóng khi còn mở: hai lời gọi cùng lúc thì một bên đổi, một bên thấy đã đóng.
        let changed = thread::Entity::update_many()
            .col_expr(thread::Column::Status, Expr::value(CLOSED))
            .col_expr(thread::Column::OpenKey, Expr::value(Option::<String>::None))
            .col_expr(thread::Column::ClosedAt, Expr::value(Some(at)))
            .col_expr(thread::Column::UpdatedAt, Expr::value(at))
            .filter(thread::Column::Id.eq(id))
            .filter(thread::Column::Status.eq(OPEN))
            .scoped(tenant)
            .exec(&transaction)
            .await?
            .rows_affected;
        if changed == 0 {
            transaction.rollback().await?;
            return self.reload(tenant, id).await;
        }
        let after = thread::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(id))?;
        let actor = actor_of(viewer);
        let event = events::ThreadClosed(Change {
            before: Some(snapshot(&before)),
            after: Some(snapshot(&after)),
            audience: audience(&after),
        });
        self.parts
            .outbox
            .record(&transaction, tenant, actor, events::thread_subject(id), &event)
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Đánh dấu đã đọc tới tin `upto` (hay tin mới nhất) của cuộc `id`: con trỏ đọc của người gọi
    /// tiến lên (không lùi), bộ đếm chưa đọc của phía người gọi về 0 khi đọc tới tin mới nhất.
    pub(crate) async fn mark_read(
        &self,
        tenant: &TenantContext,
        viewer: Viewer<'_>,
        id: Uuid,
        upto: Option<Uuid>,
    ) -> Result<Thread, Error> {
        let before = self.get(tenant, viewer, id).await?;
        let latest = message::Entity::find()
            .filter(message::Column::ThreadId.eq(id))
            .scoped(tenant)
            .order_by_desc(message::Column::Id)
            .one(tenant.database().read())
            .await?;
        let Some(latest) = latest else {
            return Ok(before);
        };
        let upto = match upto {
            Some(message_id) => {
                let known = message::Entity::find_by_id(message_id)
                    .filter(message::Column::ThreadId.eq(id))
                    .scoped(tenant)
                    .one(tenant.database().read())
                    .await?;
                if known.is_none() {
                    return Err(Error::validation(vec![FieldError::new(
                        "message_id",
                        "unknown",
                        "is not a message of this conversation",
                    )]));
                }
                message_id
            }
            None => latest.id,
        };
        let user = viewer.user_id();
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        let cursor =
            read::Entity::find_by_id((tenant.id().as_uuid(), id, user)).one(&transaction).await?;
        match cursor {
            Some(cursor) if cursor.last_read_message_id >= upto => {}
            Some(cursor) => {
                let mut row = cursor.into_active_model();
                row.last_read_message_id = Set(upto);
                row.read_at = Set(at);
                row.update(&transaction).await?;
            }
            None => {
                let row = read::ActiveModel {
                    tenant_id: Set(tenant.id().as_uuid()),
                    thread_id: Set(id),
                    user_id: Set(user),
                    last_read_message_id: Set(upto),
                    read_at: Set(at),
                };
                insert_ignoring_duplicate(&transaction, row).await?;
            }
        }
        if upto >= latest.id {
            let column = if viewer.is_staff() {
                thread::Column::UnreadStaff
            } else {
                thread::Column::UnreadCustomer
            };
            thread::Entity::update_many()
                .col_expr(column, Expr::value(0))
                .filter(thread::Column::Id.eq(id))
                .filter(message_newer_than(id, latest.id).not())
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        let after = thread::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(id))?;
        if after != before {
            let event = events::ThreadUpdated(Change {
                before: Some(snapshot(&before)),
                after: Some(snapshot(&after)),
                audience: audience(&after),
            });
            let subject = events::thread_subject(id);
            self.parts
                .outbox
                .record(&transaction, tenant, actor_of(viewer), subject, &event)
                .await?;
        }
        transaction.commit().await?;
        Ok(after)
    }

    /// Con trỏ đọc của mọi người trong cuộc `thread` (hiện "đã xem").
    pub(crate) async fn reads(
        &self,
        tenant: &TenantContext,
        thread: &Thread,
    ) -> Result<Vec<read::Model>, Error> {
        Ok(read::Entity::find()
            .filter(read::Column::ThreadId.eq(thread.id))
            .scoped(tenant)
            .order_by_asc(read::Column::ReadAt)
            .all(tenant.database().read())
            .await?)
    }
}

/// Điều kiện "cuộc `thread` có tin mới hơn `latest`" — tin tới giữa lúc đọc thì không xoá bộ đếm.
fn message_newer_than(thread: Uuid, latest: Uuid) -> Condition {
    let newer = sea_orm::sea_query::Query::select()
        .column(message::Column::Id)
        .from(message::Entity)
        .and_where(message::Column::ThreadId.eq(thread))
        .and_where(message::Column::Id.gt(latest))
        .to_owned();
    Condition::all().add(Expr::exists(newer))
}

/// Chèn dòng con trỏ đọc; hai lời gọi cùng lúc thì bên sau bỏ qua (`ON CONFLICT DO NOTHING`, không để
/// lệnh lỗi trong transaction — Postgres huỷ cả transaction sau một lệnh lỗi, D-158).
async fn insert_ignoring_duplicate<C: ConnectionTrait>(
    connection: &C,
    row: read::ActiveModel,
) -> Result<(), Error> {
    let conflict = sea_orm::sea_query::OnConflict::columns([
        read::Column::TenantId,
        read::Column::ThreadId,
        read::Column::UserId,
    ])
    .do_nothing()
    .to_owned();
    read::Entity::insert(row).on_conflict(conflict).exec_without_returning(connection).await?;
    Ok(())
}

/// Ai gây ra thay đổi (cho event).
pub(crate) fn actor_of(viewer: Viewer<'_>) -> Actor {
    match viewer {
        Viewer::Staff(principal) => Actor::from(principal),
        Viewer::Customer(user) => Actor::user(user),
    }
}

pub(crate) fn assignee_out_of_scope(assignee: Uuid) -> Error {
    Error::new(
        ErrorKind::Validation,
        "conversations.assignee_out_of_scope",
        "User `{assignee}` cannot answer conversations of this unit: they need \
         `conversations.reply` there.",
    )
    .with_param("assignee", assignee)
}
