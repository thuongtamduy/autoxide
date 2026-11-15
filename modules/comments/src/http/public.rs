//! Endpoint công khai (không cần đăng nhập, `/public/…`) và tên hiển thị của người dùng
//! (`/profile`). Không bao giờ trả email, dấu vết IP, lý do giữ lại hay bình luận chưa hiện.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::{ClientInfo, ValidJson};
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::version::Tagged;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::page_request;
use super::views::{EditedComment, PostedComment, PublicComment, TargetResponse, ThreadComment};
use crate::moderation::reports::ReportReason;
use crate::posting::NewComment;
use crate::service::CommentsService;
use crate::threads::{REPLIES_MAX, oldest_first};

/// What a website needs to know to show the comment box.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct CommentSettings {
    /// New comments are taken at all.
    pub enabled: bool,
    /// Kinds of targets that take comments.
    pub target_types: Vec<String>,
    /// Guests (no account) may comment.
    pub guests_allowed: bool,
    /// Kinds where only signed-in users may comment.
    pub signed_in_only_types: Vec<String>,
    /// Kinds where only verified accounts (phone number or identity) may comment or answer;
    /// show "verify your account" instead of the comment box to others.
    pub verified_only_types: Vec<String>,
    /// Longest comment, in characters.
    pub max_length: usize,
    /// More links than this hold a comment for approval.
    pub max_links: usize,
    /// Minutes during which authors may change their comment (0: never).
    pub edit_minutes: u32,
    /// The server can mail people about replies (show the "notify me" box).
    pub reply_mails: bool,
    /// Ask `GET /api/v1/antispam/token?scope=comments:<target_type>` for a form token each
    /// time the comment box is shown.
    #[schema(example = "comments:{target_type}")]
    pub token_scope: String,
}

/// Get the comment settings (public).
///
/// No sign-in needed. Limits and switches a website needs to show the comment box.
#[utoipa::path(
    get,
    path = "/public/settings",
    extensions(("x-public" = json!(true))),
    operation_id = "get_comment_settings",
    security((), ("bearer" = [])),
    responses((status = 200, description = "The settings.", body = Data<CommentSettings>))
)]
pub(crate) async fn get_settings(
    State(service): State<Arc<CommentsService>>,
) -> Result<Data<CommentSettings>, Error> {
    let config = &service.config;
    Ok(Data {
        data: CommentSettings {
            enabled: config.enabled,
            target_types: config.target_types.clone(),
            guests_allowed: config.allow_guests,
            signed_in_only_types: config.signed_in_only.clone(),
            verified_only_types: config.verified_only.clone(),
            max_length: config.max_length,
            max_links: config.max_links,
            edit_minutes: config.edit_minutes,
            reply_mails: config.reply_mails(),
            token_scope: "comments:{target_type}".to_owned(),
        },
    })
}

/// Which thread.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ThreadQuery {
    /// Kind of the target, e.g. `content.entry`.
    #[param(example = "content.entry")]
    pub target_type: Option<String>,
    /// Id of the target in its module.
    pub target_id: Option<String>,
    /// Replies to include with each top-level comment, 0 to 10. Defaults to 3.
    #[param(value_type = Option<u32>, minimum = 0, maximum = 10)]
    pub replies_limit: Option<String>,
    /// Only top-level comments (e.g. questions) whose text or author name contains this, 1 to
    /// 100 characters, ignoring case and accents. Matches with the exact accents come first,
    /// then the others; within each group the usual `order` applies.
    #[param(example = "bảo hành")]
    pub q: Option<String>,
}

/// Chữ tìm dài nhất (ký tự).
const SEARCH_MAX: usize = 100;

/// The target of a query; missing parts are added to `problems`.
fn target_of(
    target_type: Option<&str>,
    target_id: Option<&str>,
    problems: &mut Vec<FieldError>,
) -> (String, String) {
    let mut part = |value: Option<&str>, field: &str| {
        let value = value.map(str::trim).unwrap_or_default();
        if value.is_empty() {
            problems.push(FieldError::new(field, "required", "is required"));
        }
        value.to_owned()
    };
    (part(target_type, "target_type"), part(target_id, "target_id"))
}

/// List the comments of a target (public).
///
/// No sign-in needed. Shown top-level comments, newest first (`order=asc`: oldest first), each
/// with its first replies (oldest first) and `reply_count`. A top-level comment deleted by its
/// author stays (with `deleted: true`, no text) while it has replies. With `q`, only matching
/// top-level comments, exact-accent matches first (questions and answers on a product use the
/// same thread: a question is a top-level comment, answers are its replies). Follow
/// `pagination.next_cursor` with the same parameters for more.
#[utoipa::path(
    get,
    path = "/public/threads",
    extensions(("x-public" = json!(true))),
    operation_id = "list_comment_thread",
    security((), ("bearer" = [])),
    params(ThreadQuery, PageQuery),
    responses(
        (status = 200, description = "One page of top-level comments.", body = Page<ThreadComment>),
        (status = 404, description = "This kind of target takes no comments; code `comments.target_not_found`.", body = Problem),
        (status = 422, description = "A parameter is missing or invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_thread(
    State(service): State<Arc<CommentsService>>,
    _principal: Option<Principal>,
    Query(query): Query<ThreadQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<ThreadComment>, Error> {
    let mut problems = Vec::new();
    let (kind, id) =
        target_of(query.target_type.as_deref(), query.target_id.as_deref(), &mut problems);
    let replies = match query.replies_limit.as_deref().map(str::trim).filter(|raw| !raw.is_empty())
    {
        None => 3,
        Some(raw) => match raw.parse::<u32>() {
            Ok(value) if value <= REPLIES_MAX => value,
            _ => {
                problems.push(FieldError::new(
                    "replies_limit",
                    "range",
                    format!("must be a whole number between 0 and {REPLIES_MAX}"),
                ));
                0
            }
        },
    };
    let search = query.q.as_deref().map(str::trim).filter(|text| !text.is_empty());
    if search.is_some_and(|text| text.chars().count() > SEARCH_MAX) {
        problems.push(FieldError::new(
            "q",
            "length",
            format!("must be at most {SEARCH_MAX} characters long"),
        ));
    }
    let page = page_request(paging, problems)?;
    Ok(service.thread(&kind, &id, &page, replies, search).await?.map(ThreadComment::from))
}

/// Get a comment (public).
///
/// No sign-in needed. Only shown comments (and deleted top-level comments that still have
/// replies).
#[utoipa::path(
    get,
    path = "/public/comments/{id}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_comment",
    security((), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the comment.")),
    responses(
        (status = 200, description = "The comment.", body = Data<PublicComment>),
        (status = 404, description = "No shown comment has this id; code `comments.not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_public_comment(
    State(service): State<Arc<CommentsService>>,
    _principal: Option<Principal>,
    Path(id): Path<Uuid>,
) -> Result<Data<PublicComment>, Error> {
    Ok(Data { data: service.public_comment(id).await?.into() })
}

/// List the replies of a comment (public).
///
/// No sign-in needed. Shown replies of the thread the comment starts, oldest first
/// (`order=desc`: newest first). Follow `pagination.next_cursor` for more.
#[utoipa::path(
    get,
    path = "/public/comments/{id}/replies",
    extensions(("x-public" = json!(true))),
    operation_id = "list_comment_replies",
    security((), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the top-level comment."), PageQuery),
    responses(
        (status = 200, description = "One page of replies.", body = Page<PublicComment>),
        (status = 404, description = "No shown comment has this id; code `comments.not_found`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_replies(
    State(service): State<Arc<CommentsService>>,
    _principal: Option<Principal>,
    Path(id): Path<Uuid>,
    Query(mut paging): Query<PageQuery>,
) -> Result<Page<PublicComment>, Error> {
    paging.order = oldest_first(paging.order.as_deref());
    let page = page_request(paging, Vec::new())?;
    Ok(service.replies(id, &page).await?.map(PublicComment::from))
}

/// A new comment.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateCommentRequest {
    /// Kind of the target, e.g. `content.entry`.
    #[schema(example = "content.entry")]
    target_type: String,
    /// Id of the target in its module.
    target_id: String,
    /// Reply to this comment (a top-level comment or a reply in its thread); omit for a new
    /// thread.
    parent_id: Option<Uuid>,
    /// Plain text; line breaks are kept, `http(s)` addresses become links.
    body: String,
    /// Guests: the name to show (required). Signed-in users show the name of their profile.
    author_name: Option<String>,
    /// Address for reply mails (never shown). Required with `notify_replies`.
    email: Option<String>,
    /// Mail me when someone replies (a confirmation mail is sent to `email` first).
    #[serde(default)]
    notify_replies: bool,
    /// Language of the author: `en` or `vi` (for the mails).
    locale: Option<String>,
    /// Token from `GET /api/v1/antispam/token?scope=comments:<target_type>`; needed unless
    /// signed in.
    form_token: Option<String>,
    /// The hidden trap field: people leave it empty.
    trap: Option<String>,
    /// The value of the `altcha` widget, when the server uses a challenge.
    challenge: Option<String>,
}

/// Post a comment (public).
///
/// Guests need no sign-in (unless the server or the target kind asks for it); signed-in users
/// need `comments.post` and a display name (`PUT /api/v1/comments/profile`). Checks run in a
/// fixed order: target open, length, content, duplicate, bans, banned words (refused) and
/// suspect words or too many links (held), anti-spam, approval mode. Every input problem is
/// reported at once. The answer says `published` or `pending` (waiting for approval — held
/// or dropped comments look the same). Guests get an `edit_token`, shown only here. Sending the
/// same `form_token` again returns the same comment (with a new `edit_token`).
#[utoipa::path(
    post,
    path = "/public/comments",
    extensions(("x-public" = json!(true))),
    operation_id = "create_comment",
    security((), ("bearer" = [])),
    request_body = CreateCommentRequest,
    responses(
        (status = 201, description = "Received.", body = Data<PostedComment>),
        (status = 400, description = "The form token or challenge is missing, invalid or expired (codes `antispam.token_missing`, `antispam.token_invalid`, `antispam.token_expired`, `antispam.challenge_required`, `antispam.challenge_failed`): reload the form.", body = Problem),
        (status = 403, description = "Comments are off (`comments.disabled`), guests may not comment here (`comments.guests_not_allowed`), the author is banned (`comments.banned`), or a signed-in caller lacks `comments.post` (`permission_denied`).", body = Problem),
        (status = 404, description = "The target takes no comments (`comments.target_not_found`), or the comment to reply to is not shown (`comments.parent_not_found`).", body = Problem),
        (status = 409, description = "The target is closed (`comments.target_closed`), or a signed-in user has no display name yet (`comments.profile_required`).", body = Problem),
        (status = 413, description = "The body is too large; code `payload_too_large`.", body = Problem),
        (status = 422, description = "Invalid input (see `errors`), or a banned word (`comments.banned_word`).", body = Problem),
        (status = 429, description = "A signed-in user posted too many; code `antispam.rate_limited`.", body = Problem),
        (status = 503, description = "The anti-spam check cannot run; code `antispam.verifier_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn create_comment(
    State(service): State<Arc<CommentsService>>,
    principal: Option<Principal>,
    client: ClientInfo,
    ValidJson(input): ValidJson<CreateCommentRequest>,
) -> Result<Created<PostedComment>, Error> {
    let new = NewComment {
        target_type: input.target_type,
        target_id: input.target_id,
        parent_id: input.parent_id,
        body: input.body,
        author_name: input.author_name,
        email: input.email,
        notify_replies: input.notify_replies,
        locale: input.locale,
        form_token: input.form_token,
        trap: input.trap,
        challenge: input.challenge,
    };
    let posted = service.post(principal.as_ref(), &client, new).await?;
    let id = posted.comment.row.id;
    Ok(Created::new(
        format!("/comments/public/comments/{id}"),
        PostedComment::new(posted, service.config.edit_minutes),
    ))
}

/// A change by the author.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditCommentRequest {
    /// The new text.
    body: String,
    /// Guests: the `edit_token` received when posting. Signed-in authors omit it.
    edit_token: Option<String>,
}

/// Change one's comment (public).
///
/// The author only — signed in as the same user (with `comments.post`), or a guest sending
/// `edit_token` — within `edit_minutes` of posting. The text is checked again. A shown comment
/// waits for approval again (`status: pending`) when it now has suspect words or too many links,
/// or when a new comment of the same author on the same target would wait (approval mode `pre`,
/// or `first_time` without a shown comment).
#[utoipa::path(
    patch,
    path = "/public/comments/{id}",
    extensions(("x-public" = json!(true))),
    operation_id = "edit_comment",
    security((), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the comment.")),
    request_body = EditCommentRequest,
    responses(
        (status = 200, description = "The comment after the change, and whether it is shown.", body = Data<EditedComment>),
        (status = 403, description = "Not the author (`comments.not_author`), or banned (`comments.banned`).", body = Problem),
        (status = 404, description = "No comment has this id; code `comments.not_found`.", body = Problem),
        (status = 409, description = "Too late (`comments.edit_window_passed`), the comment was rejected or deleted (`comments.not_editable`).", body = Problem),
        (status = 412, description = "A moderator changed it at the same time; code `precondition_failed`.", body = Problem),
        (status = 422, description = "Invalid text (see `errors`), or a banned word (`comments.banned_word`).", body = Problem),
    )
)]
pub(crate) async fn edit_comment(
    State(service): State<Arc<CommentsService>>,
    principal: Option<Principal>,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<EditCommentRequest>,
) -> Result<Data<EditedComment>, Error> {
    let found =
        service.edit(principal.as_ref(), id, input.edit_token.as_deref(), &input.body).await?;
    Ok(Data { data: found.into() })
}

/// Proof of authorship.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeleteOwnRequest {
    /// Guests: the `edit_token` received when posting. Signed-in authors send `{}`.
    edit_token: Option<String>,
}

/// Delete one's comment (public).
///
/// The author only (signed in as the same user, or a guest sending `edit_token`), at any time.
/// The text, name and email are removed; the comment keeps its place in the thread so replies
/// stay. A body is used (not `DELETE`) because the token must not travel in the address. An id
/// that does not exist (deleted for good, or a comment dropped by the anti-spam checks) also
/// answers `200`, so the answer tells nothing about what happened to a comment.
#[utoipa::path(
    post,
    path = "/public/comments/{id}/delete",
    extensions(("x-public" = json!(true))),
    operation_id = "delete_own_comment",
    security((), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the comment.")),
    request_body = DeleteOwnRequest,
    responses(
        (status = 200, description = "Deleted (also when it already was, or no comment has this id).", body = Data<DeletedResponse>),
        (status = 403, description = "Not the author; code `comments.not_author`.", body = Problem),
        (status = 412, description = "A moderator changed it at the same time; code `precondition_failed`.", body = Problem),
    )
)]
pub(crate) async fn delete_own_comment(
    State(service): State<Arc<CommentsService>>,
    principal: Option<Principal>,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<DeleteOwnRequest>,
) -> Result<Data<DeletedResponse>, Error> {
    service.delete_own(principal.as_ref(), id, input.edit_token.as_deref()).await?;
    Ok(Data { data: DeletedResponse { deleted: true } })
}

/// The answer to a delete.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CommentDeleted)]
pub(crate) struct DeletedResponse {
    pub deleted: bool,
}

/// A report.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReportRequest {
    reason: ReportReason,
    /// What is wrong, for the moderators (at most 500 characters).
    note: Option<String>,
}

/// The answer to a report.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CommentReported)]
pub(crate) struct ReportedResponse {
    /// Always `true`: reporting twice, or too often, answers the same.
    pub received: bool,
}

/// Report a comment (public).
///
/// No sign-in needed. One report per person per comment (signed-in user, or network block for
/// guests); reporting again answers the same. When `AUTOXIDE_COMMENTS_REPORT_THRESHOLD`
/// reports are reached the comment waits for a moderator again (once).
#[utoipa::path(
    post,
    path = "/public/comments/{id}/reports",
    extensions(("x-public" = json!(true))),
    operation_id = "report_comment",
    security((), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the comment.")),
    request_body = ReportRequest,
    responses(
        (status = 200, description = "Received.", body = Data<ReportedResponse>),
        (status = 404, description = "No shown comment has this id; code `comments.not_found`.", body = Problem),
        (status = 422, description = "Invalid input; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn report_comment(
    State(service): State<Arc<CommentsService>>,
    principal: Option<Principal>,
    client: ClientInfo,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ReportRequest>,
) -> Result<Data<ReportedResponse>, Error> {
    service.report(principal.as_ref(), &client, id, input.reason, input.note.as_deref()).await?;
    Ok(Data { data: ReportedResponse { received: true } })
}

/// Which targets to count.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct CountsQuery {
    /// Kind of the targets, e.g. `content.entry`.
    #[param(example = "content.entry")]
    target_type: Option<String>,
    /// 1 to 100 target ids, separated by commas.
    target_ids: Option<String>,
}

/// The comment count of a target.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct CommentCount {
    pub target_id: String,
    /// Shown comments (top-level and replies).
    pub count: i32,
    /// Shown top-level comments only (e.g. the number of questions on a product tab).
    pub thread_count: i32,
}

/// Count the comments of many targets (public).
///
/// No sign-in needed. Shown comments (all, and top-level only) of up to 100 targets of one kind,
/// in the order asked; unknown and hidden targets count 0.
#[utoipa::path(
    get,
    path = "/public/counts",
    extensions(("x-public" = json!(true))),
    operation_id = "count_comments",
    security((), ("bearer" = [])),
    params(CountsQuery),
    responses(
        (status = 200, description = "The counts.", body = Data<Vec<CommentCount>>),
        (status = 404, description = "This kind of target takes no comments; code `comments.target_not_found`.", body = Problem),
        (status = 422, description = "A parameter is missing or invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn count_comments(
    State(service): State<Arc<CommentsService>>,
    _principal: Option<Principal>,
    Query(query): Query<CountsQuery>,
) -> Result<Data<Vec<CommentCount>>, Error> {
    let mut problems = Vec::new();
    let (kind, ids) =
        target_of(query.target_type.as_deref(), query.target_ids.as_deref(), &mut problems);
    if !problems.is_empty() {
        return Err(Error::validation(
            problems
                .into_iter()
                .map(|problem| {
                    if problem.field == "target_id" {
                        FieldError::new("target_ids", problem.code, problem.message)
                    } else {
                        problem
                    }
                })
                .collect(),
        ));
    }
    let ids: Vec<String> =
        ids.split(',').map(str::trim).filter(|id| !id.is_empty()).map(str::to_owned).collect();
    let counts = service.counts(&kind, &ids).await?;
    Ok(Data {
        data: counts
            .into_iter()
            .map(|(target_id, count, thread_count)| CommentCount { target_id, count, thread_count })
            .collect(),
    })
}

/// Get the comment state of a target (public).
///
/// No sign-in needed. Whether it takes comments now, when it closes, how many are shown.
#[utoipa::path(
    get,
    path = "/public/targets/{target_type}/{target_id}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_comment_target",
    security((), ("bearer" = [])),
    params(
        ("target_type" = String, Path, description = "Kind of the target, e.g. `content.entry`."),
        ("target_id" = String, Path, description = "Id of the target in its module."),
    ),
    responses(
        (status = 200, description = "The state.", body = Data<TargetResponse>),
        (status = 404, description = "This kind of target takes no comments; code `comments.target_not_found`.", body = Problem),
        (status = 422, description = "The id is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn get_public_target(
    State(service): State<Arc<CommentsService>>,
    _principal: Option<Principal>,
    Path((kind, id)): Path<(String, String)>,
) -> Result<Data<TargetResponse>, Error> {
    Ok(Data { data: TargetResponse::new(service.target(&kind, &id).await?, false) })
}

/// A token from a mail link.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CommentMailToken)]
pub(crate) struct TokenRequest {
    /// The `token` of the link.
    token: String,
}

/// The answer to an address confirmation.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct VerifiedResponse {
    pub verified: bool,
}

/// Confirm an address for reply mails (public).
///
/// The page at `AUTOXIDE_COMMENTS_VERIFY_URL` sends the token of the mail link here (a `POST`,
/// so mail scanners opening the link change nothing). From then on, the address gets a mail
/// when that comment receives a shown reply.
#[utoipa::path(
    post,
    path = "/public/verify",
    extensions(("x-public" = json!(true))),
    operation_id = "verify_comment_email",
    security((), ("bearer" = [])),
    request_body = TokenRequest,
    responses(
        (status = 200, description = "Confirmed.", body = Data<VerifiedResponse>),
        (status = 400, description = "The link is invalid, expired or used; code `comments.link_invalid`.", body = Problem),
    )
)]
pub(crate) async fn verify_email(
    State(service): State<Arc<CommentsService>>,
    ValidJson(input): ValidJson<TokenRequest>,
) -> Result<Data<VerifiedResponse>, Error> {
    service.verify(&input.token).await?;
    Ok(Data { data: VerifiedResponse { verified: true } })
}

/// Stop reply mails.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UnsubscribeRequest {
    /// The `token` of the link in a reply mail.
    token: String,
    /// Stop mails about every comment followed with this address, not just this one.
    #[serde(default)]
    all: bool,
}

/// The answer to an unsubscription.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CommentsUnsubscribed)]
pub(crate) struct UnsubscribedResponse {
    /// Always `true`, even for an unknown or used token (nothing is revealed).
    pub unsubscribed: bool,
}

/// Stop reply mails (public).
///
/// The page at `AUTOXIDE_COMMENTS_UNSUBSCRIBE_URL` sends the token of the link in a reply mail
/// here. Takes effect at once.
#[utoipa::path(
    post,
    path = "/public/unsubscribe",
    extensions(("x-public" = json!(true))),
    operation_id = "unsubscribe_comment_replies",
    security((), ("bearer" = [])),
    request_body = UnsubscribeRequest,
    responses((status = 200, description = "Done.", body = Data<UnsubscribedResponse>))
)]
pub(crate) async fn unsubscribe(
    State(service): State<Arc<CommentsService>>,
    ValidJson(input): ValidJson<UnsubscribeRequest>,
) -> Result<Data<UnsubscribedResponse>, Error> {
    service.unsubscribe(&input.token, input.all).await?;
    Ok(Data { data: UnsubscribedResponse { unsubscribed: true } })
}

/// The display name of a signed-in user.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CommentProfile)]
pub(crate) struct ProfileResponse {
    /// `null` until chosen.
    pub display_name: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Get one's display name.
///
/// Requires `comments.post`. The name shown on one's comments; `null` until chosen.
#[utoipa::path(
    get,
    path = "/profile",
    extensions(("x-permissions" = json!(["comments.post"]))),
    operation_id = "get_comment_profile",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The profile.", body = Data<ProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `comments.post`; code `permission_denied`.", body = Problem),
    )
)]
pub(crate) async fn get_profile(
    State(service): State<Arc<CommentsService>>,
    principal: Principal,
) -> Result<Data<ProfileResponse>, Error> {
    let found = service.profile(&principal).await?;
    Ok(Data {
        data: ProfileResponse {
            updated_at: found.as_ref().map(|profile| profile.updated_at),
            display_name: found.map(|profile| profile.display_name),
        },
    })
}

/// A display name.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileRequest {
    /// 1 to 50 characters; reserved names (`AUTOXIDE_COMMENTS_RESERVED_NAMES`) are for
    /// moderators; no two users share a name.
    display_name: String,
}

/// Set one's display name.
///
/// Requires `comments.post`. Needed before commenting while signed in; shown on all one's
/// comments, also the earlier ones.
#[utoipa::path(
    put,
    path = "/profile",
    extensions(("x-permissions" = json!(["comments.post"]))),
    operation_id = "set_comment_profile",
    security(("bearer" = [])),
    request_body = ProfileRequest,
    responses(
        (status = 200, description = "The profile.", body = Data<ProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `comments.post`; code `permission_denied`.", body = Problem),
        (status = 409, description = "Another user shows this name; code `comments.name_taken`.", body = Problem),
        (status = 422, description = "Invalid or reserved name; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn set_profile(
    State(service): State<Arc<CommentsService>>,
    principal: Principal,
    ValidJson(input): ValidJson<ProfileRequest>,
) -> Result<Tagged<ProfileResponse>, Error> {
    let saved = service.set_profile(&principal, &input.display_name).await?;
    Ok(Tagged {
        version: saved.updated_at,
        data: ProfileResponse {
            display_name: Some(saved.display_name),
            updated_at: Some(saved.updated_at),
        },
    })
}
