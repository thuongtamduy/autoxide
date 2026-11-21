//! Thao tác đổi trạng thái của mục: đăng, gỡ, lưu trữ, chuyển trang, xoá. Mọi thao tác bắt
//! buộc `If-Match` (D-102).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::entries::{EntryResponse, tagged};
use crate::service::ContentService;

/// Options of a publish.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublishRequest {
    /// The date shown as the publication date. Default: unchanged, or now on first publish.
    /// Needs `content.publish`: with only `content.publish_own` it must be left out (422,
    /// `errors[].code` `not_allowed`).
    pub published_at: Option<Timestamp>,
    /// For an entry marked `ai_generated`: you, a person, reviewed the content and take
    /// editorial responsibility for it. Recorded on the published version (readers see
    /// `ai_reviewed`). Only a signed-in person may send `true`; an API key gets 403
    /// `content.ai_review_requires_human`.
    #[serde(default)]
    pub ai_reviewed: bool,
}

/// Where to move a page.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MoveRequest {
    /// The new parent page; `null` moves it to the top level.
    pub parent_id: Option<Uuid>,
}

const IF_MATCH: &str = "Required: the `ETag` of the entry (or `*`). Done only if nobody changed the entry since; otherwise `412`.";

/// Publish a post or page.
///
/// Requires `content.publish`, or `content.publish_own` for an entry you created. With only
/// `content.publish_own` you cannot republish an archived entry, set `published_at`, or publish
/// at an address where an editor made a redirect. Copies the current draft to the published
/// version, at an
/// address computed by the server (`/<post prefix>/<slug>` for posts, the parent's address
/// plus the slug for pages). If the address changed, the old one (and those of published
/// subpages) redirect with 301. Every media item it uses must be public. `If-Match` is
/// required. Retry safely with an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/entries/{id}/publish",
    extensions(("x-permissions" = json!(["content.publish", "content.publish_own"]))),
    operation_id = "publish_content_entry",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry."), ("If-Match" = String, Header, description = IF_MATCH)),
    request_body(content = PublishRequest, description = "Optional; may be omitted."),
    responses(
        (status = 200, description = "Published.", body = Data<EntryResponse>, headers(("ETag" = String, description = "New version of the entry."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.publish` (or `content.publish_own`; code `permission_denied`), the entry is not yours and you only have `content.publish_own` (code `content.not_owner`), an editor archived it and you only have `content.publish_own` (code `content.archived`), or an API key sent `ai_reviewed` (code `content.ai_review_requires_human`).", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 409, description = "The address is used by another published entry (code `content.path_taken`), the parent page is not published (code `content.parent_not_published`), or an editor's redirect is at the address and you only have `content.publish_own` (code `content.path_has_redirect`).", body = Problem),
        (status = 412, description = "The entry changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A media item is missing (code `content.media_not_found`) or not public (code `content.media_not_public`), or `published_at` was sent with only `content.publish_own`; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn publish_entry(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    body: axum::body::Bytes,
) -> Result<Tagged<EntryResponse>, Error> {
    let expected = if_match.required()?;
    // Thân không bắt buộc: rỗng thì dùng mặc định.
    let input: PublishRequest = if body.iter().all(u8::is_ascii_whitespace) {
        PublishRequest::default()
    } else {
        serde_json::from_slice(&body).map_err(|error| {
            Error::validation(vec![
                autoxide_kernel::error::FieldError::new(
                    "body",
                    "invalid",
                    "is not valid: {reason}",
                )
                .with_param("reason", error),
            ])
        })?
    };
    let entry = service
        .publish(&tenant, &principal, id, expected, input.published_at, input.ai_reviewed)
        .await?;
    tagged(&service, &tenant, entry).await
}

/// Take a post or page down.
///
/// Requires `content.publish`, or `content.publish_own` for an entry you created. Readers no longer
/// see it (its address answers 404); it is a draft again and may be published later. A pending
/// scheduled publish of the entry is canceled. A page with published subpages cannot be taken
/// down. `If-Match` is required.
#[utoipa::path(
    post,
    path = "/entries/{id}/unpublish",
    extensions(("x-permissions" = json!(["content.publish", "content.publish_own"]))),
    operation_id = "unpublish_content_entry",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry."), ("If-Match" = String, Header, description = IF_MATCH)),
    responses(
        (status = 200, description = "Taken down.", body = Data<EntryResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.publish` or `content.publish_own` (code `permission_denied`), or the entry is not yours and you only have `content.publish_own` (code `content.not_owner`).", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 409, description = "Not published (code `content.not_published`), or a page with published subpages (code `content.page_has_published_children`).", body = Problem),
        (status = 412, description = "The entry changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn unpublish_entry(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<Tagged<EntryResponse>, Error> {
    let expected = if_match.required()?;
    tagged(&service, &tenant, service.unpublish(&tenant, &principal, id, expected).await?).await
}

/// Archive a post or page.
///
/// Requires `content.publish`. Takes it down for good: its address answers 410 (gone) so
/// search engines drop it. Pending schedules of the entry are canceled. It can still be
/// published again (with `content.publish`). `If-Match` is required.
#[utoipa::path(
    post,
    path = "/entries/{id}/archive",
    extensions(("x-permissions" = json!(["content.publish"]))),
    operation_id = "archive_content_entry",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry."), ("If-Match" = String, Header, description = IF_MATCH)),
    responses(
        (status = 200, description = "Archived.", body = Data<EntryResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.publish`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 409, description = "A page with published subpages; code `content.page_has_published_children`.", body = Problem),
        (status = 412, description = "The entry changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn archive_entry(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<Tagged<EntryResponse>, Error> {
    let expected = if_match.required()?;
    tagged(&service, &tenant, service.archive(&tenant, &principal, id, expected).await?).await
}

/// Bring an archived post or page back.
///
/// Requires `content.publish`. The undo of archiving: the entry becomes a draft again, unchanged.
/// Its old address keeps answering 410 until it is published again (an archived entry may also be
/// published directly). `If-Match` is required.
#[utoipa::path(
    post,
    path = "/entries/{id}/unarchive",
    extensions(("x-permissions" = json!(["content.publish"]))),
    operation_id = "unarchive_content_entry",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry."), ("If-Match" = String, Header, description = IF_MATCH)),
    responses(
        (status = 200, description = "A draft again.", body = Data<EntryResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.publish`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 409, description = "The entry is not archived; code `content.not_archived`.", body = Problem),
        (status = 412, description = "The entry changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn unarchive_entry(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<Tagged<EntryResponse>, Error> {
    let expected = if_match.required()?;
    tagged(&service, &tenant, service.unarchive(&tenant, &principal, id, expected).await?).await
}

/// Move a page under another parent.
///
/// Requires `content.write` (or `content.write_own` for a page you created, under a parent you
/// created), and `content.publish` when the page is published. Takes effect at once for the draft and the published version: the
/// page's address and those of its subpages change, and the old ones redirect with 301. `If-Match`
/// is required.
#[utoipa::path(
    post,
    path = "/entries/{id}/move",
    extensions(("x-permissions" = json!(["content.write", "content.write_own", "content.publish"]))),
    operation_id = "move_content_page",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the page."), ("If-Match" = String, Header, description = IF_MATCH)),
    request_body = MoveRequest,
    responses(
        (status = 200, description = "Moved.", body = Data<EntryResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` or `content.write_own` (code `permission_denied`), or the page or the new parent is not yours and you only have `content.write_own` (code `content.not_owner`); or the page is published and you lack `content.publish`.", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 409, description = "Under itself or a subpage (code `content.tree_cycle`), slug taken at the new place (code `content.slug_taken`), address taken (code `content.path_taken`), or the new parent is not published (code `content.parent_not_published`).", body = Problem),
        (status = 412, description = "The entry changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "Not a page, or the parent is invalid or too deep (`tree_too_deep`); see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn move_entry(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<MoveRequest>,
) -> Result<Tagged<EntryResponse>, Error> {
    let expected = if_match.required()?;
    tagged(
        &service,
        &tenant,
        service.move_page(&tenant, &principal, id, input.parent_id, expected).await?,
    )
    .await
}

/// Delete a post or page.
///
/// Requires `content.delete`. Its revisions and schedules go too. A page with subpages
/// cannot be deleted. If it was published, its address answers 410. `If-Match` is required. An
/// AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    delete,
    path = "/entries/{id}",
    extensions(("x-permissions" = json!(["content.delete"])), ("x-approval" = json!("agent"))),
    operation_id = "delete_content_entry",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry."), ("If-Match" = String, Header, description = IF_MATCH)),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.delete`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 409, description = "The page has subpages; code `content.page_has_children`.", body = Problem),
        (status = 412, description = "The entry changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn delete_entry(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<StatusCode, Error> {
    let expected = if_match.required()?;
    service.delete(&tenant, &principal, id, expected).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Ask for a review of the draft.
///
/// Requires `content.write`, or `content.write_own` for an entry you created. Marks the draft as
/// ready for a person to review and publish (`review_requested_at`) and emits the event
/// `content.review_requested`, which webhooks can forward to editors. The mark is cleared when
/// the entry is published or the request is withdrawn. Meant for AI agents that may write but
/// not publish. `If-Match` is required.
#[utoipa::path(
    post,
    path = "/entries/{id}/request-review",
    extensions(("x-permissions" = json!(["content.write", "content.write_own"]))),
    operation_id = "request_content_review",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry."), ("If-Match" = String, Header, description = IF_MATCH)),
    responses(
        (status = 200, description = "Review requested.", body = Data<EntryResponse>, headers(("ETag" = String, description = "New version of the entry."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` or `content.write_own` (code `permission_denied`), or the entry is not yours and you only have `content.write_own` (code `content.not_owner`).", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 412, description = "The entry changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn request_review(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<Tagged<EntryResponse>, Error> {
    let expected = if_match.required()?;
    tagged(
        &service,
        &tenant,
        service.set_review_request(&tenant, &principal, id, expected, false).await?,
    )
    .await
}

/// Withdraw a review request.
///
/// Requires `content.write`, or `content.write_own` for an entry you created. Clears
/// `review_requested_at` and emits `content.review_withdrawn`. `If-Match` is required.
#[utoipa::path(
    delete,
    path = "/entries/{id}/request-review",
    extensions(("x-permissions" = json!(["content.write", "content.write_own"]))),
    operation_id = "withdraw_content_review",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry."), ("If-Match" = String, Header, description = IF_MATCH)),
    responses(
        (status = 200, description = "Review request withdrawn.", body = Data<EntryResponse>, headers(("ETag" = String, description = "New version of the entry."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` or `content.write_own` (code `permission_denied`), or the entry is not yours and you only have `content.write_own` (code `content.not_owner`).", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 412, description = "The entry changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn withdraw_review(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<Tagged<EntryResponse>, Error> {
    let expected = if_match.required()?;
    tagged(
        &service,
        &tenant,
        service.set_review_request(&tenant, &principal, id, expected, true).await?,
    )
    .await
}
