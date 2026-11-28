//! Việc của tác giả: sửa trong thời hạn (`AUTOXIDE_COMMENTS_EDIT_MINUTES`), tự xoá (xoá mềm, giữ
//! chỗ trong luồng), tên hiển thị của người dùng đăng nhập (`comments_profiles`).
//!
//! Tác giả là người dùng có cùng `user_id`, hay khách gửi đúng token sửa nhận lúc tạo (máy chủ chỉ
//! giữ bản băm SHA-256). Sửa chạy lại phần kiểm nội dung của pipeline (lọc, độ dài, bị cấm, từ
//! cấm, từ nghi ngờ, link) — chặn trò "đăng chữ sạch rồi sửa thành quảng cáo": bình luận đang hiện
//! mà dính từ nghi ngờ / quá nhiều link, hay chế độ duyệt của tác giả trên đích đó (`pre`,
//! `first_time` khi chưa có bình luận nào đang hiện) sẽ giữ một bình luận mới, thì về chờ duyệt.

use autoxide_kernel::access::Principal;
use autoxide_kernel::db::{Queries as _, TenantTx};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::secret::sha256;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Duration;
use autoxide_kernel::time::Timestamp;
use autoxide_kernel::tokens::OneTimeTokens;
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
    pub updated_at: Timestamp,
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
        if minutes == 0 || before.created_at + Duration::from_mins(minutes) < now {
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

        let mut row = before.clone();
        row.search_key = autoxide_kernel::search::fold(&format!(
            "{body} {}",
            before.author_name.as_deref().unwrap_or_default()
        ));
        row.match_key = autoxide_kernel::search::exact_key(&format!(
            "{body} {}",
            before.author_name.as_deref().unwrap_or_default()
        ));
        row.content_hash = autoxide_kernel::antispam::content_hash(&body);
        row.body = body;
        row.format = format.as_str().to_owned();
        row.edited_at = Some(now);
        row.updated_at = now;
        if to != from {
            row.status = to.as_str().to_owned();
            row.hold_reason = hold;
        }
        let transaction = tenant.db().begin_write().await?;
        let after = update_if_unchanged(&transaction, row, &before).await?;
        shift_counts(&transaction, &after, Some(from), Some(to)).await?;
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
        let Some(before) = tenant
            .db()
            .read()
            .find::<comment::Model>()
            .filter(comment::Model::ID.eq(id))
            .one()
            .await?
        else {
            return Ok(());
        };
        self.check_author(&before, principal, edit_token)?;
        if Status::of(&before) == Status::Deleted {
            return Ok(());
        }
        let transaction = tenant.db().begin_write().await?;
        let actor = principal.map_or_else(Actor::public, Actor::from);
        soft_delete(
            &self.tokens,
            Some(&self.outbox),
            &transaction,
            tenant,
            &before,
            Deletion::ByAuthor(actor),
        )
        .await?;
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
        Ok(tenant
            .db()
            .read()
            .find::<profile::Model>()
            .filter(profile::Model::USER_ID.eq(principal.user_id()))
            .one()
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
        let transaction = tenant.db().begin_write().await?;
        let existing = transaction
            .find::<profile::Model>()
            .filter(profile::Model::USER_ID.eq(principal.user_id()))
            .one()
            .await?;
        let (row, is_new) = match existing {
            Some(existing) => (
                profile::Model { display_name: name, name_key: key, updated_at: now, ..existing },
                false,
            ),
            None => (
                profile::Model {
                    tenant_id: autoxide_kernel::db::TenantKey::new(tenant.id()),
                    user_id: principal.user_id(),
                    display_name: name,
                    name_key: key,
                    created_at: now,
                    updated_at: now,
                },
                true,
            ),
        };
        let saved = if is_new {
            transaction.insert(row.clone()).exec().await
        } else {
            transaction
                .update::<profile::Model>()
                .set(profile::Model::DISPLAY_NAME, row.display_name.clone())
                .set(profile::Model::NAME_KEY, row.name_key.clone())
                .set(profile::Model::UPDATED_AT, row.updated_at)
                .filter(profile::Model::USER_ID.eq(row.user_id))
                .exec()
                .await
        };
        match saved {
            Ok(_) => {
                transaction.commit().await?;
                Ok(Profile {
                    user_id: row.user_id,
                    display_name: row.display_name,
                    updated_at: row.updated_at,
                })
            }
            Err(error) if is_unique_violation(&error) => Err(name_taken()),
            Err(error) => Err(error),
        }
    }
}

/// Câu `UPDATE` ghi các cột của bình luận `row` (cột không đổi được ghi lại đúng giá trị cũ), lọc theo `id`; bên gọi
/// thêm điều kiện nếu cần. KHÔNG ghi khoá, `created_at` và hai cột đếm (`reply_count`, `report_count`): chúng chỉ đổi
/// bằng `n = n + k` trong câu lệnh (`shift_counts`, báo cáo) mà không đổi `updated_at`, nên ghi lại giá trị đã đọc sẽ
/// xoá mất phần một request khác vừa cộng.
fn write_comment<'t>(
    transaction: &'t TenantTx,
    row: &comment::Model,
) -> autoxide_kernel::db::Update<'t, autoxide_kernel::db::InTenant, comment::Model> {
    use comment::Model as M;
    transaction
        .update::<M>()
        .set(M::TARGET_TYPE, row.target_type.clone())
        .set(M::TARGET_ID, row.target_id.clone())
        .set(M::ROOT_ID, row.root_id)
        .set(M::PARENT_ID, row.parent_id)
        .set(M::REPLY_TO_ID, row.reply_to_id)
        .set(M::DEPTH, row.depth)
        .set(M::AUTHOR_KIND, row.author_kind.clone())
        .set(M::USER_ID, row.user_id)
        .set(M::AUTHOR_NAME, row.author_name.clone())
        .set(M::AUTHOR_EMAIL, row.author_email.clone())
        .set(M::EMAIL_VERIFIED, row.email_verified)
        .set(M::BODY, row.body.clone())
        .set(M::FORMAT, row.format.clone())
        .set(M::STATUS, row.status.clone())
        .set(M::HOLD_REASON, row.hold_reason.clone())
        .set(M::EDIT_TOKEN_HASH, row.edit_token_hash.clone())
        .set(M::EDITED_AT, row.edited_at)
        .set(M::PUBLISHED_AT, row.published_at)
        .set(M::IP_FINGERPRINT, row.ip_fingerprint.clone())
        .set(M::LOCALE, row.locale.clone())
        .set(M::CONTENT_HASH, row.content_hash.clone())
        .set(M::SEARCH_KEY, row.search_key.clone())
        .set(M::MATCH_KEY, row.match_key.clone())
        .set(M::BADGES, row.badges.clone())
        .set(M::REPORTS_HELD_AT, row.reports_held_at)
        .set(M::RESTRICTED_STATUS, row.restricted_status.clone())
        .set(M::UPDATED_AT, row.updated_at)
        .filter(M::ID.eq(row.id))
}

/// Ghi bình luận `row` (đã sửa) theo `id`.
pub(crate) async fn save_comment(
    transaction: &TenantTx,
    row: &comment::Model,
) -> Result<(), Error> {
    write_comment(transaction, row).exec().await?;
    Ok(())
}

/// Ghi `row` (bình luận sau khi sửa) khi bình luận còn đúng trạng thái và phiên bản của `before` (không thì
/// 412: ai đó vừa đổi nó).
pub(crate) async fn update_if_unchanged(
    transaction: &TenantTx,
    row: comment::Model,
    before: &comment::Model,
) -> Result<comment::Model, Error> {
    let updated = write_comment(transaction, &row)
        .filter(comment::Model::STATUS.eq(before.status.clone()))
        .filter(comment::Model::UPDATED_AT.eq(before.updated_at))
        .returning()
        .await?;
    updated.into_iter().next().ok_or_else(autoxide_kernel::version::changed)
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
/// theo dõi trả lời và token của nó; số đếm. Event `comments.comment_deleted` (xoá bởi tác giả) ghi
/// qua `outbox`; `None` chỉ khi module đang tắt (chế độ nghỉ, bước 70 P9 — chỉ xoá dữ liệu cá nhân).
pub(crate) async fn soft_delete(
    tokens: &OneTimeTokens,
    outbox: Option<&Outbox>,
    connection: &TenantTx,
    tenant: &TenantContext,
    before: &comment::Model,
    how: Deletion,
) -> Result<comment::Model, Error> {
    let now = now();
    let mut row = before.clone();
    row.status = Status::Deleted.as_str().to_owned();
    row.body = String::new();
    row.author_name = None;
    row.author_email = None;
    row.email_verified = false;
    row.edit_token_hash = None;
    row.hold_reason = None;
    row.search_key = String::new();
    row.match_key = String::new();
    row.content_hash = autoxide_kernel::antispam::content_hash("");
    row.updated_at = now;
    if matches!(how, Deletion::Erased) {
        row.user_id = None;
        row.ip_fingerprint = None;
    }
    let after = update_if_unchanged(connection, row, before).await?;
    shift_counts(connection, &after, Some(Status::of(before)), Some(Status::Deleted)).await?;
    connection
        .delete::<subscription::Model>()
        .filter(subscription::Model::COMMENT_ID.eq(before.id))
        .exec()
        .await?;
    tokens.forget(connection, tenant, &before.id.to_string()).await?;
    if let (Deletion::ByAuthor(actor), Some(outbox)) = (how, outbox) {
        let mut after_summary = summary(&after);
        after_summary["hard"] = json!(false);
        let change = Change { before: Some(summary(before)), after: Some(after_summary) };
        outbox
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
