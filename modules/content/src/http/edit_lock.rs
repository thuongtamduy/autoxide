//! Khoá sửa trang (bước 67): xem, chiếm / gia hạn, nhả. Logic ở `entries/edit_lock.rs`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use uuid::Uuid;

use crate::entries::edit_lock::EditLock;
use crate::service::ContentService;

/// See who is editing an entry.
///
/// Requires `content.read`. `data` is `null` when nobody holds a live lock. The lock is advisory:
/// saving still needs a matching `If-Match`.
#[utoipa::path(
    get,
    path = "/entries/{id}/edit-lock",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "get_content_entry_edit_lock",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry.")),
    responses(
        (status = 200, description = "The live lock, or `null` when nobody is editing.", body = Data<Option<EditLock>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_edit_lock(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Option<EditLock>>, Error> {
    Ok(Data { data: service.edit_lock(&tenant, &principal, id).await? })
}

/// Take or renew the edit lock of an entry.
///
/// Requires `content.write`, or `content.write_own` for an entry you created. Call it when an
/// editor opens the entry and again about every minute while it stays open: the lock lasts 2
/// minutes from each call. If someone else holds a live lock, the answer is `409` naming them and
/// when their lock ends. The lock only tells others; saving still needs a matching `If-Match`.
#[utoipa::path(
    post,
    path = "/entries/{id}/edit-lock",
    extensions(("x-permissions" = json!(["content.write", "content.write_own"]))),
    operation_id = "acquire_content_entry_edit_lock",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry.")),
    responses(
        (status = 200, description = "You hold the lock until `expires_at`.", body = Data<EditLock>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` or `content.write_own` (code `permission_denied`), or the entry is not yours and you only have `content.write_own` (code `content.not_owner`).", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 409, description = "Someone else is editing; code `content.edit_locked`, with `user` and `until` in the details.", body = Problem),
    )
)]
pub(crate) async fn acquire_edit_lock(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<EditLock>, Error> {
    Ok(Data { data: service.acquire_edit_lock(&tenant, &principal, id).await? })
}

/// Release your edit lock of an entry.
///
/// Requires `content.write`, or `content.write_own` for an entry you created. Call it when the
/// editor closes. Releasing when you hold no lock does nothing; a lock held by someone else is not
/// released (`409`).
#[utoipa::path(
    delete,
    path = "/entries/{id}/edit-lock",
    extensions(("x-permissions" = json!(["content.write", "content.write_own"]))),
    operation_id = "release_content_entry_edit_lock",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry.")),
    responses(
        (status = 204, description = "You no longer hold a lock on the entry."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` or `content.write_own` (code `permission_denied`), or the entry is not yours and you only have `content.write_own` (code `content.not_owner`).", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 409, description = "Someone else holds the lock; code `content.edit_locked`.", body = Problem),
    )
)]
pub(crate) async fn release_edit_lock(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.release_edit_lock(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
