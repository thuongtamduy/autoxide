//! Tạo bình luận, theo hợp đồng chung (`docs/prompts/reader-input-contract.md`) và pipeline kiểm
//! duyệt có thứ tự cố định của đề bài F:
//!
//! đích đóng → độ dài → lọc nội dung → trùng → bị cấm → từ cấm (từ chối) / từ nghi ngờ (giữ lại)
//! → quá nhiều link (giữ lại) → `SpamGuard` → chế độ duyệt (`pre` / `first_time` / `post`).
//!
//! 1. Mọi việc có thể gọi mạng (sổ đối tượng, chống spam) nằm NGOÀI transaction.
//! 2. `Discard` → không ghi gì, trả lời như bài bị giữ chờ duyệt (ID và token sửa giả).
//!    `Accept` / `Quarantine` → một transaction: dòng đích (tạo nếu chưa có), bình luận, số đếm,
//!    event, token xác minh email (khi xin mail báo trả lời), rồi nonce của token form
//!    (`Checked::claim`). Nonce đã có → bình luận này đã gửi rồi: rollback, trả lại bình luận lần
//!    đầu (khách nhận token sửa MỚI — token cũ thôi dùng được, vì máy chủ chỉ giữ bản băm).
//!
//! Câu trả lời chỉ nói `published` (đã hiện) hay `pending`: bị giữ chờ duyệt, bị coi là spam hay
//! bị bỏ đều là `pending` — bot không phân biệt được.
//!
//! "Đã từng được duyệt" (chế độ `first_time`) chỉ tính theo TÀI KHOẢN: khách gõ email nào cũng
//! được (kể cả email người khác đã xác minh), nên email không bao giờ giúp vượt kiểm duyệt. Chỉ
//! bình luận ĐANG hiện mới tính (bình luận về sau bị đánh spam / từ chối thì không).

pub(crate) mod authoring;
pub(crate) mod targets;

use jiff::SignedDuration;
use std::time::Duration;

use autoxide_kernel::access::Principal;
use autoxide_kernel::antispam::{self, Claim, Limits, Submission as Check, Verdict};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::secret::{random_token, sha256};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::throttle::Throttle;
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter, Set,
};
use uuid::Uuid;

use crate::config::Mode;
use crate::entity::{ban, comment, profile};
use crate::events::{self, Change};
use crate::notify::VERIFY;
use crate::permissions::{COMMENTS_MODERATE, COMMENTS_POST};
use crate::posting::targets::target_closed;
use crate::service::{
    Comment, CommentsService, MODULE, Status, comment_not_found, delta, ensure_target, now,
    shift_counts, shift_target_counts, summary,
};
use crate::text::{self, Format};

const HOUR: Duration = Duration::from_secs(3600);
const DAY: Duration = Duration::from_secs(24 * 3600);
/// Cùng chữ trên cùng đích trong chừng này giờ thì bị coi là trùng (giữ lại).
const DUPLICATE_HOURS: i64 = 24;
/// Link xác minh email sống bao lâu.
pub(crate) const VERIFY_HOURS: i64 = 72;

/// Một bình luận mới, như client gửi lên.
#[derive(Debug, Clone, Default)]
pub(crate) struct NewComment {
    pub target_type: String,
    pub target_id: String,
    /// Trả lời bình luận này (gốc hay trả lời khác cùng luồng).
    pub parent_id: Option<Uuid>,
    pub body: String,
    /// `text` (mặc định) hay `markdown` (D-252).
    pub format: Format,
    /// Tên của khách (người dùng đăng nhập lấy tên ở hồ sơ).
    pub author_name: Option<String>,
    pub email: Option<String>,
    /// Muốn nhận mail khi có trả lời (cần `email`, xác minh qua mail).
    pub notify_replies: bool,
    pub locale: Option<String>,
    pub form_token: Option<String>,
    pub trap: Option<String>,
    pub challenge: Option<String>,
}

/// Câu trả lời cho người viết.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Posted {
    pub comment: Comment,
    /// `Published` hoặc `Pending` (mọi trạng thái không hiện).
    pub shown: Status,
    /// Token sửa của khách (chỉ có ở câu trả lời này).
    pub edit_token: Option<String>,
}

pub(crate) fn disabled() -> Error {
    Error::forbidden("comments.disabled", "Comments are turned off on this server.")
}

fn guests_not_allowed() -> Error {
    Error::forbidden(
        "comments.guests_not_allowed",
        "Sign in to comment here: comments from guests are not taken on this target.",
    )
}

pub(crate) fn verification_required() -> Error {
    Error::forbidden(
        "comments.verification_required",
        "Only accounts with a verified phone number or identity may write here. Sign in and \
         verify your account first.",
    )
}

fn parent_not_found() -> Error {
    Error::not_found(
        "comments.parent_not_found",
        "The comment to reply to does not exist, is not shown, or belongs to another target.",
    )
}

pub(crate) fn banned() -> Error {
    Error::forbidden("comments.banned", "You are not allowed to comment.")
}

pub(crate) fn banned_word() -> Error {
    Error::new(
        ErrorKind::Validation,
        "comments.banned_word",
        "The comment contains a word that is not allowed here. Change it and send again.",
    )
}

pub(crate) fn profile_required() -> Error {
    Error::conflict("comments.profile_required", "Choose a display name for your comments first.")
}

impl CommentsService {
    /// Tạo bình luận. `principal`: người dùng đăng nhập (cần `comments.post`); `None`: khách.
    pub(crate) async fn post(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        client: &ClientInfo,
        input: NewComment,
    ) -> Result<Posted, Error> {
        if !self.config.enabled {
            return Err(disabled());
        }
        let kind = input.target_type.trim().to_owned();
        let target_id = input.target_id.trim().to_owned();
        self.check_target(&kind, &target_id)?;
        let guest = principal.is_none();
        match principal {
            Some(principal) => principal.require(&COMMENTS_POST)?,
            None if !self.config.allow_guests || self.config.signed_in_only.contains(&kind) => {
                return Err(guests_not_allowed());
            }
            None => {}
        }
        // Loại đích chỉ cho tài khoản đã xác thực (NĐ 147/2024, D-202): khách bị từ chối, người
        // dùng hỏi qua điểm nối của kernel (module giữ trạng thái xác thực, bước 37).
        // Khách cũng có thể đòi xác thực ở mọi loại đích (chính sách của module giữ trạng thái xác
        // thực, vd `posting.comments` của `compliance`).
        let verified_only = self.config.verified_only.contains(&kind)
            || self.verification.required_everywhere(tenant).await?;
        if verified_only {
            let verified = match principal {
                Some(principal) => {
                    self.verification.is_verified(tenant, principal.user_id()).await?
                }
                None => false,
            };
            if !verified {
                return Err(verification_required());
            }
        }
        let moderator = principal.is_some_and(|principal| principal.has(&COMMENTS_MODERATE));

        // Độ dài và lọc nội dung; mọi lỗi của đầu vào báo một lần (D-083).
        let mut problems = Vec::new();
        let body = text::clean_body(&input.body, self.config.max_length, &mut problems);
        let format = self.check_format(input.format, &mut problems);
        let locale = locale(input.locale.as_deref(), &mut problems);
        let name = if guest {
            match input.author_name.as_deref() {
                None => {
                    problems.push(FieldError::new("author_name", "required", "is required"));
                    None
                }
                Some(raw) => {
                    let name = text::clean_name(raw, "author_name", &mut problems);
                    if text::contains_name(&name, &self.config.reserved_names) {
                        problems.push(FieldError::new(
                            "author_name",
                            "reserved",
                            "is reserved; choose another name",
                        ));
                    }
                    Some(name)
                }
            }
        } else {
            if input.author_name.is_some() {
                problems.push(FieldError::new(
                    "author_name",
                    "invalid",
                    "is only for guests; signed-in users show the name of their profile",
                ));
            }
            None
        };
        let email = match input.email.as_deref().map(str::trim).filter(|raw| !raw.is_empty()) {
            Some(raw) => autoxide_kernel::email::checked(raw, "email", &mut problems),
            None => {
                if input.notify_replies {
                    problems.push(FieldError::new(
                        "email",
                        "required",
                        "is required to get mails about replies",
                    ));
                }
                None
            }
        };
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        // Người dùng đăng nhập hiện tên ở hồ sơ của mình (phải có trước).
        let shown_name = match principal {
            Some(principal) => Some(
                profile::Entity::find_by_id((tenant.id().as_uuid(), principal.user_id()))
                    .scoped(tenant)
                    .one(tenant.database().read())
                    .await?
                    .ok_or_else(profile_required)?
                    .display_name,
            ),
            None => name.clone(),
        };

        // Đích có thật, công khai (sổ đối tượng — có thể gọi module khác), đang mở.
        let resource = self.resolve_target(tenant, &kind, &target_id).await?;
        // Nhãn của tác giả trên đích (vd `seller`, `verified_buyer`), tính một lần lúc ghi qua
        // sổ đối tượng — có thể gọi module khác nên ở NGOÀI transaction (D-202).
        let badges = match principal {
            Some(principal) => {
                self.resources.badges(tenant, &kind, &target_id, principal).await?.join(",")
            }
            None => String::new(),
        };
        let now = now();
        if !self.target(tenant, &kind, &target_id).await?.is_open(now) {
            return Err(target_closed());
        }
        let (root_id, parent_id, reply_to_id, depth) = match input.parent_id {
            None => (None, None, None, 0),
            Some(id) => {
                let parent = comment::Entity::find_by_id(id)
                    .scoped(tenant)
                    .one(tenant.database().read())
                    .await?
                    .filter(|parent| {
                        parent.target_type == kind
                            && parent.target_id == target_id
                            && Status::of(parent) == Status::Published
                    })
                    .ok_or_else(parent_not_found)?;
                // Hai cấp: trả lời một trả lời thì gắn vào bình luận gốc của luồng.
                (Some(parent.root_id), Some(parent.root_id), Some(parent.id), 1)
            }
        };

        // Trùng: cùng chữ trên cùng đích gần đây.
        let content_hash = antispam::content_hash(&body);
        let duplicate = comment::Entity::find()
            .scoped(tenant)
            .filter(comment::Column::TargetType.eq(kind.as_str()))
            .filter(comment::Column::TargetId.eq(target_id.as_str()))
            .filter(comment::Column::ContentHash.eq(content_hash.as_str()))
            .filter(
                comment::Column::CreatedAt.gt(now - SignedDuration::from_hours(DUPLICATE_HOURS)),
            )
            .count(tenant.database().read())
            .await?
            > 0;

        // Bị cấm (theo người dùng, email, dấu vết IP).
        let ip_fingerprint = match client.ip {
            Some(ip) => {
                Some(autoxide_kernel::ip_hash::fingerprint(&self.secret, MODULE, ip).await?)
            }
            None => None,
        };
        let user_id = principal.map(Principal::user_id);
        if self.is_banned(tenant, user_id, email.as_deref(), ip_fingerprint.as_deref()).await? {
            return Err(banned());
        }

        // Từ cấm: từ chối. Từ nghi ngờ, quá nhiều link: giữ lại. Luật của khách, thiếu thì của
        // máy chủ.
        let rules = self.rules(tenant).await?;
        if text::contains_word(&body, &rules.banned_words)
            || name.as_deref().is_some_and(|name| text::contains_word(name, &rules.banned_words))
        {
            return Err(banned_word());
        }
        let mut holds = Vec::new();
        if text::contains_word(&body, &rules.suspect_words) {
            holds.push("suspect_word");
        }
        if text::links(&body, format) > rules.max_links {
            holds.push("too_many_links");
        }

        // Chống spam chung (pha một).
        let scope = format!("comments:{kind}");
        let limits = self.limits()?;
        let checked = self
            .guard
            .check(
                tenant,
                &Check::new(&scope, client)
                    .principal(principal)
                    .honeypot(input.trap.as_deref())
                    .form_token(input.form_token.as_deref())
                    .challenge(input.challenge.as_deref())
                    .email(if guest { email.as_deref() } else { None })
                    .content(Some(&body))
                    .duplicate(duplicate),
                &limits,
            )
            .await?;
        let author_kind = if guest { "guest" } else { "user" };
        let (status, hold_reason) = match &checked.verdict {
            Verdict::Discard { reason } => {
                tracing::info!(target_type = %kind, reason, "public comment dropped");
                let row = comment::Model {
                    id: Uuid::now_v7(),
                    tenant_id: tenant.id().as_uuid(),
                    target_type: kind,
                    target_id,
                    root_id: Uuid::nil(),
                    parent_id,
                    reply_to_id,
                    depth,
                    author_kind: author_kind.to_owned(),
                    user_id,
                    author_name: name.clone(),
                    author_email: None,
                    email_verified: false,
                    body,
                    format: format.as_str().to_owned(),
                    status: Status::Pending.as_str().to_owned(),
                    hold_reason: None,
                    edit_token_hash: None,
                    edited_at: None,
                    published_at: None,
                    ip_fingerprint: None,
                    locale: locale.map(str::to_owned),
                    content_hash,
                    search_key: String::new(),
                    match_key: String::new(),
                    badges,
                    reply_count: 0,
                    report_count: 0,
                    reports_held_at: None,
                    restricted_status: None,
                    created_at: now,
                    updated_at: now,
                };
                let row = comment::Model { root_id: root_id.unwrap_or(row.id), ..row };
                let edit_token = if guest { Some(random_token(32)?) } else { None };
                return Ok(Posted {
                    comment: Comment { row, name: shown_name },
                    shown: Status::Pending,
                    edit_token,
                });
            }
            Verdict::Quarantine { reasons } => {
                let mut all: Vec<&str> = reasons.clone();
                all.extend(holds.iter().copied());
                (Status::Spam, Some(all.join(",")))
            }
            Verdict::Accept if !holds.is_empty() => (Status::Pending, Some(holds.join(","))),
            Verdict::Accept => match self.mode_hold(tenant, &kind, principal, moderator).await? {
                None => (Status::Published, None),
                Some(reason) => (Status::Pending, Some(reason.to_owned())),
            },
        };

        // Mail xác minh (khi xin mail báo trả lời): đếm số mail tới địa chỉ này TRƯỚC transaction
        // (bộ đếm ghi bằng kết nối riêng).
        let verify_to = match email.as_deref() {
            Some(email)
                if input.notify_replies
                    && status != Status::Spam
                    && self.config.reply_mails()
                    && Throttle::new(MODULE, self.config.verify_mails_per_day, DAY)?
                        .hit(tenant, &format!("verify:{email}"))
                        .await?
                        .is_allowed() =>
            {
                Some(email.to_owned())
            }
            _ => None,
        };

        let edit_token = if guest { Some(random_token(32)?) } else { None };
        let id = Uuid::now_v7();
        let row = comment::Model {
            id,
            tenant_id: tenant.id().as_uuid(),
            target_type: kind.clone(),
            target_id: target_id.clone(),
            root_id: root_id.unwrap_or(id),
            parent_id,
            reply_to_id,
            depth,
            author_kind: author_kind.to_owned(),
            user_id,
            author_name: name.clone(),
            author_email: email,
            email_verified: false,
            search_key: autoxide_kernel::search::fold(&format!(
                "{body} {}",
                name.as_deref().unwrap_or_default()
            )),
            match_key: autoxide_kernel::search::exact_key(&format!(
                "{body} {}",
                name.as_deref().unwrap_or_default()
            )),
            badges,
            body,
            format: format.as_str().to_owned(),
            status: status.as_str().to_owned(),
            hold_reason,
            edit_token_hash: edit_token.as_deref().map(sha256),
            edited_at: None,
            published_at: (status == Status::Published).then_some(now),
            ip_fingerprint,
            locale: locale.map(str::to_owned),
            content_hash,
            reply_count: 0,
            report_count: 0,
            reports_held_at: None,
            restricted_status: None,
            created_at: now,
            updated_at: now,
        };

        // Pha hai: một transaction ngắn.
        let transaction = tenant.database().begin_write().await?;
        ensure_target(&transaction, tenant, &self.config, &kind, &target_id, now).await?;
        let row = row.into_active_model().insert(&transaction).await?;
        if row.depth == 0 {
            shift_counts(&transaction, tenant, &row, None, Some(status)).await?;
        } else {
            // Bình luận gốc được kiểm ở trên, NGOÀI transaction: nó có thể vừa bị xoá hẳn (cả
            // luồng) hay bị giữ lại. Kiểm lại bằng chính câu tăng `reply_count`, có điều kiện
            // trạng thái: câu này khoá dòng gốc (Postgres) nên xoá hẳn đang chạy phải chờ (và
            // sau đó thấy trả lời này), hay đã xong thì không khớp dòng nào → rollback.
            let touched = comment::Entity::update_many()
                .col_expr(
                    comment::Column::ReplyCount,
                    Expr::col(comment::Column::ReplyCount).add(delta(
                        None,
                        Some(status),
                        Status::Published,
                    )),
                )
                .filter(comment::Column::Id.eq(row.root_id))
                .filter(
                    comment::Column::Status
                        .is_in([Status::Published.as_str(), Status::Deleted.as_str()]),
                )
                .scoped(tenant)
                .exec(&transaction)
                .await?
                .rows_affected;
            if touched == 0 {
                transaction.rollback().await?;
                return Err(parent_not_found());
            }
            shift_target_counts(&transaction, tenant, &row, None, Some(status)).await?;
        }
        let actor = principal.map_or_else(Actor::public, Actor::from);
        let subject = events::comment_subject(row.id);
        let created = Change { before: None, after: Some(summary(&row)) };
        self.outbox
            .record(&transaction, tenant, actor, subject.clone(), &events::CommentCreated(created))
            .await?;
        if status == Status::Published {
            let published = Change { before: None, after: Some(summary(&row)) };
            self.outbox
                .record(&transaction, tenant, actor, subject, &events::CommentPublished(published))
                .await?;
        }
        let verification = match verify_to.as_deref() {
            Some(email) => Some(
                self.tokens
                    .issue(
                        &transaction,
                        tenant,
                        VERIFY,
                        &row.id.to_string(),
                        Some(email),
                        SignedDuration::from_hours(VERIFY_HOURS),
                    )
                    .await?,
            ),
            None => None,
        };
        match checked.claim(&transaction, tenant, MODULE, Some(&row.id.to_string())).await? {
            Claim::New => {
                transaction.commit().await?;
                tracing::info!(comment_id = %row.id, target_type = %row.target_type, status = status.as_str(), "comment stored");
                if let (Some(issued), Some(email)) = (verification, verify_to.as_deref()) {
                    // Mail vào hàng đợi của module `mail`; lỗi ở đây không làm hỏng bình luận.
                    if let Err(error) = self
                        .send_verification(tenant, &row, email, &issued, resource.as_ref())
                        .await
                    {
                        tracing::warn!(comment_id = %row.id, error = %error, "verification mail not queued");
                    }
                }
                let shown =
                    if status == Status::Published { Status::Published } else { Status::Pending };
                Ok(Posted { comment: Comment { row, name: shown_name }, shown, edit_token })
            }
            Claim::Replay { result } => {
                transaction.rollback().await?;
                self.replay(tenant, result.as_deref(), guest).await
            }
        }
    }

    /// Gửi lại cùng token form: bình luận lần đầu; khách nhận token sửa mới.
    async fn replay(
        &self,
        tenant: &TenantContext,
        result: Option<&str>,
        guest: bool,
    ) -> Result<Posted, Error> {
        let id: Uuid =
            result.and_then(|id| id.parse().ok()).ok_or_else(|| comment_not_found(Uuid::nil()))?;
        let found = self.find_named(tenant, id).await?;
        let edit_token = if guest {
            let token = random_token(32)?;
            let mut row = found.row.clone().into_active_model();
            row.edit_token_hash = Set(Some(sha256(&token)));
            comment::Entity::update(row)
                .validate()?
                .scoped(tenant)
                .exec(tenant.database().write())
                .await?;
            Some(token)
        } else {
            None
        };
        let shown =
            if found.status() == Status::Published { Status::Published } else { Status::Pending };
        Ok(Posted { comment: found, shown, edit_token })
    }

    /// Chế độ duyệt cho bình luận mới (và cho bản sửa của bình luận đang hiện): `None` = hiện
    /// ngay, không thì lý do giữ lại.
    pub(crate) async fn mode_hold(
        &self,
        tenant: &TenantContext,
        kind: &str,
        principal: Option<&Principal>,
        moderator: bool,
    ) -> Result<Option<&'static str>, Error> {
        if moderator {
            return Ok(None);
        }
        let mode = self.rules(tenant).await?.mode(&self.config, kind, principal.is_none());
        Ok(match (mode, principal) {
            (Mode::Post, _) => None,
            (Mode::Pre, _) => Some("moderation"),
            // Khách không có tài khoản để nhớ: email tự khai không bao giờ được tính.
            (Mode::FirstTime, None) => Some("first_comment"),
            (Mode::FirstTime, Some(principal)) => {
                // Chỉ bình luận ĐANG hiện: bình luận từng hiện rồi bị đánh spam / từ chối / giữ
                // lại không còn là bằng chứng đáng tin.
                let approved = comment::Entity::find()
                    .scoped(tenant)
                    .filter(comment::Column::UserId.eq(principal.user_id()))
                    .filter(comment::Column::Status.eq(Status::Published.as_str()))
                    .count(tenant.database().read())
                    .await?;
                (approved == 0).then_some("first_comment")
            }
        })
    }

    /// Có lệnh cấm nào còn hiệu lực cho người dùng, email hay dấu vết IP này không.
    pub(crate) async fn is_banned(
        &self,
        tenant: &TenantContext,
        user_id: Option<Uuid>,
        email: Option<&str>,
        ip_fingerprint: Option<&str>,
    ) -> Result<bool, Error> {
        let mut subjects = Vec::new();
        if let Some(user) = user_id {
            subjects.push(("user", user.to_string()));
        }
        if let Some(email) = email {
            subjects.push(("email", email.to_owned()));
        }
        if let Some(fingerprint) = ip_fingerprint {
            subjects.push(("ip", fingerprint.to_owned()));
        }
        let now = now();
        for (kind, value) in subjects {
            let active = ban::Entity::find()
                .scoped(tenant)
                .filter(ban::Column::Kind.eq(kind))
                .filter(ban::Column::Value.eq(value))
                .filter(
                    sea_orm::Condition::any()
                        .add(ban::Column::Until.is_null())
                        .add(ban::Column::Until.gt(now)),
                )
                .count(tenant.database().read())
                .await?;
            if active > 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Giới hạn số lần (theo IP băm, email, người dùng), trên bảng `comments_throttle`.
    fn limits(&self) -> Result<Limits, Error> {
        let throttle = |limit: u32| Throttle::new(MODULE, limit, HOUR);
        Ok(Limits::default()
            .ip(Some(throttle(self.config.per_ip_per_hour)?))
            .email(Some(throttle(self.config.per_email_per_hour)?))
            .user(Some(throttle(self.config.per_user_per_hour)?)))
    }
}

/// `en` / `vi` tuỳ chọn; sai thì thêm lỗi.
impl CommentsService {
    /// Định dạng của bài: Markdown chỉ khi máy chủ bật `AUTOXIDE_COMMENTS_MARKDOWN`; tắt mà
    /// gửi `markdown` thì lỗi ở trường `format` (gom cùng các lỗi khác).
    pub(crate) fn check_format(&self, format: Format, problems: &mut Vec<FieldError>) -> Format {
        if format == Format::Markdown && !self.config.markdown {
            problems.push(FieldError::new(
                "format",
                "disabled",
                "Markdown is turned off on this server (`AUTOXIDE_COMMENTS_MARKDOWN`); send `text`",
            ));
        }
        format
    }
}

pub(crate) fn locale(raw: Option<&str>, problems: &mut Vec<FieldError>) -> Option<&'static str> {
    match raw.map(str::trim).filter(|raw| !raw.is_empty())?.to_ascii_lowercase().as_str() {
        "en" => Some("en"),
        "vi" => Some("vi"),
        _ => {
            problems.push(FieldError::new("locale", "invalid", "must be `en` or `vi`"));
            None
        }
    }
}
