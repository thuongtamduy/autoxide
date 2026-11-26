//! Tenant management (spec 33 "Provisioning and moving tenants", "REST API", D-260): create, seed initial
//! data, view, suspend / resume; the caller's current tenant.
//!
//! - Creating a tenant: writes `kernel_tenants` with status `provisioning` → calls the `TenantSeed` of
//!   every module in registration order (seeds the `admin` role, lock rows, the head of the audit chain...)
//!   → `active`. If it fails midway the tenant stays `provisioning`; `autoxide tenant provision <slug>`
//!   reruns it (every step is safe to rerun).
//! - Managing tenants is the job of the **platform tenant's staff** (the default tenant), not of the admin
//!   of an ordinary tenant: the `admin` role of every tenant has every permission, so checking permission
//!   alone is a hole; it also checks the caller's tenant ([`require_platform`]), and does not let platform
//!   staff who are "entering" another tenant with a header do this from inside that tenant.
//! - `single` mode: only the default tenant exists; creating a tenant is the error `tenant_single_mode`.

use crate::time::Timestamp;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use axum::extract::{Path, Query, State};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::{
    CurrentTenant, DEFAULT_TENANT, DatabaseRegistry, MAIN_PLACEMENT, PlacementId, PlacementKind,
    SchemaState, TenancyMode, TenantContext, TenantId, TenantStatus, errors,
};
use crate::access::{Permission, Principal, Risk};
use crate::error::{Error, FieldError};
use crate::event::{Actor, Event, EventInfo, Outbox};
use crate::mcp::{NoInput, Tool};
use crate::pagination::{Page, PageQuery, PageRequest, fetch_page};
use crate::response::{Created, Data, Problem};
use crate::version::{IfMatch, Tagged};

pub const TENANTS_READ: Permission = Permission::new(
    "kernel.tenants.read",
    "See every tenant of the platform (platform staff only).",
);

pub const TENANTS_MANAGE: Permission = Permission::new(
    "kernel.tenants.manage",
    "Create tenants, rename them, suspend and resume them (platform staff only).",
)
.with_risk(Risk::Destructive);

pub const PLACEMENTS_READ: Permission = Permission::new(
    "kernel.placements.read",
    "See where tenants' data is kept (platform staff only); connection URLs are never shown.",
);

pub(crate) const TENANT_CREATED: EventInfo =
    EventInfo::new("kernel.tenant_created", "A tenant was created and is ready to use.");
pub(crate) const TENANT_UPDATED: EventInfo =
    EventInfo::new("kernel.tenant_updated", "A tenant was renamed, suspended or resumed.");

/// Seeds a module's initial data for a new tenant (e.g. role `admin`, lock rows). Must be safe to
/// rerun: a tenant whose provisioning failed midway is rerun from the start.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait TenantSeed: Send + Sync {
    async fn seed(&self, tenant: &TenantContext) -> Result<(), Error>;
}

/// One module's seeder, with the module name (so an error says which module failed).
pub(crate) type NamedSeed = (&'static str, Arc<dyn TenantSeed>);

/// The seeders of every module (initial data, and sample data when provisioning: `Module::sample_data`),
/// in registration order; filled in after every module has started. Cheap to clone.
#[derive(Clone, Default)]
pub(crate) struct Seeds(Arc<OnceLock<(Vec<NamedSeed>, Vec<NamedSeed>)>>);

impl Seeds {
    pub(crate) fn fill(&self, seeds: Vec<NamedSeed>, samples: Vec<NamedSeed>) {
        let _ = self.0.set((seeds, samples));
    }

    pub(super) fn all(&self) -> &[NamedSeed] {
        self.0.get().map_or(&[], |(seeds, _)| seeds.as_slice())
    }

    fn samples(&self) -> &[NamedSeed] {
        self.0.get().map_or(&[], |(_, samples)| samples.as_slice())
    }
}

/// The caller is platform-tenant staff, working inside the platform tenant itself.
pub fn require_platform(principal: &Principal) -> Result<(), Error> {
    if principal.tenant_id() == DEFAULT_TENANT && principal.operator_from().is_none() {
        Ok(())
    } else {
        Err(errors::tenant_platform_only())
    }
}

mod placement_list;

pub(super) mod tenant {
    //! The `kernel_tenants` table (control plane).
    use crate::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_tenants")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub slug: String,
        pub name: String,
        pub status: String,
        pub parent_id: Option<Uuid>,
        pub placement_id: Uuid,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
        /// Read-only: reads work, every write is blocked (`tenant_read_only`); independent of `status`.
        pub read_only: bool,
        /// Country where the tenant must keep its data (ISO 3166-1 alpha-2), `None` when not required (D-360).
        pub data_residency: Option<String>,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(super) mod placement {
    //! The `kernel_placements` table (control plane).
    use crate::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_placements")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub name: String,
        pub kind: String,
        pub backend: String,
        pub url_sealed: Option<String>,
        pub read_url_sealed: Option<String>,
        pub storage_sealed: Option<String>,
        pub schema_state: String,
        pub schema_checked_at: Option<Timestamp>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
        /// Country where the server is located (ISO 3166-1 alpha-2), `None` when not recorded (D-360).
        pub country_code: Option<String>,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// A customer (tenant) of the platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Tenant {
    pub id: Uuid,
    /// Short name: lower-case letters, digits and `-`; unique on the platform.
    #[schema(example = "acme")]
    pub slug: String,
    pub name: String,
    /// `provisioning`, `active`, `suspended` (every request is refused with
    /// `403 tenant_suspended`), `maintenance`, `moving` or `deleted` (its data was deleted; every
    /// request answers `404 tenant_not_found`).
    #[schema(example = "active")]
    pub status: String,
    /// Read-only: its data can be read (and its users can sign in and export), but every change
    /// is refused with `403 tenant_read_only`. Independent of `status`. Always present.
    #[serde(default)]
    #[schema(required = true)]
    pub read_only: bool,
    /// Parent tenant, when the tenant belongs to a chain.
    pub parent_id: Option<Uuid>,
    /// Name of the placement (database) that keeps its data.
    #[schema(example = "main")]
    pub placement: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// Related records asked for with `expand` in `GET /api/v1/kernel/tenants`; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded: Option<TenantExpanded>,
}

/// Related records of a tenant, asked for with `expand`. All three fields are always present
/// (`null` when not asked for or when there is nothing).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TenantExpanded {
    /// The placement that keeps the tenant's data (`expand=placement`); `null` when not asked for.
    #[schema(required = true)]
    pub placement: Option<Placement>,
    /// The tenant's subscription (`expand=subscription`), shaped like `GET
    /// /api/v1/plans/tenants/{tenant_id}/subscription` (`PlanSubscription`, with the plan's code
    /// and name, without add-ons). `null` when not asked for, when the tenant has no subscription,
    /// or without the plans module.
    #[schema(value_type = Option<Object>, required = true)]
    pub subscription: Option<serde_json::Value>,
    /// The tenant's use of every limit (`expand=usage`), shaped like `GET
    /// /api/v1/plans/tenants/{tenant_id}/usage` (a list of `LimitUse`). `null` when not asked for
    /// or without the plans module.
    #[schema(value_type = Option<Vec<Object>>, required = true)]
    pub usage: Option<serde_json::Value>,
}

/// Where tenants' data is kept: one database. Connection URLs are never shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Placement {
    pub id: Uuid,
    #[schema(example = "main")]
    pub name: String,
    /// `shared` (many tenants) or `dedicated` (one tenant).
    pub kind: String,
    /// `sqlite` or `postgres`.
    pub backend: String,
    /// `ready`, `behind` (migrations pending: its tenants answer `503`) or `migrating`.
    pub schema_state: String,
    /// The placement's own file storage; `null` when its tenants' files are kept in the
    /// platform's storage (the `files` module settings). Set with `autoxide placement storage`.
    pub storage: Option<PlacementStorage>,
    /// Country of the servers (ISO 3166-1 alpha-2), for tenants that must keep their data in one
    /// country; `null` when unknown. Set with `autoxide placement country`.
    #[serde(default)]
    pub country_code: Option<String>,
    pub created_at: Timestamp,
}

/// Where a placement keeps its tenants' files. Access keys are never shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PlacementStorage {
    /// `local` (a directory on the server), `s3` (an S3 or S3-compatible bucket), or
    /// `unreadable` (the stored settings cannot be decrypted with the configured secret key).
    #[schema(example = "s3")]
    pub kind: String,
    /// Directory on the server (`local`).
    pub dir: Option<String>,
    /// Bucket (`s3`).
    #[schema(example = "acme-files")]
    pub bucket: Option<String>,
    /// Folder inside the bucket (`s3`).
    pub prefix: Option<String>,
    /// Region of the bucket (`s3`).
    pub region: Option<String>,
    /// Address of the S3-compatible service (`s3`); `null` for Amazon S3.
    pub endpoint: Option<String>,
    /// Whether an access key is stored (`s3`); its value is never shown.
    pub credentials: bool,
}

/// A new tenant.
#[derive(Debug, Clone, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewTenant {
    /// 1 to 63 lower-case letters, digits and `-`, not starting or ending with `-`.
    #[schema(example = "acme")]
    pub slug: String,
    /// 1 to 200 characters.
    #[schema(example = "Acme Ltd")]
    pub name: String,
    /// Parent tenant, for chains.
    pub parent_id: Option<Uuid>,
    /// Name of the placement for its data; `main` (the platform's database) when omitted.
    pub placement: Option<String>,
}

/// Changes to a tenant; omitted fields stay as they are.
#[derive(Debug, Clone, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TenantChanges {
    pub name: Option<String>,
    /// `active` or `suspended`. A suspended tenant refuses every request, reads included.
    #[schema(example = "suspended")]
    pub status: Option<String>,
    /// `true`: the tenant can only read (its users still sign in and export their data), every
    /// change is refused with `403 tenant_read_only`; `false`: writable again. Independent of
    /// `status`. The platform tenant cannot be made read-only.
    pub read_only: Option<bool>,
    /// Why (1 to 500 characters), kept with the change in the audit log (event
    /// `kernel.tenant_updated`) — e.g. why a tenant was suspended.
    #[schema(example = "Unpaid invoices after the grace period")]
    pub reason: Option<String>,
}

/// Which tenants to list, and what to add to each.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TenantFilter {
    /// Only tenants in this state: `provisioning`, `active`, `suspended`, `maintenance` or
    /// `moving`.
    pub status: Option<String>,
    /// Only children of this tenant.
    pub parent_id: Option<Uuid>,
    /// Only read-only tenants (`true`) or writable ones (`false`).
    pub read_only: Option<bool>,
    /// Only tenants whose placement is of this kind: `shared` or `dedicated`.
    #[param(example = "dedicated")]
    pub placement_kind: Option<String>,
    /// Only tenants whose placement is in this state: `ready`, `behind` (migrations pending) or
    /// `migrating`.
    #[param(example = "behind")]
    pub schema_state: Option<String>,
    /// Only tenants whose subscription is in this state: `trialing`, `active`, `past_due`,
    /// `suspended`, `canceled` or `deleted`. Subscriptions come with the plans module (step 35);
    /// without it no tenant has one, so the page is empty.
    #[param(example = "past_due")]
    pub subscription_status: Option<String>,
    /// `true`: only tenants close to a limit of their plan. Limits come with the plans module
    /// (step 35); without it no tenant has limits, so the page is empty. `false` filters nothing.
    pub near_limit: Option<bool>,
    /// Related records to add to each tenant under `expanded`, comma-separated: `placement`,
    /// `subscription`, `usage`. Fetched for the whole page at once.
    #[param(example = "placement,subscription")]
    pub expand: Option<String>,
}

/// Subscription status the filter accepts (spec 35).
const SUBSCRIPTION_STATES: [&str; 6] =
    ["trialing", "active", "past_due", "suspended", "canceled", "deleted"];

/// What `expand` accepts.
const EXPANSIONS: [&str; 3] = ["placement", "subscription", "usage"];

#[derive(Debug, Serialize)]
struct TenantChanged {
    before: Option<Tenant>,
    after: Option<Tenant>,
    /// Reason given by the person making the change (only on an edit).
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(transparent)]
struct TenantCreated(TenantChanged);

#[derive(Debug, Serialize)]
#[serde(transparent)]
struct TenantUpdated(TenantChanged);

impl Event for TenantCreated {
    const NAME: &'static str = "kernel.tenant_created";
}

impl Event for TenantUpdated {
    const NAME: &'static str = "kernel.tenant_updated";
}

/// The tenant management service. Published by the `kernel` module (`context.require::<TenantAdmin>()`).
pub struct TenantAdmin {
    registry: DatabaseRegistry,
    seeds: Seeds,
    outbox: Outbox,
    /// Subscription and limits (module `plans`, step 35): filters `subscription_status`, `near_limit`.
    entitlements: crate::entitlement::EntitlementsHandle,
    /// Tables of the modules (for deleting a tenant for good, D-337); `None` when not attached.
    setup: Option<super::moves::MoveSetup>,
}

impl std::fmt::Debug for TenantAdmin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TenantAdmin").finish_non_exhaustive()
    }
}

fn now() -> Timestamp {
    Timestamp::now()
}

pub(super) fn is_slug(slug: &str) -> bool {
    (1..=63).contains(&slug.len())
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl TenantAdmin {
    pub(crate) fn new(registry: DatabaseRegistry, seeds: Seeds, outbox: Outbox) -> Self {
        TenantAdmin { registry, seeds, outbox, entitlements: Default::default(), setup: None }
    }

    /// Tables of the modules, for deleting a tenant for good (D-337).
    pub(crate) fn with_setup(mut self, setup: super::moves::MoveSetup) -> Self {
        self.setup = Some(setup);
        self
    }

    pub(super) fn setup(&self) -> Result<&super::moves::MoveSetup, Error> {
        self.setup.as_ref().ok_or_else(|| Error::internal("tenant deletion is not set up"))
    }

    pub(super) fn registry(&self) -> &DatabaseRegistry {
        &self.registry
    }

    pub(super) fn outbox(&self) -> &Outbox {
        &self.outbox
    }

    /// Subscription and limits for the tenant-list filters (step 35).
    pub(crate) fn with_entitlements(
        mut self,
        entitlements: crate::entitlement::EntitlementsHandle,
    ) -> Self {
        self.entitlements = entitlements;
        self
    }

    fn control(&self) -> &crate::database::Database {
        self.registry.control()
    }

    async fn placement_names(&self) -> Result<Vec<placement::Model>, Error> {
        Ok(placement::Entity::find().all(self.control().read()).await?)
    }

    async fn describe(&self, row: tenant::Model) -> Result<Tenant, Error> {
        let placements = self.placement_names().await?;
        Ok(describe(row, &placements))
    }

    /// One page of tenants, newest first (unless `order=asc`). Two queries for the whole page (the small
    /// placement table, then the filtered page of tenants), not one call per row.
    pub async fn list(
        &self,
        filter: &TenantFilter,
        page: &PageRequest,
    ) -> Result<Page<Tenant>, Error> {
        let mut problems = Vec::new();
        let mut select = tenant::Entity::find();
        if let Some(status) = &filter.status {
            if TenantStatus::parse(status).is_none() {
                problems.push(FieldError::new(
                    "status",
                    "invalid",
                    "must be one of provisioning, active, suspended, maintenance, moving, deleted",
                ));
            }
            select = select.filter(tenant::Column::Status.eq(status.as_str()));
        }
        if let Some(parent) = filter.parent_id {
            select = select.filter(tenant::Column::ParentId.eq(parent));
        }
        if let Some(read_only) = filter.read_only {
            select = select.filter(tenant::Column::ReadOnly.eq(read_only));
        }
        if filter.placement_kind.as_deref().is_some_and(|kind| PlacementKind::parse(kind).is_none())
        {
            problems.push(FieldError::new(
                "placement_kind",
                "invalid",
                "must be `shared` or `dedicated`",
            ));
        }
        if filter.schema_state.as_deref().is_some_and(|state| SchemaState::parse(state).is_none()) {
            problems.push(FieldError::new(
                "schema_state",
                "invalid",
                "must be one of ready, behind, migrating",
            ));
        }
        if filter
            .subscription_status
            .as_deref()
            .is_some_and(|state| !SUBSCRIPTION_STATES.contains(&state))
        {
            problems.push(FieldError::new(
                "subscription_status",
                "invalid",
                "must be one of trialing, active, past_due, suspended, canceled, deleted",
            ));
        }
        let mut expand = Vec::new();
        for item in filter.expand.as_deref().unwrap_or_default().split(',').map(str::trim) {
            if item.is_empty() || expand.contains(&item) {
                continue;
            }
            if EXPANSIONS.contains(&item) {
                expand.push(item);
            } else {
                problems.push(FieldError::new(
                    "expand",
                    "invalid",
                    "can only list placement, subscription and usage",
                ));
                break;
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let placements = self.placement_names().await?;
        if filter.placement_kind.is_some() || filter.schema_state.is_some() {
            let ids: Vec<Uuid> = placements
                .iter()
                .filter(|placement| {
                    filter.placement_kind.as_deref().is_none_or(|kind| placement.kind == kind)
                        && filter
                            .schema_state
                            .as_deref()
                            .is_none_or(|state| placement.schema_state == state)
                })
                .map(|placement| placement.id)
                .collect();
            select = select.filter(tenant::Column::PlacementId.is_in(ids));
        }
        // Thuê bao và hạn mức là của module `plans` (bước 35), hỏi qua tay cầm quyền dùng: chưa cài
        // `plans` thì không khách nào có thuê bao hay gần hạn mức — trang rỗng.
        if let Some(status) = &filter.subscription_status {
            let ids = self.entitlements.tenants_with_status(status).await?;
            select =
                select.filter(tenant::Column::Id.is_in(ids.into_iter().map(|id| id.as_uuid())));
        }
        if filter.near_limit == Some(true) {
            let ids = self.entitlements.tenants_near_limit().await?;
            select =
                select.filter(tenant::Column::Id.is_in(ids.into_iter().map(|id| id.as_uuid())));
        }
        let rows =
            fetch_page(select, tenant::Column::Id, |row| row.id, page, self.control().read())
                .await?;
        // Placement của trang, mô tả một lần mỗi cái (đọc nơi lưu riêng cần mở bản mã hoá).
        let mut described: HashMap<Uuid, Placement> = HashMap::new();
        if expand.contains(&"placement") {
            for row in &rows.data {
                if described.contains_key(&row.placement_id) {
                    continue;
                }
                if let Some(placement) =
                    placements.iter().find(|placement| placement.id == row.placement_id)
                {
                    let placement = describe_placement(&self.registry, placement.clone()).await;
                    described.insert(row.placement_id, placement);
                }
            }
        }
        // Thuê bao và lượng dùng: hỏi module `plans` (qua tay cầm quyền dùng) cho cả trang một lần.
        let ids: Vec<TenantId> = rows.data.iter().map(|row| TenantId::from_uuid(row.id)).collect();
        let mut plans = self
            .entitlements
            .describe_tenants(&ids, expand.contains(&"subscription"), expand.contains(&"usage"))
            .await?;
        Ok(rows.map(|row| {
            let placement = described.get(&row.placement_id).cloned();
            let view = plans.remove(&TenantId::from_uuid(row.id)).unwrap_or_default();
            let mut tenant = describe(row, &placements);
            if !expand.is_empty() {
                tenant.expanded = Some(TenantExpanded {
                    placement,
                    subscription: view.subscription,
                    usage: view.usage,
                });
            }
            tenant
        }))
    }

    async fn find(&self, id: Uuid) -> Result<tenant::Model, Error> {
        tenant::Entity::find_by_id(id)
            .one(self.control().read())
            .await?
            .ok_or_else(|| errors::tenant_not_found(&id.to_string()))
    }

    async fn find_slug(&self, slug: &str) -> Result<tenant::Model, Error> {
        tenant::Entity::find()
            .filter(tenant::Column::Slug.eq(slug))
            .one(self.control().read())
            .await?
            .ok_or_else(|| errors::tenant_not_found(slug))
    }

    /// A tenant by ID or slug.
    pub async fn get(&self, id_or_slug: &str) -> Result<Tenant, Error> {
        let row = match id_or_slug.parse::<Uuid>() {
            Ok(id) => self.find(id).await?,
            Err(_) => self.find_slug(id_or_slug).await?,
        };
        self.describe(row).await
    }

    /// Creates a tenant then provisions it (seeds every module's data); `single` mode: `tenant_single_mode`.
    pub async fn create(&self, actor: Actor, input: NewTenant) -> Result<Tenant, Error> {
        if self.registry.mode() == TenancyMode::Single {
            return Err(errors::tenant_single_mode());
        }
        let slug = input.slug.trim().to_owned();
        let name = input.name.trim().to_owned();
        let mut problems = Vec::new();
        if !is_slug(&slug) {
            problems.push(FieldError::new(
                "slug",
                "invalid",
                "must be 1 to 63 lower-case letters, digits and `-`, not starting or ending with `-`",
            ));
        }
        if name.is_empty() || name.chars().count() > 200 {
            problems.push(FieldError::new("name", "length", "must be 1 to 200 characters long"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let placements = self.placement_names().await?;
        let placement = match input.placement.as_deref() {
            None => MAIN_PLACEMENT.as_uuid(),
            Some(wanted) => {
                let found = placements
                    .iter()
                    .find(|placement| placement.name == wanted)
                    .ok_or_else(|| errors::tenant_placement_not_found(wanted))?;
                // Gieo dữ liệu vào placement thiếu bảng sẽ hỏng giữa chừng: báo cách sửa trước.
                if found.id != MAIN_PLACEMENT.as_uuid() && found.schema_state != "ready" {
                    return Err(errors::tenant_placement_behind(wanted));
                }
                found.id
            }
        };
        if let Some(parent) = input.parent_id {
            self.find(parent).await?;
        }
        let taken = tenant::Entity::find()
            .filter(tenant::Column::Slug.eq(slug.as_str()))
            .one(self.control().read())
            .await?;
        if taken.is_some() {
            return Err(errors::tenant_slug_taken(&slug));
        }
        let now = now();
        let id = Uuid::now_v7();
        let row = tenant::ActiveModel {
            id: Set(id),
            slug: Set(slug.clone()),
            name: Set(name),
            status: Set(TenantStatus::Provisioning.as_str().to_owned()),
            parent_id: Set(input.parent_id),
            placement_id: Set(placement),
            created_at: Set(now),
            updated_at: Set(now),
            read_only: Set(false),
            data_residency: Set(None),
        };
        // Hai yêu cầu cùng slug cùng lúc: khoá unique của bảng chặn bên sau.
        row.insert(self.control().write()).await.map_err(|error| {
            if matches!(error.sql_err(), Some(sea_orm::SqlErr::UniqueConstraintViolation(_))) {
                errors::tenant_slug_taken(&slug)
            } else {
                Error::from(error)
            }
        })?;
        tracing::info!(tenant_id = %id, slug = %slug, "tenant created; provisioning");
        self.provision_id(actor, id).await
    }

    /// Modules that have sample data for a new tenant (`Module::sample_data`), in registration order.
    pub fn sample_modules(&self) -> Vec<&'static str> {
        self.seeds.samples().iter().map(|(module, _)| *module).collect()
    }

    /// Seeds the sample data of `modules` (a module without sample data is skipped) for `tenant`:
    /// provisioning a new tenant by plan (step 35). Safe to rerun. Returns the seeded modules.
    pub async fn seed_samples(
        &self,
        tenant: TenantId,
        modules: &[&str],
    ) -> Result<Vec<&'static str>, Error> {
        let context = self.registry.tenant(tenant).await?;
        let mut seeded = Vec::new();
        for (module, seed) in self.seeds.samples() {
            if !modules.contains(module) {
                continue;
            }
            seed.seed(&context).await.map_err(|error| {
                tracing::error!(module, tenant_id = %tenant, error = %error, "sample data failed");
                error
            })?;
            seeded.push(*module);
        }
        Ok(seeded)
    }

    /// Reprovisions tenant `id_or_slug` that is `provisioning` (safe to rerun).
    pub async fn provision(&self, actor: Actor, id_or_slug: &str) -> Result<Tenant, Error> {
        let row = match id_or_slug.parse::<Uuid>() {
            Ok(id) => self.find(id).await?,
            Err(_) => self.find_slug(id_or_slug).await?,
        };
        self.provision_id(actor, row.id).await
    }

    async fn provision_id(&self, actor: Actor, id: Uuid) -> Result<Tenant, Error> {
        let tenant_id = TenantId::from_uuid(id);
        self.registry.invalidate(tenant_id);
        let context = self.registry.tenant(tenant_id).await?;
        for (module, seed) in self.seeds.all() {
            seed.seed(&context).await.map_err(|error| {
                tracing::error!(module, tenant_id = %id, error = %error, "seeding a tenant failed");
                error
            })?;
        }
        let row = self.find(id).await?;
        if row.status != TenantStatus::Provisioning.as_str() {
            return self.describe(row).await;
        }
        let mut active = row.into_active_model();
        active.status = Set(TenantStatus::Active.as_str().to_owned());
        active.updated_at = Set(now());
        let row = active.update(self.control().write()).await?;
        self.registry.invalidate(tenant_id);
        let tenant = self.describe(row).await?;
        let context = self.registry.tenant(tenant_id).await?;
        let event = TenantCreated(TenantChanged {
            before: None,
            after: Some(tenant.clone()),
            reason: None,
        });
        self.outbox
            .record(context.database().write(), &context, actor, Some(subject(id)), &event)
            .await?;
        tracing::info!(tenant_id = %id, "tenant provisioned");
        Ok(tenant)
    }

    /// Changes the name, status (`active` / `suspended`) or read-only flag of tenant `id`.
    pub async fn update(
        &self,
        actor: Actor,
        id: Uuid,
        changes: TenantChanges,
        expected: Option<Timestamp>,
    ) -> Result<Tenant, Error> {
        let row = self.find(id).await?;
        if row.status == TenantStatus::Deleted.as_str() {
            return Err(errors::tenant_not_found(&id.to_string()));
        }
        if expected.is_some_and(|expected| expected != row.updated_at) {
            return Err(crate::version::changed());
        }
        // Dữ liệu của khách đang được chép: đổi gì (kể cả mở lại) cũng ghi vào database cũ.
        if row.status == TenantStatus::Moving.as_str() {
            return Err(errors::tenant_moving());
        }
        let mut problems = Vec::new();
        let name = changes.name.map(|name| name.trim().to_owned());
        if name.as_ref().is_some_and(|name| name.is_empty() || name.chars().count() > 200) {
            problems.push(FieldError::new("name", "length", "must be 1 to 200 characters long"));
        }
        let status = changes.status.as_deref().map(TenantStatus::parse);
        match status {
            Some(Some(TenantStatus::Active | TenantStatus::Suspended)) | None => {}
            Some(_) => problems.push(FieldError::new(
                "status",
                "invalid",
                "must be `active` or `suspended`",
            )),
        }
        if id == DEFAULT_TENANT.as_uuid() && status == Some(Some(TenantStatus::Suspended)) {
            problems.push(FieldError::new(
                "status",
                "invalid",
                "the platform tenant cannot be suspended",
            ));
        }
        // Khách nền tảng chỉ đọc thì không ai gỡ được qua API (nhân sự nền tảng sống ở đó).
        if id == DEFAULT_TENANT.as_uuid() && changes.read_only == Some(true) {
            problems.push(FieldError::new(
                "read_only",
                "invalid",
                "the platform tenant cannot be made read-only",
            ));
        }
        let reason = changes
            .reason
            .map(|reason| reason.trim().to_owned())
            .filter(|reason| !reason.is_empty());
        if reason.as_ref().is_some_and(|reason| reason.chars().count() > 500) {
            problems.push(FieldError::new("reason", "length", "must be 1 to 500 characters long"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let before = self.describe(row.clone()).await?;
        let mut active = row.into_active_model();
        if let Some(name) = name {
            active.name = Set(name);
        }
        if let Some(Some(status)) = status {
            active.status = Set(status.as_str().to_owned());
        }
        if let Some(read_only) = changes.read_only {
            active.read_only = Set(read_only);
        }
        active.updated_at = Set(now());
        // Đúng phiên bản đã đọc (ghi trong điều kiện `UPDATE`): hai người sửa cùng lúc không đè nhau.
        let updated = tenant::Entity::update_many()
            .set(active)
            .filter(tenant::Column::Id.eq(id))
            .filter(tenant::Column::UpdatedAt.eq(before.updated_at))
            .exec(self.control().write())
            .await?;
        if updated.rows_affected == 0 {
            return Err(crate::version::changed());
        }
        let tenant_id = TenantId::from_uuid(id);
        self.registry.invalidate(tenant_id);
        let after = self.get(&id.to_string()).await?;
        let context = self.registry.tenant(tenant_id).await?;
        let event = TenantUpdated(TenantChanged {
            before: Some(before),
            after: Some(after.clone()),
            reason,
        });
        self.outbox
            .record(context.database().write(), &context, actor, Some(subject(id)), &event)
            .await?;
        tracing::info!(
            tenant_id = %id,
            status = %after.status,
            read_only = after.read_only,
            "tenant updated"
        );
        Ok(after)
    }

    /// Every placement, by name.
    pub async fn placements(&self) -> Result<Vec<Placement>, Error> {
        let mut rows = self.placement_names().await?;
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        let mut all = Vec::with_capacity(rows.len());
        for row in rows {
            all.push(describe_placement(&self.registry, row).await);
        }
        Ok(all)
    }
}

/// A placement for the API: never carries the URL or secret key of the store.
pub(super) async fn describe_placement(
    registry: &DatabaseRegistry,
    row: placement::Model,
) -> Placement {
    let storage = match &row.storage_sealed {
        None => None,
        Some(sealed) => {
            let id = PlacementId::from_uuid(row.id);
            Some(match registry.storage_location(id, sealed).await {
                Ok(location) => describe_storage(&location),
                Err(error) => {
                    tracing::warn!(placement = %row.name, error = %error, "cannot read the placement's file storage settings");
                    PlacementStorage {
                        kind: "unreadable".to_owned(),
                        dir: None,
                        bucket: None,
                        prefix: None,
                        region: None,
                        endpoint: None,
                        credentials: false,
                    }
                }
            })
        }
    };
    Placement {
        id: row.id,
        name: row.name,
        kind: row.kind,
        backend: row.backend,
        schema_state: row.schema_state,
        storage,
        country_code: row.country_code,
        created_at: row.created_at,
    }
}

/// The non-secret part of a dedicated store.
fn describe_storage(location: &crate::storage::StorageLocation) -> PlacementStorage {
    use crate::storage::StorageLocation;
    match location {
        StorageLocation::Local { dir } => PlacementStorage {
            kind: "local".to_owned(),
            dir: Some(dir.clone()),
            bucket: None,
            prefix: None,
            region: None,
            endpoint: None,
            credentials: false,
        },
        StorageLocation::S3(s3) => PlacementStorage {
            kind: "s3".to_owned(),
            dir: None,
            bucket: Some(s3.bucket.clone()),
            prefix: s3.prefix.clone(),
            region: Some(s3.region.clone()),
            endpoint: s3.endpoint.clone(),
            credentials: s3.access_key_id.is_some(),
        },
    }
}

fn subject(id: Uuid) -> String {
    format!("kernel.tenant:{id}")
}

fn describe(row: tenant::Model, placements: &[placement::Model]) -> Tenant {
    let placement = placements
        .iter()
        .find(|placement| placement.id == row.placement_id)
        .map_or_else(|| PlacementId::from_uuid(row.placement_id).to_string(), |p| p.name.clone());
    Tenant {
        id: row.id,
        slug: row.slug,
        name: row.name,
        status: row.status,
        read_only: row.read_only,
        parent_id: row.parent_id,
        placement,
        created_at: row.created_at,
        updated_at: row.updated_at,
        expanded: None,
    }
}

// --- REST -----------------------------------------------------------------------------------

/// Get my tenant.
///
/// Requires signing in, no permission. The tenant the request is for — the one of the caller, or
/// the one entered with `Autoxide-Tenant` by platform staff — and the mode of the installation.
#[utoipa::path(
    get,
    path = "/tenant",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_current_tenant",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The tenant of the request.", body = Data<CurrentTenant>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
    )
)]
async fn get_current_tenant(
    State(admin): State<Arc<TenantAdmin>>,
    tenant: TenantContext,
    _principal: Principal,
) -> Data<CurrentTenant> {
    Data { data: admin.current(&tenant) }
}

/// List the tenants of the platform.
///
/// Requires `kernel.tenants.read` and an account of the platform tenant (not one entered with
/// `Autoxide-Tenant`). Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/tenants",
    extensions(("x-permissions" = json!(["kernel.tenants.read"]))),
    operation_id = "list_tenants",
    security(("bearer" = [])),
    params(PageQuery, TenantFilter),
    responses(
        (status = 200, description = "One page of tenants.", body = Page<Tenant>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.read` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_tenants(
    State(admin): State<Arc<TenantAdmin>>,
    principal: Principal,
    Query(page): Query<PageQuery>,
    Query(filter): Query<TenantFilter>,
) -> Result<Page<Tenant>, Error> {
    principal.require(&TENANTS_READ)?;
    require_platform(&principal)?;
    admin.list(&filter, &PageRequest::try_from(page)?).await
}

/// Create a tenant.
///
/// Requires `kernel.tenants.manage` and an account of the platform tenant. Creates the tenant and
/// prepares it (every module sets up what a new tenant needs) before answering. Retry safely with
/// an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/tenants",
    extensions(("x-permissions" = json!(["kernel.tenants.manage"]))),
    operation_id = "create_tenant",
    security(("bearer" = [])),
    request_body = NewTenant,
    responses(
        (status = 201, description = "The tenant, ready to use.", body = Data<Tenant>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.manage` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such placement or parent; code `tenant_placement_not_found` or `tenant_not_found`.", body = Problem),
        (status = 409, description = "The slug is taken (`tenant_slug_taken`), the installation runs one tenant (`tenant_single_mode`), or the placement has migrations to apply (`tenant_placement_behind`).", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn create_tenant(
    State(admin): State<Arc<TenantAdmin>>,
    principal: Principal,
    crate::http::ValidJson(input): crate::http::ValidJson<NewTenant>,
) -> Result<Created<Tenant>, Error> {
    principal.require(&TENANTS_MANAGE)?;
    require_platform(&principal)?;
    let tenant = admin.create(Actor::from(&principal), input).await?;
    Ok(Created::new(format!("/kernel/tenants/{}", tenant.id), tenant))
}

/// Get a tenant.
///
/// Requires `kernel.tenants.read` and an account of the platform tenant.
#[utoipa::path(
    get,
    path = "/tenants/{id}",
    extensions(("x-permissions" = json!(["kernel.tenants.read"]))),
    operation_id = "get_tenant",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant.")),
    responses(
        (status = 200, description = "The tenant.", body = Data<Tenant>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.read` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such tenant; code `tenant_not_found`.", body = Problem),
    )
)]
async fn get_tenant(
    State(admin): State<Arc<TenantAdmin>>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Tagged<Tenant>, Error> {
    principal.require(&TENANTS_READ)?;
    require_platform(&principal)?;
    let tenant = admin.get(&id).await?;
    Ok(Tagged { version: tenant.updated_at, data: tenant })
}

/// Rename, suspend or resume a tenant.
///
/// Requires `kernel.tenants.manage` and an account of the platform tenant. A suspended tenant
/// answers every request with `403 tenant_suspended` (other servers see the change within
/// `AUTOXIDE_TENANCY_CACHE_SECONDS`). The platform tenant cannot be suspended.
#[utoipa::path(
    patch,
    path = "/tenants/{id}",
    extensions(("x-permissions" = json!(["kernel.tenants.manage"]))),
    operation_id = "update_tenant",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read. When given, the change is made only if nobody changed the record since; otherwise `412`.")),
    request_body = TenantChanges,
    responses(
        (status = 200, description = "The tenant after the change.", body = Data<Tenant>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.manage` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such tenant; code `tenant_not_found`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the record changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem is listed in `errors` (code `validation_failed`). Making the platform tenant read-only or suspending it is refused this way, on the field `read_only` or `status`.", body = Problem),
    )
)]
async fn update_tenant(
    State(admin): State<Arc<TenantAdmin>>,
    principal: Principal,
    Path(id): Path<String>,
    if_match: IfMatch,
    crate::http::ValidJson(changes): crate::http::ValidJson<TenantChanges>,
) -> Result<Tagged<Tenant>, Error> {
    principal.require(&TENANTS_MANAGE)?;
    require_platform(&principal)?;
    let expected = if_match.expected()?;
    // ID hay slug, như `GET /tenants/{id}`, `…/settings` và `…/move`.
    let id = admin.get(&id).await?.id;
    let tenant = admin.update(Actor::from(&principal), id, changes, expected).await?;
    Ok(Tagged { version: tenant.updated_at, data: tenant })
}

/// Routes under `/api/v1/kernel`.
pub(crate) fn routes(admin: Arc<TenantAdmin>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(get_current_tenant))
        .routes(routes!(list_tenants, create_tenant))
        .routes(routes!(get_tenant, update_tenant))
        .routes(routes!(placement_list::list_placements))
        .merge(super::erasure::routes())
        .with_state(admin)
}

// Đầu vào của tool `get_tenant`.
/// Which tenant to get.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TenantRef {
    /// Id or slug of the tenant.
    #[validate(length(min = 1, max = 63))]
    tenant: String,
}

/// MCP tools: call exactly the functions REST uses.
pub(crate) fn tools(admin: Arc<TenantAdmin>) -> Vec<Tool> {
    let (current, list, get) = (admin.clone(), admin.clone(), admin);
    vec![
        Tool::new(
            "current_tenant",
            "Tell which tenant (customer) this request is for, and whether the installation runs \
             one tenant or many.",
            move |tenant: TenantContext, _principal: Principal, _: NoInput| {
                let current = current.clone();
                async move { Ok::<_, Error>(current.current(&tenant)) }
            },
        )
        .for_any_signed_in()
        .read_only(),
        Tool::new(
            "list_tenants",
            "List the tenants of the platform, newest first (platform staff only).",
            move |_tenant: TenantContext, principal: Principal, input: crate::mcp::PageInput| {
                let list = list.clone();
                async move {
                    require_platform(&principal)?;
                    list.list(&TenantFilter::default(), &input.page()?).await
                }
            },
        )
        .requires(TENANTS_READ)
        .read_only(),
        Tool::new(
            "get_tenant",
            "Get one tenant of the platform by id or slug (platform staff only).",
            move |_tenant: TenantContext, principal: Principal, input: TenantRef| {
                let get = get.clone();
                async move {
                    require_platform(&principal)?;
                    get.get(&input.tenant).await
                }
            },
        )
        .requires(TENANTS_READ)
        .read_only(),
    ]
}

// --- Migration ------------------------------------------------------------------------------

/// The `read_only` column of `kernel_tenants`: a read-only tenant (reads work, writes are blocked),
/// independent of `status` (suspension blocks reads too); see design 2, A-3.11.
pub(crate) struct AddReadOnly;

impl crate::migration::prelude::MigrationName for AddReadOnly {
    fn name(&self) -> &str {
        "m0013_tenant_read_only"
    }
}

#[async_trait::async_trait]
impl crate::migration::prelude::MigrationTrait for AddReadOnly {
    async fn up(
        &self,
        manager: &crate::migration::prelude::SchemaManager,
    ) -> Result<(), crate::migration::prelude::DbErr> {
        use crate::migration::prelude::*;
        manager
            .alter_table(
                Table::alter()
                    .table("kernel_tenants")
                    .add_column(schema::boolean("read_only").default(false))
                    .to_owned(),
            )
            .await
    }

    async fn down(
        &self,
        manager: &crate::migration::prelude::SchemaManager,
    ) -> Result<(), crate::migration::prelude::DbErr> {
        use crate::migration::prelude::*;
        manager
            .alter_table(Table::alter().table("kernel_tenants").drop_column("read_only").to_owned())
            .await
    }
}
