//! Hộp việc theo người (bước 51 P10, D-201, D-212): hoãn một việc tới lúc nào đó (của riêng người
//! hoãn) và giao việc cho một người (chung cho mọi người thấy việc). Bảng `kernel_work_states`
//! (cấp khách hàng): dòng hoãn mang `user_id` của người hoãn; dòng giao mang `user_id` rỗng (UUID
//! nil) và `assignee_id`.
//!
//! - `POST /api/v1/me/work/{key}/snooze {until}`, `DELETE /api/v1/me/work/{key}/snooze`;
//! - `POST /api/v1/me/work/{key}/assign {user_id}` (`null` bỏ giao) — người nhận phải có permission
//!   của nguồn việc (hỏi bộ xác thực, `Authenticator::principal_of`).
//!
//! Không có "Đã xử lý": việc tự hết khi dữ liệu không còn khớp (D-212). Dòng hoãn đã hết hạn của
//! khách được dọn mỗi lần có người hoãn việc (không cần job riêng).

use std::collections::HashMap;
use std::sync::Arc;

use axum::Extension;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Duration, SubsecRound, Utc};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, OnConflict, Query as SqlQuery};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::access::{Authenticator, Principal};
use crate::entitlement::EntitlementsHandle;
use crate::error::{Error, FieldError};
use crate::http::ValidJson;
use crate::migration::prelude::*;
use crate::ops::work::WorkSources;
use crate::response::{Data, Problem};
use crate::tenancy::TenantContext;

const TABLE: &str = "kernel_work_states";
/// Hoãn tối đa chừng này ngày.
const MAX_SNOOZE_DAYS: i64 = 365;

/// Trạng thái của một việc với người gọi.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ItemState {
    pub snoozed_until: Option<DateTime<Utc>>,
    pub assignee_id: Option<Uuid>,
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Trạng thái của các việc `keys` với `user_id`: hoãn của chính người đó, giao của ai cũng thấy.
pub(crate) async fn states(
    tenant: &TenantContext,
    user_id: Uuid,
    keys: &[String],
) -> Result<HashMap<String, ItemState>, Error> {
    let mut found: HashMap<String, ItemState> = HashMap::new();
    if keys.is_empty() {
        return Ok(found);
    }
    // Từng lô 500 khoá: giới hạn tham số của câu `IN` trên cả hai database.
    for chunk in keys.chunks(500) {
        let select = SqlQuery::select()
            .columns(["key", "user_id", "snoozed_until", "assignee_id"])
            .from(Alias::new(TABLE))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("user_id")).is_in([user_id, Uuid::nil()]))
            .and_where(Expr::col(Alias::new("key")).is_in(chunk.iter().cloned()))
            .to_owned();
        for row in tenant.database().read().query_all(&select).await? {
            let key: String = row.try_get("", "key")?;
            let owner: Uuid = row.try_get("", "user_id")?;
            let entry = found.entry(key).or_default();
            if owner.is_nil() {
                entry.assignee_id = row.try_get("", "assignee_id")?;
            } else {
                entry.snoozed_until = row.try_get("", "snoozed_until")?;
            }
        }
    }
    Ok(found)
}

async fn upsert<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    key: &str,
    user_id: Uuid,
    snoozed_until: Option<DateTime<Utc>>,
    assignee_id: Option<Uuid>,
) -> Result<(), Error> {
    let insert = SqlQuery::insert()
        .into_table(Alias::new(TABLE))
        .columns([
            "id",
            "tenant_id",
            "key",
            "user_id",
            "snoozed_until",
            "assignee_id",
            "updated_at",
        ])
        .values_panic([
            Uuid::now_v7().into(),
            tenant.id().as_uuid().into(),
            key.into(),
            user_id.into(),
            snoozed_until.into(),
            assignee_id.into(),
            now().into(),
        ])
        .on_conflict(
            OnConflict::columns([
                Alias::new("tenant_id"),
                Alias::new("key"),
                Alias::new("user_id"),
            ])
            .update_columns([
                Alias::new("snoozed_until"),
                Alias::new("assignee_id"),
                Alias::new("updated_at"),
            ])
            .to_owned(),
        )
        .to_owned();
    connection.execute(&insert).await?;
    Ok(())
}

async fn remove(tenant: &TenantContext, key: &str, user_id: Uuid) -> Result<(), Error> {
    let delete = SqlQuery::delete()
        .from_table(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("key")).eq(key))
        .and_where(Expr::col(Alias::new("user_id")).eq(user_id))
        .to_owned();
    tenant.database().write().execute(&delete).await?;
    Ok(())
}

/// When to show the work again.
#[derive(Debug, Clone, Deserialize, ToSchema, validator::Validate)]
#[schema(as = WorkSnoozeRequest)]
pub(crate) struct SnoozeRequest {
    /// A time in the future, at most a year away.
    pub until: DateTime<Utc>,
}

/// Who should do the work.
#[derive(Debug, Clone, Deserialize, ToSchema, validator::Validate)]
#[schema(as = WorkAssignRequest)]
pub(crate) struct AssignRequest {
    /// A user who may see the work's source; `null` removes the assignment.
    pub user_id: Option<Uuid>,
}

/// The state of a piece of work for you.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct WorkStateView {
    pub key: String,
    /// Hidden from your inbox until then.
    pub snoozed_until: Option<DateTime<Utc>>,
    /// Who it is assigned to.
    pub assignee_id: Option<Uuid>,
}

#[derive(Clone)]
pub(crate) struct WorkStatesState {
    pub sources: WorkSources,
    pub entitlements: EntitlementsHandle,
}

impl WorkStatesState {
    /// Khoá `<nguồn>:<id>` của một nguồn người gọi được thấy; không thì 404 như không có việc.
    async fn check_key(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &str,
    ) -> Result<Arc<dyn crate::ops::work::WorkSource>, Error> {
        let not_found = || {
            Error::not_found("kernel.work_not_found", "There is no work `{key}` for you.")
                .with_param("key", key.to_owned())
        };
        let (source, record) = key.split_once(':').ok_or_else(not_found)?;
        if record.is_empty() || key.len() > 300 {
            return Err(not_found());
        }
        let (module, found) = self.sources.find(source).ok_or_else(not_found)?;
        if !self.entitlements.module_enabled(tenant.id(), module).await?
            || principal.require(&found.permission()).is_err()
        {
            return Err(not_found());
        }
        Ok(found)
    }

    async fn view(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &str,
    ) -> Result<WorkStateView, Error> {
        let state = states(tenant, principal.user_id(), &[key.to_owned()])
            .await?
            .remove(key)
            .unwrap_or_default();
        Ok(WorkStateView {
            key: key.to_owned(),
            snoozed_until: state.snoozed_until,
            assignee_id: state.assignee_id,
        })
    }
}

/// Snooze a piece of work.
///
/// Any signed-in user who sees the work's source. Hides the work (key `<source>:<record id>` from
/// `GET /api/v1/kernel/work`) from your inbox until `until`; others still see it. Snoozing again
/// moves the time.
#[utoipa::path(
    post,
    path = "/work/{key}/snooze",
    extensions(("x-permissions" = json!([]))),
    operation_id = "snooze_work",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "Key of the work, `<source>:<record id>`.")),
    request_body = SnoozeRequest,
    responses(
        (status = 200, description = "The work's state for you.", body = Data<WorkStateView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such work for you; code `kernel.work_not_found`.", body = Problem),
        (status = 422, description = "`until` is in the past or more than a year away; see `errors`.", body = Problem),
    )
)]
async fn snooze_work(
    State(state): State<Arc<WorkStatesState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(key): Path<String>,
    ValidJson(input): ValidJson<SnoozeRequest>,
) -> Result<Data<WorkStateView>, Error> {
    state.check_key(&tenant, &principal, &key).await?;
    let now = now();
    if input.until <= now || input.until > now + Duration::days(MAX_SNOOZE_DAYS) {
        return Err(Error::validation(vec![FieldError::new(
            "until",
            "invalid",
            "must be in the future, at most a year away",
        )]));
    }
    let expired = SqlQuery::delete()
        .from_table(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("snoozed_until")).lte(now))
        .and_where(Expr::col(Alias::new("assignee_id")).is_null())
        .to_owned();
    tenant.database().write().execute(&expired).await?;
    upsert(
        tenant.database().write(),
        &tenant,
        &key,
        principal.user_id(),
        Some(input.until.trunc_subsecs(6)),
        None,
    )
    .await?;
    Ok(Data { data: state.view(&tenant, &principal, &key).await? })
}

/// Stop snoozing a piece of work.
///
/// Any signed-in user. Shows the work in your inbox again; answers 204 even when it was not
/// snoozed.
#[utoipa::path(
    delete,
    path = "/work/{key}/snooze",
    extensions(("x-permissions" = json!([]))),
    operation_id = "unsnooze_work",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "Key of the work, `<source>:<record id>`.")),
    responses(
        (status = 204, description = "Shown again."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such work for you; code `kernel.work_not_found`.", body = Problem),
    )
)]
async fn unsnooze_work(
    State(state): State<Arc<WorkStatesState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(key): Path<String>,
) -> Result<StatusCode, Error> {
    state.check_key(&tenant, &principal, &key).await?;
    remove(&tenant, &key, principal.user_id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Assign a piece of work.
///
/// Any signed-in user who sees the work's source. Assigns the work to `user_id`, who must see the
/// source too (else 422); `null` removes the assignment. Everyone who sees the work sees who it is
/// assigned to (`assignee_id` in `GET /api/v1/kernel/work`).
#[utoipa::path(
    post,
    path = "/work/{key}/assign",
    extensions(("x-permissions" = json!([]))),
    operation_id = "assign_work",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "Key of the work, `<source>:<record id>`.")),
    request_body = AssignRequest,
    responses(
        (status = 200, description = "The work's state for you.", body = Data<WorkStateView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such work for you; code `kernel.work_not_found`.", body = Problem),
        (status = 422, description = "The user cannot see this work; see `errors`.", body = Problem),
    )
)]
async fn assign_work(
    State(state): State<Arc<WorkStatesState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(key): Path<String>,
    // Bộ xác thực của chương trình, lớp HTTP gắn vào mọi request (cùng bộ đã xác thực người gọi).
    authenticator: Option<Extension<Arc<dyn Authenticator>>>,
    ValidJson(input): ValidJson<AssignRequest>,
) -> Result<Data<WorkStateView>, Error> {
    let source = state.check_key(&tenant, &principal, &key).await?;
    match input.user_id {
        None => remove(&tenant, &key, Uuid::nil()).await?,
        Some(assignee) => {
            let allowed = match &authenticator {
                Some(Extension(authenticator)) => authenticator
                    .principal_of(&tenant, assignee)
                    .await?
                    .is_some_and(|them| them.require(&source.permission()).is_ok()),
                None => false,
            };
            if !allowed {
                return Err(Error::validation(vec![FieldError::new(
                    "user_id",
                    "invalid",
                    "is not an active user who can see this work",
                )]));
            }
            upsert(tenant.database().write(), &tenant, &key, Uuid::nil(), None, Some(assignee))
                .await?;
        }
    }
    Ok(Data { data: state.view(&tenant, &principal, &key).await? })
}

/// Route dưới `/api/v1/me`.
pub(crate) fn routes(state: WorkStatesState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(snooze_work, unsnooze_work))
        .routes(routes!(assign_work))
        .with_state(Arc::new(state))
}

/// Migration tạo bảng `kernel_work_states` (cấp khách hàng, mọi database dữ liệu).
pub(crate) struct CreateWorkStates;

impl MigrationName for CreateWorkStates {
    fn name(&self) -> &str {
        "m0021_create_work_states"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateWorkStates {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TABLE))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("key", 300))
                    .col(schema::uuid("user_id"))
                    .col(schema::timestamp_with_time_zone_null("snoozed_until"))
                    .col(schema::uuid_null("assignee_id"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_work_states_key")
                    .table(Alias::new(TABLE))
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("key"))
                    .col(Alias::new("user_id"))
                    .unique()
                    .to_owned(),
            )
            .await?;
        // Dọn dòng hoãn hết hạn của một khách.
        manager
            .create_index(
                Index::create()
                    .name("kernel_work_states_snooze_idx")
                    .table(Alias::new(TABLE))
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("snoozed_until"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new(TABLE)).to_owned()).await
    }
}
