//! Endpoint quản trị: hàng chờ, một bình luận, đổi trạng thái (`If-Match` bắt buộc), xoá hẳn,
//! báo cáo, cấm, trạng thái đích, xuất / xoá theo email.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::views::{BanResponse, CommentDetail, ReportResponse, TargetResponse};
use super::{choice, page_request};
use crate::moderation::{BAN_KINDS, BanFilter, NewBan, QueueFilter};
use crate::posting::targets::{TargetChange, TargetFilter};
use crate::service::{CommentsService, STATUSES, Status};

/// Which comments to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct QueueQuery {
    /// `pending` (the default: the moderation queue), `published`, `rejected`, `spam` or
    /// `deleted`.
    #[param(value_type = Option<Status>, inline)]
    pub status: Option<String>,
    /// Only comments on targets of this kind.
    pub target_type: Option<String>,
    /// Only comments on this target (with `target_type`).
    pub target_id: Option<String>,
    /// Only comments whose text or author name contains this, ignoring case and accents.
    pub q: Option<String>,
    /// `true`: only top-level comments without a shown reply yet, e.g. unanswered questions
    /// (use with `status=published`). Defaults to `false` (no such filter).
    #[param(value_type = Option<bool>)]
    pub unanswered: Option<String>,
}

impl QueueQuery {
    pub(crate) fn filter(&self, problems: &mut Vec<FieldError>) -> QueueFilter {
        let unanswered = match self.unanswered.as_deref().map(str::trim) {
            None | Some("" | "false") => false,
            Some("true") => true,
            Some(_) => {
                problems.push(FieldError::new(
                    "unanswered",
                    "invalid",
                    "must be `true` or `false`",
                ));
                false
            }
        };
        QueueFilter {
            status: choice(self.status.as_deref(), "status", STATUSES, problems),
            target_type: self.target_type.clone(),
            target_id: self.target_id.clone(),
            search: self.q.clone(),
            unanswered,
        }
    }
}

/// List comments for moderation.
///
/// Requires `comments.read`. By default the comments waiting for approval, newest first; filter
/// by status, target and text, or list unanswered questions (`status=published&unanswered=true`). Follow `pagination.next_cursor` with the same filters for more.
#[utoipa::path(
    get,
    path = "/",
    extensions(("x-permissions" = json!(["comments.read"]))),
    operation_id = "list_comments",
    security(("bearer" = [])),
    params(QueueQuery, PageQuery),
    responses(
        (status = 200, description = "One page of comments.", body = Page<CommentDetail>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `comments.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_comments(
    State(service): State<Arc<CommentsService>>,
    principal: Principal,
    Query(query): Query<QueueQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<CommentDetail>, Error> {
    let mut problems = Vec::new();
    let filter = query.filter(&mut problems);
    let page = page_request(paging, problems)?;
    let page = service.queue(&principal, &filter, &page).await?.map(CommentDetail::from);
    let autoxide_kernel::pagination::Page { data, pagination } = page;
    Ok(autoxide_kernel::pagination::Page { data: with_targets(&service, data).await?, pagination })
}

/// Bình luận kèm tiêu đề và đường dẫn của đích (một lần tra cho mỗi loại đích của trang).
async fn with_targets(
    service: &CommentsService,
    comments: Vec<CommentDetail>,
) -> Result<Vec<CommentDetail>, Error> {
    let targets: Vec<(String, String)> = comments
        .iter()
        .map(|comment| (comment.target_type.clone(), comment.target_id.clone()))
        .collect();
    let found = service.target_titles(&targets).await?;
    Ok(comments
        .into_iter()
        .map(|comment| {
            let key = (comment.target_type.clone(), comment.target_id.clone());
            let (target_title, target_url) = super::views::described(found.get(&key));
            CommentDetail { target_title, target_url, ..comment }
        })
        .collect())
}

/// Get a comment with its moderation details.
///
/// Requires `comments.read`. Any status; includes the author's email and network fingerprint.
#[utoipa::path(
    get,
    path = "/{id}",
    extensions(("x-permissions" = json!(["comments.read"]))),
    operation_id = "get_comment",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the comment.")),
    responses(
        (status = 200, description = "The comment.", body = Data<CommentDetail>, headers(("ETag" = String, description = "Version; send it in `If-Match` to change the status."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `comments.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No comment has this id; code `comments.not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_comment(
    State(service): State<Arc<CommentsService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<CommentDetail>, Error> {
    let found = service.get(&principal, id).await?;
    let version = found.row.updated_at;
    let described = with_targets(&service, vec![found.into()]).await?.into_iter().next();
    let data = described.ok_or_else(|| Error::internal("describing a comment returned nothing"))?;
    Ok(Tagged { version, data })
}

/// A moderation decision.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateCommentRequest {
    /// `published` (approve, or restore), `rejected`, `spam`, or `pending` (hold again).
    status: Status,
}

/// Approve, reject, mark as spam or hold a comment.
///
/// Requires `comments.moderate`. `If-Match` is required. Approving sends reply mails to people
/// following the comment answered (`comments.comment_published`). Every decision is recorded
/// (`comments.comment_status_changed`, in the audit log).
#[utoipa::path(
    patch,
    path = "/{id}",
    extensions(("x-permissions" = json!(["comments.moderate"]))),
    operation_id = "update_comment",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the comment."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    request_body = UpdateCommentRequest,
    responses(
        (status = 200, description = "The comment after the change.", body = Data<CommentDetail>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `comments.moderate`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No comment has this id; code `comments.not_found`.", body = Problem),
        (status = 409, description = "The author deleted it; code `comments.deleted`.", body = Problem),
        (status = 412, description = "The comment changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "The status is invalid; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn update_comment(
    State(service): State<Arc<CommentsService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateCommentRequest>,
) -> Result<Tagged<CommentDetail>, Error> {
    let expected = if_match.required()?;
    let found = service.set_status(&principal, id, input.status, expected).await?;
    Ok(Tagged { version: found.row.updated_at, data: found.into() })
}

/// The answer to a delete for good.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CommentsPurged)]
pub(crate) struct PurgedResponse {
    /// Comments deleted: the comment, and its replies when it starts a thread.
    pub deleted: u64,
}

/// Delete a comment for good.
///
/// Requires `comments.moderate`. Deleting a top-level comment deletes its whole thread. Use it
/// for takedown requests; the decision is recorded (`comments.comment_deleted`, `hard: true`).
#[utoipa::path(
    delete,
    path = "/{id}",
    extensions(("x-permissions" = json!(["comments.moderate"]))),
    operation_id = "delete_comment",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the comment.")),
    responses(
        (status = 200, description = "How many comments were deleted.", body = Data<PurgedResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `comments.moderate`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No comment has this id; code `comments.not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_comment(
    State(service): State<Arc<CommentsService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<PurgedResponse>, Error> {
    let deleted = service.hard_delete(&principal, id).await?;
    Ok(Data { data: PurgedResponse { deleted } })
}

/// List the reports of a comment.
///
/// Requires `comments.read`. Newest first.
#[utoipa::path(
    get,
    path = "/{id}/reports",
    extensions(("x-permissions" = json!(["comments.read"]))),
    operation_id = "list_comment_reports",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the comment."), PageQuery),
    responses(
        (status = 200, description = "One page of reports.", body = Page<ReportResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `comments.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No comment has this id; code `comments.not_found`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_reports(
    State(service): State<Arc<CommentsService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<ReportResponse>, Error> {
    let page = page_request(paging, Vec::new())?;
    Ok(service.reports_of(&principal, id, &page).await?.map(ReportResponse::from))
}

/// Which bans to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct BansQuery {
    /// `user`, `email` or `ip`.
    kind: Option<String>,
    /// `true`: only bans still in force.
    #[param(value_type = Option<bool>)]
    active: Option<String>,
}

/// List bans.
///
/// Requires `comments.manage`. Newest first.
#[utoipa::path(
    get,
    path = "/bans",
    extensions(("x-permissions" = json!(["comments.manage"]))),
    operation_id = "list_comment_bans",
    security(("bearer" = [])),
    params(BansQuery, PageQuery),
    responses(
        (status = 200, description = "One page of bans.", body = Page<BanResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `comments.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_bans(
    State(service): State<Arc<CommentsService>>,
    principal: Principal,
    Query(query): Query<BansQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<BanResponse>, Error> {
    let mut problems = Vec::new();
    let kind = query.kind.as_deref().map(str::trim).filter(|kind| !kind.is_empty());
    if kind.is_some_and(|kind| !matches!(kind, "user" | "email" | "ip")) {
        problems.push(
            FieldError::new("kind", "invalid", "must be one of {ban_kinds}")
                .with_param("ban_kinds", BAN_KINDS),
        );
    }
    let active = match query.active.as_deref().map(str::trim) {
        None | Some("" | "false") => false,
        Some("true") => true,
        Some(_) => {
            problems.push(FieldError::new("active", "invalid", "must be `true` or `false`"));
            false
        }
    };
    let page = page_request(paging, problems)?;
    let filter = BanFilter { kind: kind.map(str::to_owned), active };
    Ok(service.bans(&principal, &filter, &page).await?.map(BanResponse::from))
}

/// A new ban.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BanRequest {
    /// `user` (an account), `email` (an address) or `ip` (a network fingerprint).
    kind: String,
    /// The user id, the email address, or the `ip_fingerprint` shown on a comment.
    value: String,
    /// Why (required, 1 to 500 characters; for the records).
    reason: String,
    /// When it ends; omit for ever.
    until: Option<DateTime<Utc>>,
}

/// Ban someone from commenting.
///
/// Requires `comments.manage`. Their new comments (and changes) are refused with
/// `comments.banned` until `until`. Recorded as `comments.ban_created` (without the value).
#[utoipa::path(
    post,
    path = "/bans",
    extensions(("x-permissions" = json!(["comments.manage"]))),
    operation_id = "create_comment_ban",
    security(("bearer" = [])),
    request_body = BanRequest,
    responses(
        (status = 201, description = "Banned.", body = Data<BanResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `comments.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "Invalid input; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_ban(
    State(service): State<Arc<CommentsService>>,
    principal: Principal,
    ValidJson(input): ValidJson<BanRequest>,
) -> Result<Created<BanResponse>, Error> {
    let ban = service
        .ban(
            &principal,
            NewBan {
                kind: input.kind,
                value: input.value,
                reason: input.reason,
                until: input.until,
            },
        )
        .await?;
    Ok(Created::new(format!("/comments/bans/{}", ban.id), ban.into()))
}

/// Lift a ban.
///
/// Requires `comments.manage`. Recorded as `comments.ban_deleted`.
#[utoipa::path(
    delete,
    path = "/bans/{id}",
    extensions(("x-permissions" = json!(["comments.manage"]))),
    operation_id = "delete_comment_ban",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the ban.")),
    responses(
        (status = 204, description = "Lifted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `comments.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No ban has this id; code `comments.ban_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_ban(
    State(service): State<Arc<CommentsService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.unban(&principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Get the comment state of a target, with its pending count.
///
/// Requires `comments.read`.
#[utoipa::path(
    get,
    path = "/targets/{target_type}/{target_id}",
    extensions(("x-permissions" = json!(["comments.read"]))),
    operation_id = "get_comment_target",
    security(("bearer" = [])),
    params(
        ("target_type" = String, Path, description = "Kind of the target, e.g. `content.entry`."),
        ("target_id" = String, Path, description = "Id of the target in its module."),
    ),
    responses(
        (status = 200, description = "The state.", body = Data<TargetResponse>, headers(("ETag" = String, description = "Version (when the target has a row); send it in `If-Match` to change it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `comments.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "This kind of target takes no comments; code `comments.target_not_found`.", body = Problem),
        (status = 422, description = "The id is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn get_target(
    State(service): State<Arc<CommentsService>>,
    principal: Principal,
    Path((kind, id)): Path<(String, String)>,
) -> Result<axum::response::Response, Error> {
    use axum::response::IntoResponse as _;
    let target = service.admin_target(&principal, &kind, &id).await?;
    Ok(match target.updated_at {
        Some(version) => {
            Tagged { version, data: TargetResponse::new(target, true) }.into_response()
        }
        None => Data { data: TargetResponse::new(target, true) }.into_response(),
    })
}

/// Which comment targets to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct TargetsQuery {
    /// Only targets of this kind, e.g. `content.entry`.
    pub target_type: Option<String>,
    /// `open` or `closed`: the state that applies now (hidden targets and those past `closes_at`
    /// are `closed`).
    #[param(value_type = Option<TargetStateParam>, inline)]
    pub state: Option<String>,
    /// `true`: only targets with comments waiting for approval; `false`: only those without.
    #[param(value_type = Option<bool>)]
    pub pending: Option<String>,
}

/// State of a target's comments.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TargetStateParam {
    Open,
    Closed,
}

/// List the targets that have comments.
///
/// Requires `comments.read`. Each target (a post, a product…) that got a comment or had its state
/// set, newest first, with its counts and — when public — its title and path. Filter by kind,
/// state, or targets waiting for moderation (`pending=true`). Follow `pagination.next_cursor` with
/// the same filters for more.
#[utoipa::path(
    get,
    path = "/targets",
    extensions(("x-permissions" = json!(["comments.read"]))),
    operation_id = "list_comment_targets",
    security(("bearer" = [])),
    params(TargetsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of targets.", body = Page<TargetResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `comments.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_targets(
    State(service): State<Arc<CommentsService>>,
    principal: Principal,
    Query(query): Query<TargetsQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<TargetResponse>, Error> {
    let mut problems = Vec::new();
    let state = match query.state.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(state @ ("open" | "closed")) => Some(state.to_owned()),
        Some(_) => {
            problems.push(FieldError::new("state", "invalid", "must be `open` or `closed`"));
            None
        }
    };
    let pending = match query.pending.as_deref().map(str::trim) {
        None | Some("") => None,
        Some("true") => Some(true),
        Some("false") => Some(false),
        Some(_) => {
            problems.push(FieldError::new("pending", "invalid", "must be `true` or `false`"));
            None
        }
    };
    let page = page_request(paging, problems)?;
    let filter = TargetFilter { kind: query.target_type, state, pending };
    let page = service.list_targets(&principal, &filter, &page).await?;
    let keys: Vec<(String, String)> =
        page.data.iter().map(|target| (target.kind.clone(), target.id.clone())).collect();
    let found = service.target_titles(&keys).await?;
    Ok(page.map(|target| {
        let key = (target.kind.clone(), target.id.clone());
        let (title, url) = super::views::described(found.get(&key));
        TargetResponse { title, url, ..TargetResponse::new(target, true) }
    }))
}

// Trường JSON có ba trạng thái: không gửi (`None`), `null` (`Some(None)`), có giá trị.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// A new comment state for a target. Omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetRequest {
    /// `open` or `closed`.
    state: Option<String>,
    /// `true` hides every comment of the target from everyone (and closes it).
    hidden: Option<bool>,
    /// Close by itself from then on; `null` removes it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<DateTime<Utc>>, nullable)]
    closes_at: Option<Option<DateTime<Utc>>>,
}

/// Open, close or hide the comments of a target.
///
/// Requires `comments.manage`. `If-Match` is optional (send the `ETag` you read to make sure
/// nobody changed it since). Recorded as `comments.target_opened` / `comments.target_closed`.
#[utoipa::path(
    put,
    path = "/targets/{target_type}/{target_id}",
    extensions(("x-permissions" = json!(["comments.manage"]))),
    operation_id = "set_comment_target",
    security(("bearer" = [])),
    params(
        ("target_type" = String, Path, description = "Kind of the target, e.g. `content.entry`."),
        ("target_id" = String, Path, description = "Id of the target in its module."),
        ("If-Match" = Option<String>, Header, description = "Optional: the `ETag` you read."),
    ),
    request_body = TargetRequest,
    responses(
        (status = 200, description = "The state after the change.", body = Data<TargetResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `comments.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "This kind of target takes no comments; code `comments.target_not_found`.", body = Problem),
        (status = 412, description = "The target changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "Invalid input; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn set_target(
    State(service): State<Arc<CommentsService>>,
    principal: Principal,
    Path((kind, id)): Path<(String, String)>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<TargetRequest>,
) -> Result<Tagged<TargetResponse>, Error> {
    let expected = if_match.expected()?;
    let change =
        TargetChange { state: input.state, hidden: input.hidden, closes_at: input.closes_at };
    let target = service.set_target(&principal, &kind, &id, change, expected).await?;
    let version = target.updated_at.unwrap_or_default();
    Ok(Tagged { version, data: TargetResponse::new(target, true) })
}

/// Whose data.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct EmailQuery {
    /// The email address of the person (any case).
    #[param(example = "ada@example.com")]
    email: Option<String>,
}

impl EmailQuery {
    fn email(&self) -> Result<&str, Error> {
        self.email.as_deref().ok_or_else(|| {
            Error::validation(vec![FieldError::new("email", "required", "is required")])
        })
    }
}

/// A followed comment.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct CommentFollow {
    pub comment_id: Uuid,
    pub created_at: DateTime<Utc>,
}

/// Everything the comments hold about one email address.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CommentsEmailData)]
pub(crate) struct EmailDataResponse {
    /// The address, as matched (lower case).
    email: String,
    /// Every comment written with this address, oldest first, any status.
    comments: Vec<CommentDetail>,
    /// Comments whose replies are mailed to this address.
    follows: Vec<CommentFollow>,
    /// Bans of this address (kept after an erasure, so the ban still works).
    bans: Vec<BanResponse>,
}

/// The result of erasing an email address.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CommentsErasedResponse)]
pub(crate) struct ErasedResponse {
    /// Comments cleared.
    cleared: u64,
}

/// Export the data of an email address.
///
/// Requires `comments.export`. For a request of the person the data is about: every comment
/// written with this address, the comments it follows, and bans of it.
#[utoipa::path(
    get,
    path = "/data",
    extensions(("x-permissions" = json!(["comments.export"]))),
    operation_id = "export_comments_email_data",
    security(("bearer" = [])),
    params(EmailQuery),
    responses(
        (status = 200, description = "The data.", body = Data<EmailDataResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `comments.export`; code `permission_denied`.", body = Problem),
        (status = 422, description = "`email` is missing or not an email address; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn get_email_data(
    State(service): State<Arc<CommentsService>>,
    principal: Principal,
    Query(query): Query<EmailQuery>,
) -> Result<Data<EmailDataResponse>, Error> {
    // Kiểm quyền trước khi đọc `email`: người không có quyền nhận 403, không phải 422 (D-245).
    principal.require(&crate::permissions::COMMENTS_EXPORT)?;
    let found = service.email_data(&principal, query.email()?).await?;
    Ok(Data {
        data: EmailDataResponse {
            email: autoxide_kernel::email::normalize(query.email()?),
            comments: found.comments.into_iter().map(CommentDetail::from).collect(),
            follows: found
                .subscriptions
                .into_iter()
                .map(|row| CommentFollow { comment_id: row.comment_id, created_at: row.created_at })
                .collect(),
            bans: found.bans.into_iter().map(BanResponse::from).collect(),
        },
    })
}

/// Erase the data of an email address.
///
/// Requires `comments.export`. Every comment written with this address loses its text, name,
/// email and network fingerprint (its place in the thread stays), and reply mails to it stop.
/// Bans of the address are kept. Recorded as `comments.comments_erased`, without the address.
#[utoipa::path(
    delete,
    path = "/data",
    extensions(("x-permissions" = json!(["comments.export"]))),
    operation_id = "erase_comments_email_data",
    security(("bearer" = [])),
    params(EmailQuery),
    responses(
        (status = 200, description = "How many comments were cleared.", body = Data<ErasedResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `comments.export`; code `permission_denied`.", body = Problem),
        (status = 422, description = "`email` is missing or not an email address; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn delete_email_data(
    State(service): State<Arc<CommentsService>>,
    principal: Principal,
    Query(query): Query<EmailQuery>,
) -> Result<Data<ErasedResponse>, Error> {
    // Kiểm quyền trước khi đọc `email`: người không có quyền nhận 403, không phải 422 (D-245).
    principal.require(&crate::permissions::COMMENTS_EXPORT)?;
    let cleared = service.erase_email(&principal, query.email()?).await?;
    Ok(Data { data: ErasedResponse { cleared } })
}
