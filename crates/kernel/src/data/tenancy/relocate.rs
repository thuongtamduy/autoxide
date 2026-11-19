//! Tách / gộp khách (bước 38 mục 3, D-20261118-step38-relocation): tách một nhánh tổ chức của khách
//! T thành khách riêng T' ở database khác, hay gộp khách T' vào khách T dưới một nút — **giữ nguyên
//! khoá chính và lịch sử**.
//!
//! - Mỗi module khai cách từng bảng cấp khách của mình đi theo ([`Module::relocation`]): theo nút
//!   ([`TablePolicy::ByOrg`]), theo dòng cha ([`TablePolicy::FollowParent`]), chép theo dòng trỏ tới
//!   ([`TablePolicy::Referenced`]), cả khách ([`TablePolicy::TenantWide`]), phải rỗng
//!   ([`TablePolicy::MustBeEmpty`]) hay tự làm ([`TablePolicy::Custom`]). Bảng không khai thì suy
//!   từ cột (có `org_id` → theo nút; hàng đợi job / outbox → phải rỗng; còn lại → ở lại) kèm cảnh
//!   báo trong kế hoạch; test của binary chặn bảng không khai.
//! - Một lượt là một dòng `kernel_relocations` (mặt điều khiển) đi qua `planned` → `frozen` →
//!   `copied` → `verified` → `switched` → `cleaned` → `done` (hay `failed`); mỗi bước chạy lại được
//!   (`autoxide tenant split --resume <id>`, job `kernel.relocate`). Không có transaction xuyên hai
//!   database: chép là MỘT transaction ở đích, chuyển là một transaction ở database điều khiển,
//!   dọn chỉ xoá ở nguồn dòng đã có ở đích.
//! - Nguồn và đích phải là hai database khác nhau (dời trong cùng database: dùng
//!   `autoxide tenant move` trước). Khách mới khi tách có ID = ID nút gốc nhánh (nút gốc của khách
//!   có ID bằng ID khách, D-198), nên gộp ngược lại trả mọi dòng về đúng chỗ cũ.
//! - Chỉ nhân sự nền tảng chạy được (CLI, hay REST với `dry_run` mặc định, D-210); khách chỉ gửi
//!   yêu cầu (`POST /api/v1/kernel/tenant/relocation-requests`), yêu cầu vào hộp việc của nhân sự
//!   nền tảng.
//!
//! [`Module::relocation`]: crate::module::Module::relocation

mod http;
mod kernel_tables;
mod plan;
mod policy;
mod prepare;
mod run;
mod steps;
mod store;

use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use sea_orm::DatabaseTransaction;
use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, Query};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{OrgId, TenantId};
use crate::access::{Permission, Risk};
use crate::config::Setting;
use crate::database::Database;
use crate::error::Error;
use crate::event::EventInfo;

pub(crate) use http::{RelocationRequestsWork, routes};
pub(crate) use kernel_tables::kernel_relocation;
pub(crate) use policy::Policies;
pub(crate) use run::Wiring;
pub use run::{RelocationInput, TenantRelocations};
pub(crate) use store::CreateRelocations;

/// Tách / gộp khách (chỉ nhân sự nền tảng).
pub const TENANTS_RELOCATE: Permission = Permission::new(
    "kernel.tenants.relocate",
    "Split a branch of a tenant into a tenant of its own, or merge a tenant into another (platform \
     staff only); the tenants answer 503 while it runs.",
)
.with_risk(Risk::Destructive);

/// Quản trị viên của khách gửi yêu cầu tách / gộp cho nhân sự nền tảng.
pub const RELOCATIONS_REQUEST: Permission = Permission::new(
    "kernel.relocations.request",
    "Ask the platform staff to split a branch of your tenant into a tenant of its own, or to merge \
     tenants.",
);

pub(crate) const RELOCATION_STARTED: EventInfo = EventInfo::new(
    "kernel.relocation_started",
    "A split or merge of tenants started; the tenant answers 503 until it is done.",
);
pub(crate) const RELOCATION_FINISHED: EventInfo = EventInfo::new(
    "kernel.relocation_finished",
    "A split or merge of tenants finished: the rows moved to the target tenant.",
);
pub(crate) const RELOCATION_FAILED: EventInfo = EventInfo::new(
    "kernel.relocation_failed",
    "A split or merge of tenants stopped before changing anything; the tenant works again.",
);

/// Chờ outbox / job đến hạn của khách rỗng trước khi chép.
pub const DRAIN_SECONDS: Setting = Setting::new(
    "AUTOXIDE_RELOCATION_DRAIN_SECONDS",
    "Seconds a split or merge of tenants waits for the tenant's queued events and due jobs to run \
     before it gives up (1 to 86400).",
)
.with_default("300");

/// Loại job chạy một lượt tách / gộp bắt đầu qua REST.
pub(crate) const RELOCATE: &str = "kernel.relocate";

// Tách hay gộp.
/// Split or merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RelocationKind {
    /// A branch of the organization tree becomes a tenant of its own.
    Split,
    /// A tenant joins another tenant under one of its units.
    Merge,
}

impl RelocationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RelocationKind::Split => "split",
            RelocationKind::Merge => "merge",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "split" => Some(RelocationKind::Split),
            "merge" => Some(RelocationKind::Merge),
            _ => None,
        }
    }
}

/// Dòng `TenantWide` khi tách.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Split {
    /// Ở lại khách nguồn; khách mới không có.
    Stay,
    /// Chép sang khách mới, khách nguồn vẫn giữ (cấu hình dùng chung: role…).
    Copy,
}

/// Dòng `TenantWide` khi gộp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Merge {
    /// Chuyển sang khách nhận (dòng mà khoá chính đã có ở khách nhận — bản đã chép khi tách — bỏ
    /// qua: cùng một dòng).
    Import,
    /// Không chuyển: ở lại với khách đã gộp (thuê bao, phiên, bộ đếm…).
    Skip,
}

/// Cách một bảng cấp khách đi theo khi tách / gộp.
#[derive(Clone)]
#[non_exhaustive]
pub enum TablePolicy {
    /// Dòng có nút `column` thuộc nhánh tách thì đi theo nhánh; `NULL` hay nút ngoài nhánh ở lại.
    ByOrg { column: &'static str },
    /// Bảng soi cây tổ chức (một dòng mỗi nút, `id` là ID nút, `parent` là nút cha): như `ByOrg`
    /// theo `id`, kèm nút cha của gốc nhánh thành `NULL` khi tách và thành nút nhận khi gộp.
    OrgTree { id: &'static str, parent: &'static str },
    /// Dòng theo dòng cha ở `parent_table` (`column` trỏ `parent_column`): cha đi thì con đi, cha
    /// được chép thì con được chép.
    FollowParent { parent_table: &'static str, column: &'static str, parent_column: &'static str },
    /// Dòng mà dòng của `by_table` (đi hay được chép) trỏ tới qua `by_column` → `column`: được CHÉP
    /// sang khách mới, khách nguồn vẫn giữ (vd tài khoản của người có role ở nút trong nhánh).
    Referenced { by_table: &'static str, by_column: &'static str, column: &'static str },
    /// Cả khách, không theo nút.
    TenantWide { on_split: Split, on_merge: Merge },
    /// Hàng đợi (outbox, job): phần đến hạn của khách phải chạy hết trước khi chép; không chép.
    MustBeEmpty,
    /// Bảng do bảng `by` của cùng module xử lý cùng lúc (vd đầu chuỗi băm của audit).
    HandledBy { by: &'static str },
    /// Module tự làm (audit, cây tổ chức, dãy số).
    Custom(Arc<dyn RelocateTable>),
}

impl TablePolicy {
    /// Tên chính sách trong kế hoạch.
    pub fn name(&self) -> &'static str {
        match self {
            TablePolicy::ByOrg { .. } => "by_org",
            TablePolicy::OrgTree { .. } => "org_tree",
            TablePolicy::FollowParent { .. } => "follow_parent",
            TablePolicy::Referenced { .. } => "referenced",
            TablePolicy::TenantWide { .. } => "tenant_wide",
            TablePolicy::MustBeEmpty => "must_be_empty",
            TablePolicy::HandledBy { .. } => "handled_by",
            TablePolicy::Custom(_) => "custom",
        }
    }
}

impl std::fmt::Debug for TablePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Cột chứa secret đã mã hoá theo khách (`Sealer`, AAD gồm khách): khi đổi khách, giá trị được mã
/// hoá lại cho khách mới — không thì khách mới không mở được.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SealedColumn {
    pub table: &'static str,
    pub column: &'static str,
    /// Mục đích đã dùng khi mã hoá (vd `webhooks.endpoint_secret`).
    pub purpose: &'static str,
    /// Cột định danh dòng đã dùng khi mã hoá (thường `id`).
    pub row_column: &'static str,
}

/// Chính sách tách / gộp của một module ([`Module::relocation`](crate::module::Module::relocation)).
#[derive(Clone, Default)]
pub struct Relocation {
    tables: Vec<(&'static str, TablePolicy)>,
    sealed: Vec<SealedColumn>,
    switch_hooks: Vec<Arc<dyn SwitchHook>>,
}

impl std::fmt::Debug for Relocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Relocation")
            .field("tables", &self.tables)
            .field("sealed", &self.sealed)
            .field("switch_hooks", &self.switch_hooks.len())
            .finish()
    }
}

/// Lượt tách / gộp đang chuyển (bước `switch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RelocationSwitch {
    pub kind: RelocationKind,
    /// Khách có dữ liệu đi (khách bị tách nhánh; khách bị gộp).
    pub source: TenantId,
    /// Khách nhận (khách mới khi tách; khách nhận khi gộp).
    pub target: TenantId,
}

/// Việc module phải làm ĐỒNG BỘ khi một lượt tách / gộp chuyển (D-20261119-relocation-switch-hook):
/// chạy trong transaction của bước `switch` ở database điều khiển, TRƯỚC khi khách mới hoạt động.
/// Lỗi thì cả transaction bỏ: khách mới không mở, lượt dừng ở bước trước và chạy lại được. Dùng cho
/// chính sách bảo mật đặt theo ID khách ở database điều khiển (vd rào quyền của `auth`), thứ phải
/// có hiệu lực ngay từ request đầu tiên tới khách mới. Phải chạy lại được.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait SwitchHook: Send + Sync {
    async fn switch(
        &self,
        control: &DatabaseTransaction,
        relocation: &RelocationSwitch,
    ) -> Result<(), Error>;
}

impl Relocation {
    /// Chạy `hook` trong bước chuyển của mọi lượt tách / gộp ([`SwitchHook`]).
    pub fn on_switch(mut self, hook: Arc<dyn SwitchHook>) -> Self {
        self.switch_hooks.push(hook);
        self
    }

    /// Hook của bước chuyển đã khai.
    pub fn switch_hooks(&self) -> &[Arc<dyn SwitchHook>] {
        &self.switch_hooks
    }

    /// Không khai gì: mọi bảng suy từ cột (cảnh báo trong kế hoạch). Cài đặt mặc định của
    /// `Module::relocation`.
    pub fn inferred() -> Self {
        Relocation::default()
    }

    pub fn new() -> Self {
        Relocation::default()
    }

    /// Bảng `table` theo `policy`.
    pub fn table(mut self, table: &'static str, policy: TablePolicy) -> Self {
        self.tables.push((table, policy));
        self
    }

    /// Các bảng theo nút ở cột `org_id`.
    pub fn by_org(self, tables: &[&'static str]) -> Self {
        tables
            .iter()
            .fold(self, |this, table| this.table(table, TablePolicy::ByOrg { column: "org_id" }))
    }

    /// Bảng `table` theo dòng cha ở `parent_table` (`column` trỏ `parent_table.id`).
    pub fn follow(
        self,
        table: &'static str,
        column: &'static str,
        parent_table: &'static str,
    ) -> Self {
        self.table(table, TablePolicy::FollowParent { parent_table, column, parent_column: "id" })
    }

    /// Bảng `table` được chép theo dòng của `by_table` trỏ tới (`by_table.by_column` → `id`).
    pub fn referenced(
        self,
        table: &'static str,
        by_table: &'static str,
        by_column: &'static str,
    ) -> Self {
        self.table(table, TablePolicy::Referenced { by_table, by_column, column: "id" })
    }

    /// Cấu hình cả khách: chép sang khách mới khi tách, chuyển khi gộp.
    pub fn copy_on_split(self, tables: &[&'static str]) -> Self {
        self.tenant_wide(tables, Split::Copy, Merge::Import)
    }

    /// Dữ liệu cả khách ở lại khách nguồn khi tách, chuyển khi gộp (vd người mua, người đăng ký).
    pub fn stay(self, tables: &[&'static str]) -> Self {
        self.tenant_wide(tables, Split::Stay, Merge::Import)
    }

    /// Dữ liệu tạm hay riêng của khách (phiên, bộ đếm, token, thuê bao): không đi đâu cả.
    pub fn ephemeral(self, tables: &[&'static str]) -> Self {
        self.tenant_wide(tables, Split::Stay, Merge::Skip)
    }

    fn tenant_wide(self, tables: &[&'static str], on_split: Split, on_merge: Merge) -> Self {
        tables.iter().fold(self, |this, table| {
            this.table(table, TablePolicy::TenantWide { on_split, on_merge })
        })
    }

    /// Hàng đợi phải rỗng (outbox, job — nhận ra theo cột nên thường không cần khai).
    pub fn must_be_empty(self, tables: &[&'static str]) -> Self {
        tables.iter().fold(self, |this, table| this.table(table, TablePolicy::MustBeEmpty))
    }

    /// Bảng `table` do bảng `by` xử lý cùng lúc.
    pub fn handled_by(self, table: &'static str, by: &'static str) -> Self {
        self.table(table, TablePolicy::HandledBy { by })
    }

    /// Bảng `table` do module tự làm.
    pub fn custom(self, table: &'static str, handler: Arc<dyn RelocateTable>) -> Self {
        self.table(table, TablePolicy::Custom(handler))
    }

    /// Cột `table.column` chứa secret mã hoá theo khách với `purpose`, dòng định danh bằng
    /// `row_column`.
    pub fn sealed(
        mut self,
        table: &'static str,
        column: &'static str,
        purpose: &'static str,
        row_column: &'static str,
    ) -> Self {
        self.sealed.push(SealedColumn { table, column, purpose, row_column });
        self
    }

    /// Các bảng đã khai.
    pub fn tables(&self) -> &[(&'static str, TablePolicy)] {
        &self.tables
    }

    /// Các cột secret đã khai.
    pub fn sealed_columns(&self) -> &[SealedColumn] {
        &self.sealed
    }
}

/// Phần module tự làm cho một bảng ([`TablePolicy::Custom`]). Mọi hàm chạy lại được: `copy` chạy
/// trong transaction ở đích (hỏng thì đích như chưa chép), `clean` trong transaction ở nguồn và chỉ
/// xoá dòng đã có ở đích.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait RelocateTable: Send + Sync {
    /// Số dòng sẽ đi, xung đột, cảnh báo — không đổi gì.
    async fn plan(&self, context: &RelocationContext) -> Result<TableReport, Error>;
    /// Ghi phần của bảng vào đích (qua `target`); trả số dòng đã ghi.
    async fn copy(
        &self,
        context: &RelocationContext,
        target: &DatabaseTransaction,
    ) -> Result<u64, Error>;
    /// Xoá ở nguồn (qua `source`) phần đã chuyển; trả số dòng đã xoá.
    async fn clean(
        &self,
        context: &RelocationContext,
        source: &DatabaseTransaction,
    ) -> Result<u64, Error>;
}

/// Kế hoạch của một bảng `Custom`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TableReport {
    pub rows: u64,
    pub conflicts: Vec<RelocationConflict>,
    pub warnings: Vec<String>,
}

impl TableReport {
    pub fn rows(rows: u64) -> Self {
        TableReport { rows, ..TableReport::default() }
    }
}

/// Thứ một bảng `Custom` cần biết về lượt tách / gộp.
#[derive(Clone)]
pub struct RelocationContext {
    id: Uuid,
    kind: RelocationKind,
    source_tenant: TenantId,
    target_tenant: TenantId,
    source: Database,
    target: Database,
    org: OrgId,
    moved: Arc<HashSet<Uuid>>,
    tables: crate::transfer::TenantTables,
}

impl RelocationContext {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: Uuid,
        kind: RelocationKind,
        source_tenant: TenantId,
        target_tenant: TenantId,
        source: Database,
        target: Database,
        org: OrgId,
        moved: HashSet<Uuid>,
        tables: crate::transfer::TenantTables,
    ) -> Self {
        RelocationContext {
            id,
            kind,
            source_tenant,
            target_tenant,
            source,
            target,
            org,
            moved: Arc::new(moved),
            tables,
        }
    }

    /// Bảng cấp khách của hai database (đọc / ghi dòng theo kiểu cột).
    pub(crate) fn tables(&self) -> &crate::transfer::TenantTables {
        &self.tables
    }

    /// ID của lượt tách / gộp.
    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn kind(&self) -> RelocationKind {
        self.kind
    }

    /// Khách có dữ liệu đi (T khi tách, T' khi gộp).
    pub fn source_tenant(&self) -> TenantId {
        self.source_tenant
    }

    /// Khách nhận dữ liệu (T' khi tách, T khi gộp).
    pub fn target_tenant(&self) -> TenantId {
        self.target_tenant
    }

    /// Database của khách nguồn.
    pub fn source(&self) -> &Database {
        &self.source
    }

    /// Database của khách đích.
    pub fn target(&self) -> &Database {
        &self.target
    }

    /// Tách: nút gốc của nhánh (= ID khách mới). Gộp: nút của khách nhận mà gốc của khách bị gộp
    /// thành con.
    pub fn org(&self) -> OrgId {
        self.org
    }

    /// Tách: `id` là một nút của nhánh, hay khoá chính của một dòng đi / được chép theo chính sách
    /// chung. Gộp: luôn `true` (mọi thứ của khách bị gộp đi).
    pub fn moved(&self, id: Uuid) -> bool {
        self.kind == RelocationKind::Merge || self.moved.contains(&id)
    }

    /// Điều kiện "dòng của khách nguồn có `column` là một nút của nhánh" (tách), hay "dòng của
    /// khách nguồn" (gộp). Đọc ở database nguồn.
    pub fn branch_condition(&self, column: &str) -> Condition {
        let mine = Condition::all()
            .add(Expr::col(Alias::new("tenant_id")).eq(self.source_tenant.as_uuid()));
        match self.kind {
            RelocationKind::Merge => mine,
            RelocationKind::Split => mine.add(
                Expr::col(Alias::new(column)).in_subquery(
                    Query::select()
                        .column(Alias::new("descendant_id"))
                        .from(Alias::new("kernel_org_closure"))
                        .and_where(
                            Expr::col(Alias::new("tenant_id")).eq(self.source_tenant.as_uuid()),
                        )
                        .and_where(Expr::col(Alias::new("ancestor_id")).eq(self.org.as_uuid()))
                        .to_owned(),
                ),
            ),
        }
    }
}

/// Something that blocks a split or merge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RelocationBlocker {
    /// Stable code, the same as the error a real run would answer.
    #[schema(example = "tenancy.relocation_same_database")]
    pub code: String,
    pub message: String,
}

/// Rows that cannot move as they are: a reference across the boundary or a unique value taken in
/// the target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RelocationConflict {
    #[schema(example = "content_entries")]
    pub table: String,
    /// `reference` (a row that stays points to a row that goes, or the other way), `unique` (the
    /// target tenant already has the value) or a code of the module.
    #[schema(example = "unique")]
    pub code: String,
    pub message: String,
    /// Rows in conflict.
    pub count: u64,
    /// Keys of the first rows in conflict (at most 20).
    pub rows: Vec<String>,
}

impl RelocationConflict {
    pub fn new(
        table: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
        rows: Vec<String>,
    ) -> Self {
        let count = rows.len() as u64;
        RelocationConflict {
            table: table.into(),
            code: code.into(),
            message: message.into(),
            count,
            rows: rows.into_iter().take(20).collect(),
        }
    }
}

/// Something the operator should know; it does not block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RelocationWarning {
    #[schema(example = "shop_notes")]
    pub table: String,
    #[schema(example = "relocation_policy_inferred")]
    pub code: String,
    pub message: String,
}

/// What happens to one table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RelocationTable {
    #[schema(example = "ledger_entries")]
    pub table: String,
    /// The policy of the table: `by_org`, `org_tree`, `follow_parent`, `referenced`,
    /// `tenant_wide`, `must_be_empty`, `handled_by` or `custom`.
    #[schema(example = "follow_parent")]
    pub policy: String,
    /// `move` (copied to the target, then deleted from the source), `copy` (the source keeps
    /// them too), `stay`, `drain` (due queue rows that must run first) or `custom`.
    #[schema(example = "move")]
    pub action: String,
    /// Rows concerned.
    pub rows: u64,
    /// Merge: rows whose key the target already has (the copy made when the tenant was split);
    /// they are skipped as the same rows.
    pub existing: u64,
}

/// What a split or merge would do (dry run). Nothing is changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RelocationPlan {
    pub kind: RelocationKind,
    /// Tenant whose rows go.
    pub source_tenant_id: Uuid,
    #[schema(example = "acme")]
    pub source: String,
    /// Tenant that receives them (split: the new tenant, its id is the id of the branch unit).
    pub target_tenant_id: Uuid,
    #[schema(example = "acme-north")]
    pub target: String,
    /// Split: the root unit of the branch. Merge: the unit of the receiving tenant the merged
    /// tenant goes under.
    pub org_id: Uuid,
    /// Placement (database) of the target.
    #[schema(example = "acme-north-db")]
    pub placement: String,
    /// `true` when nothing blocks and there is no conflict.
    pub ready: bool,
    pub blocking: Vec<RelocationBlocker>,
    pub conflicts: Vec<RelocationConflict>,
    pub warnings: Vec<RelocationWarning>,
    /// Every tenant-level table, in copy order.
    pub tables: Vec<RelocationTable>,
    /// Rows that move or are copied.
    pub rows: u64,
}

/// A split or merge of tenants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TenantRelocation {
    pub id: Uuid,
    pub kind: RelocationKind,
    /// `requested` (asked by the tenant, waiting for the platform staff), `planned`, `frozen`
    /// (the tenants answer 503), `copied`, `verified`, `switched`, `cleaned`, `done` or `failed`.
    #[schema(example = "done")]
    pub state: String,
    pub source_tenant_id: Uuid,
    pub target_tenant_id: Option<Uuid>,
    pub org_id: Option<Uuid>,
    pub placement: Option<String>,
    pub reason: String,
    /// The plan made when it started (or when it was requested).
    pub plan: Option<RelocationPlan>,
    /// Rows written to the target per table (after `copied`).
    pub tables: Vec<super::TenantTableRows>,
    /// Why the last run stopped; running it again resumes it.
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Lỗi của phần tách / gộp (mã ổn định, bản dịch ở `crates/kernel/locales/vi.json`).
pub(crate) mod errors {
    use crate::error::Error;

    pub fn conflict(count: usize, id: &str) -> Error {
        Error::conflict(
            "tenancy.relocation_conflict",
            "The split or merge cannot run: {count} conflicts, see the report of run `{id}`.",
        )
        .with_param("count", count)
        .with_param("id", id)
    }

    pub fn in_progress(id: &str) -> Error {
        Error::conflict(
            "tenancy.relocation_in_progress",
            "This tenant has a split or merge that is not finished (`{id}`). Resume it with \
             `--resume` or wait until it is done.",
        )
        .with_param("id", id)
    }

    pub fn not_found(id: &str) -> Error {
        Error::not_found("tenancy.relocation_not_found", "There is no split or merge `{id}`.")
            .with_param("id", id)
    }

    pub fn org_invalid(org: &str, tenant: &str) -> Error {
        Error::conflict(
            "tenancy.relocation_org_invalid",
            "Unit `{org}` is not a unit below the root of tenant `{tenant}`.",
        )
        .with_param("org", org)
        .with_param("tenant", tenant)
    }

    pub fn same_database(placement: &str) -> Error {
        Error::conflict(
            "tenancy.relocation_same_database",
            "The target placement `{placement}` is the database of the source tenant. Split into \
             another database, or move one tenant first with `autoxide tenant move`.",
        )
        .with_param("placement", placement)
    }

    pub fn tenant_exists(id: &str) -> Error {
        Error::conflict(
            "tenancy.relocation_tenant_exists",
            "A tenant with id `{id}` already exists, so the branch cannot become a tenant with \
             that id.",
        )
        .with_param("id", id)
    }

    pub fn not_active(tenant: &str, status: &str) -> Error {
        Error::conflict(
            "tenancy.relocation_not_active",
            "Tenant `{tenant}` is `{status}`; only an active tenant can be split or merged.",
        )
        .with_param("tenant", tenant)
        .with_param("status", status)
    }

    pub fn platform() -> Error {
        Error::conflict(
            "tenancy.relocation_platform",
            "The platform tenant cannot be split or merged.",
        )
    }

    pub fn cycle(tenant: &str) -> Error {
        Error::conflict(
            "tenancy.relocation_cycle",
            "Tenant `{tenant}` is below the tenant being merged; merge in the other direction.",
        )
        .with_param("tenant", tenant)
    }

    pub fn schema_differs(placement: &str, pending: usize) -> Error {
        Error::conflict(
            "tenancy.relocation_schema_differs",
            "The database of placement `{placement}` misses {pending} migrations. Run \
             `autoxide migrate up --all` first.",
        )
        .with_param("placement", placement)
        .with_param("pending", pending)
    }

    pub fn drain_timeout(rows: u64) -> Error {
        Error::conflict(
            "tenancy.relocation_drain_timeout",
            "{rows} queued events or due jobs of the tenant did not run in time; the tenant works \
             again. Try later, or raise AUTOXIDE_RELOCATION_DRAIN_SECONDS.",
        )
        .with_param("rows", rows)
    }

    pub fn count_mismatch(table: &str, source: u64, target: u64) -> Error {
        Error::conflict(
            "tenancy.relocation_count_mismatch",
            "Table `{table}`: {source} rows should be in the target, {target} are. Run the split \
             or merge again to copy again.",
        )
        .with_param("table", table)
        .with_param("source", source)
        .with_param("target", target)
    }
}
