//! Việc của tác giả: sửa trong thời hạn (`AUTOXIDE_COMMENTS_EDIT_MINUTES`), tự xoá (xoá mềm, giữ
//! chỗ trong luồng), tên hiển thị của người dùng đăng nhập (`comments_profiles`).
//!
//! Tác giả là người dùng có cùng `user_id`, hay khách gửi đúng token sửa nhận lúc tạo (máy chủ chỉ
//! giữ bản băm SHA-256). Sửa chạy lại phần kiểm nội dung của pipeline (lọc, độ dài, bị cấm, từ
//! cấm, từ nghi ngờ, link) — chặn trò "đăng chữ sạch rồi sửa thành quảng cáo": bình luận đang hiện
//! mà dính từ nghi ngờ / quá nhiều link, hay chế độ duyệt của tác giả trên đích đó (`pre`,
//! `first_time` khi chưa có bình luận nào đang hiện) sẽ giữ một bình luận mới, thì về chờ duyệt.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::secret::sha256;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{comment, profile, subscription};
use crate::events::{self, Change};
use crate::permissions::{COMMENTS_MODERATE, COMMENTS_POST};
use crate::posting::{banned, banned_word};
use crate::service::{
    Comment, CommentsService, Status, is_unique_violation, now, shift_counts, summary,
};
use crate::text::{self, Format};

fn not_author() -> Error {
    Error::forbidden(
        "comments.not_author",
        "Only the author may change or delete this comment: sign in as its author, or send the \
         `edit_token` received when it was posted.",
    )
}

fn edit_window_passed() -> Error {
    Error::conflict(
        "comments.edit_window_passed",
        "The time to change this comment is over. You can still delete it.",
    )
}

fn not_editable() -> Error {
    Error::conflict(
        "comments.not_editable",
        "This comment was rejected or deleted and cannot be changed.",
    )
}

fn name_taken() -> Error {
    Error::conflict("comments.name_taken", "Someone else already shows this name. Choose another.")
}

/// Tên hiển thị của một người dùng.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Profile {
    pub user_id: Uuid,
    pub display_name: String,
    pub updated_at: DateTime<Utc>,
}

impl CommentsService {
    /// Người gọi có phải tác giả của `row` không: cùng người dùng, hay đúng token sửa của khách.
    fn check_author(
        &self,
        row: &comment::Model,
        principal: Option<&Principal>,
        edit_token: Option<&str>,
    ) -> Result<(), Error> {
        let by_user = principal.is_some_and(|principal| {
            row.user_id == Some(principal.user_id()) && principal.has(&COMMENTS_POST)
        });
        let by_token = match (edit_token.map(str::trim), row.edit_token_hash.as_deref()) {
            (Some(token), Some(hash)) if !token.is_empty() => sha256(token) == hash,
            _ => false,
        };
        if by_user || by_token { Ok(()) } else { Err(not_author()) }
    }

    /// Tác giả sửa bình luận `id`.
    pub(crate) async fn edit(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        id: Uuid,
        edit_token: Option<&str>,
        body: &str,
        format: Option<Format>,
    ) -> Result<Comment, Error> {
        let before = self.find(tenant, id).await?;
        self.check_author(&before, principal, edit_token)?;
        let from = Status::of(&before);
        if matches!(from, Status::Rejected | Status::Deleted) {
            return Err(not_editable());
        }
        let now = now();
        let minutes = i64::from(self.config.edit_minutes);
        if minutes == 0 || before.created_at + chrono::Duration::minutes(minutes) < now {
            return Err(edit_window_passed());
        }
        let mut problems = Vec::new();
        let body = text::clean_body(body, self.config.max_length, &mut problems);
        // Không gửi `format`: giữ định dạng đang có của bình luận.
        let format = format.unwrap_or_else(|| Format::from_column(&before.format));
        let format = self.check_format(format, &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        if self
            .is_banned(
                tenant,
                before.user_id,
                before.author_email.as_deref(),
                before.ip_fingerprint.as_deref(),
            )
            .await?
        {
            return Err(banned());
        }
        let rules = self.rules(tenant).await?;
        if text::contains_word(&body, &rules.banned_words) {
            return Err(banned_word());
        }
        let mut holds = Vec::new();
        if text::contains_word(&body, &rules.suspect_words) {
            holds.push("suspect_word");
        }
        if text::links(&body, format) > rules.max_links {
            holds.push("too_many_links");
        }
        // Bình luận đang hiện mà sửa: dính từ nghi ngờ / quá nhiều link thì giữ lại; không thì
        // xét lại chế độ duyệt như một bình luận mới của cùng tác giả trên cùng đích — chế độ
        // `pre` (hay `first_time` khi tác giả chưa có bình luận nào đang hiện) giữ lại bản sửa,
        // để "được duyệt rồi sửa thành chữ khác" không vượt kiểm duyệt.
        let hold = if from != Status::Published {
            None
        } else if !holds.is_empty() {
            Some(holds.join(","))
        } else {
            let author = principal.filter(|principal| before.user_id == Some(principal.user_id()));
            let moderator = author.is_some_and(|principal| principal.has(&COMMENTS_MODERATE));
            self.mode_hold(tenant, &before.target_type, author, moderator).await?.map(str::to_owned)
        };
        let to = if hold.is_some() { Status::Pending } else { from };

        let mut row = before.clone().into_active_model();
        row.search_key = Set(autoxide_kernel::search::fold(&format!(
            "{body} {}",
            before.author_name.as_deref().unwrap_or_default()
        )));
        row.match_key = Set(autoxide_kernel::search::exact_key(&format!(
            "{body} {}",
            before.author_name.as_deref().unwrap_or_default()
        )));
        row.content_hash = Set(autoxide_kernel::antispam::content_hash(&body));
        row.body = Set(body);
        row.format = Set(format.as_str().to_owned());
        row.edited_at = Set(Some(now));
        row.updated_at = Set(now);
        if to != from {
            row.status = Set(to.as_str().to_owned());
            row.hold_reason = Set(hold);
        }
        let transaction = tenant.database().begin_write().await?;
        let after = update_if_unchanged(&transaction, tenant, row, &before).await?;
        shift_counts(&transaction, tenant, &after, Some(from), Some(to)).await?;
        let actor = principal.map_or_else(Actor::public, Actor::from);
        let change = Change { before: Some(summary(&before)), after: Some(summary(&after)) };
        self.outbox
            .record(
                &transaction,
                tenant,
                actor,
                events::comment_subject(id),
                &events::CommentEdited(change),
            )
            .await?;
        transaction.commit().await?;
        self.named(tenant, vec![after])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| Error::internal("named"))
    }

    /// Tác giả xoá bình luận `id` (xoá mềm). Đã xoá rồi thì thôi. Không có bình luận `id` (đã
    /// xoá hẳn, hay là ID giả của bài bị chống spam bỏ đi) cũng trả lời như đã xoá: bot không
    /// phân biệt được bài bị bỏ với bài bị giữ chờ duyệt bằng cách xoá thử.
    pub(crate) async fn delete_own(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        id: Uuid,
        edit_token: Option<&str>,
    ) -> Result<(), Error> {
        let Some(before) =
            comment::Entity::find_by_id(id).scoped(tenant).one(tenant.database().read()).await?
        else {
            return Ok(());
        };
        self.check_author(&before, principal, edit_token)?;
        if Status::of(&before) == Status::Deleted {
            return Ok(());
        }
        let transaction = tenant.database().begin_write().await?;
        let actor = principal.map_or_else(Actor::public, Actor::from);
        soft_delete(self, &transaction, tenant, &before, Deletion::ByAuthor(actor)).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Tên hiển thị của người gọi. Cần `comments.post`.
    pub(crate) async fn profile(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<Option<Profile>, Error> {
        principal.require(&COMMENTS_POST)?;
        Ok(profile::Entity::find_by_id((tenant.id().as_uuid(), principal.user_id()))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .map(|row| Profile {
                user_id: row.user_id,
                display_name: row.display_name,
                updated_at: row.updated_at,
            }))
    }

    /// Đặt tên hiển thị của người gọi. Cần `comments.post`. Tên dành riêng chỉ người kiểm duyệt
    /// dùng được; hai người không trùng tên.
    pub(crate) async fn set_profile(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        display_name: &str,
    ) -> Result<Profile, Error> {
        principal.require(&COMMENTS_POST)?;
        let mut problems = Vec::new();
        let name = text::clean_name(display_name, "display_name", &mut problems);
        if problems.is_empty()
            && !principal.has(&COMMENTS_MODERATE)
            && text::contains_name(&name, &self.config.reserved_names)
        {
            problems.push(FieldError::new(
                "display_name",
                "reserved",
                "is reserved; choose another name",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let now = now();
        let key = text::name_key(&name);
        let existing = profile::Entity::find_by_id((tenant.id().as_uuid(), principal.user_id()))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        let saved = match existing {
            Some(existing) => {
                let mut row = existing.into_active_model();
                row.display_name = Set(name);
                row.name_key = Set(key);
                row.updated_at = Set(now);
                match profile::Entity::update(row).validate() {
                    Ok(update) => update.scoped(tenant).exec(tenant.database().write()).await,
                    Err(error) => Err(error),
                }
            }
            None => {
                profile::ActiveModel {
                    tenant_id: Set(tenant.id().as_uuid()),
                    user_id: Set(principal.user_id()),
                    display_name: Set(name),
                    name_key: Set(key),
                    created_at: Set(now),
                    updated_at: Set(now),
                }
                .insert(tenant.database().write())
                .await
            }
        };
        match saved {
            Ok(row) => Ok(Profile {
                user_id: row.user_id,
                display_name: row.display_name,
                updated_at: row.updated_at,
            }),
            Err(error) if is_unique_violation(&error) => Err(name_taken()),
            Err(error) => Err(error.into()),
        }
    }
}

/// Ghi `row` khi bình luận còn đúng trạng thái và phiên bản của `before` (không thì 412: ai đó
/// vừa đổi nó).
pub(crate) async fn update_if_unchanged<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    row: comment::ActiveModel,
    before: &comment::Model,
) -> Result<comment::Model, Error> {
    let update = comment::Entity::update(row)
        .validate()?
        .scoped(tenant)
        .filter(comment::Column::Status.eq(before.status.as_str()))
        .filter(comment::Column::UpdatedAt.eq(before.updated_at));
    match update.exec(connection).await {
        Ok(after) => Ok(after),
        Err(sea_orm::DbErr::RecordNotUpdated) => Err(autoxide_kernel::version::changed()),
        Err(error) => Err(error.into()),
    }
}

/// Ai xoá mềm, và xoá tới đâu.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Deletion {
    /// Tác giả tự xoá: một event `comments.comment_deleted`; dấu vết IP giữ tới hạn xoá thường
    /// (để còn cấm được kẻ phá).
    ByAuthor(Actor),
    /// Xoá dữ liệu cá nhân (theo email, tài khoản bị xoá): bỏ luôn `user_id` và dấu vết IP; bên
    /// gọi ghi một event chung cho cả lô.
    Erased,
}

/// Xoá mềm `before` trong `connection`: bỏ chữ, tên, email, token sửa; giữ chỗ trong luồng; bỏ
/// theo dõi trả lời và token của nó; số đếm.
pub(crate) async fn soft_delete<C: ConnectionTrait>(
    service: &CommentsService,
    connection: &C,
    tenant: &TenantContext,
    before: &comment::Model,
    how: Deletion,
) -> Result<comment::Model, Error> {
    let now = now();
    let mut row = before.clone().into_active_model();
    row.status = Set(Status::Deleted.as_str().to_owned());
    row.body = Set(String::new());
    row.author_name = Set(None);
    row.author_email = Set(None);
    row.email_verified = Set(false);
    row.edit_token_hash = Set(None);
    row.hold_reason = Set(None);
    row.search_key = Set(String::new());
    row.match_key = Set(String::new());
    row.content_hash = Set(autoxide_kernel::antispam::content_hash(""));
    row.updated_at = Set(now);
    if matches!(how, Deletion::Erased) {
        row.user_id = Set(None);
        row.ip_fingerprint = Set(None);
    }
    let after = update_if_unchanged(connection, tenant, row, before).await?;
    shift_counts(connection, tenant, &after, Some(Status::of(before)), Some(Status::Deleted))
        .await?;
    subscription::Entity::delete_many()
        .filter(subscription::Column::CommentId.eq(before.id))
        .scoped(tenant)
        .exec(connection)
        .await?;
    service.tokens.forget(connection, tenant, &before.id.to_string()).await?;
    if let Deletion::ByAuthor(actor) = how {
        let mut after_summary = summary(&after);
        after_summary["hard"] = json!(false);
        let change = Change { before: Some(summary(before)), after: Some(after_summary) };
        service
            .outbox
            .record(
                connection,
                tenant,
                actor,
                events::comment_subject(before.id),
                &events::CommentDeleted(change),
            )
            .await?;
    }
    Ok(after)
}
