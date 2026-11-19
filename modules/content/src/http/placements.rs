//! Endpoint quản lý vị trí hiển thị banner: `/api/v1/content/placements` (bước 55, D-236). Đọc cần
//! `content.manage` hay `content.banners`; ghi cần `content.manage`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::banners::placements::{Placement, PlacementInput};
use crate::banners::strings;
use crate::service::ContentService;

/// A picture ratio, width to height (e.g. 3:1).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ratio {
    #[schema(example = 3)]
    w: i32,
    #[schema(example = 1)]
    h: i32,
}

/// A display placement: where banners go on the website and how the website draws them.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct PlacementResponse {
    id: Uuid,
    #[schema(example = "home_hero")]
    key: String,
    name: String,
    description: String,
    /// Most banners shown at once (per organization unit and language).
    max_items: i32,
    /// `site` (the website of the tenant).
    audience: String,
    /// How to draw it: `single`, `carousel`, `grid`, `stack`, `strip` (a thin text bar) or
    /// `popup`. See `docs/CONTENT.md` for what each one must do (e.g. a popup always has a close
    /// button and never shows on cart, checkout, sign-in or account pages).
    display: String,
    /// Settings of the display: `carousel` — `autoplay_seconds` (0 = off, else at least 4);
    /// `grid` — `columns` (2–4); `popup` — `frequency` (`once_per_session`, `once_per_day`,
    /// `once_per_week`) and `delay_seconds` (0–60). Other displays have none.
    #[schema(value_type = Object)]
    display_settings: Value,
    /// Banners it takes: `image`, `text` or `any`.
    accepts: String,
    /// Required picture ratio (2% leeway), when set.
    ratio: Option<Ratio>,
    /// Required ratio of the phone picture; `null`: same as `ratio`.
    mobile_ratio: Option<Ratio>,
    /// Smallest picture accepted, in pixels.
    min_width: Option<i32>,
    min_height: Option<i32>,
    /// `all`, `desktop` or `mobile`: call the placement only on these devices.
    devices: String,
    /// Kinds of pages it is used on: `home`, `category`, `collection`, `product`, `search`,
    /// `cart`, `content_entry`, `content_type`, `landing`, or `any`.
    page_kinds: Vec<String>,
    /// Key in the standard set of placements, when the row came from it.
    preset_key: Option<String>,
    /// Off: public reads leave it out.
    active: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl Ratio {
    pub(crate) fn new(w: i32, h: i32) -> Self {
        Ratio { w, h }
    }
}

pub(crate) fn placement_response(row: Placement) -> PlacementResponse {
    let ratio = |w: Option<i32>, h: Option<i32>| Some(Ratio { w: w?, h: h? });
    PlacementResponse {
        id: row.id,
        ratio: ratio(row.ratio_w, row.ratio_h),
        mobile_ratio: ratio(row.mobile_ratio_w, row.mobile_ratio_h),
        page_kinds: strings(&row.page_kinds),
        display_settings: serde_json::from_str(&row.display_settings)
            .unwrap_or_else(|_| Value::Object(Map::new())),
        key: row.key,
        name: row.name,
        description: row.description,
        max_items: row.max_items,
        audience: row.audience,
        display: row.display,
        accepts: row.accepts,
        min_width: row.min_width,
        min_height: row.min_height,
        devices: row.devices,
        preset_key: row.preset_key,
        active: row.active,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// The fields of a placement. `PUT` sends the whole placement; the key never changes.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PlacementFields {
    /// 1 to 120 characters.
    name: String,
    /// At most 500 characters.
    #[serde(default)]
    description: String,
    /// 1 to 50; omitted: 10.
    max_items: Option<i32>,
    /// `single`, `carousel`, `grid`, `stack`, `strip` or `popup`.
    display: String,
    /// See the placement schema; omitted: the defaults of the display.
    #[serde(default)]
    #[schema(value_type = Object)]
    display_settings: Map<String, Value>,
    /// `image`, `text` or `any`; omitted: `text` for a `strip`, else `image`.
    accepts: Option<String>,
    /// Picture ratio, each side 1 to 100.
    ratio: Option<Ratio>,
    mobile_ratio: Option<Ratio>,
    /// 1 to 10000 pixels.
    min_width: Option<i32>,
    min_height: Option<i32>,
    /// `all` (default), `desktop` or `mobile`.
    devices: Option<String>,
    /// Omitted: `["any"]`.
    #[serde(default)]
    page_kinds: Vec<String>,
    /// Omitted: `true`.
    active: Option<bool>,
}

impl PlacementFields {
    fn into_input(self, key: String) -> PlacementInput {
        PlacementInput {
            key,
            name: self.name,
            description: self.description,
            max_items: self.max_items,
            display: self.display,
            display_settings: self.display_settings,
            accepts: self.accepts,
            ratio: self.ratio.map(|ratio| (ratio.w, ratio.h)),
            mobile_ratio: self.mobile_ratio.map(|ratio| (ratio.w, ratio.h)),
            min_width: self.min_width,
            min_height: self.min_height,
            devices: self.devices,
            page_kinds: self.page_kinds,
            active: self.active,
        }
    }
}

/// A new placement.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct CreatePlacementRequest {
    /// 1 to 64 lower-case letters, digits or `_`, e.g. `home_hero`.
    key: String,
    #[serde(flatten)]
    #[validate(nested)]
    fields: PlacementFields,
}

/// List display placements.
///
/// Requires `content.manage` or `content.banners`. Every placement of the tenant, by key.
#[utoipa::path(
    get,
    path = "/placements",
    extensions(("x-permissions" = json!(["content.banners"]))),
    operation_id = "list_content_placements",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The placements.", body = Data<Vec<PlacementResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.banners` or `content.manage`; code `permission_denied`.", body = Problem),
    )
)]
pub(crate) async fn list_placements(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<PlacementResponse>>, Error> {
    let rows = service.list_placements(&tenant, &principal).await?;
    Ok(Data { data: rows.into_iter().map(placement_response).collect() })
}

/// Create a display placement.
///
/// Requires `content.manage`. Retry safely with an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/placements",
    extensions(("x-permissions" = json!(["content.manage"]))),
    operation_id = "create_content_placement",
    security(("bearer" = [])),
    request_body = CreatePlacementRequest,
    responses(
        (status = 201, description = "Created.", body = Data<PlacementResponse>, headers(("ETag" = String, description = "Version of the placement."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "A placement with this key exists; code `content.placement_exists`.", body = Problem),
        (status = 422, description = "A value is invalid (display settings alone: code `content.placement_settings_invalid`); see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_placement(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreatePlacementRequest>,
) -> Result<Created<PlacementResponse>, Error> {
    let row =
        service.create_placement(&tenant, &principal, input.fields.into_input(input.key)).await?;
    let version = row.updated_at;
    Ok(Created::new(format!("/content/placements/{}", row.key), placement_response(row))
        .tagged(version))
}

/// Add the standard placements.
///
/// Requires `content.manage`. Adds the placements of the standard set (top strip, home hero,
/// home middle, top of category, left and right columns, within content, above footer, popup)
/// whose key the tenant does not have yet; new tenants get them already. Running it again adds
/// nothing.
#[utoipa::path(
    post,
    path = "/placements/presets",
    extensions(("x-permissions" = json!(["content.manage"]))),
    operation_id = "install_content_placement_presets",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The placements just added (empty when none was missing).", body = Data<Vec<PlacementResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.manage`; code `permission_denied`.", body = Problem),
    )
)]
pub(crate) async fn install_presets(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<PlacementResponse>>, Error> {
    let rows = service.install_presets(&tenant, &principal).await?;
    Ok(Data { data: rows.into_iter().map(placement_response).collect() })
}

/// Get a display placement.
///
/// Requires `content.manage` or `content.banners`.
#[utoipa::path(
    get,
    path = "/placements/{key}",
    extensions(("x-permissions" = json!(["content.banners"]))),
    operation_id = "get_content_placement",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "Key of the placement, e.g. `home_hero`.")),
    responses(
        (status = 200, description = "The placement.", body = Data<PlacementResponse>, headers(("ETag" = String, description = "Version of the placement: send it in `If-Match` to replace it or to order its banners."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.banners` or `content.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No placement has this key; code `content.placement_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_placement(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(key): Path<String>,
) -> Result<Tagged<PlacementResponse>, Error> {
    let row = service.get_placement(&tenant, &principal, &key).await?;
    Ok(Tagged { version: row.updated_at, data: placement_response(row) })
}

/// Replace a display placement.
///
/// Requires `content.manage`. Sends the whole placement; the key stays. `If-Match` is required.
/// Banners already saved keep their pictures even when the new size rules would refuse them;
/// the rules apply to the next save of each banner.
#[utoipa::path(
    put,
    path = "/placements/{key}",
    extensions(("x-permissions" = json!(["content.manage"]))),
    operation_id = "put_content_placement",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "Key of the placement."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    request_body = PlacementFields,
    responses(
        (status = 200, description = "The placement as saved.", body = Data<PlacementResponse>, headers(("ETag" = String, description = "New version of the placement."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No placement has this key; code `content.placement_not_found`.", body = Problem),
        (status = 412, description = "The placement changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn put_placement(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(key): Path<String>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<PlacementFields>,
) -> Result<Tagged<PlacementResponse>, Error> {
    let expected = if_match.required()?;
    let row = service
        .replace_placement(&tenant, &principal, &key, input.into_input(key.clone()), expected)
        .await?;
    Ok(Tagged { version: row.updated_at, data: placement_response(row) })
}

/// Delete a display placement.
///
/// Requires `content.manage`. Refused while it has banners (archived ones too).
#[utoipa::path(
    delete,
    path = "/placements/{key}",
    extensions(("x-permissions" = json!(["content.manage"]))),
    operation_id = "delete_content_placement",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "Key of the placement.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No placement has this key; code `content.placement_not_found`.", body = Problem),
        (status = 409, description = "The placement still has banners; code `content.placement_in_use`.", body = Problem),
    )
)]
pub(crate) async fn delete_placement(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(key): Path<String>,
) -> Result<StatusCode, Error> {
    service.delete_placement(&tenant, &principal, &key).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The new order of the banners of a placement.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct OrderRequest {
    /// Banners of the placement, first shown first. Banners left out keep their order after
    /// them.
    #[validate(length(max = 200))]
    banner_ids: Vec<Uuid>,
}

/// Order the banners of a placement.
///
/// Requires `content.banners` for the organization units of the banners listed. Sets the order
/// of the whole placement at once. `If-Match` is required: the placement's `ETag`; two people
/// ordering at the same time get one `412`.
#[utoipa::path(
    put,
    path = "/placements/{key}/order",
    extensions(("x-permissions" = json!(["content.banners"]))),
    operation_id = "order_content_placement",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "Key of the placement."), ("If-Match" = String, Header, description = "Required: the placement's `ETag` (or `*`).")),
    request_body = OrderRequest,
    responses(
        (status = 200, description = "The placement, with its new version.", body = Data<PlacementResponse>, headers(("ETag" = String, description = "New version of the placement."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.banners` for a unit of a listed banner; code `permission_denied` or `org_out_of_scope`.", body = Problem),
        (status = 404, description = "No placement has this key; code `content.placement_not_found`.", body = Problem),
        (status = 412, description = "The placement changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "An id is not a banner of this placement, or is listed twice; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn order_placement(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(key): Path<String>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<OrderRequest>,
) -> Result<Tagged<PlacementResponse>, Error> {
    let expected = if_match.required()?;
    let row =
        service.order_placement(&tenant, &principal, &key, input.banner_ids, expected).await?;
    Ok(Tagged { version: row.updated_at, data: placement_response(row) })
}
