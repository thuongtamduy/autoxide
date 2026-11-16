//! Service của ghi chú: MỌI logic nằm ở đây. Handler REST và tool MCP chỉ lấy dữ liệu
//! vào, kiểm permission, gọi service — nên hai đường cho cùng một kết quả.
//!
//! Mỗi lệnh ghi đi cùng một event trong CÙNG transaction (luật kiến trúc 3): thay đổi
//! và dòng audit log hoặc cùng có, hoặc cùng không.
//!
//! Ghi chú thuộc một khách (bước 33): mọi hàm nhận `&TenantContext`, lấy database qua
//! `tenant.database()` và lọc MỌI truy vấn bằng `.scoped(tenant)`; `INSERT` đặt `tenant_id`. Quên
//! khách thì không có database để truy vấn — trình biên dịch chặn.
//!
//! Ghi chú thuộc một nút của cây tổ chức (bước 34) — đây là ví dụ mẫu của "dữ liệu theo nút":
//! danh sách lọc bằng `principal.scope_condition(…)` (nút + người sở hữu); một ghi chú đọc bằng
//! `require_at`, ghi bằng `require_write_at` TRONG transaction ghi (nút đang khoá thì từ chối).
//! Ghi chú ngoài phạm vi báo như không có (404).

use autoxide_kernel::access::{OrgTarget, Principal, Scope};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Outbox};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, root_of};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
    SqlErr,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{NoteSummary, Notes};
use crate::entity::{self, Column, Entity};
use crate::events::{self, Change};
use crate::permissions::{NOTES_READ, NOTES_WRITE};

/// Một ghi chú.
pub(crate) type Note = entity::Model;

/// Dữ liệu để tạo ghi chú (đã qua kiểm độ dài ở REST / MCP).
#[derive(Debug, Clone)]
pub(crate) struct NewNote {
    pub title: String,
    pub body: String,
    /// Nút của ghi chú; `None`: nút duy nhất người tạo được ghi (gốc khi có quyền toàn khách).
    pub org_id: Option<Uuid>,
}

/// Thay đổi cho một ghi chú; trường `None` giữ nguyên.
#[derive(Debug, Clone, Default)]
pub(crate) struct NoteChanges {
    pub title: Option<String>,
    pub body: Option<String>,
}

// Service không giữ database: database là của khách, đi theo `TenantContext` của từng lời gọi.
pub(crate) struct NoteService {
    outbox: Outbox,
    /// Quyền dùng theo gói (bước 35): hạn mức số ghi chú.
    entitlements: autoxide_kernel::entitlement::EntitlementsHandle,
}

/// Hạn mức số ghi chú của một khách (bước 35). Mẫu cho module khác: khai trong `Module::limits`,
/// giữ chỗ trước transaction, xác nhận khi xong, trả lại khi lỗi hay khi xoá.
pub(crate) const NOTES_LIMIT: autoxide_kernel::entitlement::Limit =
    autoxide_kernel::entitlement::Limit::count("notes.notes", "Notes a tenant can keep.");

fn note_reference(id: Uuid) -> String {
    format!("notes.note:{id}")
}

impl NoteService {
    pub(crate) fn new(
        outbox: Outbox,
        entitlements: autoxide_kernel::entitlement::EntitlementsHandle,
    ) -> Self {
        NoteService { outbox, entitlements }
    }

    /// Tạo ghi chú của `actor` trong khách `tenant`. Tên trùng với ghi chú khác của cùng người
    /// thì `notes.title_taken`.
    pub(crate) async fn create(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewNote,
    ) -> Result<Note, Error> {
        // Giữ chỗ hạn mức TRƯỚC transaction ghi của module (SQLite chỉ một bên ghi), xác nhận khi
        // ghi xong, trả lại khi lỗi.
        let id = Uuid::now_v7();
        let reference = note_reference(id);
        self.entitlements.reserve(tenant.id(), &NOTES_LIMIT, 1, &reference).await?;
        let created = self.insert(tenant, principal, input, id).await;
        match &created {
            Ok(_) => self.entitlements.confirm(tenant.id(), &NOTES_LIMIT, &reference).await?,
            Err(_) => self.entitlements.release(tenant.id(), &NOTES_LIMIT, &reference).await?,
        }
        created
    }

    async fn insert(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewNote,
        id: Uuid,
    ) -> Result<Note, Error> {
        let actor = Actor::from(principal);
        let owner = owner(actor)?;
        let title = check(Some(&input.title), Some(&input.body))?.unwrap_or_default();
        let org = match input.org_id {
            Some(org) => OrgId::from_uuid(org),
            None => default_org(tenant, principal)?,
        };
        let now = now();
        let transaction = tenant.database().begin_write().await?;
        // Nút do người gọi nêu: ngoài phạm vi → 403 `org_out_of_scope`; nút đang khoá → `org_frozen`.
        principal.require_write_at(&transaction, &NOTES_WRITE, at(org, owner)).await?;
        let note = entity::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(Some(org.as_uuid())),
            owner_id: Set(owner),
            title: Set(title.clone()),
            body: Set(input.body),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await
        .map_err(|error| title_conflict(error, &title))?;
        let event = events::NoteCreated(Change { before: None, after: Some(snapshot(&note)) });
        self.outbox
            .record(&transaction, tenant, actor, events::note_subject(note.id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(note_id = %note.id, "note created");
        Ok(note)
    }

    /// Một trang ghi chú trong phạm vi `notes.read` của người gọi, mới tạo trước (mặc định).
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<Note>, Error> {
        let scope = principal.scope_condition(&NOTES_READ, Column::OrgId, Some(Column::OwnerId))?;
        let select = Entity::find().filter(scope).scoped(tenant);
        fetch_page(select, Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Ghi chú `id`. Ngoài phạm vi `notes.read` (hay của khách khác) báo như không có.
    pub(crate) async fn get(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Note, Error> {
        let read = tenant.database().read();
        let note =
            Entity::find_by_id(id).scoped(tenant).one(read).await?.ok_or_else(|| not_found(id))?;
        hidden(principal.require_at(read, &NOTES_READ, target(tenant, &note)).await, id)?;
        Ok(note)
    }

    /// Đổi tên hay nội dung ghi chú `id` của `actor`.
    pub(crate) async fn update(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        changes: NoteChanges,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Note, Error> {
        let actor = Actor::from(principal);
        let title = check(changes.title.as_deref(), changes.body.as_deref())?;
        let transaction = tenant.database().begin_write().await?;
        let before = Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(id))?;
        writable(principal, &transaction, tenant, &before).await?;
        let mut row = before.clone().into_active_model();
        if let Some(title) = &title {
            row.title = Set(title.clone());
        }
        if let Some(body) = changes.body {
            row.body = Set(body);
        }
        row.updated_at = Set(now());
        let mut update = Entity::update(row).validate()?.scoped(tenant);
        if let Some(expected) = expected {
            // Kiểm phiên bản ngay trong câu `UPDATE`: người khác vừa sửa thì 0 dòng (D-084).
            update = update.filter(Column::UpdatedAt.eq(expected));
        }
        let after = update.exec(&transaction).await.map_err(|error| match error {
            sea_orm::DbErr::RecordNotUpdated => autoxide_kernel::version::changed(),
            error => title_conflict(error, title.as_deref().unwrap_or_default()),
        })?;
        let change = Change { before: Some(snapshot(&before)), after: Some(snapshot(&after)) };
        let event = events::NoteUpdated(change);
        self.outbox.record(&transaction, tenant, actor, events::note_subject(id), &event).await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Xoá ghi chú `id` của `actor`. Hai request xoá cùng lúc: một bên nhận
    /// `notes.note_not_found`.
    pub(crate) async fn delete(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let note = Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(id))?;
        writable(principal, &transaction, tenant, &note).await?;
        let deleted = Entity::delete_many()
            .filter(Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if deleted.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(not_found(id));
        }
        let event = events::NoteDeleted(Change { before: Some(snapshot(&note)), after: None });
        self.outbox.record(&transaction, tenant, actor, events::note_subject(id), &event).await?;
        transaction.commit().await?;
        self.entitlements.release(tenant.id(), &NOTES_LIMIT, &note_reference(id)).await?;
        Ok(())
    }
}

/// Mọi ghi chú của người dùng, cho bản xuất dữ liệu cá nhân.
#[async_trait::async_trait]
impl PersonalData for NoteService {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        let notes = Entity::find()
            .filter(Column::OwnerId.eq(user_id))
            .scoped(tenant)
            .order_by_asc(Column::Id)
            .all(tenant.database().read())
            .await?;
        let notes: Vec<Value> = notes
            .iter()
            .map(|note| {
                json!({
                    "id": note.id,
                    "title": note.title,
                    "body": note.body,
                    "created_at": note.created_at,
                    "updated_at": note.updated_at,
                })
            })
            .collect();
        Ok(Value::Array(notes))
    }
}

/// Tài khoản bị xoá thì xoá mọi ghi chú của người đó — trong khách của event (cùng người ở khách
/// khác là tài khoản khác). Nhận lại event cũng không sao: lần sau không còn gì để xoá.
#[async_trait::async_trait]
impl EventHandler for NoteService {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(user_id) = deleted_user(event) else {
            return Ok(());
        };
        let tenant = &event.tenant;
        let deleted = Entity::delete_many()
            .filter(Column::OwnerId.eq(user_id))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        tracing::info!(user_id = %user_id, notes = deleted.rows_affected, "notes of a deleted user removed");
        Ok(())
    }
}

#[async_trait::async_trait]
impl Notes for NoteService {
    async fn find(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<NoteSummary>, Error> {
        let note = Entity::find_by_id(id).scoped(tenant).one(tenant.database().read()).await?;
        Ok(note.map(|note| NoteSummary { id: note.id, owner_id: note.owner_id, title: note.title }))
    }
}

/// Đích kiểm quyền của ghi chú: nút và người sở hữu.
fn target(tenant: &TenantContext, note: &Note) -> OrgTarget {
    let org = note.org_id.map_or(root_of(tenant.id()), OrgId::from_uuid);
    OrgTarget::new(org, Some(note.owner_id))
}

fn at(org: OrgId, owner: Uuid) -> OrgTarget {
    OrgTarget::new(org, Some(owner))
}

/// Sửa / xoá ghi chú: cần `notes.write` phủ ghi chú (không phủ mà đọc được thì 403, đọc cũng
/// không được thì 404 — không lộ ghi chú tồn tại); nút đang khoá → `org_frozen`.
async fn writable<C: sea_orm::ConnectionTrait>(
    principal: &Principal,
    transaction: &C,
    tenant: &TenantContext,
    note: &Note,
) -> Result<(), Error> {
    let target = target(tenant, note);
    if principal.require_at(transaction, &NOTES_READ, target).await.is_err()
        && principal.require_at(transaction, &NOTES_WRITE, target).await.is_err()
    {
        return Err(not_found(note.id));
    }
    principal.require_write_at(transaction, &NOTES_WRITE, target).await
}

/// Ghi chú ngoài phạm vi đọc báo như không có.
fn hidden(result: Result<(), Error>, id: Uuid) -> Result<(), Error> {
    result.map_err(|error| if error.code() == "org_out_of_scope" { not_found(id) } else { error })
}

/// Nút của ghi chú mới khi người tạo không nêu: gốc nếu họ có `notes.write` cho cả khách; nút
/// duy nhất nếu mọi grant ở một nút; nhiều nút thì phải chọn (`notes.org_required`).
fn default_org(tenant: &TenantContext, principal: &Principal) -> Result<OrgId, Error> {
    let grants = principal.grants(&NOTES_WRITE);
    if grants.is_empty() || grants.iter().any(|grant| grant.scope == Scope::Tenant) {
        return Ok(root_of(tenant.id()));
    }
    let first = grants[0].org;
    if grants.iter().all(|grant| grant.org == first) {
        return Ok(first);
    }
    Err(Error::new(
        autoxide_kernel::error::ErrorKind::Validation,
        "notes.org_required",
        "You work in several organization units: give `org_id` for the note.",
    )
    .with_field(FieldError::new(
        "org_id",
        "required",
        "you work in several organization units: choose the unit of the note",
    )))
}

/// Người dùng của `actor`. Ghi chú luôn thuộc một người; "hệ thống" không có ghi chú.
fn owner(actor: Actor) -> Result<Uuid, Error> {
    actor.user_id.ok_or_else(|| Error::internal("notes need a signed-in user"))
}

/// Độ dài tối đa của tên và nội dung, theo ký tự.
const TITLE_MAX_LENGTH: usize = 200;
const BODY_MAX_LENGTH: usize = 10_000;

/// Kiểm tên (bỏ khoảng trắng hai đầu, 1 tới 200 ký tự) và nội dung (tối đa 10000 ký
/// tự) — trường `None` bỏ qua. Mọi lỗi báo một lần. Trả tên đã chuẩn hoá.
fn check(title: Option<&str>, body: Option<&str>) -> Result<Option<String>, Error> {
    let title = title.map(str::trim);
    let mut problems = Vec::new();
    if title.is_some_and(|title| !(1..=TITLE_MAX_LENGTH).contains(&title.chars().count())) {
        problems.push(
            FieldError::new(
                "title",
                "length",
                "must be 1 to {title_max_length} characters, not only spaces",
            )
            .with_param("title_max_length", TITLE_MAX_LENGTH),
        );
    }
    if body.is_some_and(|body| body.chars().count() > BODY_MAX_LENGTH) {
        problems.push(
            FieldError::new("body", "length", "must be at most {body_max_length} characters long")
                .with_param("body_max_length", BODY_MAX_LENGTH),
        );
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    Ok(title.map(str::to_owned))
}

/// Vi phạm index unique (chủ, tên) thành lỗi 409 dễ hiểu; lỗi khác giữ nguyên.
fn title_conflict(error: sea_orm::DbErr, title: &str) -> Error {
    match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => Error::conflict(
            "notes.title_taken",
            "You already have a note titled `{title}`. Choose another title.",
        )
        .with_param("title", title),
        _ => Error::from(error),
    }
}

fn not_found(id: Uuid) -> Error {
    Error::not_found("notes.note_not_found", "You have no note `{id}`.").with_param("id", id)
}

/// Những gì audit cần biết về một ghi chú.
fn snapshot(note: &Note) -> Value {
    json!({"title": note.title, "body": note.body})
}

/// Bây giờ, cắt tới micro giây: Postgres lưu tới micro giây, SQLite lưu nguyên. Cắt
/// trước để hai database trả về cùng một giá trị.
fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Bộ đếm thật cho job đối soát: số ghi chú của khách.
pub(crate) struct NoteCounter;

#[async_trait::async_trait]
impl autoxide_kernel::entitlement::UsageCounter for NoteCounter {
    async fn count(&self, tenant: &TenantContext) -> Result<i64, Error> {
        use sea_orm::PaginatorTrait;
        let found = Entity::find().scoped(tenant).count(tenant.database().read()).await?;
        Ok(i64::try_from(found).unwrap_or(i64::MAX))
    }
}
