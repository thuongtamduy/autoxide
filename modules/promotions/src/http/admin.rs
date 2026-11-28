//! Endpoint quản trị chương trình, dưới `/api/v1/promotions`. Handler chỉ: lấy dữ liệu vào →
//! kiểm permission → gọi service → trả envelope.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::api::Voucher;
use crate::codes::{Generated, NewCodes};
use crate::compliance::Check;
use crate::http::dto::{CodeDto, CodesAdded, PromotionListItem, PromotionResponse, VersionDto};
use crate::model::{Kind, Status};
use crate::permissions::{CODES_MANAGE, COMPLIANCE, PUBLISH, READ, VOUCHERS_GRANT, WRITE};
use crate::programs::transitions::Transition;
use crate::programs::{FlashItemInput, PromotionFilter, PromotionInput, TargetInput};
use crate::service::PromotionsService;

pub(crate) fn routes(service: Arc<PromotionsService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_promotions, create_promotion))
        .routes(routes!(get_promotion, update_promotion, delete_promotion))
        .routes(routes!(put_translation))
        .routes(routes!(put_targets))
        .routes(routes!(put_flash_items))
        .routes(routes!(activation_check))
        .routes(routes!(activate))
        .routes(routes!(pause))
        .routes(routes!(resume))
        .routes(routes!(end))
        .routes(routes!(cancel))
        .routes(routes!(list_codes, add_codes))
        .routes(routes!(export_codes))
        .routes(routes!(grant_voucher))
        .routes(routes!(versions))
        .routes(routes!(record_notice))
        .with_state(service)
}

/// Filters of the promotion list.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct PromotionsQuery {
    /// Only promotions in this state: `draft`, `scheduled`, `active`, `paused`, `ended` or `cancelled`.
    pub status: Option<Status>,
    /// Only promotions of this kind (see `PromotionsKind`).
    pub kind: Option<Kind>,
    /// Only promotions of this unit.
    pub org_id: Option<Uuid>,
    /// Words in the title.
    pub q: Option<String>,
}

/// List the promotions in your scope.
///
/// Requires `promotions.read`; its scope decides which units' promotions you see. Newest first
/// unless `order=asc`.
#[utoipa::path(
    get,
    path = "/promotions",
    extensions(("x-permissions" = json!(["promotions.read"]))),
    operation_id = "list_promotions",
    security(("bearer" = [])),
    params(PromotionsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of promotions.", body = Page<PromotionListItem>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_promotions(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<PromotionsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<PromotionListItem>, Error> {
    principal.require(&READ)?;
    let page = PageRequest::try_from(page)?;
    let filter = PromotionFilter {
        status: query.status,
        kind: query.kind,
        org_id: query.org_id,
        q: query.q,
        window: None,
    };
    let found = service.list_promotions(&tenant, &principal, &filter, &page).await?;
    Ok(found.map(|(row, title)| PromotionListItem {
        id: row.id,
        org_id: row.org_id,
        title,
        kind: Kind::parse(&row.kind).unwrap_or(Kind::PercentOff),
        status: Status::parse(&row.status).unwrap_or(Status::Draft),
        starts_at: row.starts_at,
        ends_at: row.ends_at,
        usage_count: row.usage_count,
    }))
}

/// Create a promotion as a draft.
///
/// Requires `promotions.write` on the unit. Every promotion has an end (`ends_at`). Turn it on
/// with `POST /api/v1/promotions/{id}/activate` after checking it.
#[utoipa::path(
    post,
    path = "/promotions",
    extensions(("x-permissions" = json!(["promotions.write"]))),
    operation_id = "create_promotion",
    security(("bearer" = [])),
    request_body = PromotionInput,
    responses(
        (status = 201, description = "Created as a draft.", body = Data<PromotionResponse>, headers(("Location" = String, description = "Path of the promotion."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.write` (`permission_denied`) or the unit is outside your scope (`org_out_of_scope`).", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`; shares not adding up to 100% are `promotions.invalid_funding`), or you work in several units and gave no `org_id` (`promotions.org_required`).", body = Problem),
    )
)]
async fn create_promotion(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<PromotionInput>,
) -> Result<Created<PromotionResponse>, Error> {
    principal.require(&WRITE)?;
    let view = service.create_promotion(&tenant, &principal, input).await?;
    Ok(Created::new(
        format!("/promotions/promotions/{}", view.row.id),
        PromotionResponse::from(&view),
    ))
}

/// Get a promotion with its rule, texts, targets and counters.
///
/// Requires `promotions.read` covering the promotion's unit; outside your scope answers 404.
#[utoipa::path(
    get,
    path = "/promotions/{id}",
    extensions(("x-permissions" = json!(["promotions.read"]))),
    operation_id = "get_promotion",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion.")),
    responses(
        (status = 200, description = "The promotion.", body = Data<PromotionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
    )
)]
async fn get_promotion(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<PromotionResponse>, Error> {
    principal.require(&READ)?;
    let view = service.promotion(&tenant, &principal, id).await?;
    Ok(Data { data: PromotionResponse::from(&view) })
}

/// Change a promotion.
///
/// Requires `promotions.write`. The body has any fields of the create body; the fields given
/// replace the current ones, `null` clears an optional one. A draft can change anything; once
/// turned on, only a later `ends_at`, higher `usage_limit`, `daily_usage_limit`, `voucher_quota`
/// or `budget`, and `title` / `terms` (`promotions.locked_after_start` otherwise). Every change
/// after it was turned on adds an entry to `GET /api/v1/promotions/{id}/versions`.
#[utoipa::path(
    patch,
    path = "/promotions/{id}",
    extensions(("x-permissions" = json!(["promotions.write"]))),
    operation_id = "update_promotion",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion.")),
    request_body(content = Object, description = "Fields of `PromotionsPromotionInput` to change."),
    responses(
        (status = 200, description = "The changed promotion.", body = Data<PromotionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.write` or the unit is outside your scope.", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
        (status = 409, description = "The change is not allowed after the promotion was turned on (`promotions.locked_after_start`), or it has ended (`promotions.invalid_transition`).", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn update_promotion(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(patch): ValidJson<Patch>,
) -> Result<Data<PromotionResponse>, Error> {
    principal.require(&WRITE)?;
    let view = service.update_promotion(&tenant, &principal, id, patch.0).await?;
    Ok(Data { data: PromotionResponse::from(&view) })
}

/// Thân PATCH: object JSON bất kỳ, service gộp và kiểm.
#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub(crate) struct Patch(pub serde_json::Value);

/// Delete a draft promotion.
///
/// Requires `promotions.write`. Only drafts; end or cancel a promotion that was turned on.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    delete,
    path = "/promotions/{id}",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["promotions.write"]))),
    operation_id = "delete_promotion",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.write` or the unit is outside your scope.", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
        (status = 409, description = "The promotion is not a draft; code `promotions.not_deletable`.", body = Problem),
    )
)]
async fn delete_promotion(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&WRITE)?;
    service.delete_promotion(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Title and terms in one language.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsTranslationRequest)]
pub(crate) struct TranslationRequest {
    /// 1 to 200 characters.
    pub title: String,
    /// Conditions the buyer reads, up to 5000 characters.
    #[serde(default)]
    pub terms: String,
}

/// Set the title and terms of a promotion in one language.
///
/// Requires `promotions.write`. Allowed after the promotion was turned on (texts do not change
/// the rule); the change is kept in the audit trail.
#[utoipa::path(
    put,
    path = "/promotions/{id}/translations/{locale}",
    extensions(("x-permissions" = json!(["promotions.write"]))),
    operation_id = "put_promotion_translation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion."), ("locale" = String, Path, description = "Language code, e.g. `vi`.")),
    request_body = TranslationRequest,
    responses(
        (status = 200, description = "The promotion.", body = Data<PromotionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.write` or the unit is outside your scope.", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
        (status = 409, description = "The promotion has ended or was cancelled; code `promotions.invalid_transition`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn put_translation(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, locale)): Path<(Uuid, String)>,
    ValidJson(body): ValidJson<TranslationRequest>,
) -> Result<Data<PromotionResponse>, Error> {
    principal.require(&WRITE)?;
    let view =
        service.put_translation(&tenant, &principal, id, &locale, &body.title, &body.terms).await?;
    Ok(Data { data: PromotionResponse::from(&view) })
}

/// The whole set of targets of a promotion.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsTargetsRequest)]
pub(crate) struct TargetsRequest {
    /// Up to 1000; empty = every item of the unit.
    pub targets: Vec<TargetInput>,
}

/// Replace the targets (products, variants, categories) of a draft promotion.
///
/// Requires `promotions.write`. Categories include their subcategories.
#[utoipa::path(
    put,
    path = "/promotions/{id}/targets",
    extensions(("x-permissions" = json!(["promotions.write"]))),
    operation_id = "put_promotion_targets",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion.")),
    request_body = TargetsRequest,
    responses(
        (status = 200, description = "The promotion.", body = Data<PromotionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.write` or the unit is outside your scope.", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
        (status = 409, description = "The promotion was turned on; code `promotions.locked_after_start`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn put_targets(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(body): ValidJson<TargetsRequest>,
) -> Result<Data<PromotionResponse>, Error> {
    principal.require(&WRITE)?;
    let view = service.put_targets(&tenant, &principal, id, body.targets).await?;
    Ok(Data { data: PromotionResponse::from(&view) })
}

/// The whole set of flash-sale (or add-on deal) prices of a promotion.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsFlashItemsRequest)]
pub(crate) struct FlashItemsRequest {
    /// Up to 1000.
    pub items: Vec<FlashItemInput>,
}

/// Replace the flash-sale prices of a draft promotion.
///
/// Requires `promotions.write`. For `flash_sale` the variants on sale; for `add_on` the deal
/// items. Units sold are a real count shown to buyers.
#[utoipa::path(
    put,
    path = "/promotions/{id}/flash-items",
    extensions(("x-permissions" = json!(["promotions.write"]))),
    operation_id = "put_promotion_flash_items",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion.")),
    request_body = FlashItemsRequest,
    responses(
        (status = 200, description = "The promotion.", body = Data<PromotionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.write` or the unit is outside your scope.", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
        (status = 409, description = "The promotion was turned on; code `promotions.locked_after_start`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn put_flash_items(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(body): ValidJson<FlashItemsRequest>,
) -> Result<Data<PromotionResponse>, Error> {
    principal.require(&WRITE)?;
    let view = service.put_flash_items(&tenant, &principal, id, body.items).await?;
    Ok(Data { data: PromotionResponse::from(&view) })
}

/// Results of the checks made before turning a promotion on.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PromotionsActivationChecks)]
pub(crate) struct ChecksResponse {
    /// Every check; errors (`severity = error`, `ok = false`) block turning on.
    pub checks: Vec<Check>,
}

/// Check a promotion before turning it on, without changing anything.
///
/// Requires `promotions.publish`. Checks the legal discount limit in force (law data with
/// effective dates), the discount days in the year, the notices to each authority, the targets,
/// the price floors of the catalog and conflicts (warnings).
#[utoipa::path(
    post,
    path = "/promotions/{id}/activation-check",
    extensions(("x-permissions" = json!(["promotions.publish"]))),
    operation_id = "check_promotion_activation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion.")),
    responses(
        (status = 200, description = "The checks.", body = Data<ChecksResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.publish`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
    )
)]
async fn activation_check(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ChecksResponse>, Error> {
    principal.require(&PUBLISH)?;
    let checks = service.activation_check(&tenant, &principal, id).await?;
    Ok(Data { data: ChecksResponse { checks } })
}

/// A promotion just turned on, with the warnings of its checks.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PromotionsActivated)]
pub(crate) struct ActivatedResponse {
    pub promotion: PromotionResponse,
    /// Checks that did not pass but do not block (e.g. conflicts).
    pub warnings: Vec<Check>,
}

/// Turn a draft promotion on.
///
/// Requires `promotions.publish`. Runs the checks of `activation-check`: failing ones answer
/// with their codes in `errors` and change nothing. The promotion becomes `scheduled` (start in
/// the future) or `active`, and counts towards the plan limit `promotions.active_promotions`.
#[utoipa::path(
    post,
    path = "/promotions/{id}/activate",
    extensions(("x-permissions" = json!(["promotions.publish"]))),
    operation_id = "activate_promotion",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion.")),
    responses(
        (status = 200, description = "Turned on.", body = Data<ActivatedResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.publish` (`permission_denied`) or the plan limit is reached (`limit_exceeded`).", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
        (status = 409, description = "Not a draft (`promotions.invalid_transition`), no law data in force (`promotions.no_legal_data`), or a notice is not recorded (`promotions.notice_required`).", body = Problem),
        (status = 422, description = "A check failed: `promotions.exceeds_legal_limit`, `promotions.exceeds_duration_limit` or `promotions.activation_failed`; every failed check is in `errors`.", body = Problem),
    )
)]
async fn activate(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ActivatedResponse>, Error> {
    principal.require(&PUBLISH)?;
    let activated = service.activate(&tenant, &principal, id).await?;
    Ok(Data {
        data: ActivatedResponse {
            promotion: PromotionResponse::from(&activated.view),
            warnings: activated.warnings,
        },
    })
}

macro_rules! transition_handler {
    ($name:ident, $path:literal, $operation:literal, $transition:expr, $summary:literal, $detail:literal) => {
        #[doc = $summary]
        #[doc = ""]
        #[doc = $detail]
        #[utoipa::path(
            post,
            path = $path,
            extensions(("x-permissions" = json!(["promotions.publish"]))),
            operation_id = $operation,
            security(("bearer" = [])),
            params(("id" = Uuid, Path, description = "Id of the promotion.")),
            responses(
                (status = 200, description = "The promotion in its new state.", body = Data<PromotionResponse>),
                (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
                (status = 403, description = "Missing `promotions.publish` or the unit is outside your scope.", body = Problem),
                (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
                (status = 409, description = "Not allowed from the current state; code `promotions.invalid_transition`.", body = Problem),
            )
        )]
        async fn $name(
            State(service): State<Arc<PromotionsService>>,
            tenant: TenantContext,
            principal: Principal,
            Path(id): Path<Uuid>,
        ) -> Result<Data<PromotionResponse>, Error> {
            principal.require(&PUBLISH)?;
            let view = service.transition(&tenant, &principal, id, $transition).await?;
            Ok(Data { data: PromotionResponse::from(&view) })
        }
    };
}

transition_handler!(
    pause,
    "/promotions/{id}/pause",
    "pause_promotion",
    Transition::Pause,
    "Pause a running or scheduled promotion.",
    "Requires `promotions.publish`. Carts stop getting it until it is resumed."
);
transition_handler!(
    resume,
    "/promotions/{id}/resume",
    "resume_promotion",
    Transition::Resume,
    "Resume a paused promotion.",
    "Requires `promotions.publish`. It becomes `active` (or `scheduled`, or `ended` when its end has passed)."
);
transition_handler!(
    end,
    "/promotions/{id}/end",
    "end_promotion",
    Transition::End,
    "End a promotion now.",
    "Requires `promotions.publish`. The announced end becomes now; usage held for orders stays valid."
);
transition_handler!(
    cancel,
    "/promotions/{id}/cancel",
    "cancel_promotion",
    Transition::Cancel,
    "Cancel a promotion.",
    "Requires `promotions.publish`. Works from any state that has not ended."
);

/// List the codes of a promotion.
///
/// Requires `promotions.codes.manage` covering the promotion's unit.
#[utoipa::path(
    get,
    path = "/promotions/{id}/codes",
    extensions(("x-permissions" = json!(["promotions.codes.manage"]))),
    operation_id = "list_promotion_codes",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion."), PageQuery),
    responses(
        (status = 200, description = "One page of codes.", body = Page<CodeDto>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.codes.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
    )
)]
async fn list_codes(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Page<CodeDto>, Error> {
    principal.require(&CODES_MANAGE)?;
    let page = PageRequest::try_from(page)?;
    Ok(service.codes(&tenant, &principal, id, &page).await?.map(|row| CodeDto::from(&row)))
}

/// Add codes to a promotion: one code you choose, or a batch of random codes.
///
/// Requires `promotions.codes.manage`. `redeem` codes (coupons anyone holding them can use)
/// belong to `code` promotions; `claim` codes (put a voucher in the wallet) to `voucher`
/// promotions. Batches over 500 codes are generated in the background.
#[utoipa::path(
    post,
    path = "/promotions/{id}/codes",
    extensions(("x-permissions" = json!(["promotions.codes.manage"]))),
    operation_id = "add_promotion_codes",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion.")),
    request_body = NewCodes,
    responses(
        (status = 201, description = "Codes added (or queued).", body = Data<CodesAdded>, headers(("Location" = String, description = "Path of the code list."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.codes.manage` or the unit is outside your scope.", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
        (status = 409, description = "The code exists already; code `promotions.code_taken`.", body = Problem),
        (status = 422, description = "A value is invalid, or the purpose does not match the promotion; see `errors`.", body = Problem),
    )
)]
async fn add_codes(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(body): ValidJson<NewCodes>,
) -> Result<Created<CodesAdded>, Error> {
    principal.require(&CODES_MANAGE)?;
    let added = match service.generate_codes(&tenant, &principal, id, &body).await? {
        Generated::Created(rows) => {
            CodesAdded { codes: rows.iter().map(CodeDto::from).collect(), queued: 0 }
        }
        Generated::Queued { count } => CodesAdded { codes: Vec::new(), queued: count },
    };
    Ok(Created::new(format!("/promotions/promotions/{id}/codes"), added))
}

/// Download every code of a promotion as CSV.
///
/// Requires `promotions.codes.manage`. Cells are protected against CSV injection.
#[utoipa::path(
    get,
    path = "/promotions/{id}/codes/export",
    extensions(("x-permissions" = json!(["promotions.codes.manage"]))),
    operation_id = "export_promotion_codes",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion.")),
    responses(
        (status = 200, description = "The CSV file.", body = String, content_type = "text/csv"),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.codes.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
    )
)]
async fn export_codes(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Response, Error> {
    principal.require(&CODES_MANAGE)?;
    let csv = service.export_codes(&tenant, &principal, id).await?;
    Ok(csv_response(csv, &format!("promotion-{id}-codes.csv")))
}

pub(crate) fn csv_response(body: String, name: &str) -> Response {
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

/// A voucher given by hand.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsGrantVoucherRequest)]
pub(crate) struct GrantRequest {
    /// User whose wallet gets the voucher.
    pub user_id: Uuid,
    /// Why (kept in the audit log), 3 to 500 characters.
    pub reason: String,
    /// Your reference; sending the same one again returns the same voucher. Defaults to a new
    /// value each time.
    pub reference: Option<String>,
}

/// Put a voucher of a `voucher` promotion into a user's wallet by hand.
///
/// Requires `promotions.vouchers.grant`. A reason is required and kept in the audit log. Sending
/// the same `reference` again returns the voucher already given (200).
#[utoipa::path(
    post,
    path = "/promotions/{id}/vouchers",
    extensions(("x-permissions" = json!(["promotions.vouchers.grant"]))),
    operation_id = "grant_promotion_voucher",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion.")),
    request_body = GrantRequest,
    responses(
        (status = 201, description = "Voucher given.", body = Data<Voucher>, headers(("Location" = String, description = "Path of the promotion."))),
        (status = 200, description = "Given before with the same reference.", body = Data<Voucher>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.vouchers.grant` or the unit is outside your scope.", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
        (status = 409, description = "No vouchers left (`promotions.voucher_quota_exhausted`) or the promotion is not a running `voucher` promotion (`promotions.invalid_transition`).", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn grant_voucher(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(body): ValidJson<GrantRequest>,
) -> Result<Response, Error> {
    principal.require(&VOUCHERS_GRANT)?;
    let granted = service
        .grant_by_hand(&tenant, &principal, id, &body.reason, body.user_id, body.reference)
        .await?;
    Ok(match granted {
        autoxide_kernel::reference::Submitted::Created(voucher) => {
            Created::new(format!("/promotions/promotions/{id}"), voucher).into_response()
        }
        autoxide_kernel::reference::Submitted::Replayed(voucher) => {
            Data { data: voucher }.into_response()
        }
    })
}

/// Get the audit trail of a promotion: one entry per change after it was turned on.
///
/// Requires `promotions.read`. By version number, oldest first unless `order=desc`; paged with
/// `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/promotions/{id}/versions",
    extensions(("x-permissions" = json!(["promotions.read"]))),
    operation_id = "list_promotion_versions",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion."), NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the versions.", body = Page<VersionDto>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn versions(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<VersionDto>, Error> {
    principal.require(&READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.versions(&tenant, &principal, id, &page).await?;
    Ok(rows.map(|row| VersionDto::from(&row)))
}

/// A promotion notice recorded for one authority.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsNoticeRequest)]
pub(crate) struct NoticeRequest {
    /// `submitted` or `not_required`.
    pub status: String,
    /// Receipt number of the authority, up to 100 characters.
    pub reference: Option<String>,
    pub submitted_at: Option<Timestamp>,
}

/// A notice of a promotion to one authority.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PromotionsNotice)]
pub(crate) struct NoticeResponse {
    pub authority_code: String,
    pub status: String,
    pub reference: Option<String>,
    pub submitted_at: Option<Timestamp>,
    pub submitted_by: Option<Uuid>,
}

/// Record that a promotion notice was submitted to an authority.
///
/// Requires `promotions.compliance`. The authority code is the one shown by `activation-check`
/// (e.g. `province:79` for the Department of Industry and Trade of that province). People
/// submit the notice; this only records it.
#[utoipa::path(
    patch,
    path = "/promotions/{id}/notices/{authority_code}",
    extensions(("x-permissions" = json!(["promotions.compliance"]))),
    operation_id = "record_promotion_notice",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion."), ("authority_code" = String, Path, description = "Code of the authority.")),
    request_body = NoticeRequest,
    responses(
        (status = 200, description = "Recorded.", body = Data<NoticeResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.compliance` or the unit is outside your scope.", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn record_notice(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, authority)): Path<(Uuid, String)>,
    ValidJson(body): ValidJson<NoticeRequest>,
) -> Result<Data<NoticeResponse>, Error> {
    principal.require(&COMPLIANCE)?;
    let row = service
        .submit_notice(
            &tenant,
            &principal,
            id,
            &authority,
            &body.status,
            body.reference,
            body.submitted_at,
        )
        .await?;
    Ok(Data {
        data: NoticeResponse {
            authority_code: row.authority_code,
            status: row.status,
            reference: row.reference,
            submitted_at: row.submitted_at,
            submitted_by: row.submitted_by,
        },
    })
}

impl Validate for Patch {
    fn validate(&self) -> Result<(), validator::ValidationErrors> {
        Ok(())
    }
}
