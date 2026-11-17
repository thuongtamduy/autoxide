//! Permission của module `ledger`. Mọi permission theo phạm vi nút: người có quyền ở nhánh X chỉ
//! thấy và ghi sổ của các nút trong nhánh X.

use autoxide_kernel::access::{Permission, Risk};

pub(crate) const READ: Permission = Permission::new(
    "ledger.read",
    "See the books, accounts, balances, entries and periods of the units in your scope.",
)
.scoped();

pub(crate) const POST: Permission =
    Permission::new("ledger.post", "Post manual entries to the books of the units in your scope.")
        .scoped()
        .with_risk(Risk::Money);

pub(crate) const REVERSE: Permission =
    Permission::new("ledger.reverse", "Reverse entries in the books of the units in your scope.")
        .scoped()
        .with_risk(Risk::Money);

pub(crate) const ACCOUNTS_MANAGE: Permission = Permission::new(
    "ledger.accounts_manage",
    "Open books and accounts, set account floors, names and codes, close accounts, in your scope.",
)
.scoped()
.with_risk(Risk::Money);

pub(crate) const PERIODS_CLOSE: Permission = Permission::new(
    "ledger.periods_close",
    "Close accounting periods of the books in your scope (cannot be undone).",
)
.scoped()
.with_risk(Risk::Money);

pub(crate) fn all() -> Vec<Permission> {
    vec![READ, POST, REVERSE, ACCOUNTS_MANAGE, PERIODS_CLOSE]
}
