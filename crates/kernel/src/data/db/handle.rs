//! Handles: the only way to reach the database through `db`. The handle decides the tenant: a tenant handle
//! filters every tenant record by its tenant, a control-plane handle accepts only global records, and
//! scanning every tenant is a separate, named handle.

use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, DbErr, ExecResult,
    QueryResult, Statement,
};
use uuid::Uuid;

use super::record::{Global, Record, Scope, Tenant};
use crate::database::Database;
use crate::error::Error;
use crate::tenancy::TenantContext;

mod sealed {
    pub trait Sealed {}
}

/// Who a handle reads for. Sealed: a module cannot make a handle without the kernel.
pub trait Access: sealed::Sealed + Send + Sync {
    /// The tenant every tenant record is filtered by; `None` for the control plane and for "all tenants".
    #[doc(hidden)]
    fn tenant(&self) -> Option<Uuid>;
}

/// Access of one tenant (`tenant.db()`).
#[derive(Debug, Clone, Copy)]
pub struct InTenant(pub(crate) Uuid);

/// Access of the control plane: global records only (`context.control_plane()`).
#[derive(Debug, Clone, Copy)]
pub struct ControlPlaneOnly;

/// Access across every tenant, with a reason (`control_plane.all_tenants("…")`).
#[derive(Debug, Clone, Copy)]
pub struct EveryTenant;

impl sealed::Sealed for InTenant {}
impl sealed::Sealed for ControlPlaneOnly {}
impl sealed::Sealed for EveryTenant {}

impl Access for InTenant {
    fn tenant(&self) -> Option<Uuid> {
        Some(self.0)
    }
}

impl Access for ControlPlaneOnly {
    fn tenant(&self) -> Option<Uuid> {
        None
    }
}

impl Access for EveryTenant {
    fn tenant(&self) -> Option<Uuid> {
        None
    }
}

/// Which handles accept records of a scope: [`Global`] records everywhere, [`Tenant`] records only on a
/// tenant handle or an "all tenants" handle. A tenant record on a control-plane handle does not compile:
///
/// ```compile_fail,E0277
/// # use autoxide_kernel::db::{GlobalRead, Queries as _, Record, TenantKey};
/// #[derive(Record)]
/// #[record(table = "kernel_doc_note", scope = tenant)]
/// struct Note { id: uuid::Uuid, tenant_id: TenantKey }
/// fn list(control_plane: &GlobalRead<'_>) { let _ = control_plane.find::<Note>(); }
/// ```
///
/// The same query on a tenant handle compiles (control: one token differs):
///
/// ```no_run
/// # use autoxide_kernel::db::{TenantRead, Queries as _, Record, TenantKey};
/// #[derive(Record)]
/// #[record(table = "kernel_doc_note", scope = tenant)]
/// struct Note { id: uuid::Uuid, tenant_id: TenantKey }
/// fn list(tenant: &TenantRead<'_>) { let _ = tenant.find::<Note>(); }
/// ```
pub trait AcceptedBy<A: Access>: Scope {}

impl<A: Access> AcceptedBy<A> for Global {}
impl AcceptedBy<InTenant> for Tenant {}
impl AcceptedBy<EveryTenant> for Tenant {}

/// Reads on the read connection (the read replica when configured).
pub struct Read<'a, A> {
    pub(crate) connection: &'a DatabaseConnection,
    pub(crate) access: A,
}

/// A write transaction. Dropped without [`commit`](Tx::commit), it rolls back.
pub struct Tx<A> {
    pub(crate) transaction: DatabaseTransaction,
    pub(crate) access: A,
}

/// Reads of one tenant.
pub type TenantRead<'a> = Read<'a, InTenant>;
/// A write transaction of one tenant.
pub type TenantTx = Tx<InTenant>;
/// Control-plane reads.
pub type GlobalRead<'a> = Read<'a, ControlPlaneOnly>;
/// A control-plane write transaction.
pub type GlobalTx = Tx<ControlPlaneOnly>;
/// Reads across every tenant.
pub type AllTenantsRead<'a> = Read<'a, EveryTenant>;
/// A write transaction across every tenant.
pub type AllTenantsTx = Tx<EveryTenant>;

impl<A> std::fmt::Debug for Read<'_, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Read").finish_non_exhaustive()
    }
}

impl<A> std::fmt::Debug for Tx<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tx").finish_non_exhaustive()
    }
}

/// What every handle gives the query builders. Sealed.
pub trait Handle: sealed::Sealed + Sync {
    /// The access kind of the handle.
    type Access: Access;
    #[doc(hidden)]
    fn access(&self) -> &Self::Access;
    #[doc(hidden)]
    fn connection(&self) -> Connection<'_>;
}

impl<A: Access> sealed::Sealed for Read<'_, A> {}
impl<A: Access> sealed::Sealed for Tx<A> {}

impl<A: Access> Handle for Read<'_, A> {
    type Access = A;
    fn access(&self) -> &A {
        &self.access
    }
    fn connection(&self) -> Connection<'_> {
        Connection(Inner::Read(self.connection))
    }
}

impl<A: Access> Handle for Tx<A> {
    type Access = A;
    fn access(&self) -> &A {
        &self.access
    }
    fn connection(&self) -> Connection<'_> {
        Connection(Inner::Write(&self.transaction))
    }
}

impl<A: Access> Tx<A> {
    /// Commits the transaction.
    pub async fn commit(self) -> Result<(), Error> {
        Ok(self.transaction.commit().await?)
    }

    /// Rolls the transaction back (dropping it does the same).
    pub async fn rollback(self) -> Result<(), Error> {
        Ok(self.transaction.rollback().await?)
    }

    /// Locks by name until the transaction ends, when there is no stable row to lock (Postgres advisory lock;
    /// SQLite: a write transaction already holds the whole database). On a tenant handle the name is prefixed
    /// with the tenant, so two tenants never wait on each other. Prefix `key` with the module
    /// (`catalog.similar:<…>`).
    pub async fn lock_key(&self, key: &str) -> Result<(), Error> {
        let key = match self.access.tenant() {
            Some(tenant) => format!("{tenant}:{key}"),
            None => key.to_owned(),
        };
        Ok(crate::rows::lock_key(&self.transaction, &key).await?)
    }
}

/// The connection behind a handle. Opaque: nothing in it is reachable outside the kernel.
#[doc(hidden)]
pub struct Connection<'a>(Inner<'a>);

#[derive(Clone, Copy)]
enum Inner<'a> {
    Read(&'a DatabaseConnection),
    Write(&'a DatabaseTransaction),
    // Kết nối của đường cũ (`Database::read()`, `begin_write()`…) đi qua `Executor` trong lúc chuyển.
    Legacy(&'a (dyn RawConnection + 'a)),
}

impl Connection<'_> {
    pub(crate) fn backend(&self) -> DbBackend {
        match self.0 {
            Inner::Read(connection) => connection.get_database_backend(),
            Inner::Write(transaction) => transaction.get_database_backend(),
            Inner::Legacy(connection) => connection.backend(),
        }
    }

    pub(crate) async fn all(&self, statement: Statement) -> Result<Vec<QueryResult>, DbErr> {
        match self.0 {
            Inner::Read(connection) => ConnectionTrait::query_all_raw(connection, statement).await,
            Inner::Write(transaction) => {
                ConnectionTrait::query_all_raw(transaction, statement).await
            }
            Inner::Legacy(connection) => connection.raw_query_all(statement).await,
        }
    }

    // Như `all`, database dừng câu sau `limit` (`super::bounded`). Chỉ trên handle đọc: transaction
    // ghi không mở được transaction chỉ đọc riêng, và câu ghi không nên bị cắt giữa chừng.
    pub(crate) async fn all_within(
        &self,
        statement: Statement,
        limit: std::time::Duration,
    ) -> Result<Vec<QueryResult>, Error> {
        match self.0 {
            Inner::Read(connection) => {
                super::bounded::all_within(connection, statement, limit).await
            }
            Inner::Write(_) | Inner::Legacy(_) => {
                Err(Error::internal("a time limit applies only to reads on a read handle of `db`"))
            }
        }
    }

    pub(crate) async fn execute(&self, statement: Statement) -> Result<u64, DbErr> {
        Ok(match self.0 {
            Inner::Read(connection) => ConnectionTrait::execute_raw(connection, statement).await?,
            Inner::Write(transaction) => {
                ConnectionTrait::execute_raw(transaction, statement).await?
            }
            Inner::Legacy(connection) => connection.raw_execute(statement).await?,
        }
        .rows_affected())
    }

    pub(crate) fn build(&self, statement: &impl sea_orm::StatementBuilder) -> Statement {
        statement.build(&self.backend())
    }
}

/// `tenant.db()`: the handles of one tenant. Every record of scope `Tenant` read or written through them is
/// filtered by this tenant.
#[derive(Debug, Clone, Copy)]
pub struct TenantDb<'a> {
    database: &'a Database,
    tenant: Uuid,
}

impl<'a> TenantDb<'a> {
    pub(crate) fn new(tenant: &'a TenantContext) -> Self {
        TenantDb { database: tenant.database(), tenant: tenant.id().as_uuid() }
    }

    /// Reads (read connection).
    pub fn read(&self) -> TenantRead<'a> {
        Read { connection: self.database.read(), access: InTenant(self.tenant) }
    }

    /// Opens a write transaction (`BEGIN IMMEDIATE` on SQLite).
    pub async fn begin_write(&self) -> Result<TenantTx, Error> {
        Ok(Tx { transaction: self.database.begin_write().await?, access: InTenant(self.tenant) })
    }
}

/// The control-plane handles of one module, from `StartContext::control_plane` (modules that own global
/// tables, such as `auth` and `plans`). Global records only; scanning every tenant is
/// [`all_tenants`](Self::all_tenants).
#[derive(Clone)]
pub struct ControlPlane {
    database: Database,
    module: &'static str,
}

impl std::fmt::Debug for ControlPlane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlPlane").field("module", &self.module).finish_non_exhaustive()
    }
}

impl ControlPlane {
    pub(crate) fn new(database: Database, module: &'static str) -> Self {
        ControlPlane { database, module }
    }

    /// Control-plane reads.
    pub fn read(&self) -> GlobalRead<'_> {
        Read { connection: self.database.read(), access: ControlPlaneOnly }
    }

    /// A control-plane write transaction.
    pub async fn begin_write(&self) -> Result<GlobalTx, Error> {
        Ok(Tx { transaction: self.database.begin_write().await?, access: ControlPlaneOnly })
    }

    /// Handles that read and write tenant records of **every** tenant, without the tenant filter: jobs that
    /// sweep all tenants, platform reports. `reason` says why, in a few words; each use is counted in the
    /// metric `kernel_all_tenant_queries{module}`, so the sweeps of a module are visible.
    pub fn all_tenants(&self, reason: &'static str) -> AllTenants<'_> {
        metrics::counter!("kernel_all_tenant_queries", "module" => self.module).increment(1);
        tracing::debug!(module = self.module, reason, "query across every tenant");
        AllTenants { database: &self.database }
    }
}

/// See [`ControlPlane::all_tenants`].
#[derive(Debug, Clone, Copy)]
pub struct AllTenants<'a> {
    database: &'a Database,
}

impl<'a> AllTenants<'a> {
    /// Reads across every tenant.
    pub fn read(&self) -> AllTenantsRead<'a> {
        Read { connection: self.database.read(), access: EveryTenant }
    }

    /// A write transaction across every tenant.
    pub async fn begin_write(&self) -> Result<AllTenantsTx, Error> {
        Ok(Tx { transaction: self.database.begin_write().await?, access: EveryTenant })
    }
}

/// Only for tests: handles across every tenant of `database`, used by the test kit to look at tables
/// directly. Turned on by `autoxide-testkit` (feature `testing`); never in an application.
#[cfg(feature = "testing")]
#[doc(hidden)]
pub fn all_tenants_for_tests(database: &Database) -> AllTenants<'_> {
    AllTenants { database }
}

/// Only for tests: the control-plane handle module `module` gets at start, for a test that builds a service by
/// hand. Turned on by `autoxide-testkit` (feature `testing`); never in an application.
#[cfg(feature = "testing")]
#[doc(hidden)]
pub fn control_plane_for_tests(database: &Database, module: &'static str) -> ControlPlane {
    ControlPlane::new(database.clone(), module)
}

/// Whether the handle accepts records of `R` (always true where it compiles; the bound is the check).
pub(crate) fn tenant_of_handle<H: Handle, R: Record>(handle: &H) -> Option<Uuid>
where
    R::Scope: AcceptedBy<H::Access>,
{
    if R::Scope::TENANT { handle.access().tenant() } else { None }
}

/// What a kernel function that reads or writes alongside a module accepts (`Outbox::record`, `SyncLog`,
/// `Principal::require_at`, `OneTimeTokens`…): a handle of `db` ([`TenantTx`], [`TenantRead`]…), so the kernel's
/// own statements join the module's transaction. These functions build their statements themselves and filter
/// by tenant themselves: through `Executor`, a handle only runs the kernel's own statements. Pass a write
/// transaction to a function that writes; a read handle refuses writes when the statement runs.
///
/// Until every module has moved to `db`, the connections of the old path (`Database::read()`,
/// `Database::begin_write()`) are accepted too; that part is not in the contract and goes away before 1.0.
pub trait Executor: sealed_executor::Sealed + Sync {
    #[doc(hidden)]
    fn executor(&self) -> Connection<'_>;
}

mod sealed_executor {
    // Niêm phong một nửa: module ngoài không cài thẳng `Executor` được, nhưng impl chung dưới đây cho mọi kiểu có
    // `ConnectionTrait` — crate ngoài tự cài `ConnectionTrait` cho kiểu của mình vẫn thành `Executor`. Thành niêm
    // phong thật khi gỡ impl chung (cùng lúc gỡ đường cũ, D-20261128-library-layer-plan mục 5).
    pub trait Sealed {}
    impl<C: sea_orm::ConnectionTrait> Sealed for C {}
}

// Mọi kết nối là `Executor`: kết nối SeaORM của module chưa chuyển, và — qua cầu `ConnectionTrait` ở dưới — handle
// của `db`. Thứ tự khi gỡ cầu (D-20261128-library-layer-plan mục 5): xoá hai `impl ConnectionTrait` của `Read`/`Tx`
// ở cuối file TRƯỚC, rồi mới thêm `impl Executor` + `impl Sealed` riêng cho chúng (trả `Inner::Read` /
// `Inner::Write`) — còn cầu thì hai impl đó xung đột với impl chung. Impl chung ở lại tới khi module cuối đã chuyển.
impl<C: ConnectionTrait> Executor for C {
    fn executor(&self) -> Connection<'_> {
        Connection(Inner::Legacy(self))
    }
}

// Phần gọi được qua `dyn` của `ConnectionTrait` (trait đó có hàm generic nên không dùng `dyn` trực tiếp được).
#[async_trait::async_trait]
pub(crate) trait RawConnection: Sync {
    fn backend(&self) -> DbBackend;
    async fn raw_execute(&self, statement: Statement) -> Result<ExecResult, DbErr>;
    async fn raw_execute_unprepared(&self, sql: &str) -> Result<ExecResult, DbErr>;
    async fn raw_query_one(&self, statement: Statement) -> Result<Option<QueryResult>, DbErr>;
    async fn raw_query_all(&self, statement: Statement) -> Result<Vec<QueryResult>, DbErr>;
}

#[async_trait::async_trait]
impl<C: ConnectionTrait> RawConnection for C {
    fn backend(&self) -> DbBackend {
        self.get_database_backend()
    }
    async fn raw_execute(&self, statement: Statement) -> Result<ExecResult, DbErr> {
        ConnectionTrait::execute_raw(self, statement).await
    }
    async fn raw_execute_unprepared(&self, sql: &str) -> Result<ExecResult, DbErr> {
        ConnectionTrait::execute_unprepared(self, sql).await
    }
    async fn raw_query_one(&self, statement: Statement) -> Result<Option<QueryResult>, DbErr> {
        ConnectionTrait::query_one_raw(self, statement).await
    }
    async fn raw_query_all(&self, statement: Statement) -> Result<Vec<QueryResult>, DbErr> {
        ConnectionTrait::query_all_raw(self, statement).await
    }
}

/// The connection of an [`Executor`], as a `ConnectionTrait` for the kernel's own statements. Kernel-only: it is
/// never handed to a module.
pub(crate) struct Via<'a>(Connection<'a>);

impl<'a> Via<'a> {
    pub(crate) fn of<E: Executor + ?Sized>(executor: &'a E) -> Self {
        Via(executor.executor())
    }
}

#[async_trait::async_trait]
impl ConnectionTrait for Via<'_> {
    fn get_database_backend(&self) -> DbBackend {
        self.0.backend()
    }
    async fn execute_raw(&self, statement: Statement) -> Result<ExecResult, DbErr> {
        match self.0.0 {
            Inner::Read(connection) => ConnectionTrait::execute_raw(connection, statement).await,
            Inner::Write(transaction) => ConnectionTrait::execute_raw(transaction, statement).await,
            Inner::Legacy(connection) => connection.raw_execute(statement).await,
        }
    }
    async fn execute_unprepared(&self, sql: &str) -> Result<ExecResult, DbErr> {
        match self.0.0 {
            Inner::Read(connection) => ConnectionTrait::execute_unprepared(connection, sql).await,
            Inner::Write(transaction) => {
                ConnectionTrait::execute_unprepared(transaction, sql).await
            }
            Inner::Legacy(connection) => connection.raw_execute_unprepared(sql).await,
        }
    }
    async fn query_one_raw(&self, statement: Statement) -> Result<Option<QueryResult>, DbErr> {
        match self.0.0 {
            Inner::Read(connection) => ConnectionTrait::query_one_raw(connection, statement).await,
            Inner::Write(transaction) => {
                ConnectionTrait::query_one_raw(transaction, statement).await
            }
            Inner::Legacy(connection) => connection.raw_query_one(statement).await,
        }
    }
    async fn query_all_raw(&self, statement: Statement) -> Result<Vec<QueryResult>, DbErr> {
        match self.0.0 {
            Inner::Read(connection) => ConnectionTrait::query_all_raw(connection, statement).await,
            Inner::Write(transaction) => {
                ConnectionTrait::query_all_raw(transaction, statement).await
            }
            Inner::Legacy(connection) => connection.raw_query_all(statement).await,
        }
    }
}

// Cầu chuyển tiếp (D-20261126-kernel-data-layer). KHÔNG thuộc hợp đồng (impl trait của thư viện trên kiểu của
// kernel). Hàm kernel đã nhận `Executor`, nên cầu chỉ còn giữ cho các chỗ module chạy câu SeaORM tự dựng trên handle
// (OAuth của `auth`, `ledger`, `compliance`, `conversations`, `orders` — câu đó KHÔNG qua bộ lọc khách của handle).
// Gỡ khi các chỗ đó chuyển sang `Record` (D-20261128-library-layer-plan mục 5); không thêm chỗ dùng mới.
#[async_trait::async_trait]
impl<A: Access> ConnectionTrait for Read<'_, A> {
    fn get_database_backend(&self) -> DbBackend {
        self.connection.get_database_backend()
    }
    async fn execute_raw(&self, statement: Statement) -> Result<ExecResult, DbErr> {
        self.connection.execute_raw(statement).await
    }
    async fn execute_unprepared(&self, sql: &str) -> Result<ExecResult, DbErr> {
        self.connection.execute_unprepared(sql).await
    }
    async fn query_one_raw(&self, statement: Statement) -> Result<Option<QueryResult>, DbErr> {
        self.connection.query_one_raw(statement).await
    }
    async fn query_all_raw(&self, statement: Statement) -> Result<Vec<QueryResult>, DbErr> {
        self.connection.query_all_raw(statement).await
    }
}

#[async_trait::async_trait]
impl<A: Access> ConnectionTrait for Tx<A> {
    fn get_database_backend(&self) -> DbBackend {
        self.transaction.get_database_backend()
    }
    async fn execute_raw(&self, statement: Statement) -> Result<ExecResult, DbErr> {
        self.transaction.execute_raw(statement).await
    }
    async fn execute_unprepared(&self, sql: &str) -> Result<ExecResult, DbErr> {
        self.transaction.execute_unprepared(sql).await
    }
    async fn query_one_raw(&self, statement: Statement) -> Result<Option<QueryResult>, DbErr> {
        self.transaction.query_one_raw(statement).await
    }
    async fn query_all_raw(&self, statement: Statement) -> Result<Vec<QueryResult>, DbErr> {
        self.transaction.query_all_raw(statement).await
    }
}
