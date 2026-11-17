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

mod http;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, Query};
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

pub(crate) use http::{routes, tools};

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
/// Hỏi lại job / event còn bị giữ ở nguồn sau chừng này.
const CLAIM_POLL: Duration = Duration::from_millis(500);
/// Chờ job / event ở nguồn hết bị giữ tối đa chừng này (job giữ tối đa 5 phút).
const CLAIM_WAIT: Duration = Duration::from_secs(330);
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

/// Bảng `kernel_tenant_moves` (mặt điều khiển, chỉ database điều khiển có dữ liệu).
pub(crate) struct CreateTenantMoves;

impl MigrationName for CreateTenantMoves {
    fn name(&self) -> &str {
        "m0011_create_tenant_moves"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateTenantMoves {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("kernel_tenant_moves")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("from_placement_id"))
                    .col(schema::uuid("to_placement_id"))
                    .col(schema::boolean("keep_source"))
                    .col(schema::string_len("state", 16))
                    .col(schema::text("tables"))
                    .col(schema::text_null("error"))
                    .col(schema::string_len_null("runner", 64))
                    .col(schema::timestamp_with_time_zone_null("runner_until"))
                    .col(schema::timestamp_with_time_zone("moving_since"))
                    .col(schema::uuid_null("requested_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .col(schema::timestamp_with_time_zone_null("finished_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_tenant_moves_tenant_idx")
                    .table("kernel_tenant_moves")
                    .col("tenant_id")
                    .col("id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("kernel_tenant_moves").to_owned()).await
    }
}

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
}

impl MoveSetup {
    pub(crate) fn fill(
        &self,
        modules: Vec<&'static str>,
        control_plane: Vec<&'static str>,
        migrator: Migrator,
        placements: super::Placements,
    ) {
        let _ = self.0.set(Setup { modules, control_plane, migrator, placements });
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
fn describe(error: &Error) -> String {
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

    /// Kiểm lượt dời khách `tenant` sang placement `to`: mọi lý do chặn (không dừng ở lý do đầu)
    /// và số dòng sẽ chép. Khách hay placement không có: lỗi.
    async fn check(&self, tenant: &str, to: &str) -> Result<Checked, Error> {
        let row = self.tenant_row(tenant).await?;
        let placements = self.placements().await?;
        let target = placements
            .iter()
            .find(|placement| placement.name == to)
            .cloned()
            .ok_or_else(|| errors::tenant_placement_not_found(to))?;
        let name_of = |id: Uuid| {
            placements
                .iter()
                .find(|placement| placement.id == id)
                .map_or_else(|| id.to_string(), |placement| placement.name.clone())
        };
        let mut blockers = Vec::new();
        let mut resume = None;
        if row.id == DEFAULT_TENANT.as_uuid() {
            blockers.push(errors::tenant_move_platform());
        }
        let unfinished = self.unfinished(row.id).await?;
        let from_id = match &unfinished {
            Some(moving) => {
                let now = now();
                if moving.runner_until.is_some_and(|until| until > now) {
                    blockers.push(errors::tenant_move_running(&name_of(moving.to_placement_id)));
                } else if moving.to_placement_id != target.id {
                    blockers.push(errors::tenant_move_unfinished(&name_of(moving.to_placement_id)));
                } else {
                    resume = Some(moving.clone());
                }
                moving.from_placement_id
            }
            None => {
                if target.id == row.placement_id {
                    blockers.push(errors::tenant_move_same_placement(to));
                }
                if row.status != TenantStatus::Active.as_str() {
                    blockers.push(errors::tenant_move_not_active(&row.status));
                }
                row.placement_id
            }
        };
        let source = placements
            .iter()
            .find(|placement| placement.id == from_id)
            .cloned()
            .ok_or_else(|| errors::tenant_placement_not_found(&from_id.to_string()))?;
        if target.id != MAIN_PLACEMENT.as_uuid() && target.schema_state != "ready" {
            blockers.push(errors::tenant_placement_behind(to));
        }
        let mut tables = Vec::new();
        let source_db = self.database(&source).await;
        let target_db = self.database(&target).await;
        match (source_db, target_db) {
            (Ok(source_db), Ok(target_db)) => {
                let schema = self.schema_problems(&source, &source_db, &target, &target_db).await?;
                let schema_ok = schema.is_empty();
                blockers.extend(schema);
                if schema_ok {
                    let setup = self.setup.get()?;
                    let found = TenantTables::read(
                        &source_db,
                        &target_db,
                        &setup.modules,
                        &setup.control_plane,
                    )
                    .await
                    .map_err(transfer)?;
                    let tenant = TenantId::from_uuid(row.id);
                    tables = found
                        .count(&source_db, tenant)
                        .await
                        .map_err(transfer)?
                        .into_iter()
                        .map(TenantTableRows::from)
                        .collect();
                }
            }
            (source_db, target_db) => {
                blockers.extend(source_db.err());
                blockers.extend(target_db.err());
            }
        }
        let mut files = 0;
        match self.file_stores(&source, &target).await {
            Ok(Some((from_store, _))) => {
                match from_store.keys_with_prefix(&tenant_prefix(row.id)).await {
                    Ok(keys) => files = keys.len() as u64,
                    Err(error) => blockers.push(errors::tenant_move_storage_unavailable(
                        &source.name,
                        &error.to_string(),
                    )),
                }
            }
            Ok(None) => {}
            Err(error) => blockers.push(error),
        }
        Ok(Checked { tenant: row, from: source, to: target, blockers, resume, tables, files })
    }

    /// Nơi lưu file của placement `placement`: nơi lưu riêng của nó, hay nơi lưu chung. Kèm cấu hình
    /// nơi lưu riêng (`None`: nơi lưu chung) để so hai placement.
    async fn file_store(
        &self,
        placement: &placement::Model,
    ) -> Result<(Option<crate::storage::StorageLocation>, Arc<dyn Storage>), Error> {
        let unavailable = |error: Error| {
            errors::tenant_move_storage_unavailable(&placement.name, &error.to_string())
        };
        let id = PlacementId::from_uuid(placement.id);
        match (&placement.storage_sealed, id == MAIN_PLACEMENT) {
            (Some(sealed), false) => {
                let location =
                    self.registry.storage_location(id, sealed).await.map_err(unavailable)?;
                let store = self.registry.placement_storage(id).await.map_err(unavailable)?;
                let store = store.ok_or_else(|| {
                    unavailable(Error::internal("the placement no longer has its own storage"))
                })?;
                Ok((Some(location), store))
            }
            _ => {
                let store = self.registry.shared_storage().ok_or_else(|| {
                    unavailable(Error::internal("no module provides the file storage"))
                })?;
                Ok((None, store))
            }
        }
    }

    /// Nơi lưu file của `from` và của `to` khi hai placement dùng hai nơi lưu khác nhau; `None`: cùng
    /// nơi lưu (cùng nơi lưu chung, hay nơi lưu riêng chỉ cùng một chỗ) — không có file nào phải
    /// chép hay xoá.
    async fn file_stores(
        &self,
        from: &placement::Model,
        to: &placement::Model,
    ) -> Result<Option<(Arc<dyn Storage>, Arc<dyn Storage>)>, Error> {
        let own = |placement: &placement::Model| {
            placement.storage_sealed.is_some() && placement.id != MAIN_PLACEMENT.as_uuid()
        };
        if !own(from) && !own(to) {
            return Ok(None);
        }
        let (from_location, from_store) = self.file_store(from).await?;
        let (to_location, to_store) = self.file_store(to).await?;
        if let (Some(a), Some(b)) = (&from_location, &to_location)
            && a.same_place(b)
        {
            return Ok(None);
        }
        Ok(Some((from_store, to_store)))
    }

    /// Lý do hai database chưa chép được cho nhau: cùng một database, hay thiếu migration.
    async fn schema_problems(
        &self,
        source: &placement::Model,
        source_db: &Database,
        target: &placement::Model,
        target_db: &Database,
    ) -> Result<Vec<Error>, Error> {
        let mut problems = Vec::new();
        if same_database(source_db, target_db).await.map_err(transfer)? {
            problems.push(errors::tenant_move_same_database(&target.name));
        }
        let migrator = &self.setup.get()?.migrator;
        for (placement, database) in [(source, source_db), (target, target_db)] {
            let pending = migrator.pending(database).await.map_err(|error| {
                errors::tenant_move_unreachable(&placement.name, &error.to_string())
            })?;
            if !pending.is_empty() {
                problems.push(errors::tenant_move_schema_differs(&placement.name, pending.len()));
            }
        }
        Ok(problems)
    }

    /// Kế hoạch dời khách `tenant` (ID hay slug) sang placement `to`, không đổi gì: lý do chặn
    /// (câu theo `language`) và số dòng sẽ chép.
    pub async fn plan(
        &self,
        tenant: &str,
        to: &str,
        keep_source: bool,
        language: Language,
    ) -> Result<TenantMovePlan, Error> {
        let checked = self.check(tenant, to).await?;
        let blocking: Vec<TenantMoveBlocker> = checked
            .blockers
            .iter()
            .map(|error| TenantMoveBlocker {
                code: error.code().to_owned(),
                message: self.translations.message(error, language),
            })
            .collect();
        Ok(TenantMovePlan {
            tenant_id: checked.tenant.id,
            tenant: checked.tenant.slug,
            from: checked.from.name,
            to: checked.to.name,
            keep_source: checked.resume.as_ref().map_or(keep_source, |moving| moving.keep_source),
            ready: blocking.is_empty(),
            blocking,
            resumes: checked.resume.map(|moving| moving.id),
            rows: checked.tables.iter().map(|table| table.rows).sum(),
            tables: checked.tables,
            files: checked.files,
            settle_seconds: self.settle.as_secs(),
        })
    }

    /// Bắt đầu dời khách `tenant` sang `to` (khách thành `moving`), hay trả lượt dời sang `to` đã
    /// dừng giữa chừng để chạy tiếp. Lý do chặn đầu tiên thành lỗi. Chưa chép gì: gọi
    /// [`run`](Self::run) (CLI) — `enqueue` thì một job nền chạy (REST / MCP).
    pub async fn start(
        &self,
        actor: Actor,
        tenant: &str,
        to: &str,
        keep_source: bool,
        enqueue: bool,
    ) -> Result<TenantMove, Error> {
        let checked = self.check(tenant, to).await?;
        if let Some(blocker) = checked.blockers.into_iter().next() {
            return Err(blocker);
        }
        if let Some(moving) = checked.resume {
            tracing::info!(move_id = %moving.id, tenant_id = %moving.tenant_id, state = %moving.state, "resuming a tenant move");
            if enqueue {
                self.jobs
                    .enqueue_database(
                        self.control().write(),
                        &MoveTenant { move_id: moving.id },
                        EnqueueOptions::default(),
                    )
                    .await?;
            }
            return self.describe(moving).await;
        }
        let row = checked.tenant;
        let at = now();
        let model = entity::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(row.id),
            from_placement_id: Set(row.placement_id),
            to_placement_id: Set(checked.to.id),
            keep_source: Set(keep_source),
            state: Set(COPYING.to_owned()),
            tables: Set("[]".to_owned()),
            error: Set(None),
            runner: Set(None),
            runner_until: Set(None),
            moving_since: Set(at),
            requested_by: Set(actor.user_id),
            created_at: Set(at),
            updated_at: Set(at),
            finished_at: Set(None),
        };
        // Khách thành `moving` và lượt dời được ghi cùng lúc (cùng database điều khiển): hai lệnh dời
        // cùng lúc thì điều kiện `status = active` chỉ cho một bên qua.
        let transaction = self.control().begin_write().await?;
        let updated = tenant::Entity::update_many()
            .col_expr(tenant::Column::Status, Expr::value(TenantStatus::Moving.as_str()))
            .col_expr(tenant::Column::UpdatedAt, Expr::value(at))
            .filter(tenant::Column::Id.eq(row.id))
            .filter(tenant::Column::Status.eq(TenantStatus::Active.as_str()))
            .filter(tenant::Column::PlacementId.eq(row.placement_id))
            .exec(&transaction)
            .await?;
        if updated.rows_affected != 1 {
            return Err(errors::tenant_move_not_active(TenantStatus::Moving.as_str()));
        }
        let moving = model.insert(&transaction).await?;
        if enqueue {
            self.jobs
                .enqueue_database(
                    &transaction,
                    &MoveTenant { move_id: moving.id },
                    EnqueueOptions::default(),
                )
                .await?;
        }
        transaction.commit().await?;
        self.registry.invalidate(TenantId::from_uuid(row.id));
        tracing::info!(
            move_id = %moving.id,
            tenant_id = %row.id,
            from = %checked.from.name,
            to = %checked.to.name,
            keep_source,
            "tenant move started; the tenant answers 503 until the copy is done"
        );
        self.describe(moving).await
    }

    /// Chạy lượt dời `id` tới xong (`done` / `aborted`). `runner` phân biệt bên chạy (CLI, job).
    /// Lỗi: lượt dời dừng ở trạng thái đang có, lỗi được ghi vào `error`; gọi lại thì chạy tiếp.
    pub async fn run(&self, id: Uuid, runner: &str) -> Result<TenantMove, Error> {
        loop {
            let moving = self.step(id, runner).await?;
            if !UNFINISHED.contains(&moving.state.as_str()) {
                return Ok(moving);
            }
        }
    }

    /// Làm một bước của lượt dời `id` (`copying` → `copied` → `switched` → `done`) rồi trả lượt dời.
    /// Lỗi được ghi vào `error` của lượt dời.
    pub async fn step(&self, id: Uuid, runner: &str) -> Result<TenantMove, Error> {
        let Some(moving) = self.acquire(id, runner).await? else {
            let moving = self.find(id).await?;
            return self.describe(moving).await;
        };
        let advanced = self.holding(id, runner, self.advance(&moving, runner)).await;
        let released = self.release(id, runner).await;
        match advanced {
            Ok(moving) => {
                released?;
                self.describe(moving).await
            }
            Err(error) => {
                let message = describe(&error);
                tracing::warn!(move_id = %id, error = %message, "tenant move stopped");
                let _ = Entity::update_many()
                    .col_expr(Column::Error, Expr::value(message))
                    .col_expr(Column::UpdatedAt, Expr::value(now()))
                    .filter(Column::Id.eq(id))
                    .exec(self.control().write())
                    .await;
                Err(error)
            }
        }
    }

    async fn find(&self, id: Uuid) -> Result<Model, Error> {
        Entity::find_by_id(id)
            .one(self.control().write())
            .await?
            .ok_or_else(|| errors::tenant_move_not_found(&id.to_string()))
    }

    /// Nhận lease của lượt dời `id`. `None`: lượt dời đã kết thúc. Bên khác đang giữ:
    /// `tenant_move_running`.
    async fn acquire(&self, id: Uuid, runner: &str) -> Result<Option<Model>, Error> {
        let at = now();
        let until = at + chrono::Duration::from_std(LEASE).map_err(Error::internal)?;
        let taken = Entity::update_many()
            .col_expr(Column::Runner, Expr::value(runner))
            .col_expr(Column::RunnerUntil, Expr::value(until))
            .filter(Column::Id.eq(id))
            .filter(Column::State.is_in(UNFINISHED))
            .filter(
                Condition::any()
                    .add(Column::RunnerUntil.is_null())
                    .add(Column::RunnerUntil.lt(at))
                    .add(Column::Runner.eq(runner)),
            )
            .exec(self.control().write())
            .await?;
        let moving = self.find(id).await?;
        if taken.rows_affected == 1 {
            return Ok(Some(moving));
        }
        if UNFINISHED.contains(&moving.state.as_str()) {
            let placements = self.placements().await?;
            let to = placements
                .iter()
                .find(|placement| placement.id == moving.to_placement_id)
                .map_or_else(|| moving.to_placement_id.to_string(), |p| p.name.clone());
            return Err(errors::tenant_move_running(&to));
        }
        Ok(None)
    }

    async fn release(&self, id: Uuid, runner: &str) -> Result<(), Error> {
        Entity::update_many()
            .col_expr(Column::RunnerUntil, Expr::value(Option::<DateTime<Utc>>::None))
            .filter(Column::Id.eq(id))
            .filter(Column::Runner.eq(runner))
            .exec(self.control().write())
            .await?;
        Ok(())
    }

    /// Chạy `work`, gia hạn lease của lượt dời `id` mỗi một phần ba thời hạn cho tới khi xong.
    /// Gia hạn hỏng (vd database điều khiển SQLite đang bận vì chính bước chép) chỉ ghi log: bên
    /// khác cũng cần kết nối ghi đó để nhận lease, nên không chen vào được.
    async fn holding<T>(
        &self,
        id: Uuid,
        runner: &str,
        work: impl Future<Output = Result<T, Error>>,
    ) -> Result<T, Error> {
        let control = self.control().clone();
        let runner = runner.to_owned();
        let renew = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(LEASE / 3);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let until = now()
                    + chrono::Duration::seconds(
                        <i64 as TryFrom<u64>>::try_from(LEASE.as_secs()).unwrap_or(120),
                    );
                let renewed = Entity::update_many()
                    .col_expr(Column::RunnerUntil, Expr::value(until))
                    .filter(Column::Id.eq(id))
                    .filter(Column::Runner.eq(runner.as_str()))
                    .exec(control.write())
                    .await;
                if let Err(error) = renewed {
                    tracing::warn!(move_id = %id, error = %error, "cannot extend the tenant move lease");
                }
            }
        });
        let outcome = work.await;
        renew.abort();
        outcome
    }

    async fn advance(&self, moving: &Model, runner: &str) -> Result<Model, Error> {
        match moving.state.as_str() {
            COPYING => self.copy(moving, runner).await,
            COPIED => self.switch(moving, runner).await,
            SWITCHED => self.clean(moving, runner).await,
            _ => Ok(moving.clone()),
        }
    }

    async fn placement(&self, id: Uuid) -> Result<placement::Model, Error> {
        placement::Entity::find_by_id(id)
            .one(self.control().write())
            .await?
            .ok_or_else(|| errors::tenant_placement_not_found(&id.to_string()))
    }

    async fn tables(&self, source: &Database, target: &Database) -> Result<TenantTables, Error> {
        let setup = self.setup.get()?;
        TenantTables::read(source, target, &setup.modules, &setup.control_plane)
            .await
            .map_err(transfer)
    }

    /// Ghi trạng thái mới của lượt dời — chỉ khi bên này còn giữ lease và trạng thái chưa đổi.
    async fn set_state<C: ConnectionTrait>(
        &self,
        connection: &C,
        moving: &Model,
        runner: &str,
        to: &str,
        state: &str,
        tables: Option<&[CopiedTable]>,
    ) -> Result<(), Error> {
        let at = now();
        let mut update = Entity::update_many()
            .col_expr(Column::State, Expr::value(state))
            .col_expr(Column::Error, Expr::value(Option::<String>::None))
            .col_expr(Column::UpdatedAt, Expr::value(at))
            .filter(Column::Id.eq(moving.id))
            .filter(Column::State.eq(moving.state.as_str()))
            .filter(Column::Runner.eq(runner));
        if let Some(tables) = tables {
            let rows: Vec<TenantTableRows> =
                tables.iter().cloned().map(TenantTableRows::from).collect();
            let json = serde_json::to_string(&rows).map_err(Error::internal)?;
            update = update.col_expr(Column::Tables, Expr::value(json));
        }
        if state == DONE || state == ABORTED {
            update = update
                .col_expr(Column::FinishedAt, Expr::value(at))
                .col_expr(Column::RunnerUntil, Expr::value(Option::<DateTime<Utc>>::None));
        }
        if update.exec(connection).await?.rows_affected != 1 {
            // Bên này mất lease (giữ quá lâu mà không gia hạn được) và bên khác đã nhận lượt dời.
            return Err(errors::tenant_move_running(to));
        }
        Ok(())
    }

    /// `copying` → `copied`.
    async fn copy(&self, moving: &Model, runner: &str) -> Result<Model, Error> {
        let tenant = TenantId::from_uuid(moving.tenant_id);
        let row = self.tenant_row(&moving.tenant_id.to_string()).await?;
        let (from, to) = (
            self.placement(moving.from_placement_id).await?,
            self.placement(moving.to_placement_id).await?,
        );
        // Máy chủ khác thấy `moving` sau tối đa TTL cache; request đã bắt đầu trước đó chạy xong
        // trong thời gian tối đa của một request.
        let ready_at = moving.moving_since
            + chrono::Duration::from_std(self.settle).map_err(Error::internal)?;
        if let Ok(wait) = (ready_at - now()).to_std() {
            tracing::info!(move_id = %moving.id, seconds = wait.as_secs(), "waiting for other servers to see the move");
            tokio::time::sleep(wait).await;
        }
        let source = self.database(&from).await?;
        let target = self.database(&to).await?;
        if let Some(problem) =
            self.schema_problems(&from, &source, &to, &target).await?.into_iter().next()
        {
            return Err(problem);
        }
        let tables = self.tables(&source, &target).await?;
        wait_claims(&source, &tables, tenant).await?;
        // File trước dòng: chép file hỏng thì đích chưa có gì đổi; chạy lại chép lại từ đầu.
        let files = match self.file_stores(&from, &to).await? {
            Some((from_store, to_store)) => {
                copy_files(&*from_store, &*to_store, &tenant_prefix(moving.tenant_id))
                    .await
                    .map_err(|error| {
                        errors::tenant_move_storage_unavailable(&to.name, &error.to_string())
                    })?
            }
            None => 0,
        };
        let context = TenantContext::new(
            tenant,
            row.slug.clone(),
            row.name.clone(),
            TenantStatus::Moving,
            PlacementId::from_uuid(to.id),
            target.clone(),
        );
        let actor = moving.requested_by.map_or_else(Actor::system, Actor::user);
        let transaction = target.begin_write().await?;
        let copied = tables.copy(&source, &transaction, tenant).await.map_err(transfer)?;
        let event = TenantMoved {
            tenant_id: moving.tenant_id,
            from: from.name.clone(),
            to: to.name.clone(),
            rows: copied.iter().map(|table| table.rows).sum(),
        };
        let subject = format!("kernel.tenant:{}", moving.tenant_id);
        self.outbox.record(&transaction, &context, actor, Some(subject), &event).await?;
        transaction.commit().await?;
        verify(&tables, &source, &target, tenant).await?;
        self.set_state(self.control().write(), moving, runner, &to.name, COPIED, Some(&copied))
            .await?;
        tracing::info!(move_id = %moving.id, tenant_id = %tenant, rows = event.rows, files, "tenant rows and files copied");
        self.find(moving.id).await
    }

    /// `copied` → `switched`: khách dùng placement mới, `active`.
    async fn switch(&self, moving: &Model, runner: &str) -> Result<Model, Error> {
        let tenant = TenantId::from_uuid(moving.tenant_id);
        let (from, to) = (
            self.placement(moving.from_placement_id).await?,
            self.placement(moving.to_placement_id).await?,
        );
        let source = self.database(&from).await?;
        let target = self.database(&to).await?;
        let tables = self.tables(&source, &target).await?;
        if let Err(error) = verify(&tables, &source, &target, tenant).await {
            // Nguồn đổi sau khi chép (không được xảy ra khi khách `moving`): lần chạy sau chép lại.
            let _ = self
                .set_state(self.control().write(), moving, runner, &to.name, COPYING, None)
                .await;
            return Err(error);
        }
        let at = now();
        let transaction = self.control().begin_write().await?;
        let switched = tenant::Entity::update_many()
            .col_expr(tenant::Column::PlacementId, Expr::value(to.id))
            .col_expr(tenant::Column::Status, Expr::value(TenantStatus::Active.as_str()))
            .col_expr(tenant::Column::UpdatedAt, Expr::value(at))
            .filter(tenant::Column::Id.eq(moving.tenant_id))
            .filter(tenant::Column::PlacementId.eq(from.id))
            .filter(tenant::Column::Status.eq(TenantStatus::Moving.as_str()))
            .exec(&transaction)
            .await?;
        if switched.rows_affected != 1 {
            return Err(Error::internal(format!(
                "tenant {} is no longer `moving` on `{}`; the move cannot switch it",
                moving.tenant_id, from.name
            )));
        }
        self.set_state(&transaction, moving, runner, &to.name, SWITCHED, None).await?;
        transaction.commit().await?;
        self.registry.invalidate(tenant);
        tracing::info!(move_id = %moving.id, tenant_id = %tenant, placement = %to.name, "tenant switched to its new placement");
        self.find(moving.id).await
    }

    /// `switched` → `done`: xoá dòng của khách ở nguồn (`keep_source`: chỉ hàng đợi job và outbox).
    async fn clean(&self, moving: &Model, runner: &str) -> Result<Model, Error> {
        let tenant = TenantId::from_uuid(moving.tenant_id);
        let (from, to) = (
            self.placement(moving.from_placement_id).await?,
            self.placement(moving.to_placement_id).await?,
        );
        let source = self.database(&from).await?;
        let target = self.database(&to).await?;
        // Chặn lần nữa ngay trước khi xoá: xoá "nguồn" là cùng database với đích thì mất dữ liệu.
        if same_database(&source, &target).await.map_err(transfer)? {
            return Err(errors::tenant_move_same_database(&to.name));
        }
        let tables = self.tables(&source, &target).await?;
        let keep = moving.keep_source;
        let transaction = source.begin_write().await?;
        let deleted = tables
            .delete(&transaction, tenant, |table| !keep || tables.kind(table) != TableKind::Data)
            .await
            .map_err(transfer)?;
        transaction.commit().await?;
        let mut files = 0;
        if !keep && let Some((from_store, to_store)) = self.file_stores(&from, &to).await? {
            files = delete_files(&*from_store, &*to_store, &tenant_prefix(moving.tenant_id))
                .await
                .map_err(|error| {
                    errors::tenant_move_storage_unavailable(&from.name, &error.to_string())
                })?;
        }
        self.set_state(self.control().write(), moving, runner, &to.name, DONE, None).await?;
        tracing::info!(move_id = %moving.id, tenant_id = %tenant, deleted, files, keep_source = keep, "tenant move done");
        self.find(moving.id).await
    }

    /// Huỷ lượt dời chưa đổi placement của khách `tenant`: xoá dòng đã chép ở đích, khách trở lại
    /// `active` ở placement cũ. Đã đổi placement: `tenant_move_not_abortable` (chạy tiếp thay vì huỷ).
    pub async fn abort(&self, tenant: &str) -> Result<TenantMove, Error> {
        let row = self.tenant_row(tenant).await?;
        let moving = self
            .unfinished(row.id)
            .await?
            .ok_or_else(|| errors::tenant_move_not_found("unfinished"))?;
        if moving.state == SWITCHED {
            return Err(errors::tenant_move_not_abortable());
        }
        let runner = format!("abort-{}", Uuid::now_v7());
        let Some(moving) = self.acquire(moving.id, &runner).await? else {
            return self.describe(self.find(moving.id).await?).await;
        };
        let tenant_id = TenantId::from_uuid(moving.tenant_id);
        let (from, to) = (
            self.placement(moving.from_placement_id).await?,
            self.placement(moving.to_placement_id).await?,
        );
        let source = self.database(&from).await?;
        // Đích hỏng hẳn (lý do hay gặp để huỷ): vẫn huỷ được; dòng đã chép còn ở đó thì xoá sau bằng
        // `purge`.
        let cleaned = async {
            let target = self.database(&to).await?;
            let tables = self.tables(&source, &target).await?;
            let transaction = target.begin_write().await?;
            tables.delete(&transaction, tenant_id, |_| true).await.map_err(transfer)?;
            transaction.commit().await?;
            // File đã chép sang nơi lưu của đích (nếu khác nơi lưu của nguồn).
            if let Some((from_store, to_store)) = self.file_stores(&from, &to).await? {
                delete_files(&*to_store, &*from_store, &tenant_prefix(moving.tenant_id)).await?;
            }
            Ok::<_, Error>(())
        }
        .await;
        if let Err(error) = &cleaned {
            tracing::warn!(move_id = %moving.id, error = %error, "cannot delete the copied rows at the target; purge them later");
        }
        let at = now();
        let transaction = self.control().begin_write().await?;
        tenant::Entity::update_many()
            .col_expr(tenant::Column::Status, Expr::value(TenantStatus::Active.as_str()))
            .col_expr(tenant::Column::UpdatedAt, Expr::value(at))
            .filter(tenant::Column::Id.eq(moving.tenant_id))
            .filter(tenant::Column::PlacementId.eq(from.id))
            .filter(tenant::Column::Status.eq(TenantStatus::Moving.as_str()))
            .exec(&transaction)
            .await?;
        self.set_state(&transaction, &moving, &runner, &to.name, ABORTED, None).await?;
        if let Err(error) = cleaned {
            Entity::update_many()
                .col_expr(Column::Error, Expr::value(describe(&error)))
                .filter(Column::Id.eq(moving.id))
                .exec(&transaction)
                .await?;
        }
        transaction.commit().await?;
        self.registry.invalidate(tenant_id);
        tracing::info!(move_id = %moving.id, tenant_id = %tenant_id, "tenant move aborted");
        self.describe(self.find(moving.id).await?).await
    }

    /// Xoá bản sao dữ liệu của khách `tenant` còn ở placement `placement` (để lại bởi `keep_source`
    /// hay lượt dời huỷ mà không xoá được đích). Không bao giờ xoá placement khách đang dùng. Trả
    /// số dòng đã xoá mỗi bảng.
    pub async fn purge(
        &self,
        tenant: &str,
        placement: &str,
    ) -> Result<Vec<TenantTableRows>, Error> {
        let row = self.tenant_row(tenant).await?;
        let placements = self.placements().await?;
        let copy = placements
            .iter()
            .find(|found| found.name == placement)
            .ok_or_else(|| errors::tenant_placement_not_found(placement))?;
        if copy.id == row.placement_id {
            return Err(errors::tenant_purge_current(placement));
        }
        if self.unfinished(row.id).await?.is_some() {
            return Err(errors::tenant_moving());
        }
        let current = placements
            .iter()
            .find(|found| found.id == row.placement_id)
            .ok_or_else(|| errors::tenant_placement_not_found(&row.placement_id.to_string()))?;
        let live = self.database(current).await?;
        let database = self.database(copy).await?;
        if same_database(&live, &database).await.map_err(transfer)? {
            return Err(errors::tenant_move_same_database(placement));
        }
        let tables = self.tables(&live, &database).await?;
        let tenant_id = TenantId::from_uuid(row.id);
        let counts = tables.count(&database, tenant_id).await.map_err(transfer)?;
        let transaction = database.begin_write().await?;
        tables.delete(&transaction, tenant_id, |_| true).await.map_err(transfer)?;
        transaction.commit().await?;
        let mut files = 0;
        if let Some((copy_store, live_store)) = self.file_stores(copy, current).await? {
            files =
                delete_files(&*copy_store, &*live_store, &tenant_prefix(row.id)).await.map_err(
                    |error| errors::tenant_move_storage_unavailable(placement, &error.to_string()),
                )?;
        }
        tracing::info!(tenant_id = %row.id, placement, files, "copy of the tenant's data deleted");
        Ok(counts.into_iter().map(TenantTableRows::from).filter(|table| table.rows > 0).collect())
    }

    /// Lượt dời `id` của khách `tenant` (ID hay slug).
    pub async fn get(&self, tenant: &str, id: Uuid) -> Result<TenantMove, Error> {
        let row = self.tenant_row(tenant).await?;
        let moving = Entity::find_by_id(id)
            .filter(Column::TenantId.eq(row.id))
            .one(self.control().write())
            .await?
            .ok_or_else(|| errors::tenant_move_not_found(&id.to_string()))?;
        self.describe(moving).await
    }

    async fn describe(&self, moving: Model) -> Result<TenantMove, Error> {
        let placements = self.placements().await?;
        let name_of = |id: Uuid| {
            placements
                .iter()
                .find(|placement| placement.id == id)
                .map_or_else(|| id.to_string(), |placement| placement.name.clone())
        };
        let tables: Vec<TenantTableRows> =
            serde_json::from_str(&moving.tables).map_err(Error::internal)?;
        Ok(TenantMove {
            id: moving.id,
            tenant_id: moving.tenant_id,
            from: name_of(moving.from_placement_id),
            to: name_of(moving.to_placement_id),
            keep_source: moving.keep_source,
            state: moving.state,
            rows: tables.iter().map(|table| table.rows).sum(),
            tables,
            error: moving.error,
            created_at: moving.created_at,
            updated_at: moving.updated_at,
            finished_at: moving.finished_at,
        })
    }
}

/// Tiền tố khoá file của khách `tenant` (không phải khách mặc định — khách đó không dời được).
fn tenant_prefix(tenant: Uuid) -> String {
    format!("tenants/{tenant}/")
}

/// `a` và `b` thật ra là cùng một nơi lưu: ghi một file đánh dấu (dưới `prefix`) vào `b` rồi tìm nó
/// ở `a`. Cấu hình khác nhau vẫn có thể trỏ cùng một chỗ (vd thư mục của nơi lưu chung) — khi đó
/// chép hay xoá "nguồn" là xoá file đang dùng.
async fn same_store(a: &dyn Storage, b: &dyn Storage, prefix: &str) -> Result<bool, Error> {
    let marker = format!("{prefix}move-check-{}", Uuid::now_v7());
    let content = futures_util::stream::iter([Ok(bytes::Bytes::from_static(b"autoxide"))]);
    b.put(&marker, Box::pin(content)).await?;
    let found = a.get(&marker).await;
    b.delete(&marker).await?;
    Ok(found?.is_some())
}

/// Chép mọi file dưới `prefix` từ `source` sang `target` (đè file cùng khoá, xoá file thừa dưới
/// `prefix` ở `target`), rồi so số file. Trả số file đã chép; `0` khi hai bên là cùng một nơi lưu.
async fn copy_files(
    source: &dyn Storage,
    target: &dyn Storage,
    prefix: &str,
) -> Result<u64, Error> {
    if same_store(source, target, prefix).await? {
        return Ok(0);
    }
    let keys = source.keys_with_prefix(prefix).await?;
    let wanted: std::collections::HashSet<&str> = keys.iter().map(String::as_str).collect();
    for stale in target.keys_with_prefix(prefix).await? {
        if !wanted.contains(stale.as_str()) {
            target.delete(&stale).await?;
        }
    }
    for key in &keys {
        // File biến mất giữa lúc liệt kê và lúc đọc (khách `moving` không ghi gì; chỉ job dọn có
        // thể xoá): bỏ qua, phép so bên dưới bắt lệch.
        let Some(file) = source.get(key).await? else { continue };
        target.put(key, file.content).await?;
    }
    let copied = target.keys_with_prefix(prefix).await?.len();
    if copied != keys.len() {
        return Err(errors::tenant_move_count_mismatch("files", keys.len() as u64, copied as u64));
    }
    Ok(keys.len() as u64)
}

/// Xoá mọi file dưới `prefix` ở `doomed` — trừ khi `doomed` thật ra là cùng nơi lưu với `kept`.
/// Trả số file đã xoá.
async fn delete_files(
    doomed: &dyn Storage,
    kept: &dyn Storage,
    prefix: &str,
) -> Result<u64, Error> {
    if same_store(doomed, kept, prefix).await? {
        return Ok(0);
    }
    let keys = doomed.keys_with_prefix(prefix).await?;
    for key in &keys {
        doomed.delete(key).await?;
    }
    Ok(keys.len() as u64)
}

/// So số dòng của khách ở hai database, từng bảng; đích có thêm đúng một event
/// `kernel.tenant_moved`.
async fn verify(
    tables: &TenantTables,
    source: &Database,
    target: &Database,
    tenant: TenantId,
) -> Result<(), Error> {
    let before = tables.count(source, tenant).await.map_err(transfer)?;
    let after = tables.count(target, tenant).await.map_err(transfer)?;
    for (source, target) in before.iter().zip(&after) {
        let expected = source.rows + u64::from(source.table == KERNEL_EVENTS);
        if target.rows != expected {
            return Err(errors::tenant_move_count_mismatch(
                &source.table,
                source.rows,
                target.rows,
            ));
        }
    }
    Ok(())
}

/// Chờ tới khi không job hay event nào của khách `tenant` ở `source` còn bị giữ (worker hay bộ phát
/// đang chạy nó). Worker và bộ phát không nhận việc mới của khách `moving`, nên chỉ phải chờ phần đã
/// nhận trước đó — tối đa thời hạn giữ của job.
async fn wait_claims(
    source: &Database,
    tables: &TenantTables,
    tenant: TenantId,
) -> Result<(), Error> {
    let started = std::time::Instant::now();
    loop {
        let at = now();
        let mut busy = 0u64;
        for (table, kind) in tables.queues() {
            let finished = match kind {
                TableKind::Jobs => "completed_at",
                _ => "delivered_at",
            };
            let select = Query::select()
                .expr_as(Expr::cust("COUNT(*)"), Alias::new("busy"))
                .from(Alias::new(table))
                .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.as_uuid()))
                .and_where(Expr::col(Alias::new("claimed_until")).gt(at))
                .and_where(Expr::col(Alias::new(finished)).is_null())
                .and_where(Expr::col(Alias::new("failed_at")).is_null())
                .to_owned();
            let row = source.write().query_one(&select).await?;
            let count: i64 = match row {
                Some(row) => row.try_get("", "busy")?,
                None => 0,
            };
            busy += <u64 as TryFrom<i64>>::try_from(count).unwrap_or(0);
        }
        if busy == 0 {
            return Ok(());
        }
        if started.elapsed() > CLAIM_WAIT {
            return Err(errors::tenant_move_busy(busy));
        }
        tokio::time::sleep(CLAIM_POLL).await;
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
