//! Dời khách giữa các placement — `shared` ↔ `dedicated` (đặc tả 33 "Cấp phát và dời khách"):
//! lệnh `autoxide tenant move` (chạy ngay trên máy chủ) và REST / MCP chỉ cho nhân sự nền tảng
//! (dry-run mặc định; chạy thật thì một job nền làm).
//!
//! Một lượt dời là một dòng `kernel_tenant_moves` (mặt điều khiển) đi qua các trạng thái:
//!
//! 1. Bắt đầu: trong MỘT transaction ở database điều khiển, khách `active` → `moving` (mọi request
//!    tới khách nhận `503 tenant_maintenance`) và ghi lượt dời `copying`.
//! 2. `copying` → `copied`: chờ máy chủ khác thấy `moving` (TTL cache khách) và request đang chạy
//!    xong (`AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS`); chờ job / event của khách đang được chạy ở
//!    nguồn hết giữ (`claimed_until`) — worker và bộ phát không nhận việc mới của khách `moving`;
//!    hai database phải đủ migration và khác nhau; chép mọi dòng `tenant_id = X` của mọi bảng cấp
//!    khách hàng (kể cả outbox và hàng đợi job — event chưa giao, job chưa chạy đi theo dữ liệu)
//!    trong MỘT transaction ở đích, kèm event `kernel.tenant_moved`; so số dòng từng bảng.
//! 3. `copied` → `switched`: so lại số dòng, rồi trong một transaction ở database điều khiển đổi
//!    `placement_id` của khách sang đích và đặt lại `active` — khách phục vụ ngay từ database mới.
//! 4. `switched` → `done`: xoá dòng của khách ở nguồn (`keep_source`: chỉ xoá hàng đợi job và
//!    outbox, để không gì chạy hai lần; dữ liệu còn lại là bản sao dự phòng, xoá sau bằng
//!    [`TenantMoves::purge`]).
//!
//! - **Chạy lại được**: mỗi bước một lease (`runner`, `runner_until`, gia hạn khi đang chạy) — hai
//!   bên không cùng chạy một lượt. Tiến trình chết giữa chừng: khách giữ `moving` (hay đã `active`
//!   ở đích nếu đã qua bước 3); dời lại sang cùng placement thì chạy tiếp từ trạng thái đã ghi
//!   (bước 2 xoá dòng của khách ở đích rồi chép lại). Huỷ được trước bước 3 ([`TenantMoves::abort`]).
//! - Không bao giờ có hai bản "đang sống": `placement_id` chỉ đổi sau khi số dòng khớp, nguồn chỉ bị
//!   xoá sau khi đã đổi.
//! - Không chép: bảng mặt điều khiển (`kernel_tenants`, `kernel_placements`, tên miền…).
//! - File lưu trữ (khoá `tenants/<id>/…`): hai placement cùng nơi lưu (cùng nơi lưu chung, hay nơi
//!   lưu riêng chỉ cùng một chỗ) thì không chép gì. Khác nơi lưu: bước 2 chép mọi file của khách sang
//!   nơi lưu của đích (xoá file thừa của khách ở đó trước, so số file sau), bước 4 xoá file ở nguồn
//!   (trừ `keep_source`; `purge` xoá sau). Trước khi chép hay xoá, ghi thử một file đánh dấu ở đích
//!   rồi tìm ở nguồn: hai cấu hình khác nhau mà cùng một chỗ (vd nơi lưu riêng trỏ đúng thư mục của
//!   nơi lưu chung) thì coi là cùng nơi lưu — không bao giờ xoá file đang dùng.

mod claims;
mod http;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::{Condition, Expr};
use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::admin::{placement, tenant};
use super::{
    DEFAULT_TENANT, DatabaseRegistry, MAIN_PLACEMENT, PlacementId, TenantContext, TenantId,
    TenantStatus, errors,
};
use crate::access::{Permission, Risk};
use crate::database::Database;
use crate::error::Error;
use crate::event::{Actor, Event, EventInfo, Outbox};
use crate::i18n::{Language, Translations};
use crate::job::{EnqueueOptions, Job, JobContext, JobHandler, JobQueue};
use crate::migration::Migrator;
use crate::migration::prelude::*;
use crate::storage::Storage;
use crate::transfer::{CopiedTable, TableKind, TenantTables, TransferError, same_database};

pub(super) use claims::wait_claims;
pub(crate) use http::{routes, tools};

mod migration;

pub(crate) use migration::CreateTenantMoves;
mod files;
mod finish;
mod phases;
mod preflight;
mod steps;

/// Dời dữ liệu của khách sang placement khác.
pub const TENANTS_MOVE: Permission = Permission::new(
    "kernel.tenants.move",
    "Move a tenant's data to another placement (database); the tenant is unavailable during the \
     move (platform staff only).",
)
.with_risk(Risk::Destructive);

pub(crate) const TENANT_MOVED: EventInfo = EventInfo::new(
    "kernel.tenant_moved",
    "The tenant's data was moved to another placement (database).",
);

/// Loại job chạy một lượt dời bắt đầu qua REST / MCP.
pub(crate) const MOVE_TENANT: &str = "kernel.move_tenant";

const COPYING: &str = "copying";
const COPIED: &str = "copied";
const SWITCHED: &str = "switched";
const DONE: &str = "done";
const ABORTED: &str = "aborted";
const UNFINISHED: [&str; 3] = [COPYING, COPIED, SWITCHED];

/// Một bên giữ lượt dời tối đa chừng này mà không gia hạn; đang chạy thì gia hạn mỗi một phần ba.
const LEASE: Duration = Duration::from_secs(120);
/// Bảng outbox của `kernel`: có thêm event `kernel.tenant_moved` ở đích.
const KERNEL_EVENTS: &str = "kernel_events";

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

mod entity {
    //! Bảng `kernel_tenant_moves` (mặt điều khiển).
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_tenant_moves")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub from_placement_id: Uuid,
        pub to_placement_id: Uuid,
        pub keep_source: bool,
        pub state: String,
        /// JSON `[{"table": …, "rows": …}]`: dòng đã chép mỗi bảng.
        pub tables: String,
        pub error: Option<String>,
        pub runner: Option<String>,
        pub runner_until: Option<DateTimeUtc>,
        /// Lúc khách thành `moving`: chờ cache của máy chủ khác tính từ đây.
        pub moving_since: DateTimeUtc,
        pub requested_by: Option<Uuid>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
        pub finished_at: Option<DateTimeUtc>,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

use entity::{Column, Entity, Model};

/// Rows of the tenant in one table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TenantTableRows {
    #[schema(example = "notes_note")]
    pub table: String,
    pub rows: u64,
}

impl From<CopiedTable> for TenantTableRows {
    fn from(copied: CopiedTable) -> Self {
        TenantTableRows { table: copied.table, rows: copied.rows }
    }
}

/// Why the move cannot run now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TenantMoveBlocker {
    /// Stable code, the same as the error a real run would answer.
    #[schema(example = "tenant_move_schema_differs")]
    pub code: String,
    pub message: String,
}

/// What a move would do (dry run). Nothing is changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TenantMovePlan {
    pub tenant_id: Uuid,
    /// Slug of the tenant.
    #[schema(example = "acme")]
    pub tenant: String,
    /// Placement that keeps the tenant's data now.
    #[schema(example = "main")]
    pub from: String,
    /// Placement the data would move to.
    #[schema(example = "acme-db")]
    pub to: String,
    pub keep_source: bool,
    /// `true` when nothing blocks the move (`blocking` is empty).
    pub ready: bool,
    /// What must be fixed first; empty when the move can run.
    pub blocking: Vec<TenantMoveBlocker>,
    /// An interrupted move to `to` that a real run resumes.
    pub resumes: Option<Uuid>,
    /// Rows of the tenant that would be copied, per table, in copy order.
    pub tables: Vec<TenantTableRows>,
    /// Sum of `tables[].rows`.
    pub rows: u64,
    /// Files of the tenant that would be copied to the file storage of `to`. `0` in two cases:
    /// both placements keep files in the same storage (nothing to copy), or the tenant has no
    /// files.
    pub files: u64,
    /// Seconds the tenant answers `503 tenant_maintenance` before copying starts (other servers
    /// see the move and requests in progress finish); copying takes longer for many rows.
    pub settle_seconds: u64,
}

/// A move of a tenant to another placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TenantMove {
    pub id: Uuid,
    pub tenant_id: Uuid,
    #[schema(example = "main")]
    pub from: String,
    #[schema(example = "acme-db")]
    pub to: String,
    pub keep_source: bool,
    /// `copying` (the tenant answers `503`), `copied`, `switched` (the tenant works from `to`;
    /// the old placement is being cleaned), `done` or `aborted`.
    #[schema(example = "done")]
    pub state: String,
    /// Rows copied per table (after `copied`).
    pub tables: Vec<TenantTableRows>,
    /// Sum of `tables[].rows`.
    pub rows: u64,
    /// Why the last run stopped, when it did; running the move again resumes it.
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct TenantMoved {
    tenant_id: Uuid,
    from: String,
    to: String,
    rows: u64,
}

impl Event for TenantMoved {
    const NAME: &'static str = "kernel.tenant_moved";
}

/// Job chạy lượt dời `move_id` tới xong.
#[derive(Debug, Serialize, Deserialize)]
struct MoveTenant {
    move_id: Uuid,
}

impl Job for MoveTenant {
    const KIND: &'static str = MOVE_TENANT;
}

/// Thứ lượt dời cần từ mọi module: tên module (chủ bảng), bảng mặt điều khiển, migration. Điền một
/// lần sau khi mọi module khởi động (`Modules::start_in`). Clone rẻ.
#[derive(Clone, Default)]
pub(crate) struct MoveSetup(Arc<OnceLock<Setup>>);

struct Setup {
    modules: Vec<&'static str>,
    control_plane: Vec<&'static str>,
    migrator: Migrator,
    /// Quản lý placement (migrate placement bằng API, D-334).
    placements: super::Placements,
    /// Bảng giữ lại khi xoá hẳn khách (`Module::retained_tables`, D-337).
    retained: Vec<&'static str>,
    /// Chính sách tách / gộp của mọi module (`Module::relocation`, bước 38).
    policies: super::relocate::Policies,
}

impl MoveSetup {
    pub(crate) fn fill(
        &self,
        modules: Vec<&'static str>,
        control_plane: Vec<&'static str>,
        migrator: Migrator,
        placements: super::Placements,
        retained: Vec<&'static str>,
        policies: super::relocate::Policies,
    ) {
        let _ =
            self.0.set(Setup { modules, control_plane, migrator, placements, retained, policies });
    }

    /// Chính sách tách / gộp của mọi module.
    pub(crate) fn policies(&self) -> Result<super::relocate::Policies, Error> {
        Ok(self.get()?.policies.clone())
    }

    /// Migration còn thiếu ở `database`, theo mọi module đã đăng ký.
    pub(crate) async fn pending(
        &self,
        database: &Database,
    ) -> Result<usize, crate::migration::MigrationError> {
        match self.get() {
            Ok(setup) => Ok(setup.migrator.pending(database).await?.len()),
            Err(_) => Ok(0),
        }
    }

    /// Bảng mọi module khai giữ lại khi xoá hẳn khách.
    pub(crate) fn retained(&self) -> Result<Vec<&'static str>, Error> {
        Ok(self.get()?.retained.clone())
    }

    /// Quản lý placement, sau khi mọi module đã khởi động.
    pub(crate) fn placements(&self) -> Result<super::Placements, Error> {
        Ok(self.get()?.placements.clone())
    }

    fn get(&self) -> Result<&Setup, Error> {
        self.0.get().ok_or_else(|| Error::internal("tenant moves are used before modules started"))
    }

    /// Tên mọi module và bảng mặt điều khiển của chúng (xuất dữ liệu khách dùng chung, D-328).
    pub(crate) fn tables(&self) -> Result<(Vec<&'static str>, Vec<&'static str>), Error> {
        let setup = self.get()?;
        Ok((setup.modules.clone(), setup.control_plane.clone()))
    }
}

/// Kết quả kiểm một lượt dời, trước khi chạy.
struct Checked {
    tenant: tenant::Model,
    from: placement::Model,
    to: placement::Model,
    blockers: Vec<Error>,
    resume: Option<Model>,
    tables: Vec<TenantTableRows>,
    files: u64,
}

/// Dời khách giữa các placement. Module `kernel` công bố (`context.require::<TenantMoves>()`).
pub struct TenantMoves {
    registry: DatabaseRegistry,
    setup: MoveSetup,
    outbox: Outbox,
    jobs: JobQueue,
    /// Chờ chừng này sau khi đặt `moving` rồi mới chép (TTL cache khách + thời gian tối đa của một
    /// request).
    settle: Duration,
    translations: Translations,
}

impl std::fmt::Debug for TenantMoves {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TenantMoves").field("settle", &self.settle).finish_non_exhaustive()
    }
}

fn transfer(error: TransferError) -> Error {
    Error::internal(error.to_string())
}

/// Lỗi kèm nguyên nhân gốc, cho người vận hành đọc (cột `error`, log).
pub(super) fn describe(error: &Error) -> String {
    match std::error::Error::source(error) {
        Some(source) => format!("{}: {source}", error.code()),
        None => format!("{}: {}", error.code(), error.message()),
    }
}

impl TenantMoves {
    pub(crate) fn new(
        registry: DatabaseRegistry,
        setup: MoveSetup,
        outbox: Outbox,
        jobs: JobQueue,
        request_timeout: Duration,
        translations: Translations,
    ) -> Self {
        let settle = Duration::from_secs(registry.config().cache_seconds) + request_timeout;
        TenantMoves { registry, setup, outbox, jobs, settle, translations }
    }

    fn control(&self) -> &Database {
        self.registry.control()
    }

    async fn tenant_row(&self, id_or_slug: &str) -> Result<tenant::Model, Error> {
        let select = match id_or_slug.parse::<Uuid>() {
            Ok(id) => tenant::Entity::find_by_id(id),
            Err(_) => tenant::Entity::find().filter(tenant::Column::Slug.eq(id_or_slug)),
        };
        select
            .one(self.control().write())
            .await?
            .ok_or_else(|| errors::tenant_not_found(id_or_slug))
    }

    async fn placements(&self) -> Result<Vec<placement::Model>, Error> {
        Ok(placement::Entity::find().all(self.control().write()).await?)
    }

    async fn unfinished(&self, tenant: Uuid) -> Result<Option<Model>, Error> {
        Ok(Entity::find()
            .filter(Column::TenantId.eq(tenant))
            .filter(Column::State.is_in(UNFINISHED))
            .one(self.control().write())
            .await?)
    }

    async fn database(&self, placement: &placement::Model) -> Result<Database, Error> {
        self.registry
            .placement_database(PlacementId::from_uuid(placement.id))
            .await
            .map_err(|error| errors::tenant_move_unreachable(&placement.name, &error.message()))
    }
}

/// Bộ chạy job `kernel.move_tenant` (job của database điều khiển, không thuộc khách nào): chạy lượt
/// dời tới xong. Lần chạy lại của cùng job nhận lại lease của chính nó ngay.
pub(crate) struct MoveTenantRunner(pub(crate) Arc<TenantMoves>);

#[async_trait::async_trait]
impl JobHandler for MoveTenantRunner {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let payload: MoveTenant =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        let moved = self.0.run(payload.move_id, &format!("job-{}", job.id)).await?;
        tracing::info!(move_id = %moved.id, state = %moved.state, "tenant move job finished");
        Ok(())
    }
}
