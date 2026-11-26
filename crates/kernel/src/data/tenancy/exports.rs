//! Export of a whole tenant's data (step 35 "design 2 review", A-b9, D-328).
//!
//! One export run is one `kernel_tenant_exports` row (control plane) and one `kernel.export_tenant` job of
//! the control database: the job reads every row of the tenant in every tenant-level table (the same
//! column-typed reading as a tenant move) and writes it as ONE JSON Lines file to the tenant's store
//! (`exports/<id>.jsonl`). The job queue / outbox and secret columns (hashes, secrets) are left out. The
//! content of files users uploaded is NOT in the export (the export has `files_file` / `media_*` rows
//! pointing to them; download each file through the module's API).
//!
//! - The tenant's own administrators (`kernel.data.export`) can request an export; platform staff
//!   (`kernel.tenants.operate`) can export another tenant only after ENTERING that tenant through a support
//!   access grant that is still valid (D-313, D-20261121-export-entry). Requesting, downloading and
//!   creating a link all write events into the tenant itself.
//! - Download with a signed-in request, or with a time-limited link (24 hours) to open in a browser.
//! - The file is kept `AUTOXIDE_TENANCY_EXPORT_RETENTION_DAYS` days (default 30), then a cleanup job deletes
//!   it.
//! - The `plans` module requests an export itself when a subscription is canceled.

use crate::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use crate::migration::prelude::{
    DbErr, Index, MigrationName, MigrationTrait, SchemaManager, Table,
};
use crate::migration::schema;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use sea_orm::entity::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use validator::Validate;

use super::{DatabaseRegistry, TenantContext, TenantId, errors};
use crate::access::{Permission, Principal, Risk};
use crate::config::Setting;
use crate::error::Error;
use crate::event::{Actor, Event, EventInfo, Outbox};
use crate::job::{EnqueueOptions, Job, JobContext, JobHandler, JobQueue};
use crate::response::{Created, Data, Problem};
use crate::storage::{Storage, TenantStorage};

/// Job that exports one tenant (a control-database job).
pub(crate) const EXPORT_TENANT: &str = "kernel.export_tenant";
/// Job that cleans up expired exports (daily, control database).
pub(crate) const CLEAN_EXPORTS: &str = "kernel.clean_exports";
/// The download link that needs no sign-in lives this long.
const LINK_HOURS: i64 = 24;

pub const EXPORT_RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_TENANCY_EXPORT_RETENTION_DAYS",
    "Days a tenant data export is kept for download before it is deleted (1 to 365).",
)
.with_default("30");

/// Event written into the exported tenant ITSELF (the tenant's audit shows who exported and through which
/// path, including a platform staff member's support access grant; D-20261121-export-entry).
pub(crate) const TENANT_EXPORT_REQUESTED: EventInfo = EventInfo::new(
    "kernel.tenant_export_requested",
    "An export of all the data of the tenant was requested (`export_id`, `reason`).",
);
pub(crate) const TENANT_EXPORT_ACCESSED: EventInfo = EventInfo::new(
    "kernel.tenant_export_accessed",
    "A data export of the tenant was downloaded or got a download link (`export_id`, `via`: \
     `download`, `link_created` or `link`).",
);

#[derive(Debug, Serialize)]
struct ExportRequested<'a> {
    export_id: Uuid,
    reason: &'a str,
}

impl Event for ExportRequested<'_> {
    const NAME: &'static str = "kernel.tenant_export_requested";
}

#[derive(Debug, Serialize)]
struct ExportAccessed {
    export_id: Uuid,
    via: &'static str,
}

impl Event for ExportAccessed {
    const NAME: &'static str = "kernel.tenant_export_accessed";
}

/// Permission of a tenant's administrator: export all of their own tenant's data.
pub const DATA_EXPORT: Permission = Permission::new(
    "kernel.data.export",
    "Export all the data of your tenant as one file (to keep a copy or move to another service).",
)
.with_risk(Risk::SensitiveData);

mod entity {
    //! The `kernel_tenant_exports` table (control plane).
    use crate::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_tenant_exports")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// `pending`, `running`, `ready`, `failed`, `expired`.
        pub status: String,
        /// `manual` or `subscription_canceled`.
        pub reason: String,
        pub requested_by: Option<Uuid>,
        /// JSON `[{"table": …, "rows": …}]`.
        pub tables: String,
        pub size: Option<i64>,
        pub error: Option<String>,
        /// Hash of the token in the currently valid download link.
        pub link_hash: Option<String>,
        pub link_expires_at: Option<Timestamp>,
        pub expires_at: Option<Timestamp>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

use entity::{Column, Entity, Model};

/// The `kernel_tenant_exports` table.
pub(crate) struct CreateTenantExports;

impl MigrationName for CreateTenantExports {
    fn name(&self) -> &str {
        "m0015_create_tenant_exports"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateTenantExports {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("kernel_tenant_exports")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len("reason", 32))
                    .col(schema::uuid_null("requested_by"))
                    .col(schema::text("tables"))
                    .col(schema::big_integer_null("size"))
                    .col(schema::text_null("error"))
                    .col(schema::string_len_null("link_hash", 64))
                    .col(schema::timestamp_with_time_zone_null("link_expires_at"))
                    .col(schema::timestamp_with_time_zone_null("expires_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        for (name, columns) in [
            ("kernel_tenant_exports_tenant_idx", &["tenant_id", "id"][..]),
            ("kernel_tenant_exports_expiry_idx", &["status", "expires_at"][..]),
        ] {
            let mut index = Index::create();
            index.name(name).table("kernel_tenant_exports");
            for column in columns {
                index.col(*column);
            }
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("kernel_tenant_exports").to_owned()).await
    }
}

/// An export of all the data of a tenant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct TenantExport {
    pub id: Uuid,
    pub tenant_id: Uuid,
    /// `pending`, `running`, `ready` (download it), `failed` (see `error`) or `expired` (deleted
    /// after the retention days).
    #[schema(example = "ready")]
    pub status: String,
    /// `manual`, or `subscription_canceled` when the subscription was canceled.
    pub reason: String,
    pub requested_by: Option<Uuid>,
    /// Rows exported per table.
    pub tables: Vec<super::moves::TenantTableRows>,
    /// Size of the file in bytes, once ready.
    pub size: Option<i64>,
    pub error: Option<String>,
    /// The file is deleted after this time.
    pub expires_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl From<Model> for TenantExport {
    fn from(row: Model) -> Self {
        TenantExport {
            id: row.id,
            tenant_id: row.tenant_id,
            status: row.status,
            reason: row.reason,
            requested_by: row.requested_by,
            tables: serde_json::from_str(&row.tables).unwrap_or_default(),
            size: row.size,
            error: row.error,
            expires_at: row.expires_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A link to download an export without signing in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct TenantExportLink {
    /// Path to open, relative to the API, e.g. `/api/v1/kernel/exports/download?token=…`. Anyone
    /// with it can download until `expires_at`; a new link replaces the previous one.
    pub url: String,
    pub expires_at: Timestamp,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExportTenant {
    export_id: Uuid,
}

impl Job for ExportTenant {
    const KIND: &'static str = EXPORT_TENANT;
}

fn now() -> Timestamp {
    Timestamp::now()
}

/// Tenant data export. Published by the `kernel` module (`context.require::<TenantExports>()`).
pub struct TenantExports {
    registry: DatabaseRegistry,
    setup: super::moves::MoveSetup,
    jobs: JobQueue,
    outbox: Outbox,
    retention: SignedDuration,
}

impl std::fmt::Debug for TenantExports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TenantExports").field("retention", &self.retention).finish_non_exhaustive()
    }
}

impl TenantExports {
    pub(crate) fn new(
        registry: DatabaseRegistry,
        setup: super::moves::MoveSetup,
        jobs: JobQueue,
        outbox: Outbox,
        retention_days: i64,
    ) -> Self {
        TenantExports {
            registry,
            setup,
            jobs,
            outbox,
            retention: SignedDuration::from_hours(24 * retention_days),
        }
    }

    fn control(&self) -> &crate::database::Database {
        self.registry.control()
    }

    /// Requests an export of tenant `tenant`'s data (`reason`: `manual`, `subscription_canceled`). If a run is
    /// already unfinished it returns that run and queues no more.
    pub async fn request(
        &self,
        actor: Actor,
        tenant: TenantId,
        reason: &str,
    ) -> Result<TenantExport, Error> {
        // Khách phải tồn tại.
        self.registry.tenant(tenant).await?;
        let transaction = self.control().begin_write().await?;
        let busy = Entity::find()
            .filter(Column::TenantId.eq(tenant.as_uuid()))
            .filter(Column::Status.is_in(["pending", "running"]))
            .one(&transaction)
            .await?;
        if let Some(busy) = busy {
            return Ok(busy.into());
        }
        let at = now();
        let row = entity::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.as_uuid()),
            status: Set("pending".to_owned()),
            reason: Set(reason.to_owned()),
            requested_by: Set(actor.user_id),
            tables: Set("[]".to_owned()),
            size: Set(None),
            error: Set(None),
            link_hash: Set(None),
            link_expires_at: Set(None),
            expires_at: Set(None),
            created_at: Set(at),
            updated_at: Set(at),
        }
        .insert(&transaction)
        .await?;
        self.jobs
            .enqueue_database(
                &transaction,
                &ExportTenant { export_id: row.id },
                EnqueueOptions { max_attempts: Some(3), ..EnqueueOptions::default() },
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(tenant_id = %tenant, export_id = %row.id, reason, "tenant export requested");
        let event = ExportRequested { export_id: row.id, reason };
        self.record(tenant, actor, row.id, &event).await?;
        Ok(row.into())
    }

    /// Writes an event to the outbox of tenant `tenant` itself (the tenant's database may differ from the
    /// control database, so this is a separate transaction, after the export row has committed).
    async fn record<E: Event + Sync>(
        &self,
        tenant: TenantId,
        actor: Actor,
        export_id: Uuid,
        event: &E,
    ) -> Result<(), Error> {
        let context = self.registry.tenant(tenant).await?;
        let subject = Some(format!("kernel.tenant_export:{export_id}"));
        self.outbox.record(context.database().write(), &context, actor, subject, event).await
    }

    /// Export `id` of tenant `tenant`.
    pub async fn get(&self, tenant: TenantId, id: Uuid) -> Result<TenantExport, Error> {
        Ok(self.find(tenant, id).await?.into())
    }

    async fn find(&self, tenant: TenantId, id: Uuid) -> Result<Model, Error> {
        Entity::find_by_id(id)
            .filter(Column::TenantId.eq(tenant.as_uuid()))
            .one(self.control().read())
            .await?
            .ok_or_else(|| export_not_found(id))
    }

    /// The exports of tenant `tenant`, newest first (at most 50).
    pub async fn list(&self, tenant: TenantId) -> Result<Vec<TenantExport>, Error> {
        let rows = Entity::find()
            .filter(Column::TenantId.eq(tenant.as_uuid()))
            .order_by_desc(Column::Id)
            .limit(50)
            .all(self.control().read())
            .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Runs export `id`: writes the file to the tenant's store, then `ready` (or `failed`).
    async fn run(&self, id: Uuid) -> Result<(), Error> {
        let Some(row) = Entity::find_by_id(id).one(self.control().write()).await? else {
            return Ok(());
        };
        if !["pending", "running"].contains(&row.status.as_str()) {
            return Ok(());
        }
        let tenant = TenantId::from_uuid(row.tenant_id);
        self.set(id, |active| active.status = Set("running".to_owned())).await?;
        match self.write_file(tenant, id).await {
            Ok((tables, size)) => {
                let expires = now() + self.retention;
                let tables = serde_json::to_string(&tables).map_err(Error::internal)?;
                self.set(id, |active| {
                    active.status = Set("ready".to_owned());
                    active.tables = Set(tables);
                    active.size = Set(Some(size));
                    active.expires_at = Set(Some(expires));
                    active.error = Set(None);
                })
                .await?;
                tracing::info!(tenant_id = %tenant, export_id = %id, size, "tenant export ready");
                Ok(())
            }
            Err(error) => {
                let message: String = error.to_string().chars().take(2000).collect();
                self.set(id, |active| {
                    active.status = Set("failed".to_owned());
                    active.error = Set(Some(message));
                })
                .await?;
                Err(error)
            }
        }
    }

    /// Writes the JSON Lines file of tenant `tenant`; returns the row count per table and the file size.
    async fn write_file(
        &self,
        tenant: TenantId,
        id: Uuid,
    ) -> Result<(Vec<super::moves::TenantTableRows>, i64), Error> {
        let context = self.registry.tenant(tenant).await?;
        let store = self.store(&context)?;
        let (modules, control_plane) = self.setup.tenant_tables()?;
        let database = context.database().clone();
        let tables =
            crate::transfer::TenantTables::read(&database, &database, &modules, &control_plane)
                .await
                .map_err(|error| Error::internal(error.to_string()))?;
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        let header = serde_json::json!({
            "format": "autoxide-tenant-export",
            "version": 1,
            "tenant_id": tenant,
            "created_at": now(),
        });
        let mut first = serde_json::to_vec(&header).map_err(Error::internal)?;
        first.push(b'\n');
        sender.send(Ok(bytes::Bytes::from(first))).await.map_err(Error::internal)?;
        // `unfold`: biến đầu nhận của kênh thành luồng byte cho nơi lưu đọc dần.
        let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|item| (item, receiver))
        });
        let key = key(id);
        let producer = async move {
            let result = tables.export(&database, tenant, &sender).await;
            if let Err(error) = &result {
                let _ = sender.send(Err(Error::internal(error.to_string()))).await;
            }
            // Đóng kênh: nơi lưu biết đã hết nội dung.
            drop(sender);
            result
        };
        let (stored, exported) = tokio::join!(store.put(&key, Box::pin(stream)), producer);
        let exported = exported.map_err(|error| Error::internal(error.to_string()))?;
        let size = stored?;
        let tables = exported
            .into_iter()
            .map(|table| super::moves::TenantTableRows { table: table.table, rows: table.rows })
            .collect();
        Ok((tables, i64::try_from(size).unwrap_or(i64::MAX)))
    }

    fn store(&self, tenant: &TenantContext) -> Result<TenantStorage, Error> {
        let shared = self.registry.shared_storage().ok_or_else(|| {
            Error::unavailable(
                "kernel.export_storage_unavailable",
                "Exports need file storage; the `files` module is not installed.",
            )
        })?;
        Ok(TenantStorage::new(shared, tenant))
    }

    async fn set(
        &self,
        id: Uuid,
        edit: impl FnOnce(&mut entity::ActiveModel),
    ) -> Result<(), Error> {
        let mut active = entity::ActiveModel { id: Set(id), ..Default::default() };
        edit(&mut active);
        active.updated_at = Set(now());
        active.update(self.control().write()).await?;
        Ok(())
    }

    /// A download link that needs no sign-in for export `id` (24 hours, replaces the old link); `actor` is
    /// written to the audit of the tenant.
    pub async fn link(
        &self,
        actor: Actor,
        tenant: TenantId,
        id: Uuid,
    ) -> Result<TenantExportLink, Error> {
        let row = self.find(tenant, id).await?;
        if row.status != "ready" {
            return Err(export_not_ready(&row.status));
        }
        let raw = crate::secret::random_token(32)?;
        let expires_at = now() + SignedDuration::from_hours(LINK_HOURS);
        let hash = crate::secret::sha256(&raw);
        self.set(id, |active| {
            active.link_hash = Set(Some(hash));
            active.link_expires_at = Set(Some(expires_at));
        })
        .await?;
        self.record(tenant, actor, id, &ExportAccessed { export_id: id, via: "link_created" })
            .await?;
        Ok(TenantExportLink {
            url: format!("{}/kernel/exports/download?token={id}.{raw}", crate::http::API_PREFIX),
            expires_at,
        })
    }

    /// Content of export `id` of tenant `tenant` (the caller's permission has already been checked);
    /// `actor` is written to the tenant's audit.
    pub async fn content(
        &self,
        actor: Actor,
        tenant: TenantId,
        id: Uuid,
    ) -> Result<Response, Error> {
        let row = self.find(tenant, id).await?;
        let response = self.stream(&row).await?;
        self.record(tenant, actor, id, &ExportAccessed { export_id: id, via: "download" }).await?;
        Ok(response)
    }

    /// Content by the link's token (`<id>.<raw part>`). Wrong, expired or replaced: 404 as if it did not exist.
    pub async fn content_by_token(&self, token: &str) -> Result<Response, Error> {
        self.content_by_token_for(token, None).await
    }

    /// Like [`TenantExports::content_by_token`], for a request from the web origin of tenant `origin`'s verified
    /// domain: a link to another tenant's export answers like a wrong link (D-20261125-tenant-cors).
    pub(crate) async fn content_by_token_for(
        &self,
        token: &str,
        origin: Option<TenantId>,
    ) -> Result<Response, Error> {
        let missing = || export_link_invalid();
        let (id, raw) = token.split_once('.').ok_or_else(missing)?;
        let id = Uuid::parse_str(id).map_err(|_| missing())?;
        let row = Entity::find_by_id(id).one(self.control().read()).await?.ok_or_else(missing)?;
        let valid = row.link_hash.as_deref() == Some(crate::secret::sha256(raw).as_str())
            && row.link_expires_at.is_some_and(|until| until > now())
            && origin.is_none_or(|owner| owner.as_uuid() == row.tenant_id);
        if !valid {
            return Err(missing());
        }
        let response = self.stream(&row).await?;
        // Link không mang người dùng: bên tạo link đã ghi ở `link_created`.
        let event = ExportAccessed { export_id: row.id, via: "link" };
        self.record(TenantId::from_uuid(row.tenant_id), Actor::system(), row.id, &event).await?;
        Ok(response)
    }

    async fn stream(&self, row: &Model) -> Result<Response, Error> {
        if row.status != "ready" {
            return Err(export_not_ready(&row.status));
        }
        let tenant = self.registry.tenant(TenantId::from_uuid(row.tenant_id)).await?;
        let stored = self
            .store(&tenant)?
            .get(&key(row.id))
            .await?
            .ok_or_else(|| export_not_ready("expired"))?;
        let name = format!("tenant-{}-{}.jsonl", tenant.slug(), row.created_at.format("%Y%m%d"));
        let mut response = Body::from_stream(stored.content).into_response();
        let headers = response.headers_mut();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/x-ndjson"));
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from(stored.size));
        let disposition =
            crate::http::content_disposition(crate::http::Disposition::Attachment, &name);
        if let Ok(value) = HeaderValue::from_str(&disposition) {
            headers.insert(header::CONTENT_DISPOSITION, value);
        }
        Ok(response)
    }

    /// Deletes the files of exports past their retention; returns the number of exports cleaned.
    pub(crate) async fn clean(&self, at: Timestamp) -> Result<usize, Error> {
        let due = Entity::find()
            .filter(Column::Status.eq("ready"))
            .filter(Column::ExpiresAt.lte(at))
            .limit(200)
            .all(self.control().read())
            .await?;
        for row in &due {
            if let Ok(tenant) = self.registry.tenant(TenantId::from_uuid(row.tenant_id)).await
                && let Err(error) = self.store(&tenant)?.delete(&key(row.id)).await
            {
                tracing::warn!(export_id = %row.id, error = %error, "cannot delete an expired export");
                continue;
            }
            self.set(row.id, |active| {
                active.status = Set("expired".to_owned());
                active.link_hash = Set(None);
            })
            .await?;
        }
        Ok(due.len())
    }
}

fn key(id: Uuid) -> String {
    format!("exports/{id}.jsonl")
}

pub(crate) fn export_not_found(id: Uuid) -> Error {
    Error::not_found("kernel.export_not_found", "There is no data export `{id}` of this tenant.")
        .with_param("id", id)
}

fn export_not_ready(status: &str) -> Error {
    Error::conflict(
        "kernel.export_not_ready",
        "The data export is `{status}`: only a `ready` export can be downloaded.",
    )
    .with_param("status", status.to_owned())
}

fn export_link_invalid() -> Error {
    Error::not_found(
        "kernel.export_link_invalid",
        "This download link is wrong, was replaced by a newer one, or has expired. Ask for a new \
         link.",
    )
}

/// Runner of the job `kernel.export_tenant`.
pub(crate) struct ExportRunner(pub(crate) Arc<TenantExports>);

#[async_trait::async_trait]
impl JobHandler for ExportRunner {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let payload: ExportTenant =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.run(payload.export_id).await
    }
}

/// Runner of the job `kernel.clean_exports`.
pub(crate) struct CleanRunner(pub(crate) Arc<TenantExports>);

#[async_trait::async_trait]
impl JobHandler for CleanRunner {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        self.0.clean(now()).await.map(|_| ())
    }
}

pub(crate) fn schedule() -> crate::job::Schedule {
    crate::job::Schedule::new(CLEAN_EXPORTS, CLEAN_EXPORTS, std::time::Duration::from_secs(86_400))
        .on_control_only()
}

// ---- REST ----------------------------------------------------------------------------------------

/// The tenant of the `{id}` path (ID or slug) that the caller may export (D-313, D-20261121-export-entry):
///
/// - the tenant's own administrator, who has `kernel.data.export`;
/// - platform staff who have ENTERED that tenant (header `Autoxide-Tenant`, through a support access grant
///   still valid; grounds `support` or `legal_request` with a document number) and have
///   `kernel.tenants.operate` or `kernel.data.export`;
/// - platform staff in the platform tenant can export only the platform tenant itself. Targeting another
///   tenant without entering it gives 403 `kernel.export_needs_tenant_access`; there is no shortcut where
///   "`kernel.tenants.operate` exports every tenant" (a tenant's personal data leaves only with grounds and
///   leaves a trace).
async fn allowed(
    exports: &TenantExports,
    principal: &Principal,
    id: &str,
) -> Result<TenantId, Error> {
    let operate = principal.has(&crate::builtin::TENANTS_OPERATE);
    // Ở khách nền tảng, chưa vào khách nào.
    let platform_staff = operate && super::require_platform(principal).is_ok();
    // Nhân sự nền tảng đang ở trong khách khác (đường vào đã do bộ xác thực quyết ở request.rs).
    let entered_staff = operate && principal.operator_from() == Some(super::DEFAULT_TENANT);
    // Không có permission nào thì báo 403 trước khi tra khách, như mọi endpoint khác.
    if !platform_staff && !entered_staff {
        principal.require(&DATA_EXPORT)?;
    }
    let tenant = match Uuid::parse_str(id) {
        Ok(uuid) => exports.registry.tenant(TenantId::from_uuid(uuid)).await?,
        Err(_) => exports.registry.tenant_by_slug(id).await?,
    };
    let tenant = tenant.id();
    if principal.tenant_id() == tenant {
        return Ok(tenant);
    }
    if platform_staff {
        return Err(export_needs_tenant_access(tenant));
    }
    // Khách khác: như không có (không lộ khách nào tồn tại).
    Err(errors::tenant_not_found(id))
}

fn export_needs_tenant_access(tenant: TenantId) -> Error {
    Error::forbidden(
        "kernel.export_needs_tenant_access",
        "Exporting the data of another tenant needs access to that tenant: get a support access \
         grant in force for it, then call with the header `Autoxide-Tenant: {tenant}`.",
    )
    .with_param("tenant", tenant.as_uuid())
}

pub(crate) fn routes(exports: Arc<TenantExports>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(request_export, list_exports))
        .routes(routes!(get_export))
        .routes(routes!(download_export))
        .routes(routes!(export_link))
        .routes(routes!(download_by_link))
        .with_state(exports)
}

/// Export all the data of a tenant.
///
/// An administrator of the tenant itself with `kernel.data.export`, or platform staff with
/// `kernel.tenants.operate` who entered the tenant under a support access grant in force (header
/// `Autoxide-Tenant`). The request and every download are recorded in the tenant's own audit
/// events (`kernel.tenant_export_requested`, `kernel.tenant_export_accessed`). The export runs in the background into one JSON Lines file (one line per
/// record, `{"table": …, "row": {…}}`; password hashes and secrets left out; uploaded file contents
/// are not included). Poll `GET …/exports/{export_id}` until it is `ready`. If an export is already
/// running it is returned instead of starting another.
#[utoipa::path(
    post,
    path = "/tenants/{id}/exports",
    extensions(("x-permissions" = json!(["kernel.data.export", "kernel.tenants.operate"]))),
    operation_id = "request_tenant_export",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant.")),
    responses(
        (status = 201, description = "The export, `pending`; `Location` is its status.", body = Data<TenantExport>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing both `kernel.data.export` and `kernel.tenants.operate` (`permission_denied`), or platform staff targeting another tenant without having entered it under a support access grant (`kernel.export_needs_tenant_access`).", body = Problem),
        (status = 404, description = "No such tenant you may export; code `tenant_not_found`.", body = Problem),
    )
)]
async fn request_export(
    State(exports): State<Arc<TenantExports>>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Created<TenantExport>, Error> {
    let tenant = allowed(&exports, &principal, &id).await?;
    let export = exports.request(Actor::from(&principal), tenant, "manual").await?;
    Ok(Created::new(format!("/kernel/tenants/{tenant}/exports/{}", export.id), export))
}

/// List the data exports of a tenant.
///
/// Same permissions as `POST /tenants/{id}/exports`. Newest first, at most 50.
#[utoipa::path(
    get,
    path = "/tenants/{id}/exports",
    extensions(("x-permissions" = json!(["kernel.data.export", "kernel.tenants.operate"]))),
    operation_id = "list_tenant_exports",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant.")),
    responses(
        (status = 200, description = "The exports, newest first.", body = Data<Vec<TenantExport>>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing both `kernel.data.export` and `kernel.tenants.operate` (`permission_denied`), or platform staff targeting another tenant without having entered it under a support access grant (`kernel.export_needs_tenant_access`).", body = Problem),
        (status = 404, description = "No such tenant you may export; code `tenant_not_found`.", body = Problem),
    )
)]
async fn list_exports(
    State(exports): State<Arc<TenantExports>>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Data<Vec<TenantExport>>, Error> {
    let tenant = allowed(&exports, &principal, &id).await?;
    Ok(Data { data: exports.list(tenant).await? })
}

/// Get a data export of a tenant.
///
/// Same permissions as `POST /tenants/{id}/exports`.
#[utoipa::path(
    get,
    path = "/tenants/{id}/exports/{export_id}",
    extensions(("x-permissions" = json!(["kernel.data.export", "kernel.tenants.operate"]))),
    operation_id = "get_tenant_export",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant."), ("export_id" = Uuid, Path, description = "Id of the export.")),
    responses(
        (status = 200, description = "The export.", body = Data<TenantExport>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing both `kernel.data.export` and `kernel.tenants.operate` (`permission_denied`), or platform staff targeting another tenant without having entered it under a support access grant (`kernel.export_needs_tenant_access`).", body = Problem),
        (status = 404, description = "No such tenant or export; code `tenant_not_found` or `kernel.export_not_found`.", body = Problem),
    )
)]
async fn get_export(
    State(exports): State<Arc<TenantExports>>,
    principal: Principal,
    Path((id, export_id)): Path<(String, Uuid)>,
) -> Result<Data<TenantExport>, Error> {
    let tenant = allowed(&exports, &principal, &id).await?;
    Ok(Data { data: exports.get(tenant, export_id).await? })
}

/// Download a data export of a tenant.
///
/// Same permissions as `POST /tenants/{id}/exports`. A JSON Lines file: the first line describes
/// the export, then one line per record.
#[utoipa::path(
    get,
    path = "/tenants/{id}/exports/{export_id}/content",
    extensions(("x-permissions" = json!(["kernel.data.export", "kernel.tenants.operate"]))),
    operation_id = "download_tenant_export",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant."), ("export_id" = Uuid, Path, description = "Id of the export.")),
    responses(
        (status = 200, description = "The file.", body = String, content_type = "application/x-ndjson"),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing both `kernel.data.export` and `kernel.tenants.operate` (`permission_denied`), or platform staff targeting another tenant without having entered it under a support access grant (`kernel.export_needs_tenant_access`).", body = Problem),
        (status = 404, description = "No such tenant or export; code `tenant_not_found` or `kernel.export_not_found`.", body = Problem),
        (status = 409, description = "The export is not `ready`; code `kernel.export_not_ready`.", body = Problem),
    )
)]
async fn download_export(
    State(exports): State<Arc<TenantExports>>,
    principal: Principal,
    Path((id, export_id)): Path<(String, Uuid)>,
) -> Result<Response, Error> {
    let tenant = allowed(&exports, &principal, &id).await?;
    exports.content(Actor::from(&principal), tenant, export_id).await
}

/// Create a download link for a data export.
///
/// Same permissions as `POST /tenants/{id}/exports`. The link works without signing in for 24
/// hours (to open in a browser); a new link replaces the previous one.
#[utoipa::path(
    post,
    path = "/tenants/{id}/exports/{export_id}/link",
    extensions(("x-permissions" = json!(["kernel.data.export", "kernel.tenants.operate"]))),
    operation_id = "create_tenant_export_link",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant."), ("export_id" = Uuid, Path, description = "Id of the export.")),
    responses(
        (status = 200, description = "The link.", body = Data<TenantExportLink>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing both `kernel.data.export` and `kernel.tenants.operate` (`permission_denied`), or platform staff targeting another tenant without having entered it under a support access grant (`kernel.export_needs_tenant_access`).", body = Problem),
        (status = 404, description = "No such tenant or export; code `tenant_not_found` or `kernel.export_not_found`.", body = Problem),
        (status = 409, description = "The export is not `ready`; code `kernel.export_not_ready`.", body = Problem),
    )
)]
async fn export_link(
    State(exports): State<Arc<TenantExports>>,
    principal: Principal,
    Path((id, export_id)): Path<(String, Uuid)>,
) -> Result<Data<TenantExportLink>, Error> {
    let tenant = allowed(&exports, &principal, &id).await?;
    Ok(Data { data: exports.link(Actor::from(&principal), tenant, export_id).await? })
}

/// The download link of a data export.
#[derive(Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
struct LinkQuery {
    /// The token from `POST /tenants/{id}/exports/{export_id}/link`.
    #[validate(length(min = 1, max = 200))]
    token: String,
}

/// Download a data export with a link.
///
/// No sign-in: the token in the link is the permission. Wrong, replaced and expired links answer
/// the same 404.
#[utoipa::path(
    get,
    path = "/exports/download",
    extensions(("x-public" = json!(true))),
    operation_id = "download_tenant_export_by_link",
    params(LinkQuery),
    responses(
        (status = 200, description = "The file.", body = String, content_type = "application/x-ndjson"),
        (status = 404, description = "The link is wrong, replaced or expired; code `kernel.export_link_invalid`.", body = Problem),
        (status = 409, description = "The export was deleted after its retention days; code `kernel.export_not_ready`.", body = Problem),
    )
)]
async fn download_by_link(
    State(exports): State<Arc<TenantExports>>,
    origin: Option<axum::Extension<crate::http::OriginGrant>>,
    Query(query): Query<LinkQuery>,
) -> Result<Response, Error> {
    let origin = match origin {
        Some(axum::Extension(crate::http::OriginGrant::Tenant(owner))) => Some(owner),
        _ => None,
    };
    exports.content_by_token_for(&query.token, origin).await
}

// ---- MCP -------------------------------------------------------------------------------------

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExportTenantInput {
    /// Id or slug of the tenant; your own tenant when left out.
    tenant: Option<String>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExportRef {
    /// Id or slug of the tenant; your own tenant when left out.
    tenant: Option<String>,
    /// Id of the export.
    export_id: Uuid,
}

pub(crate) fn tools(exports: Arc<TenantExports>) -> Vec<crate::mcp::Tool> {
    use crate::mcp::Tool;
    let (start, get) = (exports.clone(), exports);
    let own = |principal: &Principal, tenant: Option<String>| {
        tenant.unwrap_or_else(|| principal.tenant_id().to_string())
    };
    vec![
        Tool::new(
            "export_tenant",
            "Start an export of all the data of a tenant (yours by default) into one JSON Lines file, \
             in the background; follow it with `kernel.get_tenant_export` until it is `ready`, \
             then download it over REST.",
            move |_tenant: TenantContext, principal: Principal, input: ExportTenantInput| {
                let exports = start.clone();
                async move {
                    let id = own(&principal, input.tenant);
                    let tenant = allowed(&exports, &principal, &id).await?;
                    exports.request(Actor::from(&principal), tenant, "manual").await
                }
            },
        )
        .requires(DATA_EXPORT),
        Tool::new(
            "get_tenant_export",
            "Get a data export of a tenant: its `status` (`pending`, `running`, `ready`, `failed`, \
             `expired`), rows per table and size.",
            move |_tenant: TenantContext, principal: Principal, input: ExportRef| {
                let exports = get.clone();
                async move {
                    let id = own(&principal, input.tenant);
                    let tenant = allowed(&exports, &principal, &id).await?;
                    exports.get(tenant, input.export_id).await
                }
            },
        )
        .requires(DATA_EXPORT)
        .read_only(),
    ]
}
