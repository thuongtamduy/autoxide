//! Service của sandbox: tạo, gia hạn, xoá, hết hạn; job sao cấu hình rồi kích hoạt. REST và tool
//! MCP cùng gọi vào đây.

use crate::time::Timestamp;
use jiff::SignedDuration;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::entity;
use super::{
    EXPIRE_SANDBOXES, SANDBOX_COPY, SANDBOX_DAYS, SANDBOX_MAX_PRODUCTS, SANDBOXES_LIMIT,
    SANDBOXES_MANAGE, SANDBOXES_READ, TenantKind, sandbox_nested, sandbox_not_found,
};
use crate::access::Principal;
use crate::config::{ConfigReader, Invalid};
use crate::entitlement::EntitlementsHandle;
use crate::error::{Error, FieldError};
use crate::event::{Actor, Event, Outbox};
use crate::job::{EnqueueOptions, JobContext, JobHandler, JobQueue, JobRegistration, Schedule};
use crate::module::StartContext;
use crate::pagination::{Page, PageRequest, fetch_page};
use crate::tenancy::{
    DatabaseRegistry, TENANTS_MANAGE, TenantAdmin, TenantChanges, TenantContext, TenantId,
    TenantStatus, errors, require_platform,
};

pub(super) fn now() -> Timestamp {
    Timestamp::now()
}

/// A sandbox: a test copy of a tenant with its configuration and no personal data. Partners are
/// always simulated there and mail and webhooks do not leave it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Sandbox {
    /// Id of the sandbox tenant: send it in `Autoxide-Tenant` to work inside it.
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    /// `provisioning` (being copied), `active`, `suspended` (expired, being deleted) or `deleted`.
    #[schema(example = "active")]
    pub status: String,
    /// The real tenant it was made from.
    pub sandbox_of: Uuid,
    /// When it is deleted unless extended.
    pub expires_at: Option<Timestamp>,
    /// Products asked to copy.
    pub products: u32,
    /// Role of the real tenant whose holders may enter the sandbox; `null`: the `admin` role.
    pub role_id: Option<Uuid>,
    pub created_at: Timestamp,
}

/// A new sandbox and the job copying the configuration into it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SandboxCreated {
    #[serde(flatten)]
    pub sandbox: Sandbox,
    /// The background job copying the configuration; follow it at
    /// `GET /api/v1/kernel/jobs/{id}`. The sandbox becomes `active` when it is done.
    pub job_id: Option<Uuid>,
}

/// A new sandbox.
#[derive(Debug, Clone, Default, Deserialize, ToSchema, validator::Validate)]
#[serde(deny_unknown_fields)]
pub struct NewSandbox {
    /// 1 to 200 characters.
    #[schema(example = "Try the new theme")]
    pub name: String,
    /// Products to copy (0 by default), at most `AUTOXIDE_TENANCY_SANDBOX_MAX_PRODUCTS`.
    pub products: Option<u32>,
    /// Role of your tenant whose holders may enter the sandbox; the `admin` role when omitted.
    pub role_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
struct Options {
    #[serde(default)]
    products: u32,
    #[serde(default)]
    role_id: Option<Uuid>,
}

fn options_of(row: &entity::Model) -> Options {
    row.sandbox_options
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default()
}

fn describe(row: &entity::Model) -> Sandbox {
    let options = options_of(row);
    Sandbox {
        id: row.id,
        slug: row.slug.clone(),
        name: row.name.clone(),
        status: row.status.clone(),
        sandbox_of: row.sandbox_of.unwrap_or_default(),
        expires_at: row.sandbox_expires_at,
        products: options.products,
        role_id: options.role_id,
        created_at: row.created_at,
    }
}

/// Payload chung của event sandbox (ghi ở khách gốc).
#[derive(Debug, Serialize)]
struct SandboxChanged {
    sandbox_id: Uuid,
    sandbox_of: Uuid,
    expires_at: Option<Timestamp>,
    role_id: Option<Uuid>,
}

macro_rules! event {
    ($kind:ident, $name:literal) => {
        #[derive(Debug, Serialize)]
        #[serde(transparent)]
        struct $kind(SandboxChanged);

        impl Event for $kind {
            const NAME: &'static str = $name;
        }
    };
}

event!(SandboxCreatedEvent, "kernel.sandbox_created");
event!(SandboxExtended, "kernel.sandbox_extended");
event!(SandboxDeleted, "kernel.sandbox_deleted");

fn changed(row: &entity::Model) -> SandboxChanged {
    SandboxChanged {
        sandbox_id: row.id,
        sandbox_of: row.sandbox_of.unwrap_or_default(),
        expires_at: row.sandbox_expires_at,
        role_id: options_of(row).role_id,
    }
}

/// Sandbox của khách: tạo, gia hạn, xoá, hết hạn.
pub struct TenantSandboxes {
    pub(super) registry: DatabaseRegistry,
    admin: Arc<TenantAdmin>,
    pub(super) outbox: Outbox,
    jobs: JobQueue,
    entitlements: EntitlementsHandle,
    days: i64,
    max_products: u32,
    /// Sao / đẩy của mọi module (điền sau khi mọi module khởi động).
    pub(super) catalog: super::copy::SandboxCatalog,
}

impl TenantSandboxes {
    fn control(&self) -> &crate::database::Database {
        self.registry.control()
    }

    /// Khách gốc `id` mà người gọi (ở khách `tenant`) được quản lý sandbox: chính khách của họ, hay
    /// nhân sự nền tảng có `kernel.tenants.manage`. Khác: `tenant_not_found` (không lộ khách).
    async fn source(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<TenantContext, Error> {
        if tenant.id().as_uuid() == id {
            return Ok(tenant.clone());
        }
        if require_platform(principal).is_ok() && principal.has(&TENANTS_MANAGE) {
            return self.registry.tenant(TenantId::from_uuid(id)).await;
        }
        Err(errors::tenant_not_found(&id.to_string()))
    }

    /// Dòng sandbox `id` của khách người gọi (hay nhân sự nền tảng); khác → `tenancy.sandbox_not_found`.
    pub(super) async fn owned(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<entity::Model, Error> {
        let row = entity::Entity::find_by_id(id)
            .filter(entity::Column::Kind.eq(TenantKind::Sandbox.as_str()))
            .filter(entity::Column::Status.ne(TenantStatus::Deleted.as_str()))
            .one(self.control().read())
            .await?
            .ok_or_else(|| sandbox_not_found(id))?;
        let mine = row.sandbox_of == Some(tenant.id().as_uuid());
        let staff = require_platform(principal).is_ok() && principal.has(&TENANTS_MANAGE);
        if mine || staff { Ok(row) } else { Err(sandbox_not_found(id)) }
    }

    /// Một trang sandbox (chưa xoá) của khách `source_id`, mới nhất trước.
    pub async fn list(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        source_id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<Sandbox>, Error> {
        principal.require(&SANDBOXES_READ)?;
        let source = self.source(tenant, principal, source_id).await?;
        let select = entity::Entity::find()
            .filter(entity::Column::Kind.eq(TenantKind::Sandbox.as_str()))
            .filter(entity::Column::SandboxOf.eq(source.id().as_uuid()))
            .filter(entity::Column::Status.ne(TenantStatus::Deleted.as_str()));
        let page =
            fetch_page(select, entity::Column::Id, |row| row.id, page, self.control().read())
                .await?;
        Ok(page.map(|row| describe(&row)))
    }

    /// Số sandbox khách được giữ cùng lúc: mức của gói, gói không đặt (hay không thuê bao) thì 1.
    async fn allowed(&self, source: TenantId) -> Result<Option<i64>, Error> {
        let snapshot = self.entitlements.snapshot(source).await?;
        if snapshot.unrestricted {
            return Ok(Some(1));
        }
        Ok(match snapshot.limits.get(SANDBOXES_LIMIT.key()) {
            None => Some(1),
            Some(limit) => *limit,
        })
    }

    /// Tạo sandbox của khách `source_id`: dòng `provisioning` cùng placement với khách gốc, rồi job
    /// `kernel.sandbox_copy` gieo dữ liệu ban đầu, sao cấu hình và kích hoạt.
    pub async fn create(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        source_id: Uuid,
        input: NewSandbox,
    ) -> Result<SandboxCreated, Error> {
        principal.require(&SANDBOXES_MANAGE)?;
        if tenant.is_sandbox() {
            return Err(sandbox_nested());
        }
        // Sandbox sao dữ liệu của khách (cấu hình, sản phẩm): chỉ tạo được từ BÊN TRONG khách đó —
        // nhân sự nền tảng phải vào khách bằng lần cấp quyền vào hỗ trợ trước, như xuất dữ liệu
        // (D-313, D-20261121-export-entry); xem danh sách, gia hạn, xoá vẫn làm từ khách nền tảng.
        if tenant.id().as_uuid() != source_id {
            if require_platform(principal).is_ok() && principal.has(&TENANTS_MANAGE) {
                return Err(super::sandbox_needs_tenant_access(source_id));
            }
            return Err(errors::tenant_not_found(&source_id.to_string()));
        }
        let source = tenant.clone();
        if source.is_sandbox() {
            return Err(sandbox_nested());
        }
        if source.is_platform() {
            return Err(super::sandbox_forbidden("sandbox_of_platform"));
        }
        let name = input.name.trim().to_owned();
        let products = input.products.unwrap_or(0);
        let mut problems = Vec::new();
        if name.is_empty() || name.chars().count() > 200 {
            problems.push(FieldError::new("name", "length", "must be 1 to 200 characters long"));
        }
        if products > self.max_products {
            problems.push(
                FieldError::new("products", "range", "must be at most {max}")
                    .with_param("max", self.max_products),
            );
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let used = entity::Entity::find()
            .filter(entity::Column::Kind.eq(TenantKind::Sandbox.as_str()))
            .filter(entity::Column::SandboxOf.eq(source.id().as_uuid()))
            .filter(entity::Column::Status.ne(TenantStatus::Deleted.as_str()))
            .count(self.control().read())
            .await?;
        let used = i64::try_from(used).unwrap_or(i64::MAX);
        if let Some(max) = self.allowed(source.id()).await?
            && used >= max
        {
            return Err(crate::entitlement::limit_exceeded(SANDBOXES_LIMIT.key(), used, max));
        }
        let now = now();
        let id = Uuid::now_v7();
        let simple = id.simple().to_string();
        let prefix: String = source.slug().chars().take(40).collect();
        let slug = format!("{}-sandbox-{}", prefix.trim_end_matches('-'), &simple[24..]);
        let options = Options { products, role_id: input.role_id };
        let row = entity::ActiveModel {
            id: Set(id),
            slug: Set(slug),
            name: Set(name),
            status: Set(TenantStatus::Provisioning.as_str().to_owned()),
            parent_id: Set(None),
            placement_id: Set(source.placement().as_uuid()),
            created_at: Set(now),
            updated_at: Set(now),
            read_only: Set(false),
            kind: Set(TenantKind::Sandbox.as_str().to_owned()),
            sandbox_of: Set(Some(source.id().as_uuid())),
            sandbox_expires_at: Set(Some(now + SignedDuration::from_hours(24 * self.days))),
            sandbox_options: Set(Some(serde_json::to_string(&options).map_err(Error::internal)?)),
        }
        .insert(self.control().write())
        .await?;
        let job_id = self.enqueue_copy(&source, principal, &row).await?;
        tracing::info!(sandbox_id = %row.id, tenant_id = %source.id(), "sandbox created; copying");
        Ok(SandboxCreated { sandbox: describe(&row), job_id })
    }

    /// Xếp job sao cấu hình ở database của khách gốc, kèm dòng theo dõi (job API chung).
    async fn enqueue_copy(
        &self,
        source: &TenantContext,
        principal: &Principal,
        row: &entity::Model,
    ) -> Result<Option<Uuid>, Error> {
        let transaction = source.database().begin_write().await?;
        let options = EnqueueOptions::default().unique_key(format!("sandbox_copy:{}", row.id));
        let payload = json!({"sandbox_id": row.id});
        let job = self
            .jobs
            .enqueue_value(&transaction, source.id(), SANDBOX_COPY, payload, options)
            .await?;
        if let Some(job) = job {
            let label = format!("Copy of the configuration into the sandbox “{}”", row.name);
            let by = Some(principal.user_id());
            crate::job::runs::track(
                &transaction,
                source,
                "kernel",
                job,
                SANDBOX_COPY,
                &label,
                by,
                None,
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(job)
    }

    /// Job `kernel.sandbox_copy`: gieo dữ liệu ban đầu của mọi module (như khách mới), kích hoạt,
    /// ghi `kernel.sandbox_created` ở khách gốc. Chạy lại được (bên nghe event chịu nhận lại).
    async fn copy(&self, sandbox_id: Uuid) -> Result<(), Error> {
        let Some(row) = entity::Entity::find_by_id(sandbox_id).one(self.control().read()).await?
        else {
            return Ok(());
        };
        if row.status == TenantStatus::Deleted.as_str() {
            return Ok(());
        }
        let source_id = row.sandbox_of.ok_or_else(|| Error::internal("a sandbox has no source"))?;
        let source = self.registry.tenant(TenantId::from_uuid(source_id)).await?;
        self.admin.provision(Actor::system(), &sandbox_id.to_string()).await?;
        // Sao cấu hình trước khi ghi `kernel.sandbox_created`: nhân viên chỉ vào được (uỷ quyền tạo
        // khi nghe event) khi bản sao đã đủ. Chạy lại được: bản đã sao bị bỏ qua.
        self.registry.invalidate(TenantId::from_uuid(sandbox_id));
        let sandbox = self.registry.tenant(TenantId::from_uuid(sandbox_id)).await?;
        let options = super::copy::SandboxOptions { products: options_of(&row).products };
        self.copy_configuration(&source, &sandbox, options).await?;
        let event = SandboxCreatedEvent(changed(&row));
        self.record(&source, Actor::system(), row.id, &event).await?;
        tracing::info!(sandbox_id = %sandbox_id, "sandbox ready");
        Ok(())
    }

    pub(super) async fn record<E: Event>(
        &self,
        source: &TenantContext,
        actor: Actor,
        id: Uuid,
        event: &E,
    ) -> Result<(), Error> {
        let subject = Some(format!("kernel.sandbox:{id}"));
        self.outbox.record(source.database().write(), source, actor, subject, event).await
    }

    /// Gia hạn sandbox `id`: hết hạn lại sau `AUTOXIDE_TENANCY_SANDBOX_DAYS` ngày tính từ bây giờ.
    pub async fn extend(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Sandbox, Error> {
        principal.require(&SANDBOXES_MANAGE)?;
        let row = self.owned(tenant, principal, id).await?;
        let mut active = row.into_active_model();
        active.sandbox_expires_at = Set(Some(now() + SignedDuration::from_hours(24 * self.days)));
        active.updated_at = Set(now());
        let row = active.update(self.control().write()).await?;
        self.registry.invalidate(TenantId::from_uuid(id));
        let source = self.registry.tenant(TenantId::from_uuid(row.sandbox_of.unwrap_or_default()));
        self.record(&source.await?, Actor::from(principal), id, &SandboxExtended(changed(&row)))
            .await?;
        Ok(describe(&row))
    }

    /// Xoá ngay sandbox `id` theo luồng xoá khách (tạm khoá rồi xoá hẳn dữ liệu).
    pub async fn delete(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&SANDBOXES_MANAGE)?;
        let row = self.owned(tenant, principal, id).await?;
        self.remove(row, Actor::from(principal), "Sandbox deleted").await
    }

    async fn remove(&self, row: entity::Model, actor: Actor, reason: &str) -> Result<(), Error> {
        let id = row.id;
        if row.status != TenantStatus::Suspended.as_str() {
            let changes = TenantChanges {
                status: Some(TenantStatus::Suspended.as_str().to_owned()),
                reason: Some(reason.to_owned()),
                ..TenantChanges::default()
            };
            self.admin.update(actor, id, changes, None).await?;
        }
        self.admin.erase(actor, &id.to_string(), reason).await?;
        if let Some(source) = row.sandbox_of {
            match self.registry.tenant(TenantId::from_uuid(source)).await {
                Ok(source) => {
                    self.record(&source, actor, id, &SandboxDeleted(changed(&row))).await?
                }
                // Khách gốc đã xoá: không còn chỗ ghi event, cũng không còn ai cần thu hồi.
                Err(error) => tracing::warn!(%error, sandbox_id = %id, "sandbox source is gone"),
            }
        }
        tracing::info!(sandbox_id = %id, reason, "sandbox deleted");
        Ok(())
    }

    /// Lịch `kernel.expire_sandboxes`: xoá sandbox quá hạn, và sandbox mà khách gốc đã xoá / gộp.
    /// Trả số sandbox đã xoá (lệnh vận hành và test gọi thẳng được).
    pub async fn expire_due(&self) -> Result<u64, Error> {
        let now = now();
        let rows = entity::Entity::find()
            .filter(entity::Column::Kind.eq(TenantKind::Sandbox.as_str()))
            .filter(entity::Column::Status.ne(TenantStatus::Deleted.as_str()))
            .all(self.control().read())
            .await?;
        let mut removed = 0;
        for row in rows {
            let expired = row.sandbox_expires_at.is_some_and(|at| at <= now);
            let orphan = match row.sandbox_of {
                None => true,
                Some(source) => entity::Entity::find_by_id(source)
                    .one(self.control().read())
                    .await?
                    .is_none_or(|source| {
                        source.status == TenantStatus::Deleted.as_str()
                            || source.status == TenantStatus::Merged.as_str()
                    }),
            };
            if !(expired || orphan) {
                continue;
            }
            let id = row.id;
            let reason = if expired { "Sandbox expired" } else { "The sandbox's tenant is gone" };
            match self.remove(row, Actor::system(), reason).await {
                Ok(()) => removed += 1,
                // Một sandbox hỏng không chặn các sandbox khác; lượt sau thử lại.
                Err(error) => tracing::warn!(%error, sandbox_id = %id, "expiring a sandbox failed"),
            }
        }
        Ok(removed)
    }
}

/// Bộ chạy job `kernel.sandbox_copy` (job của khách gốc, payload `{"sandbox_id": …}`).
pub(crate) struct CopyRunner(pub(crate) Arc<TenantSandboxes>);

#[async_trait::async_trait]
impl JobHandler for CopyRunner {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let id = job.payload["sandbox_id"]
            .as_str()
            .and_then(|raw| raw.parse::<Uuid>().ok())
            .ok_or_else(|| Error::internal("a sandbox copy job without `sandbox_id`"))?;
        self.0.copy(id).await
    }
}

/// Bộ chạy lịch `kernel.expire_sandboxes` (database điều khiển).
pub(crate) struct ExpireRunner(pub(crate) Arc<TenantSandboxes>);

#[async_trait::async_trait]
impl JobHandler for ExpireRunner {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        let removed = self.0.expire_due().await?;
        if removed > 0 {
            tracing::info!(removed, "expired sandboxes deleted");
        }
        Ok(())
    }
}

/// Phần của module `kernel` cho sandbox: cấu hình, service, route, tool, job, lịch.
pub(crate) struct Wiring {
    days: i64,
    max_products: u32,
    service: Option<Arc<TenantSandboxes>>,
}

impl Default for Wiring {
    fn default() -> Self {
        Wiring { days: 30, max_products: 50, service: None }
    }
}

impl Wiring {
    /// Đọc `AUTOXIDE_TENANCY_SANDBOX_DAYS` và `AUTOXIDE_TENANCY_SANDBOX_MAX_PRODUCTS`.
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<(i64, u32), Invalid> {
        let days =
            reader.get_with(&SANDBOX_DAYS, |raw| crate::config::parse_in_range(raw, 1i64, 365));
        let products = reader
            .get_with(&SANDBOX_MAX_PRODUCTS, |raw| crate::config::parse_in_range(raw, 0u32, 1000));
        Ok((days?, products?))
    }

    pub(crate) fn set(&mut self, (days, max_products): (i64, u32)) {
        self.days = days;
        self.max_products = max_products;
    }

    pub(crate) fn start(
        &mut self,
        context: &mut StartContext<'_>,
        admin: Arc<TenantAdmin>,
    ) -> Result<(), Error> {
        let service = Arc::new(TenantSandboxes {
            registry: context.tenancy().clone(),
            admin,
            outbox: context.outbox()?,
            jobs: context.jobs(),
            entitlements: context.entitlements(),
            days: self.days,
            max_products: self.max_products,
            catalog: context.sandbox_catalog(),
        });
        context.provide::<TenantSandboxes>(service.clone())?;
        self.service = Some(service);
        Ok(())
    }

    pub(crate) fn routes(&self) -> utoipa_axum::router::OpenApiRouter {
        self.service
            .clone()
            .map_or_else(utoipa_axum::router::OpenApiRouter::new, super::http::routes)
    }

    pub(crate) fn tools(&self) -> Vec<crate::mcp::Tool> {
        self.service.clone().map_or_else(Vec::new, super::tools::all)
    }

    pub(crate) fn jobs(&self) -> Vec<JobRegistration> {
        let Some(service) = &self.service else { return Vec::new() };
        vec![
            JobRegistration::new(SANDBOX_COPY, Arc::new(CopyRunner(service.clone()))),
            JobRegistration::new(EXPIRE_SANDBOXES, Arc::new(ExpireRunner(service.clone()))),
        ]
    }

    /// Mỗi giờ, chỉ trên database điều khiển (nơi duy nhất có sổ khách); chế độ `single` không có.
    pub(crate) fn schedules(&self, multi: bool) -> Vec<Schedule> {
        if !multi || self.service.is_none() {
            return Vec::new();
        }
        let every = std::time::Duration::from_secs(3600);
        vec![Schedule::new(EXPIRE_SANDBOXES, EXPIRE_SANDBOXES, every).on_control_only()]
    }
}
