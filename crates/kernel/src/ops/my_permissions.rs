//! `GET /api/v1/me/permissions` (UI brief A5, D-264): the caller's own permissions, with descriptions per
//! `Accept-Language` and a risk level. A person creating an API key picks permissions from here — without
//! needing `auth.roles.read` to read the whole system's permission registry.

use std::sync::Arc;

use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::access::{PermissionCatalog, Principal};
use crate::error::Error;
use crate::i18n::{Language, Translations};
use crate::pagination::{NaturalOrderPageQuery, Page, PageRequest, page_of};
use crate::response::Problem;

pub(crate) struct MyPermissions {
    pub(crate) catalog: PermissionCatalog,
    pub(crate) translations: Translations,
}

/// A permission the caller holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub(crate) struct MyPermission {
    #[schema(example = "notes.write")]
    name: String,
    /// What it allows, in the language of `Accept-Language`.
    description: String,
    /// How careful a grant needs to be: `read`, `write`, `destructive`, `sensitive_data` or `money`.
    #[schema(example = "write")]
    risk: String,
}

pub(crate) fn routes(state: MyPermissions) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(list_my_permissions)).with_state(Arc::new(state))
}

/// List your own permissions.
///
/// Any signed-in user. Every permission the caller holds (through roles, or the permissions of
/// the API key used), with its description and risk, by name (`order=desc` reverses it); paged
/// with `limit` / `cursor`. Use it to offer the permissions a new API key may get: a key cannot
/// have a permission its owner lacks.
#[utoipa::path(
    get,
    path = "/permissions",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_permissions",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the caller's permissions, by name.", body = Page<MyPermission>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_my_permissions(
    State(state): State<Arc<MyPermissions>>,
    principal: Principal,
    language: Language,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<MyPermission>, Error> {
    let page = PageRequest::try_from(page)?;
    // Sổ permission do module khai trong code, lọc theo người gọi: phân trang trong bộ nhớ theo tên
    // (D-20261125-list-pages mục 4).
    let data: Vec<MyPermission> = state
        .catalog
        .iter()
        .filter(|permission| principal.has(permission))
        .map(|permission| MyPermission {
            name: permission.name().to_owned(),
            description: state.translations.describe(permission.description(), language),
            risk: permission.risk().as_str().to_owned(),
        })
        .collect();
    page_of(data, |permission| permission.name.clone(), &page)
}
