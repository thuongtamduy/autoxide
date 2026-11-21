//! Schema bộ lọc và view đã lưu (bước 51 P7, D-201).
//!
//! - **Schema bộ lọc.** Mỗi endpoint danh sách (`GET` có tham số `cursor`) được gắn trong OpenAPI
//!   `x-autoxide-list` (id danh sách, vd `content.list_content_entries`) và `x-autoxide-filters` (tên, kiểu,
//!   giá trị cho phép của từng bộ lọc) — **sinh từ chính tham số `IntoParams`** của endpoint lúc
//!   dựng route, nên không viết hai lần và không lệch được. Giao diện và agent dựng chip lọc từ đó.
//! - **View đã lưu.** `GET/POST /api/v1/me/views`, `PATCH/DELETE /api/v1/me/views/{id}` (bảng
//!   `kernel_saved_views`, cấp khách hàng): `params` kiểm theo schema trên khi lưu — sai thì 422 nêu
//!   từng tham số. Phạm vi `personal` (của người lưu), `node` (một nút, kèm `org_id`) hay `tenant`;
//!   hai phạm vi chung cần `kernel.views.share`.

use crate::time::{Date, Timestamp};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Cond, Expr, Order, Query as SqlQuery};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::openapi::OpenApi;
use utoipa::openapi::path::ParameterIn;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::access::{OrgTarget, Permission, Principal};
use crate::error::{Error, FieldError};
use crate::http::ValidJson;
use crate::migration::prelude::*;
use crate::response::{Created, Data, Problem};
use crate::tenancy::{OrgId, TenantContext};

const TABLE: &str = "kernel_saved_views";

/// Lưu view dùng chung cho một nút hay cả khách.
pub const VIEWS_SHARE: Permission = Permission::new(
    "kernel.views.share",
    "Save list views shared with an organization unit or the whole tenant.",
);

/// Tham số phân trang: không phải bộ lọc, không lưu trong view.
const PAGING: [&str; 2] = ["limit", "cursor"];
/// Tham số được lưu trong view nhưng không phải bộ lọc (thứ tự, phần đính kèm).
const NOT_FILTERS: [&str; 3] = ["order", "sort", "include"];
/// Mỗi người tối đa chừng này view.
const MAX_PER_USER: u64 = 200;

/// One parameter of a list, as a saved view or a filter chip may use it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub(crate) struct ListParameter {
    #[schema(example = "status")]
    pub name: String,
    /// `string`, `integer`, `number`, `boolean`, `uuid`, `date-time` or `date`.
    #[schema(example = "string")]
    pub kind: String,
    /// The allowed values, when the parameter takes one of a fixed list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
    /// `eq` (the value must match) or `contains` (free text, `q`).
    #[schema(example = "eq")]
    pub operator: String,
    /// What it does, in English.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Danh sách đã biết: id → tham số (gồm cả `order`, `sort`). Clone rẻ.
#[derive(Clone, Default)]
pub(crate) struct ListSchemas(Arc<HashMap<String, Vec<ListParameter>>>);

impl ListSchemas {
    fn get(&self, resource: &str) -> Option<&Vec<ListParameter>> {
        self.0.get(resource)
    }

    /// Tham số của danh sách `resource` (gồm `order`, `sort`).
    pub(crate) fn parameters(&self, resource: &str) -> Option<&[ListParameter]> {
        self.0.get(resource).map(Vec::as_slice)
    }
}

/// Gắn `x-autoxide-list` / `x-autoxide-filters` vào mọi endpoint danh sách của `module` và trả
/// tham số của từng danh sách (id `<module>.<operation id>`).
pub(crate) fn annotate(module: &str, openapi: &mut OpenApi) -> Vec<(String, Vec<ListParameter>)> {
    // Đọc schema qua JSON: kiểu của tham số có thể nằm trực tiếp, trong `oneOf` với `null`, hay
    // là `$ref` tới `components` — JSON gom mọi trường hợp về một cách đọc.
    let components = serde_json::to_value(&openapi.components)
        .ok()
        .and_then(|components| components.get("schemas").cloned())
        .unwrap_or(Value::Null);
    let mut found = Vec::new();
    for item in openapi.paths.paths.values_mut() {
        let Some(operation) = item.get.as_mut() else { continue };
        let parameters = operation.parameters.clone().unwrap_or_default();
        let query: Vec<_> = parameters
            .iter()
            .filter_map(|parameter| match parameter {
                utoipa::openapi::RefOr::T(parameter) => Some(parameter),
                utoipa::openapi::RefOr::Ref(_) => None,
            })
            .filter(|parameter| matches!(parameter.parameter_in, ParameterIn::Query))
            .collect();
        if !query.iter().any(|parameter| parameter.name == "cursor") {
            continue;
        }
        let Some(id) = operation.operation_id.clone() else { continue };
        let resource = format!("{module}.{id}");
        let mut all = Vec::new();
        for parameter in query {
            if PAGING.contains(&parameter.name.as_str()) {
                continue;
            }
            let schema = parameter
                .schema
                .as_ref()
                .and_then(|schema| serde_json::to_value(schema).ok())
                .unwrap_or(Value::Null);
            let (kind, values) = describe(&schema, &components);
            all.push(ListParameter {
                operator: if parameter.name == "q" { "contains" } else { "eq" }.to_owned(),
                name: parameter.name.clone(),
                kind,
                values,
                description: parameter.description.clone(),
            });
        }
        let filters: Vec<&ListParameter> = all
            .iter()
            .filter(|parameter| !NOT_FILTERS.contains(&parameter.name.as_str()))
            .collect();
        let extensions = operation.extensions.get_or_insert_with(Default::default);
        extensions.insert("x-autoxide-list".to_owned(), Value::String(resource.clone()));
        extensions.insert(
            "x-autoxide-filters".to_owned(),
            serde_json::to_value(&filters).unwrap_or(Value::Null),
        );
        found.push((resource, all));
    }
    found
}

/// Gom danh sách của mọi module thành sổ.
pub(crate) fn schemas(lists: Vec<(String, Vec<ListParameter>)>) -> ListSchemas {
    ListSchemas(Arc::new(lists.into_iter().collect()))
}

/// Kiểu và giá trị cho phép của một schema JSON (đã đổi `$ref`, bỏ `null`).
fn describe(schema: &Value, components: &Value) -> (String, Option<Vec<String>>) {
    let resolved = resolve(schema, components);
    let variants: Vec<&Value> = match resolved.get("oneOf").or_else(|| resolved.get("anyOf")) {
        Some(Value::Array(items)) => items.iter().collect(),
        _ => vec![resolved],
    };
    let mut kind = None;
    let mut values = Vec::new();
    for variant in variants {
        let variant = resolve(variant, components);
        let types: Vec<&str> = match variant.get("type") {
            Some(Value::String(one)) => vec![one.as_str()],
            Some(Value::Array(many)) => many.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if types == ["null"] {
            continue;
        }
        if let Some(Value::Array(allowed)) = variant.get("enum") {
            values.extend(allowed.iter().filter_map(Value::as_str).map(str::to_owned));
        }
        let format = variant.get("format").and_then(Value::as_str);
        let base = types.into_iter().find(|kind| *kind != "null").unwrap_or("string");
        kind.get_or_insert(match (base, format) {
            ("string", Some(format @ ("uuid" | "date-time" | "date"))) => format.to_owned(),
            (base, _) => base.to_owned(),
        });
    }
    (kind.unwrap_or_else(|| "string".to_owned()), (!values.is_empty()).then_some(values))
}

fn resolve<'a>(schema: &'a Value, components: &'a Value) -> &'a Value {
    match schema.get("$ref").and_then(Value::as_str) {
        Some(reference) => {
            reference.rsplit('/').next().and_then(|name| components.get(name)).unwrap_or(schema)
        }
        None => schema,
    }
}

/// Kiểm `params` của view theo tham số của danh sách; lỗi ghi vào `problems` với tên
/// `params.<tham số>`.
pub(crate) fn check_params(
    parameters: &[ListParameter],
    params: &serde_json::Map<String, Value>,
    problems: &mut Vec<FieldError>,
) {
    for (name, value) in params {
        let field = format!("params.{name}");
        let Some(parameter) = parameters.iter().find(|parameter| &parameter.name == name) else {
            problems.push(FieldError::new(field, "unknown", "is not a parameter of this list"));
            continue;
        };
        let text = match value {
            Value::String(text) => text.clone(),
            Value::Bool(flag) => flag.to_string(),
            Value::Number(number) => number.to_string(),
            _ => {
                problems.push(FieldError::new(
                    field,
                    "invalid",
                    "must be a string, number or boolean",
                ));
                continue;
            }
        };
        let fits = match parameter.kind.as_str() {
            _ if text.chars().count() > 500 => false,
            "integer" => text.parse::<i64>().is_ok(),
            "number" => text.parse::<f64>().is_ok(),
            "boolean" => text == "true" || text == "false",
            "uuid" => text.parse::<Uuid>().is_ok(),
            "date-time" => Timestamp::parse_rfc3339(&text).is_ok(),
            "date" => Date::parse_from_str(&text, "%Y-%m-%d").is_ok(),
            _ => parameter.values.as_ref().is_none_or(|allowed| allowed.contains(&text)),
        };
        if !fits {
            problems.push(
                FieldError::new(field, "invalid", "is not a valid `{kind}` value for this list")
                    .with_param("kind", parameter.kind.clone()),
            );
        }
    }
}

/// A saved list view.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SavedView {
    pub id: Uuid,
    /// The list, as `x-autoxide-list` of its endpoint in this document, e.g.
    /// `content.list_content_entries`.
    pub resource: String,
    pub name: String,
    /// `personal` (only its owner sees it), `node` (everyone at `org_id`) or `tenant`.
    #[schema(example = "personal")]
    pub scope: String,
    pub org_id: Option<Uuid>,
    /// Who saved it.
    pub owner: Uuid,
    /// The query parameters of the list (filters, `sort`, `order`).
    #[schema(value_type = Object)]
    pub params: serde_json::Map<String, Value>,
    /// The columns shown, in order.
    pub columns: Vec<String>,
    pub group_by: Option<String>,
    /// `compact` or `comfortable`.
    pub density: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A view to save.
#[derive(Debug, Clone, Deserialize, ToSchema, validator::Validate)]
pub(crate) struct NewView {
    /// The list, as `x-autoxide-list` of its endpoint.
    pub resource: String,
    /// 1 to 100 characters.
    pub name: String,
    /// `personal` (the default), `node` (needs `org_id`) or `tenant`; the last two need
    /// `kernel.views.share`.
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub org_id: Option<Uuid>,
    /// Query parameters of the list; each is checked against `x-autoxide-filters` and the
    /// parameters of the endpoint.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub params: serde_json::Map<String, Value>,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub group_by: Option<String>,
    #[serde(default)]
    pub density: Option<String>,
}

/// Changes to a saved view; omitted fields stay as they are.
#[derive(Debug, Clone, Default, Deserialize, ToSchema, validator::Validate)]
pub(crate) struct ViewChanges {
    pub name: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub params: Option<serde_json::Map<String, Value>>,
    pub columns: Option<Vec<String>>,
    /// `""` clears it.
    pub group_by: Option<String>,
    /// `""` clears it.
    pub density: Option<String>,
}

/// Which views.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ViewsQuery {
    /// Only the views of this list, e.g. `content.list_content_entries`.
    resource: Option<String>,
}

#[derive(Clone)]
pub(crate) struct ViewsState {
    pub lists: ListSchemas,
}

fn now() -> Timestamp {
    Timestamp::now()
}

fn check_shape(
    name: Option<&str>,
    columns: Option<&[String]>,
    group_by: Option<&str>,
    density: Option<&str>,
    problems: &mut Vec<FieldError>,
) {
    if let Some(name) = name
        && !(1..=100).contains(&name.trim().chars().count())
    {
        problems.push(FieldError::new("name", "invalid", "must be 1 to 100 characters long"));
    }
    let token = |text: &str| {
        (1..=64).contains(&text.len())
            && text
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._".contains(&b))
    };
    if let Some(columns) = columns
        && (columns.len() > 50 || !columns.iter().all(|column| token(column)))
    {
        problems.push(FieldError::new(
            "columns",
            "invalid",
            "must be at most 50 names of 1 to 64 lower-case letters, digits, `_` and `.`",
        ));
    }
    if let Some(group) = group_by.filter(|group| !group.is_empty())
        && !token(group)
    {
        problems.push(FieldError::new(
            "group_by",
            "invalid",
            "must be 1 to 64 lower-case letters, digits, `_` and `.`",
        ));
    }
    if let Some(density) = density.filter(|density| !density.is_empty())
        && !["compact", "comfortable"].contains(&density)
    {
        problems.push(FieldError::new("density", "invalid", "must be `compact` or `comfortable`"));
    }
}

const COLUMNS: [&str; 12] = [
    "id",
    "resource",
    "name",
    "scope",
    "org_id",
    "owner_id",
    "params",
    "columns",
    "group_by",
    "density",
    "created_at",
    "updated_at",
];

fn view(row: &sea_orm::QueryResult) -> Result<SavedView, Error> {
    let params: String = row.try_get("", "params")?;
    let columns: String = row.try_get("", "columns")?;
    Ok(SavedView {
        id: row.try_get("", "id")?,
        resource: row.try_get("", "resource")?,
        name: row.try_get("", "name")?,
        scope: row.try_get("", "scope")?,
        org_id: row.try_get("", "org_id")?,
        owner: row.try_get("", "owner_id")?,
        params: serde_json::from_str(&params).unwrap_or_default(),
        columns: serde_json::from_str(&columns).unwrap_or_default(),
        group_by: row.try_get("", "group_by")?,
        density: row.try_get("", "density")?,
        created_at: row.try_get("", "created_at")?,
        updated_at: row.try_get("", "updated_at")?,
    })
}

fn not_found(id: Uuid) -> Error {
    Error::not_found("kernel.view_not_found", "There is no saved view `{id}`.").with_param("id", id)
}

/// View người gọi thấy: của mình, của cả khách, và của các nút (giao diện lọc theo nút đang chọn).
async fn find(tenant: &TenantContext, principal: &Principal, id: Uuid) -> Result<SavedView, Error> {
    let mut select = SqlQuery::select();
    select
        .columns(COLUMNS)
        .from(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("id")).eq(id));
    let found = tenant.database().read().query_one(&select).await?;
    found
        .as_ref()
        .map(view)
        .transpose()?
        .filter(|saved| saved.scope != "personal" || saved.owner == principal.user_id())
        .ok_or_else(|| not_found(id))
}

/// Người gọi được sửa / xoá view: của mình (riêng), hay có `kernel.views.share` ở nút của view chung.
async fn require_manage(
    tenant: &TenantContext,
    principal: &Principal,
    scope: &str,
    org_id: Option<Uuid>,
) -> Result<(), Error> {
    if scope == "personal" {
        return Ok(());
    }
    let target =
        OrgTarget::new(org_id.map_or(crate::tenancy::root_of(tenant.id()), OrgId::from_uuid), None);
    principal.require_at(tenant.database().read(), &VIEWS_SHARE, target).await
}

/// List my saved views.
///
/// Any signed-in user. Your personal views, and the views shared with the whole tenant or with an
/// organization unit, newest first.
#[utoipa::path(
    get,
    path = "/views",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_views",
    security(("bearer" = [])),
    params(ViewsQuery),
    responses(
        (status = 200, description = "The views.", body = Data<Vec<SavedView>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn list_views(
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ViewsQuery>,
) -> Result<Data<Vec<SavedView>>, Error> {
    let mut select = SqlQuery::select();
    select
        .columns(COLUMNS)
        .from(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .cond_where(
            Cond::any()
                .add(Expr::col(Alias::new("scope")).ne("personal"))
                .add(Expr::col(Alias::new("owner_id")).eq(principal.user_id())),
        )
        .order_by(Alias::new("id"), Order::Desc)
        .limit(500);
    if let Some(resource) = query.resource.as_deref().filter(|resource| !resource.is_empty()) {
        select.and_where(Expr::col(Alias::new("resource")).eq(resource));
    }
    let rows = tenant.database().read().query_all(&select).await?;
    Ok(Data { data: rows.iter().map(view).collect::<Result<_, _>>()? })
}

/// Save a list view.
///
/// Any signed-in user for a `personal` view; `kernel.views.share` at `org_id` (or the tenant) for a
/// `node` or `tenant` view. `params` must be parameters of the list, each with a value it accepts
/// (see `x-autoxide-filters` on the list's endpoint); every problem is listed at once.
#[utoipa::path(
    post,
    path = "/views",
    extensions(("x-permissions" = json!([]))),
    operation_id = "create_view",
    security(("bearer" = [])),
    request_body = NewView,
    responses(
        (status = 201, description = "Saved.", body = Data<SavedView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "A shared view without `kernel.views.share`; code `permission_denied`.", body = Problem),
        (status = 409, description = "Too many saved views; code `kernel.view_limit_reached`.", body = Problem),
        (status = 422, description = "Invalid fields or parameters; see `errors`.", body = Problem),
    )
)]
async fn create_view(
    State(state): State<Arc<ViewsState>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<NewView>,
) -> Result<Created<SavedView>, Error> {
    let mut problems = Vec::new();
    let scope = input.scope.clone().unwrap_or_else(|| "personal".to_owned());
    if !["personal", "node", "tenant"].contains(&scope.as_str()) {
        problems.push(FieldError::new(
            "scope",
            "invalid",
            "must be `personal`, `node` or `tenant`",
        ));
    }
    match (scope.as_str(), input.org_id) {
        ("node", None) => problems.push(FieldError::new("org_id", "required", "is required")),
        ("personal" | "tenant", Some(_)) => {
            problems.push(FieldError::new("org_id", "invalid", "is only for a `node` view"))
        }
        _ => {}
    }
    match state.lists.get(&input.resource) {
        Some(parameters) => check_params(parameters, &input.params, &mut problems),
        None => problems.push(FieldError::new(
            "resource",
            "unknown",
            "is not a list of this server; see `x-autoxide-list` in the API document",
        )),
    }
    check_shape(
        Some(&input.name),
        Some(&input.columns),
        input.group_by.as_deref(),
        input.density.as_deref(),
        &mut problems,
    );
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    require_manage(&tenant, &principal, &scope, input.org_id).await?;
    let read = tenant.database().read();
    let count = SqlQuery::select()
        .expr(Expr::col(Alias::new("id")).count())
        .from(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("owner_id")).eq(principal.user_id()))
        .to_owned();
    let owned: i64 = match read.query_one(&count).await? {
        Some(row) => row.try_get_by_index(0)?,
        None => 0,
    };
    if <u64 as TryFrom<i64>>::try_from(owned).unwrap_or(0) >= MAX_PER_USER {
        return Err(Error::conflict(
            "kernel.view_limit_reached",
            "You already have {max} saved views; delete one first.",
        )
        .with_param("max", MAX_PER_USER));
    }
    let id = Uuid::now_v7();
    let now = now();
    let insert = SqlQuery::insert()
        .into_table(Alias::new(TABLE))
        .columns([
            "id",
            "tenant_id",
            "resource",
            "name",
            "scope",
            "org_id",
            "owner_id",
            "params",
            "columns",
            "group_by",
            "density",
            "created_at",
            "updated_at",
        ])
        .values_panic([
            id.into(),
            tenant.id().as_uuid().into(),
            input.resource.clone().into(),
            input.name.trim().to_owned().into(),
            scope.into(),
            input.org_id.into(),
            principal.user_id().into(),
            Value::Object(input.params).to_string().into(),
            serde_json::to_string(&input.columns).map_err(Error::internal)?.into(),
            input.group_by.filter(|group| !group.is_empty()).into(),
            input.density.filter(|density| !density.is_empty()).into(),
            now.into(),
            now.into(),
        ])
        .to_owned();
    tenant.database().write().execute(&insert).await?;
    let saved = find(&tenant, &principal, id).await?;
    Ok(Created::new(format!("/api/v1/me/views/{id}"), saved))
}

/// Change a saved view.
///
/// Your own personal view, or a shared view with `kernel.views.share` at its unit.
#[utoipa::path(
    patch,
    path = "/views/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "update_view",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the view.")),
    request_body = ViewChanges,
    responses(
        (status = 200, description = "The view.", body = Data<SavedView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "A shared view without `kernel.views.share`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such view; code `kernel.view_not_found`.", body = Problem),
        (status = 422, description = "Invalid fields or parameters; see `errors`.", body = Problem),
    )
)]
async fn update_view(
    State(state): State<Arc<ViewsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(changes): ValidJson<ViewChanges>,
) -> Result<Data<SavedView>, Error> {
    let saved = find(&tenant, &principal, id).await?;
    let mut problems = Vec::new();
    if let Some(params) = &changes.params {
        let parameters = state.lists.get(&saved.resource).map(Vec::as_slice).unwrap_or_default();
        check_params(parameters, params, &mut problems);
    }
    check_shape(
        changes.name.as_deref(),
        changes.columns.as_deref(),
        changes.group_by.as_deref(),
        changes.density.as_deref(),
        &mut problems,
    );
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    require_manage(&tenant, &principal, &saved.scope, saved.org_id).await?;
    let mut update = SqlQuery::update();
    update
        .table(Alias::new(TABLE))
        .value(Alias::new("updated_at"), now())
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("id")).eq(id));
    if let Some(name) = changes.name {
        update.value(Alias::new("name"), name.trim().to_owned());
    }
    if let Some(params) = changes.params {
        update.value(Alias::new("params"), Value::Object(params).to_string());
    }
    if let Some(columns) = changes.columns {
        update.value(
            Alias::new("columns"),
            serde_json::to_string(&columns).map_err(Error::internal)?,
        );
    }
    if let Some(group) = changes.group_by {
        update.value(Alias::new("group_by"), (!group.is_empty()).then_some(group));
    }
    if let Some(density) = changes.density {
        update.value(Alias::new("density"), (!density.is_empty()).then_some(density));
    }
    tenant.database().write().execute(&update).await?;
    Ok(Data { data: find(&tenant, &principal, id).await? })
}

/// Delete a saved view.
///
/// Your own personal view, or a shared view with `kernel.views.share` at its unit.
#[utoipa::path(
    delete,
    path = "/views/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "delete_view",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the view.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "A shared view without `kernel.views.share`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such view; code `kernel.view_not_found`.", body = Problem),
    )
)]
async fn delete_view(
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    let saved = find(&tenant, &principal, id).await?;
    require_manage(&tenant, &principal, &saved.scope, saved.org_id).await?;
    let delete = SqlQuery::delete()
        .from_table(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("id")).eq(id))
        .to_owned();
    tenant.database().write().execute(&delete).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Route dưới `/api/v1/me`.
pub(crate) fn routes(state: ViewsState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_views, create_view))
        .routes(routes!(update_view, delete_view))
        .with_state(Arc::new(state))
}

/// Xoá view riêng của người dùng bị xoá (dữ liệu cá nhân); view chung giữ lại.
pub(crate) async fn forget_user<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<(), Error> {
    let delete = SqlQuery::delete()
        .from_table(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("owner_id")).eq(user_id))
        .and_where(Expr::col(Alias::new("scope")).eq("personal"))
        .to_owned();
    connection.execute(&delete).await?;
    Ok(())
}

/// Bản xuất dữ liệu cá nhân: view người dùng đã lưu.
pub(crate) async fn export_user(
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<Vec<BTreeMap<&'static str, Value>>, Error> {
    let select = SqlQuery::select()
        .columns(["resource", "name", "scope", "params", "created_at"])
        .from(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("owner_id")).eq(user_id))
        .order_by(Alias::new("id"), Order::Asc)
        .to_owned();
    let rows = tenant.database().read().query_all(&select).await?;
    rows.iter()
        .map(|row| {
            let params: String = row.try_get("", "params")?;
            let created: Timestamp = row.try_get("", "created_at")?;
            Ok(BTreeMap::from([
                ("resource", Value::String(row.try_get("", "resource")?)),
                ("name", Value::String(row.try_get("", "name")?)),
                ("scope", Value::String(row.try_get("", "scope")?)),
                ("params", serde_json::from_str(&params).unwrap_or(Value::Null)),
                ("created_at", Value::String(created.to_rfc3339())),
            ]))
        })
        .collect()
}

/// Migration tạo bảng `kernel_saved_views` (cấp khách hàng, mọi database dữ liệu).
pub(crate) struct CreateSavedViews;

impl MigrationName for CreateSavedViews {
    fn name(&self) -> &str {
        "m0019_create_saved_views"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateSavedViews {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TABLE))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("resource", 200))
                    .col(schema::string_len("name", 400))
                    .col(schema::string_len("scope", 16))
                    .col(schema::uuid_null("org_id"))
                    .col(schema::uuid("owner_id"))
                    .col(schema::text("params"))
                    .col(schema::text("columns"))
                    .col(schema::string_len_null("group_by", 64))
                    .col(schema::string_len_null("density", 16))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_saved_views_owner_idx")
                    .table(Alias::new(TABLE))
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("owner_id"))
                    .col(Alias::new("id"))
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new(TABLE)).to_owned()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parameter(name: &str, kind: &str, values: Option<&[&str]>) -> ListParameter {
        ListParameter {
            name: name.to_owned(),
            kind: kind.to_owned(),
            values: values.map(|values| values.iter().map(|v| (*v).to_owned()).collect()),
            operator: "eq".to_owned(),
            description: None,
        }
    }

    #[test]
    fn views_params_sai_schema_bi_tu_choi() {
        let parameters = [
            parameter("status", "string", Some(&["draft", "published"])),
            parameter("scheduled", "boolean", None),
            parameter("author_id", "uuid", None),
        ];
        let params = serde_json::json!({
            "status": "gone",
            "scheduled": "yes",
            "author_id": "x",
            "colour": "red",
        });
        let mut problems = Vec::new();
        check_params(&parameters, params.as_object().unwrap(), &mut problems);
        let mut fields: Vec<String> = problems.iter().map(|p| p.field.to_string()).collect();
        fields.sort();
        assert_eq!(
            fields,
            ["params.author_id", "params.colour", "params.scheduled", "params.status"]
        );
        let good = serde_json::json!({"status": "draft", "scheduled": true});
        let mut none = Vec::new();
        check_params(&parameters, good.as_object().unwrap(), &mut none);
        assert!(none.is_empty());
    }

    #[test]
    fn kieu_tham_so_doc_qua_ref_va_null() {
        let components = serde_json::json!({"Kind": {"type": "string", "enum": ["post", "page"]}});
        let (kind, values) = describe(
            &serde_json::json!({"oneOf": [{"type": "null"}, {"$ref": "#/components/schemas/Kind"}]}),
            &components,
        );
        assert_eq!((kind.as_str(), values), ("string", Some(vec!["post".into(), "page".into()])));
        let (kind, _) = describe(
            &serde_json::json!({"type": ["string", "null"], "format": "uuid"}),
            &components,
        );
        assert_eq!(kind, "uuid");
    }
}
