//! Giải thích quyền (bước 51 P11, D-201): `POST /api/v1/me/can` nhận tối đa 50 cặp
//! (permission, nút) và trả, cho từng cặp, được hay không và **vì sao không** — để giao diện hiện
//! lý do nút bị khoá thay vì ẩn nút. Kết quả dùng đúng các hàm kiểm thật (`module_enabled` của gói,
//! `Principal::require_at`), theo cùng thứ tự với một request thật: module bật trước, permission
//! sau.
//!
//! Lý do: `unknown_permission` (không có permission đó), `module_not_enabled` (gói của khách chưa
//! có module, kèm `required_module`), `missing_permission` (người gọi không có permission ở nút đó).

use std::sync::Arc;

use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::access::{OrgTarget, PermissionCatalog, Principal};
use crate::entitlement::EntitlementsHandle;
use crate::error::{Error, FieldError};
use crate::http::ValidJson;
use crate::response::{Data, Problem};
use crate::tenancy::{OrgId, TenantContext, root_of};

/// Hỏi tối đa chừng này cặp mỗi lần.
const MAX_CHECKS: usize = 50;

/// One question: may I use `permission` at `org_id`?
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub(crate) struct CanCheck {
    #[schema(example = "content.publish")]
    pub permission: String,
    /// The organization unit; omitted: the whole tenant.
    #[serde(default)]
    pub org_id: Option<Uuid>,
}

/// The questions.
#[derive(Debug, Clone, Deserialize, ToSchema, validator::Validate)]
pub(crate) struct CanRequest {
    /// 1 to 50 questions.
    pub checks: Vec<CanCheck>,
}

/// The answer to one question.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct CanAnswer {
    pub permission: String,
    pub org_id: Option<Uuid>,
    pub allowed: bool,
    /// Why not: `unknown_permission`, `module_not_enabled` (the tenant's plan does not include the
    /// module) or `missing_permission` (none of your roles grants it at this unit).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// With `module_not_enabled`: the module the plan must include.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_module: Option<String>,
}

#[derive(Clone)]
pub(crate) struct CanState {
    pub permissions: PermissionCatalog,
    pub entitlements: EntitlementsHandle,
}

/// Ask what you may do, and why not.
///
/// Any signed-in user. For each `{permission, org_id}` (up to 50), says whether you may use it
/// there and, when not, the reason — the same checks a real request makes, in the same order: the
/// module must be in the tenant's plan, then one of your roles must grant the permission at that
/// unit. Use it to show why a button is disabled instead of hiding it.
#[utoipa::path(
    post,
    path = "/can",
    extensions(("x-permissions" = json!([]))),
    operation_id = "check_my_permissions",
    security(("bearer" = [])),
    request_body = CanRequest,
    responses(
        (status = 200, description = "One answer per question, in order.", body = Data<Vec<CanAnswer>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "No question or more than 50; see `errors`.", body = Problem),
    )
)]
async fn check_my_permissions(
    State(state): State<Arc<CanState>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CanRequest>,
) -> Result<Data<Vec<CanAnswer>>, Error> {
    if input.checks.is_empty() || input.checks.len() > MAX_CHECKS {
        return Err(Error::validation(vec![
            FieldError::new("checks", "invalid", "must hold 1 to {max} questions")
                .with_param("max", MAX_CHECKS),
        ]));
    }
    let read = tenant.database().read();
    let mut answers = Vec::with_capacity(input.checks.len());
    for check in input.checks {
        let answer = |allowed: bool, reason: Option<&str>, module: Option<&str>| CanAnswer {
            permission: check.permission.clone(),
            org_id: check.org_id,
            allowed,
            reason: reason.map(str::to_owned),
            required_module: module.map(str::to_owned),
        };
        let Some(permission) =
            state.permissions.iter().find(|known| known.name() == check.permission).cloned()
        else {
            answers.push(answer(false, Some("unknown_permission"), None));
            continue;
        };
        let module = permission.name().split('.').next().unwrap_or_default();
        if !state.entitlements.module_enabled(tenant.id(), module).await? {
            answers.push(answer(false, Some("module_not_enabled"), Some(module)));
            continue;
        }
        let target =
            OrgTarget::new(check.org_id.map_or(root_of(tenant.id()), OrgId::from_uuid), None);
        match principal.require_at(read, &permission, target).await {
            Ok(()) => answers.push(answer(true, None, None)),
            Err(_) => answers.push(answer(false, Some("missing_permission"), None)),
        }
    }
    Ok(Data { data: answers })
}

/// Route dưới `/api/v1/me`.
pub(crate) fn routes(state: CanState) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(check_my_permissions)).with_state(Arc::new(state))
}
