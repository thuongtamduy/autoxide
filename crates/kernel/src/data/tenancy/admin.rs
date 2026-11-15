//! Quản lý khách (đặc tả 33 "Cấp phát và dời khách", "API REST", D-260): tạo, gieo dữ liệu ban
//! đầu, xem, tạm khoá / mở lại; khách hiện tại của người gọi.
//!
//! - Tạo khách: ghi `kernel_tenants` trạng thái `provisioning` → gọi `TenantSeed` của mọi module
//!   theo thứ tự đăng ký (gieo role `admin`, dòng khoá, đầu chuỗi audit…) → `active`. Hỏng giữa
//!   chừng thì khách giữ `provisioning`; `autoxide tenant provision <slug>` chạy lại (mọi bước chạy
//!   lại được).
//! - Quản lý khách là việc của **nhân sự khách nền tảng** (khách mặc định), không phải của admin một
//!   khách thường: role `admin` của khách nào cũng có mọi permission, nên chỉ kiểm permission là lỗ
//!   hổng — kiểm thêm khách của người gọi ([`require_platform`]), và không cho nhân sự nền tảng
//!   đang "vào" khách khác bằng header làm việc này từ trong khách đó.
//! - Chế độ `single`: chỉ có khách mặc định; tạo khách là lỗi `tenant_single_mode`.

use std::sync::{Arc, OnceLock};

use axum::extract::{Path, Query, State};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::{
    DEFAULT_TENANT, DatabaseRegistry, MAIN_PLACEMENT, PlacementId, TenancyMode, TenantContext,
    TenantId, TenantStatus, errors,
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

/// Gieo dữ liệu ban đầu của một module cho khách mới (vd role `admin`, dòng khoá). Phải chạy lại
/// được: khách cấp phát hỏng giữa chừng thì chạy lại từ đầu.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait TenantSeed: Send + Sync {
    async fn seed(&self, tenant: &TenantContext) -> Result<(), Error>;
}

/// Bộ gieo của một module, kèm tên module (để báo lỗi rõ module nào hỏng).
pub(crate) type NamedSeed = (&'static str, Arc<dyn TenantSeed>);

/// Bộ gieo của mọi module, theo thứ tự đăng ký; điền sau khi mọi module đã khởi động. Clone rẻ.
#[derive(Clone, Default)]
pub(crate) struct Seeds(Arc<OnceLock<Vec<NamedSeed>>>);

impl Seeds {
    pub(crate) fn fill(&self, seeds: Vec<NamedSeed>) {
        let _ = self.0.set(seeds);
    }

    fn all(&self) -> &[NamedSeed] {
        self.0.get().map_or(&[], Vec::as_slice)
    }
}

/// Người gọi là nhân sự khách nền tảng, đang làm việc trong chính khách nền tảng.
pub fn require_platform(principal: &Principal) -> Result<(), Error> {
    if principal.tenant_id() == DEFAULT_TENANT && principal.operator_from().is_none() {
        Ok(())
    } else {
        Err(errors::tenant_platform_only())
    }
}

mod tenant {
    //! Bảng `kernel_tenants` (mặt điều khiển).
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
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(super) mod placement {
    //! Bảng `kernel_placements` (mặt điều khiển).
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
        pub schema_checked_at: Option<DateTimeUtc>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
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
    /// `provisioning`, `active`, `suspended`, `maintenance` or `moving`.
    #[schema(example = "active")]
    pub status: String,
    /// Parent tenant, when the tenant belongs to a chain.
    pub parent_id: Option<Uuid>,
    /// Name of the placement (database) that keeps its data.
    #[schema(example = "main")]
    pub placement: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The tenant of the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CurrentTenant {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub status: String,
    /// `single` (one tenant) or `multi` (many tenants on one installation).
    #[schema(example = "single")]
    pub mode: String,
    /// Whether this is the platform tenant (the operator's own).
    pub platform: bool,
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
    pub created_at: DateTime<Utc>,
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
    /// `active` or `suspended`.
    #[schema(example = "suspended")]
    pub status: Option<String>,
}

/// Which tenants to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TenantFilter {
    /// Only tenants in this state.
    pub status: Option<String>,
    /// Only children of this tenant.
    pub parent_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct TenantChanged {
    before: Option<Tenant>,
    after: Option<Tenant>,
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

/// Dịch vụ quản lý khách. Module `kernel` công bố (`context.require::<TenantAdmin>()`).
pub struct TenantAdmin {
    registry: DatabaseRegistry,
    seeds: Seeds,
    outbox: Outbox,
}

impl std::fmt::Debug for TenantAdmin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TenantAdmin").finish_non_exhaustive()
    }
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

pub(super) fn is_slug(slug: &str) -> bool {
    (1..=63).contains(&slug.len())
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl TenantAdmin {
    pub(crate) fn new(registry: DatabaseRegistry, seeds: Seeds, outbox: Outbox) -> Self {
        TenantAdmin { registry, seeds, outbox }
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

    /// Khách của người gọi.
    pub fn current(&self, tenant: &TenantContext) -> CurrentTenant {
        CurrentTenant {
            id: tenant.id().as_uuid(),
            slug: tenant.slug().to_owned(),
            name: tenant.name().to_owned(),
            status: tenant.status().as_str().to_owned(),
            mode: match self.registry.mode() {
                TenancyMode::Single => "single",
                TenancyMode::Multi => "multi",
            }
            .to_owned(),
            platform: tenant.is_platform(),
        }
    }

    /// Một trang khách, mới nhất trước (trừ khi `order=asc`).
    pub async fn list(
        &self,
        filter: &TenantFilter,
        page: &PageRequest,
    ) -> Result<Page<Tenant>, Error> {
        let mut select = tenant::Entity::find();
        if let Some(status) = &filter.status {
            if TenantStatus::parse(status).is_none() {
                return Err(Error::validation(vec![FieldError::new(
                    "status",
                    "invalid",
                    "must be one of provisioning, active, suspended, maintenance, moving",
                )]));
            }
            select = select.filter(tenant::Column::Status.eq(status.as_str()));
        }
        if let Some(parent) = filter.parent_id {
            select = select.filter(tenant::Column::ParentId.eq(parent));
        }
        let rows =
            fetch_page(select, tenant::Column::Id, |row| row.id, page, self.control().read())
                .await?;
        let placements = self.placement_names().await?;
        Ok(rows.map(|row| describe(row, &placements)))
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

    /// Khách theo ID hay slug.
    pub async fn get(&self, id_or_slug: &str) -> Result<Tenant, Error> {
        let row = match id_or_slug.parse::<Uuid>() {
            Ok(id) => self.find(id).await?,
            Err(_) => self.find_slug(id_or_slug).await?,
        };
        self.describe(row).await
    }

    /// Tạo khách rồi cấp phát (gieo dữ liệu của mọi module). Chế độ `single`: `tenant_single_mode`.
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

    /// Cấp phát lại khách `id_or_slug` đang `provisioning` (chạy lại được).
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
        let event = TenantCreated(TenantChanged { before: None, after: Some(tenant.clone()) });
        self.outbox
            .record(context.database().write(), &context, actor, Some(subject(id)), &event)
            .await?;
        tracing::info!(tenant_id = %id, "tenant provisioned");
        Ok(tenant)
    }

    /// Đổi tên hay trạng thái (`active` / `suspended`) của khách `id`.
    pub async fn update(
        &self,
        actor: Actor,
        id: Uuid,
        changes: TenantChanges,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Tenant, Error> {
        let row = self.find(id).await?;
        if expected.is_some_and(|expected| expected != row.updated_at) {
            return Err(crate::version::changed());
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
        let event =
            TenantUpdated(TenantChanged { before: Some(before), after: Some(after.clone()) });
        self.outbox
            .record(context.database().write(), &context, actor, Some(subject(id)), &event)
            .await?;
        tracing::info!(tenant_id = %id, status = %after.status, "tenant updated");
        Ok(after)
    }

    /// Mọi placement, theo tên.
    pub async fn placements(&self) -> Result<Vec<Placement>, Error> {
        let mut rows = self.placement_names().await?;
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(rows.into_iter().map(describe_placement).collect())
    }
}

pub(super) fn describe_placement(row: placement::Model) -> Placement {
    Placement {
        id: row.id,
        name: row.name,
        kind: row.kind,
        backend: row.backend,
        schema_state: row.schema_state,
        created_at: row.created_at,
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
        parent_id: row.parent_id,
        placement,
        created_at: row.created_at,
        updated_at: row.updated_at,
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
    params(("id" = Uuid, Path, description = "Id of the tenant."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read. When given, the change is made only if nobody changed the record since; otherwise `412`.")),
    request_body = TenantChanges,
    responses(
        (status = 200, description = "The tenant after the change.", body = Data<Tenant>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.manage` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such tenant; code `tenant_not_found`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the record changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn update_tenant(
    State(admin): State<Arc<TenantAdmin>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    crate::http::ValidJson(changes): crate::http::ValidJson<TenantChanges>,
) -> Result<Tagged<Tenant>, Error> {
    principal.require(&TENANTS_MANAGE)?;
    require_platform(&principal)?;
    let expected = if_match.expected()?;
    let tenant = admin.update(Actor::from(&principal), id, changes, expected).await?;
    Ok(Tagged { version: tenant.updated_at, data: tenant })
}

/// List the placements.
///
/// Requires `kernel.placements.read` and an account of the platform tenant. Where tenants' data
/// is kept; connection URLs are never shown (placements are added with `autoxide placement add`).
#[utoipa::path(
    get,
    path = "/placements",
    extensions(("x-permissions" = json!(["kernel.placements.read"]))),
    operation_id = "list_placements",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every placement, sorted by name.", body = Data<Vec<Placement>>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.placements.read` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
    )
)]
async fn list_placements(
    State(admin): State<Arc<TenantAdmin>>,
    principal: Principal,
) -> Result<Data<Vec<Placement>>, Error> {
    principal.require(&PLACEMENTS_READ)?;
    require_platform(&principal)?;
    Ok(Data { data: admin.placements().await? })
}

/// Route dưới `/api/v1/kernel`.
pub(crate) fn routes(admin: Arc<TenantAdmin>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(get_current_tenant))
        .routes(routes!(list_tenants, create_tenant))
        .routes(routes!(get_tenant, update_tenant))
        .routes(routes!(list_placements))
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

/// Tool MCP — gọi đúng các hàm REST dùng.
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
