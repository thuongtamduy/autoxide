//! REST and MCP of user-defined custom fields.

use super::*;
use crate::time::Timestamp;

impl CustomFieldsState {
    pub(super) fn require_read(
        &self,
        principal: &Principal,
        target: &str,
    ) -> Result<CustomFieldTarget, Error> {
        let found = self.fields.target(target).ok_or_else(|| {
            Error::validation(vec![FieldError::new(
                "target",
                "unknown",
                "is not a record type that takes custom fields",
            )])
        })?;
        if principal.require(&CUSTOM_FIELDS_MANAGE).is_err() {
            principal.require(&found.read_permission)?;
        }
        Ok(found)
    }
}

/// List custom fields.
///
/// `kernel.custom_fields.manage`, or the permission that reads `target`'s records (without
/// `target`: the fields of every record type you may read). The fields the tenant defined, in order — interfaces and agents build forms and filter chips from them.
#[utoipa::path(
    get,
    path = "/custom-fields",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_custom_fields",
    security(("bearer" = [])),
    params(FieldsQuery),
    responses(
        (status = 200, description = "The fields.", body = Data<Vec<CustomFieldView>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission; code `permission_denied`.", body = Problem),
        (status = 422, description = "`target` takes no custom fields; see `errors`.", body = Problem),
    )
)]
pub(super) async fn list_custom_fields(
    State(state): State<Arc<CustomFieldsState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<FieldsQuery>,
) -> Result<Data<Vec<CustomFieldView>>, Error> {
    if let Some(target) = &query.target {
        state.require_read(&principal, target)?;
    }
    // Không nêu loại: chỉ trường của loại bản ghi người gọi đọc được.
    let fields = list(&tenant, query.target.as_deref())
        .await?
        .into_iter()
        .filter(|field| state.require_read(&principal, &field.target).is_ok())
        .collect();
    Ok(Data { data: fields })
}

/// Define a custom field.
///
/// Requires `kernel.custom_fields.manage`. At most 100 fields per record type.
#[utoipa::path(
    post,
    path = "/custom-fields",
    extensions(("x-permissions" = json!(["kernel.custom_fields.manage"]))),
    operation_id = "create_custom_field",
    security(("bearer" = [])),
    request_body = NewCustomField,
    responses(
        (status = 201, description = "Defined.", body = Data<CustomFieldView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.custom_fields.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "The key is taken (`kernel.custom_field_key_taken`) or the record type has 100 fields (`kernel.custom_field_limit_reached`).", body = Problem),
        (status = 422, description = "Invalid definition; see `errors`.", body = Problem),
    )
)]
pub(super) async fn create_custom_field(
    State(state): State<Arc<CustomFieldsState>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<NewCustomField>,
) -> Result<Created<CustomFieldView>, Error> {
    principal.require(&CUSTOM_FIELDS_MANAGE)?;
    let target = state.require_read(&principal, &input.target)?;
    let mut problems = Vec::new();
    if !is_key(&input.key) {
        problems.push(FieldError::new(
            "key",
            "invalid",
            "must be 1 to 64 lower-case letters, digits and `_`, starting with a letter",
        ));
    }
    let now = Timestamp::now();
    let mut field = CustomFieldView {
        id: Uuid::now_v7(),
        target: input.target.clone(),
        key: input.key.clone(),
        data_type: input.data_type.clone(),
        labels: BTreeMap::new(),
        help: BTreeMap::new(),
        required: false,
        options: Vec::new(),
        scale: None,
        min: None,
        max: None,
        reference_kind: None,
        default: None,
        visibility: "internal".to_owned(),
        filterable: false,
        personal_data: false,
        position: 0,
        active: true,
        created_at: now,
        updated_at: now,
    };
    apply(&mut field, input.settings, target, &mut problems);
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    let existing = list(&tenant, Some(&input.target)).await?;
    if existing.iter().any(|known| known.key == field.key) {
        return Err(Error::conflict(
            "kernel.custom_field_key_taken",
            "Field `{key}` already exists.",
        )
        .with_param("key", field.key));
    }
    if existing.len() >= MAX_PER_TARGET {
        return Err(Error::conflict(
            "kernel.custom_field_limit_reached",
            "A record type has at most {max} custom fields.",
        )
        .with_param("max", MAX_PER_TARGET));
    }
    // Giữ một suất của hạn mức trước khi lưu; lưu hỏng thì trả lại.
    let reference = limit_reference(field.id);
    state.entitlements.reserve(tenant.id(), &CUSTOM_FIELDS_LIMIT, 1, &reference).await?;
    if let Err(error) = save(&tenant, &field, true).await {
        state.entitlements.release(tenant.id(), &CUSTOM_FIELDS_LIMIT, &reference).await?;
        return Err(error);
    }
    state.entitlements.confirm(tenant.id(), &CUSTOM_FIELDS_LIMIT, &reference).await?;
    Ok(Created::new(format!("/api/v1/kernel/custom-fields/{}", field.id), field))
}

pub(super) async fn find(tenant: &TenantContext, id: Uuid) -> Result<CustomFieldView, Error> {
    list(tenant, None).await?.into_iter().find(|field| field.id == id).ok_or_else(|| not_found(id))
}

/// Get a custom field.
///
/// `kernel.custom_fields.manage`, or the permission that reads its record type.
#[utoipa::path(
    get,
    path = "/custom-fields/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_custom_field",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the field.")),
    responses(
        (status = 200, description = "The field.", body = Data<CustomFieldView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such field; code `kernel.custom_field_not_found`.", body = Problem),
    )
)]
pub(super) async fn get_custom_field(
    State(state): State<Arc<CustomFieldsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<CustomFieldView>, Error> {
    let field = find(&tenant, id).await?;
    state.require_read(&principal, &field.target)?;
    Ok(Data { data: field })
}

/// Change a custom field.
///
/// Requires `kernel.custom_fields.manage`. Labels, options, constraints and the rest change; the
/// key never does, and the type only while no record has a value (`kernel.custom_field_type_locked`).
#[utoipa::path(
    patch,
    path = "/custom-fields/{id}",
    extensions(("x-permissions" = json!(["kernel.custom_fields.manage"]))),
    operation_id = "update_custom_field",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the field.")),
    request_body = FieldSettings,
    responses(
        (status = 200, description = "The field.", body = Data<CustomFieldView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.custom_fields.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such field; code `kernel.custom_field_not_found`.", body = Problem),
        (status = 409, description = "Changing the type of a field records already use; code `kernel.custom_field_type_locked`.", body = Problem),
        (status = 422, description = "Invalid settings; see `errors`.", body = Problem),
    )
)]
pub(super) async fn update_custom_field(
    State(state): State<Arc<CustomFieldsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(settings): ValidJson<FieldSettings>,
) -> Result<Data<CustomFieldView>, Error> {
    principal.require(&CUSTOM_FIELDS_MANAGE)?;
    let mut field = find(&tenant, id).await?;
    let target = state.require_read(&principal, &field.target)?;
    if let Some(data_type) = settings.data_type.clone()
        && data_type != field.data_type
    {
        if field.updated_at != field.created_at || field.filterable {
            // Đơn giản và an toàn: trường đã dùng (đã sửa hay đã có index) thì khoá kiểu.
            return Err(Error::conflict(
                "kernel.custom_field_type_locked",
                "Field `{key}` may hold values already: create a new field instead of changing its type.",
            )
            .with_param("key", field.key));
        }
        field.data_type = data_type;
    }
    let mut problems = Vec::new();
    apply(&mut field, settings, target, &mut problems);
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    field.updated_at = Timestamp::now();
    save(&tenant, &field, false).await?;
    Ok(Data { data: find(&tenant, id).await? })
}

/// Turn a custom field off.
///
/// Requires `kernel.custom_fields.manage`. The field stops being shown and accepted; values already
/// on records stay (a field is never deleted with its data).
#[utoipa::path(
    delete,
    path = "/custom-fields/{id}",
    extensions(("x-permissions" = json!(["kernel.custom_fields.manage"]))),
    operation_id = "delete_custom_field",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the field.")),
    responses(
        (status = 204, description = "Turned off."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.custom_fields.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such field; code `kernel.custom_field_not_found`.", body = Problem),
    )
)]
pub(super) async fn delete_custom_field(
    State(state): State<Arc<CustomFieldsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&CUSTOM_FIELDS_MANAGE)?;
    let mut field = find(&tenant, id).await?;
    let was_active = field.active;
    field.active = false;
    field.updated_at = Timestamp::now();
    save(&tenant, &field, false).await?;
    if was_active {
        state.entitlements.release(tenant.id(), &CUSTOM_FIELDS_LIMIT, &limit_reference(id)).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Which custom fields to list.
#[derive(Debug, Default, Deserialize, validator::Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListFieldsInput {
    /// Only the fields of this record type, e.g. `content.entry`.
    target: Option<String>,
}

pub(crate) fn tools(fields: CustomFields) -> Vec<crate::mcp::Tool> {
    vec![
        crate::mcp::Tool::new(
            "list_custom_fields",
            "List the custom fields this tenant added to record types you may read (key, type, \
             labels, options, whether lists filter on it with `cf.<key>=`). Write their values in \
             a record's `custom_fields`.",
            move |tenant: TenantContext, principal: Principal, input: ListFieldsInput| {
                let state = CustomFieldsState {
                    fields: fields.clone(),
                    entitlements: crate::entitlement::EntitlementsHandle::default(),
                };
                async move {
                    if let Some(target) = &input.target {
                        state.require_read(&principal, target)?;
                    }
                    Ok::<Vec<CustomFieldView>, Error>(
                        list(&tenant, input.target.as_deref())
                            .await?
                            .into_iter()
                            .filter(|field| {
                                field.active
                                    && state.require_read(&principal, &field.target).is_ok()
                            })
                            .collect(),
                    )
                }
            },
        )
        .for_any_signed_in()
        .read_only(),
    ]
}

pub(crate) fn routes(state: CustomFieldsState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_custom_fields, create_custom_field))
        .routes(routes!(get_custom_field, update_custom_field, delete_custom_field))
        .with_state(Arc::new(state))
}
