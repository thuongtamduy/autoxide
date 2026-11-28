//! The kernel's database layer for modules (D-20261126-kernel-data-layer): records, typed columns, queries
//! and writes, without any database library type in a module's code. When the library under it moves to a
//! new major version, only the kernel changes.
//!
//! ```ignore
//! use autoxide_kernel::db::{Queries as _, Record, TenantKey};
//!
//! #[derive(Record, Clone, Debug)]
//! #[record(table = "notes_note", scope = tenant)]
//! pub(crate) struct Note {
//!     #[record(primary_key)]
//!     pub id: Uuid,
//!     pub tenant_id: TenantKey,
//!     pub title: String,
//! }
//!
//! let db = tenant.db();
//! let note = db.read().find::<Note>().filter(Note::ID.eq(id)).one().await?;
//! let tx = db.begin_write().await?;
//! tx.update::<Note>().set(Note::TITLE, title).filter(Note::ID.eq(id)).exec().await?;
//! tx.commit().await?;
//! ```
//!
//! Rules the layer enforces:
//!
//! - **The handle decides the tenant.** `tenant.db()` filters every tenant record by the tenant, on reads,
//!   writes, joins and subqueries; an insert of a row of another tenant is refused; `ON CONFLICT DO UPDATE`
//!   never touches a row of another tenant. The control-plane handle (`StartContext::control_plane`) does
//!   not compile with tenant records; sweeping every tenant is `control_plane.all_tenants("reason")`.
//! - **A module only touches its own tables** (architecture rule 1): `#[derive(Record)]` refuses, at compile
//!   time, a table not named `<module>_…`. Two modules whose names would share a prefix (`notes`, `notes_extra`)
//!   cannot be registered together, so the prefix is unambiguous.
//! - **Every record is checked at startup** against the live schema of the control database (table, columns,
//!   nullability, `tenant_id` against the scope; a global record only on a table the module lists in
//!   `control_plane_tables`); `cargo xtask check` makes sure every derived record is declared in
//!   `Module::records`. Placements built by the same migrations are not checked again.
//! - **Both databases behave the same**: row locks, bounded `adjust`, integer aggregates, `LIKE` escaping.
//!
//! The record must declare its scope and, for a tenant record, a `tenant_id: TenantKey` field:
//!
//! ```compile_fail
//! # use autoxide_kernel::db::Record;
//! #[derive(Record)]
//! #[record(table = "kernel_doc_note")]
//! struct Note { id: uuid::Uuid }
//! ```
//!
//! ```compile_fail
//! # use autoxide_kernel::db::{Record, TenantKey};
//! #[derive(Record)]
//! #[record(table = "kernel_doc_note", scope = global)]
//! struct Note { id: uuid::Uuid, tenant_id: TenantKey }
//! ```
//!
//! ```compile_fail
//! # use autoxide_kernel::db::Record;
//! #[derive(Record)]
//! #[record(table = "kernel_doc_note", scope = tenant)]
//! struct Note { id: uuid::Uuid, tenant_id: uuid::Uuid }
//! ```
//!
//! ```compile_fail
//! # use autoxide_kernel::db::{Record, TenantKey};
//! #[derive(Record)]
//! #[record(table = "orders_order", scope = tenant)]
//! struct Order { id: uuid::Uuid, tenant_id: TenantKey }
//! ```
//!
//! ```compile_fail
//! # use autoxide_kernel::db::{Record, TenantKey};
//! #[derive(Record)]
//! #[record(table = "kernel_doc_note", scope = tenant)]
//! struct Note { id: uuid::Uuid, r#tenant_id: TenantKey }
//! ```
//!
//! ```compile_fail
//! # use autoxide_kernel::db::Record;
//! struct TenantKey(uuid::Uuid);
//! #[derive(Record)]
//! #[record(table = "kernel_doc_note", scope = tenant)]
//! struct Note { id: uuid::Uuid, tenant_id: TenantKey }
//! ```
//!
//! Controls — each differs from one case above by one token, and compiles:
//!
//! ```
//! # use autoxide_kernel::db::{Record, TenantKey};
//! #[derive(Record)]
//! #[record(table = "kernel_doc_note", scope = global)]
//! struct Plain { id: uuid::Uuid }
//! #[derive(Record)]
//! #[record(table = "kernel_doc_note", scope = tenant)]
//! struct Note { id: uuid::Uuid, tenant_id: TenantKey }
//! #[derive(Record)]
//! #[record(table = "kernel_order", scope = tenant)]
//! struct Order { id: uuid::Uuid, tenant_id: TenantKey }
//! ```
//!
//! The library types stay inside the kernel. Kernel functions that write alongside a module take
//! `&impl Executor`, which every handle is.

mod bounded;
mod check;
mod handle;
mod query;
mod record;
mod sql;
mod value;
mod write;

/// `#[derive(Record)]`: see [`Record`].
pub use autoxide_macros::Record;
pub use bounded::{QUERY_TIMEOUT, is_query_timeout};
pub use check::RecordInfo;
pub(crate) use check::control_plane_prefix_problem;
pub(crate) use check::problems as record_problems;
pub(crate) use check::register_control_plane;
#[cfg(feature = "testing")]
#[doc(hidden)]
pub use check::register_control_plane_for_tests;
#[doc(hidden)]
pub use handle::Connection;
pub(crate) use handle::Via;
pub use handle::{
    AcceptedBy, Access, AllTenants, AllTenantsRead, AllTenantsTx, ControlPlane, ControlPlaneOnly,
    EveryTenant, Executor, GlobalRead, GlobalTx, Handle, InTenant, Read, TenantDb, TenantRead,
    TenantTx, Tx,
};
#[cfg(feature = "testing")]
#[doc(hidden)]
pub use handle::{all_tenants_for_tests, control_plane_for_tests};
pub use query::{Find, Grouped, HasId, Queries, Sub};
pub use record::{
    Agg, Col, ColName, Cond, Expr, Global, On, Record, Scope, Selectable, Selection, Tenant,
    Updated, count, count_distinct, max, min, sum,
};
pub use sql::Sql;
pub use value::{DbValue, FromRow, Row, Settable, SortKey, TenantKey, UuidKey, Value};
pub use write::{Bounds, Delete, Insert, UNIQUE_VIOLATION, Update, is_unique_violation};
