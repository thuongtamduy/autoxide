//! Permanently erasing a tenant (step 35, D-327, D-337): when a subscription's data retention period ends, or
//! when platform staff decide. Cannot be undone.
//!
//! - Only a tenant that is **suspended** (`suspended`) can be erased: the earlier suspension makes every
//!   server stop serving the tenant (tenant cache TTL) and is the moment for a final data export. A platform
//!   tenant, or a tenant whose database is being moved, is never erased.
//! - Deletes every row with `tenant_id = X` in every tenant-level table in the tenant's database (the outbox
//!   and the job queue included: nothing keeps running for the tenant), EXCEPT the tables a module declares to
//!   retain on tenant erasure (`Module::retained_tables`; accounting documents must be kept at least 10 years,
//!   Accounting Law 88/2015 Article 41, D-190; declared in step 36).
//! - Deletes every file under `tenants/<id>/` in the shared storage and the placement's own storage (data
//!   exports included), and the tenant's domains and exports on the control plane.
//! - Keeps the `kernel_tenants` row with status `deleted` (a tombstone): control-plane tables of other modules
//!   (e.g. the billing periods of `plans`, the operator's books) can still point to the tenant; the slug cannot
//!   be reused. Requests to an erased tenant receive `tenant_not_found`.
//! - The event `kernel.tenant_deleted` is recorded at the platform tenant (the erased tenant's outbox is gone).
//!
//! Re-runnable: if it fails midway (files, network), the tenant stays `suspended` and calling erase again
//! continues.

use crate::time::Timestamp;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};
use sea_orm::{ActiveModelTrait, ConnectionTrait, EntityTrait, IntoActiveModel, Set};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::admin::{TenantAdmin, require_platform, tenant};
use super::moves::TenantTableRows;
use super::{DEFAULT_TENANT, PlacementId, TenantId, TenantStatus, errors};
use crate::access::{Permission, Principal, Risk};
use crate::error::{Error, ErrorKind, FieldError};
use crate::event::{Actor, Event, EventInfo};
use crate::response::{Data, Problem};
use crate::storage::Storage;

/// Permanently erases a tenant: separate from `kernel.tenants.manage` because it cannot be undone.
pub const TENANTS_DELETE: Permission = Permission::new(
    "kernel.tenants.delete",
    "Delete a suspended tenant and its data for good (platform staff only). Cannot be undone.",
)
.with_risk(Risk::Destructive);

pub(crate) const TENANT_DELETED: EventInfo = EventInfo::new(
    "kernel.tenant_deleted",
    "A tenant and its data were deleted for good (`tenant_id`, `slug`, `reason`, `rows`, `files`, \
     `retained_tables`).",
);

/// What deleting a tenant removed.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TenantErasure {
    pub tenant_id: Uuid,
    pub slug: String,
    /// Rows deleted, per table (tables with none are left out).
    pub tables: Vec<TenantTableRows>,
    /// Total rows deleted.
    pub rows: u64,
    /// Files deleted.
    pub files: u64,
    /// Tables kept, as modules require (accounting records must be kept for years).
    pub retained_tables: Vec<String>,
}

#[derive(Debug, Serialize)]
struct TenantDeleted<'a> {
    tenant_id: Uuid,
    slug: &'a str,
    reason: &'a str,
    rows: u64,
    files: u64,
    retained_tables: &'a [String],
}

impl Event for TenantDeleted<'_> {
    const NAME: &'static str = "kernel.tenant_deleted";
}

fn now() -> Timestamp {
    Timestamp::now()
}

pub(crate) fn tenant_not_suspended() -> Error {
    Error::new(
        ErrorKind::Conflict,
        "tenant_not_suspended",
        "Only a suspended tenant can be deleted: suspend it first (`PATCH` the tenant with \
         `status: suspended`), export its data if needed, then delete it.",
    )
}

/// Deletes every file under `prefix` in `store`; returns the file count.
async fn delete_files(store: &dyn Storage, prefix: &str) -> Result<u64, Error> {
    let keys = store.keys_with_prefix(prefix).await?;
    for key in &keys {
        store.delete(key).await?;
    }
    Ok(keys.len() as u64)
}

impl TenantAdmin {
    /// Permanently erases the tenant `id_or_slug` (currently `suspended`) and its data. `reason`: 1-500
    /// characters, goes into the event `kernel.tenant_deleted`. Cannot be undone.
    pub async fn erase(
        &self,
        actor: Actor,
        id_or_slug: &str,
        reason: &str,
    ) -> Result<TenantErasure, Error> {
        let reason = reason.trim();
        if reason.is_empty() || reason.chars().count() > 500 {
            return Err(Error::validation(vec![FieldError::new(
                "reason",
                "length",
                "must be 1 to 500 characters long",
            )]));
        }
        let row = self.row(id_or_slug).await?;
        let tenant_id = TenantId::from_uuid(row.id);
        if tenant_id == DEFAULT_TENANT {
            return Err(Error::validation(vec![FieldError::new(
                "id",
                "invalid",
                "the platform tenant cannot be deleted",
            )]));
        }
        if row.status == TenantStatus::Deleted.as_str() {
            return Err(errors::tenant_not_found(id_or_slug));
        }
        if row.status != TenantStatus::Suspended.as_str() {
            return Err(tenant_not_suspended());
        }
        let setup = self.setup()?;
        let (modules, control_plane) = setup.tenant_tables()?;
        let retained = setup.retained()?;
        // Database của khách: mở theo placement (khách tạm khoá vẫn mở được — chỉ request bị chặn).
        let context = self.registry().tenant(tenant_id).await?;
        let database = context.database().clone();
        let tables =
            crate::transfer::TenantTables::read(&database, &database, &modules, &control_plane)
                .await
                .map_err(|error| Error::internal(error.to_string()))?;
        let keep = |table: &str| retained.contains(&table);
        let counts = tables
            .count(&database, tenant_id)
            .await
            .map_err(|error| Error::internal(error.to_string()))?;
        let transaction = database.begin_write().await?;
        let rows = tables
            .delete(&transaction, tenant_id, |table| !keep(table))
            .await
            .map_err(|error| Error::internal(error.to_string()))?;
        transaction.commit().await?;
        // File: nơi lưu chung (bản xuất, file của module) và nơi lưu riêng của placement nếu có.
        let prefix = format!("tenants/{}/", row.id);
        let mut files = 0;
        let mut stores: Vec<Arc<dyn Storage>> = Vec::new();
        if let Some(shared) = self.registry().shared_storage() {
            stores.push(shared);
        }
        if let Some(own) =
            self.registry().placement_storage(PlacementId::from_uuid(row.placement_id)).await?
        {
            stores.push(own);
        }
        for store in &stores {
            files += delete_files(&**store, &prefix).await?;
        }
        // Mặt điều khiển: tên miền (trả cho người khác dùng), lượt xuất; khách thành bia mộ — một
        // transaction, kèm event ở khách nền tảng.
        let control = self.registry().control();
        let domains: Vec<String> = {
            let select = Query::select()
                .column(Alias::new("domain"))
                .from(Alias::new("kernel_tenant_domains"))
                .and_where(Expr::col(Alias::new("tenant_id")).eq(row.id))
                .to_owned();
            control
                .write()
                .query_all(&select)
                .await?
                .iter()
                .filter_map(|found| found.try_get::<String>("", "domain").ok())
                .collect()
        };
        let retained_tables: Vec<String> =
            tables.names().into_iter().filter(|table| keep(table)).map(str::to_owned).collect();
        let slug = row.slug.clone();
        let platform = self.registry().default_tenant();
        let transaction = control.begin_write().await?;
        for table in ["kernel_tenant_domains", "kernel_tenant_exports"] {
            let delete = Query::delete()
                .from_table(Alias::new(table))
                .and_where(Expr::col(Alias::new("tenant_id")).eq(row.id))
                .to_owned();
            transaction.execute(&delete).await?;
        }
        let mut tombstone = row.into_active_model();
        tombstone.status = Set(TenantStatus::Deleted.as_str().to_owned());
        tombstone.read_only = Set(true);
        tombstone.updated_at = Set(now());
        tombstone.update(&transaction).await?;
        let event = TenantDeleted {
            tenant_id: tenant_id.as_uuid(),
            slug: &slug,
            reason,
            rows,
            files,
            retained_tables: &retained_tables,
        };
        self.outbox()
            .record(
                &transaction,
                &platform,
                actor,
                Some(format!("kernel.tenant:{}", tenant_id.as_uuid())),
                &event,
            )
            .await?;
        transaction.commit().await?;
        self.registry().invalidate(tenant_id);
        for domain in &domains {
            self.registry().invalidate_domain(domain);
        }
        tracing::warn!(tenant_id = %tenant_id, slug = %slug, rows, files, "tenant deleted for good");
        Ok(TenantErasure {
            tenant_id: tenant_id.as_uuid(),
            slug,
            tables: counts
                .into_iter()
                .filter(|table| table.rows > 0 && !keep(&table.table))
                .map(TenantTableRows::from)
                .collect(),
            rows,
            files,
            retained_tables,
        })
    }

    pub(super) async fn row(&self, id_or_slug: &str) -> Result<tenant::Model, Error> {
        use sea_orm::{ColumnTrait, QueryFilter};
        let select = match id_or_slug.parse::<Uuid>() {
            Ok(id) => tenant::Entity::find_by_id(id),
            Err(_) => tenant::Entity::find().filter(tenant::Column::Slug.eq(id_or_slug)),
        };
        select
            .one(self.registry().control().write())
            .await?
            .ok_or_else(|| errors::tenant_not_found(id_or_slug))
    }
}

/// Delete a tenant for good.
#[derive(Debug, Deserialize, validator::Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct EraseRequest {
    /// The tenant's slug again, to confirm.
    #[schema(example = "acme")]
    confirm: String,
    /// Why, 1 to 500 characters (kept in the audit log of the platform).
    reason: String,
}

pub(super) fn routes() -> utoipa_axum::router::OpenApiRouter<Arc<TenantAdmin>> {
    utoipa_axum::router::OpenApiRouter::new().routes(utoipa_axum::routes!(erase_tenant))
}

/// Delete a tenant and its data for good.
///
/// Requires `kernel.tenants.delete` and an account of the platform tenant. Cannot be undone. The
/// tenant must be suspended first (`PATCH /tenants/{id}` with `status: suspended`); export its data
/// before if it is wanted (`POST /tenants/{id}/exports`). Every row of the tenant in its database
/// and every file of it is deleted, except the tables modules must keep (accounting records); its
/// domains are released. The tenant stays listed with status `deleted`, and every request to it
/// answers `404 tenant_not_found`.
#[utoipa::path(
    post,
    path = "/tenants/{id}/erase",
    extensions(("x-permissions" = json!(["kernel.tenants.delete"]))),
    operation_id = "erase_tenant",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant.")),
    request_body = EraseRequest,
    responses(
        (status = 200, description = "Deleted; what was removed.", body = Data<TenantErasure>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.delete` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such tenant, or already deleted; code `tenant_not_found`.", body = Problem),
        (status = 409, description = "The tenant is not suspended; code `tenant_not_suspended`.", body = Problem),
        (status = 422, description = "`confirm` is not the tenant's slug, `reason` is missing or too long, or the tenant is the platform; every problem in `errors`.", body = Problem),
    )
)]
async fn erase_tenant(
    axum::extract::State(admin): axum::extract::State<Arc<TenantAdmin>>,
    principal: Principal,
    axum::extract::Path(id): axum::extract::Path<String>,
    crate::http::ValidJson(input): crate::http::ValidJson<EraseRequest>,
) -> Result<Data<TenantErasure>, Error> {
    principal.require(&TENANTS_DELETE)?;
    require_platform(&principal)?;
    let row = admin.row(&id).await?;
    if input.confirm.trim() != row.slug {
        return Err(Error::validation(vec![FieldError::new(
            "confirm",
            "mismatch",
            "must be the slug of the tenant",
        )]));
    }
    let erased = admin.erase(Actor::from(&principal), &row.id.to_string(), &input.reason).await?;
    Ok(Data { data: erased })
}
