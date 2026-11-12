//! Kiểm duyệt (quản trị viên, tool MCP): hàng chờ có lọc và tìm không dấu, đổi trạng thái (duyệt,
//! từ chối, spam, giữ lại), xoá hẳn, cấm / bỏ cấm, xuất / xoá theo email. Mọi thao tác ghi event
//! (module `audit` ghi lại: ai, lúc nào, làm gì — phục vụ yêu cầu gỡ trong 24 giờ của Nghị định
//! 147/2024 nếu áp dụng). Event không mang chữ hay email.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use serde_json::json;
use uuid::Uuid;

use crate::authoring::{Deletion, soft_delete, update_if_unchanged};
use crate::entity::{ban, comment, report, subscription};
use crate::events::{self, Change};
use crate::permissions::{COMMENTS_EXPORT, COMMENTS_MANAGE, COMMENTS_MODERATE, COMMENTS_READ};
use crate::service::{
    Comment, CommentsService, Status, comment_not_found, now, shift_counts, summary,
};
use crate::text;

/// Bộ lọc hàng chờ.
#[derive(Debug, Clone, Default)]
pub(crate) struct QueueFilter {
    /// `None`: `pending` (hàng chờ duyệt).
    pub status: Option<Status>,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    /// Chữ hay tên chứa chuỗi này (không phân biệt hoa thường, dấu).
    pub search: Option<String>,
}

/// Một lệnh cấm mới.
#[derive(Debug, Clone, Default)]
pub(crate) struct NewBan {
    /// `user`, `email` hoặc `ip`.
    pub kind: String,
    pub value: String,
    pub reason: String,
    pub until: Option<DateTime<Utc>>,
}

/// Bộ lọc danh sách lệnh cấm.
#[derive(Debug, Clone, Default)]
pub(crate) struct BanFilter {
    pub kind: Option<String>,
    /// Chỉ lệnh còn hiệu lực.
    pub active: bool,
}

/// Mọi thứ module giữ về một email.
#[derive(Debug, Clone)]
pub(crate) struct EmailData {
    pub comments: Vec<Comment>,
    pub subscriptions: Vec<subscription::Model>,
    pub bans: Vec<ban::Model>,
}

fn deleted_by_author() -> Error {
    Error::conflict(
        "comments.deleted",
        "This comment was deleted; its status cannot change. Delete it for good if needed.",
    )
}

fn ban_not_found(id: Uuid) -> Error {
    Error::not_found("comments.ban_not_found", "No ban has the id `{id}`.").with_param("id", id)
}

pub(crate) const BAN_KINDS: &str = "`user`, `email`, `ip`";

impl CommentsService {
    /// Một trang của hàng chờ (mặc định `pending`), mới trước. Cần `comments.read`.
    pub(crate) async fn queue(
        &self,
        principal: &Principal,
        filter: &QueueFilter,
        page: &PageRequest,
    ) -> Result<Page<Comment>, Error> {
        principal.require(&COMMENTS_READ)?;
        let status = filter.status.unwrap_or(Status::Pending);
        let mut select =
            comment::Entity::find().filter(comment::Column::Status.eq(status.as_str()));
        if let Some(kind) = filter.target_type.as_deref().filter(|kind| !kind.is_empty()) {
            select = select.filter(comment::Column::TargetType.eq(kind));
        }
        if let Some(id) = filter.target_id.as_deref().filter(|id| !id.is_empty()) {
            select = select.filter(comment::Column::TargetId.eq(id));
        }
        if let Some(search) = filter.search.as_deref().filter(|search| !search.trim().is_empty()) {
            select = select
                .filter(autoxide_kernel::search::contains(comment::Column::SearchKey, search));
        }
        let Page { data, pagination } =
            fetch_page(select, comment::Column::Id, |row| row.id, page, self.database.read())
                .await?;
        Ok(Page { data: self.named(data).await?, pagination })
    }

    /// Bình luận `id`, mọi trạng thái. Cần `comments.read`.
    pub(crate) async fn get(&self, principal: &Principal, id: Uuid) -> Result<Comment, Error> {
        principal.require(&COMMENTS_READ)?;
        self.find_named(id).await
    }

    /// Đổi trạng thái bình luận `id`: `published` (duyệt / khôi phục), `rejected`, `spam`,
    /// `pending` (giữ lại). Cần `comments.moderate`. `expected`: phiên bản đã đọc (`If-Match`).
    pub(crate) async fn set_status(
        &self,
        principal: &Principal,
        id: Uuid,
        to: Status,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Comment, Error> {
        principal.require(&COMMENTS_MODERATE)?;
        if to == Status::Deleted {
            return Err(Error::validation(vec![FieldError::new(
                "status",
                "invalid",
                "must be `published`, `rejected`, `spam` or `pending`; delete with `DELETE`",
            )]));
        }
        let before = self.find(id).await?;
        if expected.is_some_and(|expected| expected != before.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        let from = Status::of(&before);
        if from == Status::Deleted {
            return Err(deleted_by_author());
        }
        if from == to {
            return self.find_named(id).await;
        }
        let now = now();
        let mut row = before.clone().into_active_model();
        row.status = Set(to.as_str().to_owned());
        row.updated_at = Set(now);
        match to {
            Status::Published => {
                row.hold_reason = Set(None);
                if before.published_at.is_none() {
                    row.published_at = Set(Some(now));
                }
            }
            Status::Pending => row.hold_reason = Set(Some("moderation".to_owned())),
            _ => {}
        }
        let transaction = self.database.begin_write().await?;
        let after = update_if_unchanged(&transaction, row, &before).await?;
        shift_counts(&transaction, &after, Some(from), Some(to)).await?;
        let actor = Actor::from(principal);
        let subject = events::comment_subject(id);
        let change = Change { before: Some(summary(&before)), after: Some(summary(&after)) };
        self.outbox
            .record(&transaction, actor, subject.clone(), &events::CommentStatusChanged(change))
            .await?;
        if to == Status::Published {
            let change = Change { before: Some(summary(&before)), after: Some(summary(&after)) };
            self.outbox
                .record(&transaction, actor, subject, &events::CommentPublished(change))
                .await?;
        }
        transaction.commit().await?;
        tracing::info!(comment_id = %id, from = from.as_str(), to = to.as_str(), "comment moderated");
        self.named(vec![after]).await?.into_iter().next().ok_or_else(|| comment_not_found(id))
    }

    /// Xoá hẳn bình luận `id` (bình luận gốc: cả luồng). Cần `comments.moderate`. Trả số bình
    /// luận đã xoá.
    pub(crate) async fn hard_delete(&self, principal: &Principal, id: Uuid) -> Result<u64, Error> {
        principal.require(&COMMENTS_MODERATE)?;
        let row = self.find(id).await?;
        let transaction = self.database.begin_write().await?;
        let rows = if row.depth == 0 {
            comment::Entity::find().filter(comment::Column::RootId.eq(id)).all(&transaction).await?
        } else {
            comment::Entity::find_by_id(id).all(&transaction).await?
        };
        if rows.is_empty() {
            return Err(comment_not_found(id));
        }
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        for found in &rows {
            shift_counts(&transaction, found, Some(Status::of(found)), None).await?;
            self.tokens.forget(&transaction, &found.id.to_string()).await?;
        }
        report::Entity::delete_many()
            .filter(report::Column::CommentId.is_in(ids.clone()))
            .exec(&transaction)
            .await?;
        subscription::Entity::delete_many()
            .filter(subscription::Column::CommentId.is_in(ids.clone()))
            .exec(&transaction)
            .await?;
        let deleted = comment::Entity::delete_many()
            .filter(comment::Column::Id.is_in(ids))
            .exec(&transaction)
            .await?
            .rows_affected;
        let change = Change {
            before: Some(summary(&row)),
            after: Some(json!({"hard": true, "count": deleted})),
        };
        self.outbox
            .record(
                &transaction,
                Actor::from(principal),
                events::comment_subject(id),
                &events::CommentDeleted(change),
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(comment_id = %id, count = deleted, "comment deleted for good");
        Ok(deleted)
    }

    /// Một trang lệnh cấm, mới trước. Cần `comments.manage`.
    pub(crate) async fn bans(
        &self,
        principal: &Principal,
        filter: &BanFilter,
        page: &PageRequest,
    ) -> Result<Page<ban::Model>, Error> {
        principal.require(&COMMENTS_MANAGE)?;
        let mut select = ban::Entity::find();
        if let Some(kind) = filter.kind.as_deref() {
            select = select.filter(ban::Column::Kind.eq(kind));
        }
        if filter.active {
            select = select.filter(
                sea_orm::Condition::any()
                    .add(ban::Column::Until.is_null())
                    .add(ban::Column::Until.gt(now())),
            );
        }
        fetch_page(select, ban::Column::Id, |row| row.id, page, self.database.read()).await
    }

    /// Cấm một người bình luận. Cần `comments.manage`.
    pub(crate) async fn ban(
        &self,
        principal: &Principal,
        input: NewBan,
    ) -> Result<ban::Model, Error> {
        principal.require(&COMMENTS_MANAGE)?;
        let mut problems = Vec::new();
        let raw = input.value.trim();
        let value = match input.kind.as_str() {
            "user" => match raw.parse::<Uuid>() {
                Ok(id) => Some(id.to_string()),
                Err(_) => {
                    problems.push(FieldError::new("value", "invalid", "must be a user id"));
                    None
                }
            },
            "email" => text::checked_email(raw, "value", &mut problems),
            "ip" => {
                let valid = (1..=64).contains(&raw.len())
                    && raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
                if !valid {
                    problems.push(FieldError::new(
                        "value",
                        "invalid",
                        "must be the `ip_fingerprint` of a comment",
                    ));
                }
                valid.then(|| raw.to_owned())
            }
            _ => {
                problems.push(FieldError::new(
                    "kind",
                    "invalid",
                    format!("must be one of {BAN_KINDS}"),
                ));
                None
            }
        };
        let reason = input.reason.trim().to_owned();
        if reason.is_empty() || reason.chars().count() > 500 {
            problems.push(FieldError::new("reason", "length", "must be 1 to 500 characters long"));
        }
        let now = now();
        if input.until.is_some_and(|until| until <= now) {
            problems.push(FieldError::new("until", "invalid", "must be in the future"));
        }
        let Some(value) = value.filter(|_| problems.is_empty()) else {
            return Err(Error::validation(problems));
        };
        let transaction = self.database.begin_write().await?;
        let row = ban::ActiveModel {
            id: Set(Uuid::now_v7()),
            kind: Set(input.kind),
            value: Set(value),
            reason: Set(reason),
            until: Set(input.until),
            created_by: Set(Some(principal.user_id())),
            created_at: Set(now),
        }
        .insert(&transaction)
        .await?;
        let change = Change { before: None, after: Some(ban_summary(&row)) };
        self.outbox
            .record(
                &transaction,
                Actor::from(principal),
                events::ban_subject(row.id),
                &events::BanCreated(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Bỏ lệnh cấm `id`. Cần `comments.manage`.
    pub(crate) async fn unban(&self, principal: &Principal, id: Uuid) -> Result<(), Error> {
        principal.require(&COMMENTS_MANAGE)?;
        let transaction = self.database.begin_write().await?;
        let row = ban::Entity::find_by_id(id)
            .one(&transaction)
            .await?
            .ok_or_else(|| ban_not_found(id))?;
        ban::Entity::delete_by_id(id).exec(&transaction).await?;
        let change = Change { before: Some(ban_summary(&row)), after: None };
        self.outbox
            .record(
                &transaction,
                Actor::from(principal),
                events::ban_subject(id),
                &events::BanDeleted(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Mọi thứ module giữ về `email` (yêu cầu xuất của chủ dữ liệu). Cần `comments.export`.
    pub(crate) async fn email_data(
        &self,
        principal: &Principal,
        email: &str,
    ) -> Result<EmailData, Error> {
        principal.require(&COMMENTS_EXPORT)?;
        let email = checked_email(email)?;
        let mut rows = comment::Entity::find()
            .filter(comment::Column::AuthorEmail.eq(email.as_str()))
            .all(self.database.read())
            .await?;
        rows.sort_by_key(|row| row.id);
        let subscriptions = subscription::Entity::find()
            .filter(subscription::Column::Email.eq(email.as_str()))
            .all(self.database.read())
            .await?;
        let bans = ban::Entity::find()
            .filter(ban::Column::Kind.eq("email"))
            .filter(ban::Column::Value.eq(email.as_str()))
            .all(self.database.read())
            .await?;
        Ok(EmailData { comments: self.named(rows).await?, subscriptions, bans })
    }

    /// Xoá dữ liệu của `email` (yêu cầu xoá của chủ dữ liệu): mọi bình luận viết với email này
    /// bị xoá mềm (chữ, tên, email, dấu vết IP; chỗ trong luồng giữ lại), mọi dòng theo dõi bị
    /// xoá. Lệnh cấm theo email GIỮ LẠI (để còn chặn được; người cài bỏ bằng `DELETE /bans/{id}`).
    /// Cần `comments.export`. Trả số bình luận đã xoá.
    pub(crate) async fn erase_email(
        &self,
        principal: &Principal,
        email: &str,
    ) -> Result<u64, Error> {
        principal.require(&COMMENTS_EXPORT)?;
        let email = checked_email(email)?;
        let transaction = self.database.begin_write().await?;
        let rows = comment::Entity::find()
            .filter(comment::Column::AuthorEmail.eq(email.as_str()))
            .all(&transaction)
            .await?;
        let mut ids = Vec::new();
        for row in &rows {
            soft_delete(self, &transaction, row, Deletion::Erased).await?;
            ids.push(row.id);
        }
        let follows = subscription::Entity::delete_many()
            .filter(subscription::Column::Email.eq(email.as_str()))
            .exec(&transaction)
            .await?
            .rows_affected;
        if !ids.is_empty() || follows > 0 {
            let change = Change {
                before: Some(
                    json!({"comment_ids": ids, "count": ids.len(), "subscriptions": follows}),
                ),
                after: None,
            };
            self.outbox
                .record(&transaction, Actor::from(principal), None, &events::CommentsErased(change))
                .await?;
        }
        transaction.commit().await?;
        tracing::info!(count = ids.len(), "comments of an email address erased");
        Ok(ids.len() as u64)
    }
}

/// Tóm tắt lệnh cấm cho event: không có giá trị (email, dấu vết IP).
fn ban_summary(row: &ban::Model) -> serde_json::Value {
    json!({"id": row.id, "kind": row.kind, "until": row.until})
}

/// Email của yêu cầu xuất / xoá, đã chuẩn hoá; sai dạng thì 422.
fn checked_email(email: &str) -> Result<String, Error> {
    let mut problems = Vec::new();
    match text::checked_email(email, "email", &mut problems) {
        Some(email) => Ok(email),
        None => Err(Error::validation(problems)),
    }
}
