//! Hình dạng trả về của module, dùng chung cho REST và tool MCP. Kiểu công khai (`PublicComment`…)
//! KHÔNG BAO GIỜ có email, dấu vết IP, lý do giữ lại hay trạng thái thật của bình luận chưa hiện.
//! Tên schema mang tiền tố `Comment…` để không trùng module khác (D-140).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{ban, report};
use crate::posting::Posted;
use crate::service::{Comment, Status, badges_of};
use crate::targets::Target;
use crate::text;
use crate::threads::ThreadRoot;

/// Who wrote a comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = CommentAuthorKind)]
pub(crate) enum AuthorKind {
    /// A signed-in user; the name is the display name they chose.
    User,
    /// Someone without an account; the name is what they typed (show it as a guest).
    Guest,
}

/// The author of a comment, as shown to everyone.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CommentAuthor)]
pub(crate) struct Author {
    pub kind: AuthorKind,
    /// `null` when the comment or the account was deleted.
    pub name: Option<String>,
}

/// A comment, as shown to everyone.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicComment {
    pub id: Uuid,
    pub target_type: String,
    pub target_id: String,
    /// The comment that starts the thread (itself for a top-level comment).
    pub root_id: Uuid,
    /// The top-level comment, for a reply; `null` for a top-level comment.
    pub parent_id: Option<Uuid>,
    /// The comment this one answers (the top-level comment or another reply in the thread).
    pub reply_to_id: Option<Uuid>,
    pub author: Author,
    /// Stable codes of what the author was on the target when writing, e.g. `seller` (manages
    /// the product) or `verified_buyer` (received it); empty for most comments. Show your own
    /// label for each code.
    #[schema(example = json!(["verified_buyer"]))]
    pub badges: Vec<String>,
    /// The text as written (plain text, line breaks kept). Never insert it as HTML.
    pub body: String,
    /// The text as safe HTML: everything escaped, line breaks as `<br>`, `http(s)` addresses as
    /// links with `rel="nofollow ugc noopener"`.
    pub body_html: String,
    /// Deleted by its author: no text or name, kept so its replies stay in place.
    pub deleted: bool,
    /// When the author last changed it.
    pub edited_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// Shown replies (top-level comments only).
    pub reply_count: i32,
}

impl From<Comment> for PublicComment {
    fn from(found: Comment) -> Self {
        let deleted = found.status() == Status::Deleted;
        let row = found.row;
        let badges = badges_of(&row);
        PublicComment {
            id: row.id,
            body_html: text::to_html(&row.body),
            body: row.body,
            target_type: row.target_type,
            target_id: row.target_id,
            root_id: row.root_id,
            parent_id: row.parent_id,
            reply_to_id: row.reply_to_id,
            author: Author {
                kind: if row.author_kind == "user" { AuthorKind::User } else { AuthorKind::Guest },
                name: found.name,
            },
            badges: if deleted { Vec::new() } else { badges },
            deleted,
            edited_at: row.edited_at,
            created_at: row.created_at,
            reply_count: if row.depth == 0 { row.reply_count } else { 0 },
        }
    }
}

/// A top-level comment with its first replies.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct ThreadComment {
    #[serde(flatten)]
    pub comment: PublicComment,
    /// The first shown replies, oldest first (at most `replies_limit`); get the rest from
    /// `GET /api/v1/comments/public/comments/{id}/replies`.
    pub replies: Vec<PublicComment>,
}

impl From<ThreadRoot> for ThreadComment {
    fn from(found: ThreadRoot) -> Self {
        ThreadComment {
            comment: found.root.into(),
            replies: found.replies.into_iter().map(PublicComment::from).collect(),
        }
    }
}

/// Whether a new comment is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = CommentVisibility)]
pub(crate) enum Visibility {
    /// Shown now.
    Published,
    /// Waiting for a moderator (show "your comment awaits approval").
    Pending,
}

/// A comment just posted, for its author.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PostedComment {
    #[serde(flatten)]
    pub comment: PublicComment,
    pub status: Visibility,
    /// Guests only: keep it (e.g. in the browser) to change or delete the comment; it is shown
    /// only here. Posting again with the same `form_token` returns a new one.
    pub edit_token: Option<String>,
    /// Until when the author may change it (`null`: never).
    pub editable_until: Option<DateTime<Utc>>,
}

impl PostedComment {
    pub(crate) fn new(posted: Posted, edit_minutes: u32) -> Self {
        let editable_until = (edit_minutes > 0).then(|| {
            posted.comment.row.created_at + chrono::Duration::minutes(i64::from(edit_minutes))
        });
        PostedComment {
            status: if posted.shown == Status::Published {
                Visibility::Published
            } else {
                Visibility::Pending
            },
            comment: posted.comment.into(),
            edit_token: posted.edit_token,
            editable_until,
        }
    }
}

/// A comment just changed, for its author.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct EditedComment {
    #[serde(flatten)]
    pub comment: PublicComment,
    /// `pending` when the change waits for approval (show "your change awaits approval").
    pub status: Visibility,
}

impl From<Comment> for EditedComment {
    fn from(found: Comment) -> Self {
        EditedComment {
            status: if found.status() == Status::Published {
                Visibility::Published
            } else {
                Visibility::Pending
            },
            comment: found.into(),
        }
    }
}

/// A comment with everything moderators see.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct CommentDetail {
    pub id: Uuid,
    pub target_type: String,
    pub target_id: String,
    pub root_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub reply_to_id: Option<Uuid>,
    pub status: Status,
    /// Why it is held: `moderation`, `first_comment`, `reported`, `suspect_word`,
    /// `too_many_links`, or the anti-spam reasons (`too_fast`, `duplicate`, `classifier`…),
    /// joined by commas.
    pub hold_reason: Option<String>,
    pub author_kind: AuthorKind,
    /// The account, for a signed-in author.
    pub user_id: Option<Uuid>,
    /// Guest name or the user's display name.
    pub author_name: Option<String>,
    /// Codes of what the author was on the target when writing (`seller`, `verified_buyer`…).
    pub badges: Vec<String>,
    /// Address given for reply mails (lower case).
    pub author_email: Option<String>,
    pub email_verified: bool,
    pub body: String,
    pub body_html: String,
    /// Keyed hash of the author's network block (never the address itself); use it to ban;
    /// cleared after `AUTOXIDE_COMMENTS_IP_RETENTION_DAYS`.
    pub ip_fingerprint: Option<String>,
    /// `en` or `vi`.
    pub locale: Option<String>,
    pub reply_count: i32,
    pub report_count: i32,
    pub edited_at: Option<DateTime<Utc>>,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Comment> for CommentDetail {
    fn from(found: Comment) -> Self {
        let status = found.status();
        let row = found.row;
        let badges = badges_of(&row);
        CommentDetail {
            id: row.id,
            target_type: row.target_type,
            target_id: row.target_id,
            root_id: row.root_id,
            parent_id: row.parent_id,
            reply_to_id: row.reply_to_id,
            status,
            hold_reason: row.hold_reason,
            author_kind: if row.author_kind == "user" {
                AuthorKind::User
            } else {
                AuthorKind::Guest
            },
            user_id: row.user_id,
            author_name: found.name,
            badges,
            author_email: row.author_email,
            email_verified: row.email_verified,
            body_html: text::to_html(&row.body),
            body: row.body,
            ip_fingerprint: row.ip_fingerprint,
            locale: row.locale,
            reply_count: row.reply_count,
            report_count: row.report_count,
            edited_at: row.edited_at,
            published_at: row.published_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Whether a target takes comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = CommentTargetState)]
pub(crate) enum TargetState {
    /// Takes new comments.
    Open,
    /// Takes no new comments (closed by hand, past `closes_at`, or hidden).
    Closed,
}

/// A target's comment state and counts.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CommentTarget)]
pub(crate) struct TargetResponse {
    pub target_type: String,
    pub target_id: String,
    /// What applies now.
    pub state: TargetState,
    /// The comments are hidden from everyone (e.g. the target was deleted).
    pub hidden: bool,
    /// When it closes by itself, if set.
    pub closes_at: Option<DateTime<Utc>>,
    /// Shown comments (0 when hidden).
    pub comment_count: i32,
    /// Comments waiting for approval (`null` in the public answer).
    pub pending_count: Option<i32>,
    /// Shown top-level comments, e.g. questions on a product (0 when hidden).
    pub thread_count: i32,
    /// Top-level comments waiting for approval (`null` in the public answer).
    pub pending_thread_count: Option<i32>,
}

impl TargetResponse {
    pub(crate) fn new(target: Target, admin: bool) -> Self {
        TargetResponse {
            state: if target.effective_state() == "open" {
                TargetState::Open
            } else {
                TargetState::Closed
            },
            comment_count: if target.hidden && !admin { 0 } else { target.published_count },
            pending_count: admin.then_some(target.pending_count),
            thread_count: if target.hidden && !admin { 0 } else { target.published_threads },
            pending_thread_count: admin.then_some(target.pending_threads),
            hidden: target.hidden,
            closes_at: target.closes_at,
            target_type: target.kind,
            target_id: target.id,
        }
    }
}

/// A ban.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CommentBan)]
pub(crate) struct BanResponse {
    pub id: Uuid,
    /// `user`, `email` or `ip`.
    pub kind: String,
    /// User id, email address or `ip_fingerprint`.
    pub value: String,
    pub reason: String,
    /// When it ends; `null`: never.
    pub until: Option<DateTime<Utc>>,
    /// Still in force.
    pub active: bool,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

impl From<ban::Model> for BanResponse {
    fn from(row: ban::Model) -> Self {
        BanResponse {
            active: row.until.is_none_or(|until| until > Utc::now()),
            id: row.id,
            kind: row.kind,
            value: row.value,
            reason: row.reason,
            until: row.until,
            created_by: row.created_by,
            created_at: row.created_at,
        }
    }
}

/// A report of a comment.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CommentReport)]
pub(crate) struct ReportResponse {
    pub id: Uuid,
    pub comment_id: Uuid,
    /// `spam`, `abuse`, `illegal` or `other`.
    pub reason: String,
    pub note: Option<String>,
    /// The reporting account, when signed in.
    pub reporter_user_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

impl From<report::Model> for ReportResponse {
    fn from(row: report::Model) -> Self {
        ReportResponse {
            id: row.id,
            comment_id: row.comment_id,
            reason: row.reason,
            note: row.note,
            reporter_user_id: row.reporter_user_id,
            created_at: row.created_at,
        }
    }
}
