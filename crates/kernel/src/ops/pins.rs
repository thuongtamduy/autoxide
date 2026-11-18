//! Ghim của người dùng (bước 52 mục 4): mỗi người ghim tối đa 20 màn hình (mục của sổ điều hướng)
//! hay bản ghi, theo khách, để giao diện hiện ở chỗ dễ với tới (đầu thanh bên, màn hình chính của
//! điện thoại).
//!
//! - Bảng `kernel_pins`, cấp khách hàng; (`tenant_id`, `user_id`, `target`) duy nhất nên ghim lại
//!   cùng đích không ra hai dòng.
//! - Đích là `id` của mục điều hướng (kiểm với sổ điều hướng đã gom) hay `<loại>:<uuid>` của bản
//!   ghi kèm nhãn do giao diện gửi (kernel không biết tiêu đề bản ghi của module). Kernel không kiểm
//!   người ghim còn quyền xem đích: ghim chỉ là lối tắt, mở ra thì endpoint của module kiểm quyền.
//! - Hai request ghim MỚI cùng lúc có thể cùng lọt qua lần đếm và vượt 20 một chút — như cờ theo
//!   người dùng (D-207), đây là giới hạn giao diện, không phải số tiền.
//! - Là dữ liệu cá nhân: có trong bản xuất, xoá khi tài khoản bị xoá (cùng chỗ với cờ).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::access::Principal;
use crate::error::{Error, FieldError};
use crate::i18n::{Language, Translations};
use crate::migration::prelude::*;
use crate::ops::module_catalog::ModuleCatalog;
use crate::response::{Created, Data, Problem};
use crate::tenancy::{ScopeExt, TenantContext, TenantScoped};

/// Số ghim tối đa của một người ở một khách.
pub(crate) const MAX_PINS: usize = 20;
const TARGET_MAX: usize = 200;
const LABEL_MAX: usize = 200;
const TABLE: &str = "kernel_pins";

mod entity {
    //! Bảng `kernel_pins`.
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_pins")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub user_id: Uuid,
        pub kind: String,
        pub target: String,
        pub label: Option<String>,
        pub position: i32,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

use entity::{Column, Entity};

impl TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}

/// A shortcut the signed-in user pinned: a screen of the navigation or a record.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct Pin {
    pub id: Uuid,
    /// `navigation` (a screen) or `record`.
    #[schema(example = "navigation")]
    pub kind: String,
    /// The screen id from `GET /api/v1/kernel/navigation` (e.g. `content.entries`), or
    /// `<record kind>:<id>` for a record (e.g. `content.entry:0190…`).
    #[schema(example = "content.entries")]
    pub target: String,
    /// The screen's label in the language of `Accept-Language`, or the label sent when pinning
    /// a record.
    pub label: String,
    /// Display order, smallest first.
    pub position: i32,
    pub created_at: DateTime<Utc>,
}

/// What to pin.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PinInput {
    /// `navigation` or `record`.
    #[schema(example = "navigation")]
    kind: String,
    /// For `navigation`, a screen id from `GET /api/v1/kernel/navigation`; for `record`,
    /// `<record kind>:<id>`.
    #[schema(example = "content.entries")]
    target: String,
    /// Required for `record` (1 to 200 characters, e.g. the record's title); ignored for
    /// `navigation`, whose label comes from the navigation.
    label: Option<String>,
}

/// The new order of all my pins.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PinOrder {
    /// Ids of every pin of the caller, in the wanted order.
    ids: Vec<Uuid>,
}

#[derive(Clone)]
pub(crate) struct PinsState {
    pub catalog: ModuleCatalog,
    pub translations: Translations,
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Khoá dạng `a-z0-9._-`, 1–100 ký tự (loại bản ghi, như ở nhãn và ghi chú bản ghi).
fn is_kind(kind: &str) -> bool {
    (1..=100).contains(&kind.len())
        && kind.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

fn limit_reached() -> Error {
    Error::conflict(
        "kernel.pin_limit_reached",
        "You already have {max} pins, the most allowed. Unpin one with \
         `DELETE /api/v1/me/pins/{id}`, then try again.",
    )
    .with_param("max", MAX_PINS)
}

fn not_found() -> Error {
    Error::not_found("kernel.pin_not_found", "No pin of yours has this id.")
}

impl PinsState {
    fn label_of_screen(&self, id: &str, language: Language) -> Option<String> {
        let catalog = self.catalog.navigation()?;
        let (_, item) = catalog.items.iter().find(|(_, item)| item.id == id)?;
        Some(self.translations.describe(item.label, language))
    }

    async fn rows(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        connection: &impl ConnectionTrait,
    ) -> Result<Vec<entity::Model>, Error> {
        Ok(Entity::find()
            .filter(Column::UserId.eq(user_id))
            .scoped(tenant)
            .order_by_asc(Column::Position)
            .order_by_asc(Column::Id)
            .all(connection)
            .await?)
    }

    /// Dòng thành ghim; mục điều hướng không còn trong sổ (module đã gỡ) thì bỏ.
    fn view(&self, row: entity::Model, language: Language) -> Option<Pin> {
        let label = if row.kind == "navigation" {
            self.label_of_screen(&row.target, language)?
        } else {
            row.label.unwrap_or_default()
        };
        Some(Pin {
            id: row.id,
            kind: row.kind,
            target: row.target,
            label,
            position: row.position,
            created_at: row.created_at,
        })
    }

    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        language: Language,
    ) -> Result<Vec<Pin>, Error> {
        let rows = self.rows(tenant, principal.user_id(), tenant.database().read()).await?;
        Ok(rows.into_iter().filter_map(|row| self.view(row, language)).collect())
    }

    /// Ghim; trả `(ghim, true)` khi mới tạo, `(ghim, false)` khi đích đã được ghim từ trước.
    pub(crate) async fn pin(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: PinInput,
        language: Language,
    ) -> Result<(Pin, bool), Error> {
        let mut problems = Vec::new();
        let label = match input.kind.as_str() {
            "navigation" => {
                if self.label_of_screen(&input.target, language).is_none() {
                    problems.push(FieldError::new(
                        "target",
                        "invalid",
                        "must be a screen id from `GET /api/v1/kernel/navigation`",
                    ));
                }
                None
            }
            "record" => {
                let valid = input.target.len() <= TARGET_MAX
                    && input
                        .target
                        .split_once(':')
                        .is_some_and(|(kind, id)| is_kind(kind) && Uuid::parse_str(id).is_ok());
                if !valid {
                    problems.push(FieldError::new(
                        "target",
                        "invalid",
                        "must be `<record kind>:<id>`, e.g. `content.entry:0190…`",
                    ));
                }
                let label = input.label.as_deref().map(str::trim).unwrap_or_default();
                if label.is_empty() || label.chars().count() > LABEL_MAX {
                    problems.push(
                        FieldError::new("label", "length", "must be 1 to {max} characters long")
                            .with_param("max", LABEL_MAX),
                    );
                }
                Some(label.to_owned())
            }
            _ => {
                problems.push(FieldError::new(
                    "kind",
                    "invalid",
                    "must be `navigation` or `record`",
                ));
                None
            }
        };
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let user_id = principal.user_id();
        let rows = self.rows(tenant, user_id, tenant.database().read()).await?;
        if let Some(row) = rows.iter().find(|row| row.target == input.target) {
            return Ok((self.view(row.clone(), language).ok_or_else(not_found)?, false));
        }
        if rows.len() >= MAX_PINS {
            return Err(limit_reached());
        }
        let position = rows.iter().map(|row| row.position).max().map_or(1, |max| max + 1);
        let id = Uuid::now_v7();
        let row = entity::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            user_id: Set(user_id),
            kind: Set(input.kind),
            target: Set(input.target.clone()),
            label: Set(label),
            position: Set(position),
            created_at: Set(now()),
        };
        // Hai request ghim cùng đích cùng lúc: khoá duy nhất giữ một dòng, bên sau không ghi gì.
        Entity::insert(row)
            .on_conflict(
                OnConflict::columns([Column::TenantId, Column::UserId, Column::Target])
                    .do_nothing()
                    .to_owned(),
            )
            .exec_without_returning(tenant.database().write())
            .await?;
        let stored = Entity::find()
            .filter(Column::UserId.eq(user_id))
            .filter(Column::Target.eq(input.target))
            .scoped(tenant)
            .one(tenant.database().write())
            .await?
            .ok_or_else(not_found)?;
        let created = stored.id == id;
        Ok((self.view(stored, language).ok_or_else(not_found)?, created))
    }

    pub(crate) async fn unpin(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let deleted = Entity::delete_many()
            .filter(Column::Id.eq(id))
            .filter(Column::UserId.eq(principal.user_id()))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        if deleted.rows_affected == 0 {
            return Err(not_found());
        }
        Ok(())
    }

    /// Xếp lại: `ids` phải đúng là mọi ghim của người gọi, mỗi cái một lần.
    pub(crate) async fn reorder(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        ids: &[Uuid],
        language: Language,
    ) -> Result<Vec<Pin>, Error> {
        let user_id = principal.user_id();
        let transaction = tenant.database().begin_write().await?;
        let rows = self.rows(tenant, user_id, &transaction).await?;
        let mut sent = ids.to_vec();
        sent.sort();
        sent.dedup();
        let mut kept: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        kept.sort();
        if sent.len() != ids.len() || sent != kept {
            return Err(Error::validation(vec![FieldError::new(
                "ids",
                "invalid",
                "must list every one of your pins exactly once",
            )]));
        }
        for (index, id) in ids.iter().enumerate() {
            let position = <i32 as TryFrom<usize>>::try_from(index + 1).map_err(Error::internal)?;
            Entity::update_many()
                .col_expr(Column::Position, Expr::value(position))
                .filter(Column::Id.eq(*id))
                .filter(Column::UserId.eq(user_id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        transaction.commit().await?;
        self.list(tenant, principal, language).await
    }
}

/// Bản xuất dữ liệu cá nhân: ghim của người dùng.
pub(crate) async fn export_user(tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
    let rows = Entity::find()
        .filter(Column::UserId.eq(user_id))
        .scoped(tenant)
        .order_by_asc(Column::Position)
        .all(tenant.database().read())
        .await?;
    let pins: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            serde_json::json!({
                "kind": row.kind,
                "target": row.target,
                "label": row.label,
                "created_at": row.created_at,
            })
        })
        .collect();
    Ok(Value::Array(pins))
}

/// Tài khoản bị xoá: xoá ghim của người đó.
pub(crate) async fn forget_user(tenant: &TenantContext, user_id: Uuid) -> Result<u64, Error> {
    let deleted = Entity::delete_many()
        .filter(Column::UserId.eq(user_id))
        .scoped(tenant)
        .exec(tenant.database().write())
        .await?;
    Ok(deleted.rows_affected)
}

// --- REST -----------------------------------------------------------------------------------

/// List my pins.
///
/// Requires signing in, no permission: the caller's own pins in this tenant, in display order.
/// A pinned screen whose module is gone is left out.
#[utoipa::path(
    get,
    path = "/pins",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_pins",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The caller's pins, at most 20.", body = Data<Vec<Pin>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn list_pins(
    State(state): State<Arc<PinsState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
) -> Result<Data<Vec<Pin>>, Error> {
    Ok(Data { data: state.list(&tenant, &principal, language).await? })
}

/// Pin a screen or a record.
///
/// Requires signing in, no permission. Pinning a target already pinned returns that pin with
/// `200`, so sending it again is safe. A user keeps at most 20 pins per tenant. Opening a pinned
/// record still goes through the record's own permission check.
#[utoipa::path(
    post,
    path = "/pins",
    extensions(("x-permissions" = json!([]))),
    operation_id = "create_my_pin",
    security(("bearer" = [])),
    request_body = PinInput,
    responses(
        (status = 201, description = "Pinned; the new pin is last.", body = Data<Pin>, headers(("Location" = String, description = "Path of the new pin."))),
        (status = 200, description = "The target was already pinned; the existing pin.", body = Data<Pin>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 409, description = "The caller already has 20 pins; unpin one first. Code `kernel.pin_limit_reached`.", body = Problem),
        (status = 422, description = "`kind`, `target` or `label` is invalid; see `errors`.", body = Problem),
    )
)]
async fn create_pin(
    State(state): State<Arc<PinsState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
    crate::http::ValidJson(input): crate::http::ValidJson<PinInput>,
) -> Result<Response, Error> {
    let (pin, created) = state.pin(&tenant, &principal, input, language).await?;
    if created {
        Ok(Created::new(format!("/api/v1/me/pins/{}", pin.id), pin).into_response())
    } else {
        Ok(Data { data: pin }.into_response())
    }
}

/// Reorder my pins.
///
/// Requires signing in, no permission. Send the ids of all of your pins in the wanted order.
#[utoipa::path(
    put,
    path = "/pins/order",
    extensions(("x-permissions" = json!([]))),
    operation_id = "reorder_my_pins",
    security(("bearer" = [])),
    request_body = PinOrder,
    responses(
        (status = 200, description = "The pins in their new order.", body = Data<Vec<Pin>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "`ids` is not exactly the set of the caller's pins; see `errors`.", body = Problem),
    )
)]
async fn reorder_pins(
    State(state): State<Arc<PinsState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
    crate::http::ValidJson(input): crate::http::ValidJson<PinOrder>,
) -> Result<Data<Vec<Pin>>, Error> {
    Ok(Data { data: state.reorder(&tenant, &principal, &input.ids, language).await? })
}

/// Unpin.
///
/// Requires signing in, no permission; only the caller's own pins.
#[utoipa::path(
    delete,
    path = "/pins/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "delete_my_pin",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the pin.")),
    responses(
        (status = 204, description = "Unpinned."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "The caller has no pin with this id. Code `kernel.pin_not_found`.", body = Problem),
    )
)]
async fn delete_pin(
    State(state): State<Arc<PinsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    state.unpin(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Route dưới `/api/v1/me`.
pub(crate) fn routes(state: PinsState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_pins, create_pin))
        .routes(routes!(reorder_pins))
        .routes(routes!(delete_pin))
        .with_state(Arc::new(state))
}

// --- Migration ------------------------------------------------------------------------------

/// Bảng `kernel_pins`.
pub(crate) struct CreatePins;

impl MigrationName for CreatePins {
    fn name(&self) -> &str {
        "m0030_create_pins"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreatePins {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TABLE))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::string_len("kind", 16))
                    .col(schema::string_len("target", 200))
                    .col(schema::string_len_null("label", 800))
                    .col(schema::integer("position"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_pins_user_target")
                    .table(Alias::new(TABLE))
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("user_id"))
                    .col(Alias::new("target"))
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new(TABLE)).to_owned()).await
    }
}
