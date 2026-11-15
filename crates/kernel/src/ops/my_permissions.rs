//! `GET /api/v1/me/permissions` (đề bài giao diện A5, D-264): permission của chính người gọi, kèm
//! mô tả theo `Accept-Language` và mức rủi ro. Người tạo API key chọn permission từ đây — không
//! cần `auth.roles.read` để đọc sổ permission của cả hệ thống.

use std::sync::Arc;

use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::access::{PermissionCatalog, Principal};
use crate::error::Error;
use crate::i18n::{Language, Translations};
use crate::response::{Data, Problem};

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
/// the API key used), with its description and risk, sorted by name. Use it to offer the
/// permissions a new API key may get: a key cannot have a permission its owner lacks.
#[utoipa::path(
    get,
    path = "/permissions",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_permissions",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The caller's permissions, by name.", body = Data<Vec<MyPermission>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn list_my_permissions(
    State(state): State<Arc<MyPermissions>>,
    principal: Principal,
    language: Language,
) -> Result<Data<Vec<MyPermission>>, Error> {
    let mut data: Vec<MyPermission> = state
        .catalog
        .iter()
        .filter(|permission| principal.has(permission))
        .map(|permission| MyPermission {
            name: permission.name().to_owned(),
            description: state.translations.describe(permission.description(), language),
            risk: permission.risk().as_str().to_owned(),
        })
        .collect();
    data.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Data { data })
}
