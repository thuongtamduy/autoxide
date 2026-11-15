//! Service của ghi chú: MỌI logic nằm ở đây. Handler REST và tool MCP chỉ lấy dữ liệu
//! vào, kiểm permission, gọi service — nên hai đường cho cùng một kết quả.
//!
//! Mỗi lệnh ghi đi cùng một event trong CÙNG transaction (luật kiến trúc 3): thay đổi
//! và dòng audit log hoặc cùng có, hoặc cùng không.

use autoxide_kernel::database::Database;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Outbox};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::tenancy::TenantContext;
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

/// Một ghi chú.
pub(crate) type Note = entity::Model;

/// Dữ liệu để tạo ghi chú (đã qua kiểm độ dài ở REST / MCP).
#[derive(Debug, Clone)]
pub(crate) struct NewNote {
    pub title: String,
    pub body: String,
}

/// Thay đổi cho một ghi chú; trường `None` giữ nguyên.
#[derive(Debug, Clone, Default)]
pub(crate) struct NoteChanges {
    pub title: Option<String>,
    pub body: Option<String>,
}

pub(crate) struct NoteService {
    database: Database,
    outbox: Outbox,
}

impl NoteService {
    pub(crate) fn new(database: Database, outbox: Outbox) -> Self {
        NoteService { database, outbox }
    }

    /// Tạo ghi chú của `actor`. Tên trùng với ghi chú khác của cùng người thì
    /// `notes.title_taken`.
    pub(crate) async fn create(&self, actor: Actor, input: NewNote) -> Result<Note, Error> {
        let owner = owner(actor)?;
        let title = check(Some(&input.title), Some(&input.body))?.unwrap_or_default();
        let now = now();
        let transaction = self.database.begin_write().await?;
        let note = entity::ActiveModel {
            id: Set(Uuid::now_v7()),
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
            .record(
                &transaction,
                &TenantContext::legacy_default(&self.database),
                actor,
                events::note_subject(note.id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(note_id = %note.id, "note created");
        Ok(note)
    }

    /// Một trang ghi chú của `actor`, mới tạo trước (mặc định).
    pub(crate) async fn list(&self, actor: Actor, page: &PageRequest) -> Result<Page<Note>, Error> {
        let select = Entity::find().filter(Column::OwnerId.eq(owner(actor)?));
        fetch_page(select, Column::Id, |row| row.id, page, self.database.read()).await
    }

    /// Ghi chú `id` của `actor`. Ghi chú của người khác báo như không có.
    pub(crate) async fn get(&self, actor: Actor, id: Uuid) -> Result<Note, Error> {
        let note = Entity::find_by_id(id).one(self.database.read()).await?;
        note.filter(|note| Some(note.owner_id) == actor.user_id).ok_or_else(|| not_found(id))
    }

    /// Đổi tên hay nội dung ghi chú `id` của `actor`.
    pub(crate) async fn update(
        &self,
        actor: Actor,
        id: Uuid,
        changes: NoteChanges,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Note, Error> {
        let title = check(changes.title.as_deref(), changes.body.as_deref())?;
        let transaction = self.database.begin_write().await?;
        let before = Entity::find_by_id(id)
            .one(&transaction)
            .await?
            .filter(|note| Some(note.owner_id) == actor.user_id)
            .ok_or_else(|| not_found(id))?;
        let mut row = before.clone().into_active_model();
        if let Some(title) = &title {
            row.title = Set(title.clone());
        }
        if let Some(body) = changes.body {
            row.body = Set(body);
        }
        row.updated_at = Set(now());
        let mut update = Entity::update(row).validate()?;
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
        self.outbox
            .record(
                &transaction,
                &TenantContext::legacy_default(&self.database),
                actor,
                events::note_subject(id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Xoá ghi chú `id` của `actor`. Hai request xoá cùng lúc: một bên nhận
    /// `notes.note_not_found`.
    pub(crate) async fn delete(&self, actor: Actor, id: Uuid) -> Result<(), Error> {
        let transaction = self.database.begin_write().await?;
        let note = Entity::find_by_id(id)
            .one(&transaction)
            .await?
            .filter(|note| Some(note.owner_id) == actor.user_id)
            .ok_or_else(|| not_found(id))?;
        let deleted = Entity::delete_by_id(id).exec(&transaction).await?;
        if deleted.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(not_found(id));
        }
        let event = events::NoteDeleted(Change { before: Some(snapshot(&note)), after: None });
        self.outbox
            .record(
                &transaction,
                &TenantContext::legacy_default(&self.database),
                actor,
                events::note_subject(id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}

/// Mọi ghi chú của người dùng, cho bản xuất dữ liệu cá nhân.
#[async_trait::async_trait]
impl PersonalData for NoteService {
    async fn export(&self, user_id: Uuid) -> Result<Value, Error> {
        let notes = Entity::find()
            .filter(Column::OwnerId.eq(user_id))
            .order_by_asc(Column::Id)
            .all(self.database.read())
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

/// Tài khoản bị xoá thì xoá mọi ghi chú của người đó. Nhận lại event cũng không sao:
/// lần sau không còn gì để xoá.
#[async_trait::async_trait]
impl EventHandler for NoteService {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(user_id) = deleted_user(event) else {
            return Ok(());
        };
        let deleted = Entity::delete_many()
            .filter(Column::OwnerId.eq(user_id))
            .exec(self.database.write())
            .await?;
        tracing::info!(user_id = %user_id, notes = deleted.rows_affected, "notes of a deleted user removed");
        Ok(())
    }
}

#[async_trait::async_trait]
impl Notes for NoteService {
    async fn find(&self, id: Uuid) -> Result<Option<NoteSummary>, Error> {
        let note = Entity::find_by_id(id).one(self.database.read()).await?;
        Ok(note.map(|note| NoteSummary { id: note.id, owner_id: note.owner_id, title: note.title }))
    }
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
        problems.push(FieldError::new(
            "title",
            "length",
            format!("must be 1 to {TITLE_MAX_LENGTH} characters, not only spaces"),
        ));
    }
    if body.is_some_and(|body| body.chars().count() > BODY_MAX_LENGTH) {
        problems.push(FieldError::new(
            "body",
            "length",
            format!("must be at most {BODY_MAX_LENGTH} characters long"),
        ));
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
