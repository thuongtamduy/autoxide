//! Service của `comments`: MỌI logic nằm ở đây và các file theo tính năng — `posting.rs` +
//! `posting/` (gửi, sửa của khách, đích), `threads.rs` (đọc), `moderation.rs` + `moderation/`
//! (kiểm duyệt, báo cáo), `notify.rs`. Handler REST và tool MCP chỉ lấy dữ liệu vào rồi gọi service
//! — cùng kết quả, cùng permission (luật kiến trúc 8). Mỗi lệnh ghi đi cùng event trong CÙNG
//! transaction (luật kiến trúc 3); gọi mạng (chống spam, sổ đối tượng, gửi mail) nằm NGOÀI
//! transaction.
//!
//! Số đếm (`comments_targets.published_count` / `pending_count`, `reply_count` của bình luận gốc)
//! đổi trong CÙNG transaction với trạng thái, bằng `UPDATE … SET n = n + k` (không đọc rồi ghi),
//! nên đúng cả khi nhiều request chạy cùng lúc, trên cả hai database.

use autoxide_kernel::time::{Duration, Timestamp};
use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::access::PostingVerification;
use autoxide_kernel::antispam::SpamGuard;
use autoxide_kernel::db::{AcceptedBy, Expr, Queries, TenantKey, TenantTx};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Outbox;
use autoxide_kernel::http::Schema;
use autoxide_kernel::i18n::Translations;
use autoxide_kernel::mail::Mailer;
use autoxide_kernel::resource::ResourceDirectory;
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext, TenantSettings, parsed_setting};
use autoxide_kernel::tokens::OneTimeTokens;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::config::{self, CommentsConfig, Rules};
use crate::entity::{comment, profile, target};

/// Tên module (bảng, khoá đếm, nonce, token).
pub(crate) const MODULE: &str = "comments";

pub(crate) struct CommentsService {
    /// Sổ database: endpoint nhận token trong link tìm khách từ token.
    pub(crate) databases: DatabaseRegistry,
    pub(crate) outbox: Outbox,
    pub(crate) guard: Arc<SpamGuard>,
    pub(crate) secret: SecretKey,
    pub(crate) mailer: Arc<dyn Mailer>,
    pub(crate) translations: Translations,
    pub(crate) resources: ResourceDirectory,
    pub(crate) verification: PostingVerification,
    pub(crate) tokens: OneTimeTokens,
    pub(crate) config: CommentsConfig,
    /// Cấu hình theo khách (từ cấm, chế độ duyệt…) của kernel.
    pub(crate) settings: Arc<dyn TenantSettings>,
    /// Link trong thư theo khách (`kernel.public_url`, `comments.link.*`).
    pub(crate) links: autoxide_kernel::tenancy::SiteLinks,
}

/// Thứ service cần từ kernel và module khác.
pub(crate) struct Parts {
    pub databases: DatabaseRegistry,
    pub outbox: Outbox,
    pub guard: Arc<SpamGuard>,
    pub secret: SecretKey,
    pub mailer: Arc<dyn Mailer>,
    pub translations: Translations,
    pub resources: ResourceDirectory,
    pub verification: PostingVerification,
    pub settings: Arc<dyn TenantSettings>,
    pub links: autoxide_kernel::tenancy::SiteLinks,
}

impl CommentsService {
    pub(crate) fn new(parts: Parts, config: CommentsConfig) -> Self {
        CommentsService {
            databases: parts.databases,
            outbox: parts.outbox,
            guard: parts.guard,
            secret: parts.secret,
            mailer: parts.mailer,
            translations: parts.translations,
            resources: parts.resources,
            verification: parts.verification,
            tokens: OneTimeTokens::new(MODULE),
            config,
            settings: parts.settings,
            links: parts.links,
        }
    }

    /// Luật kiểm duyệt của khách `tenant`: cấu hình khách đặt (`comments.banned_words`…), thiếu
    /// thì biến môi trường. Đọc qua cache của kernel, không thêm truy vấn mỗi request.
    pub(crate) async fn rules(&self, tenant: &TenantContext) -> Result<Rules, Error> {
        let settings = self.settings.as_ref();
        let mut rules = Rules::of(&self.config);
        if let Some(words) =
            parsed_setting(settings, tenant, &config::TENANT_BANNED_WORDS, config::tenant_words)
                .await?
        {
            rules.banned_words = words;
        }
        if let Some(words) =
            parsed_setting(settings, tenant, &config::TENANT_SUSPECT_WORDS, config::tenant_words)
                .await?
        {
            rules.suspect_words = words;
        }
        if let Some(links) =
            parsed_setting(settings, tenant, &config::TENANT_MAX_LINKS, config::tenant_max_links)
                .await?
        {
            rules.max_links = links;
        }
        if let Some(mode) =
            parsed_setting(settings, tenant, &config::TENANT_GUEST_MODERATION, config::tenant_mode)
                .await?
        {
            rules.guest_mode = mode;
        }
        if let Some(mode) =
            parsed_setting(settings, tenant, &config::TENANT_USER_MODERATION, config::tenant_mode)
                .await?
        {
            rules.user_mode = mode;
        }
        Ok(rules)
    }
}

/// Where a comment stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Schema)]
#[serde(rename_all = "snake_case")]
#[schema(as = CommentStatus)]
pub(crate) enum Status {
    /// Waiting for approval; deleted after `AUTOXIDE_COMMENTS_RETENTION_DAYS` if it was never
    /// shown.
    Pending,
    /// Shown.
    Published,
    /// Refused by a moderator; not shown.
    Rejected,
    /// Held by the anti-spam checks or marked by a moderator; deleted after
    /// `AUTOXIDE_COMMENTS_RETENTION_DAYS` if it was never shown.
    Spam,
    /// Deleted by its author (or after an erasure request): text and name are gone, its place in
    /// the thread stays.
    Deleted,
}

impl Status {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Published => "published",
            Status::Rejected => "rejected",
            Status::Spam => "spam",
            Status::Deleted => "deleted",
        }
    }

    pub(crate) fn parse(raw: &str) -> Option<Status> {
        match raw {
            "pending" => Some(Status::Pending),
            "published" => Some(Status::Published),
            "rejected" => Some(Status::Rejected),
            "spam" => Some(Status::Spam),
            "deleted" => Some(Status::Deleted),
            _ => None,
        }
    }

    /// Trạng thái đã lưu (do chính module ghi).
    pub(crate) fn of(row: &comment::Model) -> Status {
        Status::parse(&row.status).unwrap_or(Status::Pending)
    }
}

pub(crate) const STATUSES: &str = "`pending`, `published`, `rejected`, `spam`, `deleted`";

/// Bây giờ, cắt tới micro giây để hai database trả cùng giá trị.
pub(crate) fn now() -> Timestamp {
    Timestamp::now()
}

pub(crate) use autoxide_kernel::db::is_unique_violation;

pub(crate) fn comment_not_found(id: Uuid) -> Error {
    Error::not_found("comments.not_found", "No comment has the id `{id}`.").with_param("id", id)
}

/// Tóm tắt bình luận cho event: không chữ, không tên, không email.
pub(crate) fn summary(row: &comment::Model) -> Value {
    json!({
        "id": row.id,
        "target_type": row.target_type,
        "target_id": row.target_id,
        "root_id": row.root_id,
        "parent_id": row.parent_id,
        "reply_to_id": row.reply_to_id,
        "author_kind": row.author_kind,
        "status": row.status,
    })
}

/// Dòng `comments_targets` của đích, tạo nếu chưa có (`ON CONFLICT DO NOTHING`: hai request
/// cùng lúc ra một dòng). Đích mới tự đóng sau `AUTOXIDE_COMMENTS_AUTO_CLOSE_DAYS` (nếu đặt).
pub(crate) async fn ensure_target(
    transaction: &TenantTx,
    tenant: &TenantContext,
    config: &CommentsConfig,
    kind: &str,
    id: &str,
    now: Timestamp,
) -> Result<(), Error> {
    let closes_at = (config.auto_close_days > 0)
        .then(|| now + Duration::from_days(i64::from(config.auto_close_days)));
    let row = target::Model {
        tenant_id: TenantKey::new(tenant.id()),
        target_type: kind.to_owned(),
        target_id: id.to_owned(),
        uid: Some(Uuid::now_v7()),
        state: "open".to_owned(),
        hidden: false,
        closes_at,
        published_count: 0,
        pending_count: 0,
        published_thread_count: 0,
        pending_thread_count: 0,
        created_at: now,
        updated_at: now,
    };
    transaction
        .insert(row)
        .on_conflict_do_nothing([
            target::Model::TENANT_ID.key(),
            target::Model::TARGET_TYPE.key(),
            target::Model::TARGET_ID.key(),
        ])
        .exec()
        .await?;
    Ok(())
}

/// Đổi số đếm khi bình luận `row` chuyển từ `from` sang `to` (`None`: chưa có / đã xoá hẳn).
/// Gọi trong transaction của thay đổi.
///
/// Thứ tự khoá cố định trong cả module: dòng bình luận đang đổi → dòng bình luận gốc
/// (`reply_count`) → dòng đích. Mọi lệnh ghi đi cùng một thứ tự thì Postgres không gặp
/// deadlock (hai transaction giữ khoá của nhau rồi chờ nhau).
pub(crate) async fn shift_counts(
    transaction: &TenantTx,
    row: &comment::Model,
    from: Option<Status>,
    to: Option<Status>,
) -> Result<(), Error> {
    let published = delta(from, to, Status::Published);
    if published != 0 && row.depth > 0 {
        transaction
            .update::<comment::Model>()
            .set_expr(
                comment::Model::REPLY_COUNT,
                comment::Model::REPLY_COUNT.expr() + Expr::value(published),
            )
            .filter(comment::Model::ID.eq(row.root_id))
            .exec()
            .await?;
    }
    shift_target_counts(transaction, row, from, to).await
}

/// `+1` / `-1` / `0`: bình luận vào hay ra khỏi trạng thái `status`.
pub(crate) fn delta(from: Option<Status>, to: Option<Status>, status: Status) -> i32 {
    i32::from(to == Some(status)) - i32::from(from == Some(status))
}

/// Phần số đếm của đích trong [`shift_counts`] (không đụng `reply_count`). Bình luận gốc đổi
/// thêm số câu hỏi (`…_thread_count`, D-202) trong cùng câu lệnh.
pub(crate) async fn shift_target_counts(
    transaction: &TenantTx,
    row: &comment::Model,
    from: Option<Status>,
    to: Option<Status>,
) -> Result<(), Error> {
    let (published, pending) =
        (delta(from, to, Status::Published), delta(from, to, Status::Pending));
    if published != 0 || pending != 0 {
        let root = i32::from(row.depth == 0);
        // `n = n + k` trong câu lệnh (không đọc rồi ghi): đúng cả khi nhiều request chạy cùng lúc.
        let plus = |column: autoxide_kernel::db::Col<target::Model, i32>, by: i32| {
            column.expr() + Expr::value(by)
        };
        transaction
            .update::<target::Model>()
            .set_expr(
                target::Model::PUBLISHED_COUNT,
                plus(target::Model::PUBLISHED_COUNT, published),
            )
            .set_expr(target::Model::PENDING_COUNT, plus(target::Model::PENDING_COUNT, pending))
            .set_expr(
                target::Model::PUBLISHED_THREAD_COUNT,
                plus(target::Model::PUBLISHED_THREAD_COUNT, published * root),
            )
            .set_expr(
                target::Model::PENDING_THREAD_COUNT,
                plus(target::Model::PENDING_THREAD_COUNT, pending * root),
            )
            .filter(target::Model::TARGET_TYPE.eq(row.target_type.clone()))
            .filter(target::Model::TARGET_ID.eq(row.target_id.clone()))
            .exec()
            .await?;
    }
    Ok(())
}

/// Tên hiển thị của người dùng `user_ids` (bảng hồ sơ của module), đọc qua handle `db` của khách
/// (đọc hay transaction ghi).
pub(crate) async fn display_names<H>(
    db: &H,
    user_ids: impl IntoIterator<Item = Uuid>,
) -> Result<HashMap<Uuid, String>, Error>
where
    H: Queries,
    autoxide_kernel::db::Tenant: AcceptedBy<H::Access>,
{
    let mut ids: Vec<Uuid> = user_ids.into_iter().collect();
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(db
        .find::<profile::Model>()
        .filter(profile::Model::USER_ID.is_in(ids))
        .all()
        .await?
        .into_iter()
        .map(|row| (row.user_id, row.display_name))
        .collect())
}

/// Mã nhãn đã lưu của bình luận (cột `badges`), theo thứ tự lưu.
pub(crate) fn badges_of(row: &comment::Model) -> Vec<String> {
    row.badges.split(',').filter(|code| !code.is_empty()).map(str::to_owned).collect()
}

/// Một bình luận kèm tên hiển thị của tác giả.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Comment {
    pub row: comment::Model,
    /// Tên của khách, hay tên hồ sơ của người dùng; `None` khi đã xoá.
    pub name: Option<String>,
}

impl Comment {
    pub(crate) fn status(&self) -> Status {
        Status::of(&self.row)
    }
}

impl CommentsService {
    /// Gắn tên hiển thị cho `rows` (một câu truy vấn hồ sơ cho cả trang).
    pub(crate) async fn named(
        &self,
        tenant: &TenantContext,
        rows: Vec<comment::Model>,
    ) -> Result<Vec<Comment>, Error> {
        let names =
            display_names(&tenant.db().read(), rows.iter().filter_map(|row| row.user_id)).await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let name = match row.user_id {
                    Some(user) => names.get(&user).cloned(),
                    None => row.author_name.clone(),
                };
                let name = if Status::of(&row) == Status::Deleted { None } else { name };
                Comment { row, name }
            })
            .collect())
    }

    /// Bình luận `id`, mọi trạng thái.
    pub(crate) async fn find(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<comment::Model, Error> {
        tenant
            .db()
            .read()
            .find::<comment::Model>()
            .filter(comment::Model::ID.eq(id))
            .one()
            .await?
            .ok_or_else(|| comment_not_found(id))
    }

    pub(crate) async fn find_named(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Comment, Error> {
        let row = self.find(tenant, id).await?;
        self.named(tenant, vec![row])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| Error::internal("named returned nothing"))
    }
}
