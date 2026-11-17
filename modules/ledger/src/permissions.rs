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

pub(crate) const PAYMENTS_CREATE: Permission = Permission::new(
    "ledger.payments_create",
    "Create payments through the payment partners for the units in your scope.",
)
.scoped()
.with_risk(Risk::Money);

pub(crate) const REFUNDS_CREATE: Permission = Permission::new(
    "ledger.refunds_create",
    "Refund payments received by the units in your scope.",
)
.scoped()
.with_risk(Risk::Money);

pub(crate) const RECONCILE: Permission = Permission::new(
    "ledger.reconcile",
    "Run reconciliations with partner statements and resolve the differences found.",
)
.with_risk(Risk::Money);

pub(crate) const SETTINGS_MANAGE: Permission = Permission::new(
    "ledger.settings_manage",
    "Set the money-flow mode, hold days and partners, and register bank accounts, for the units in \
     your scope.",
)
.scoped()
.with_risk(Risk::Money);

pub(crate) const PAYOUTS_CREATE: Permission = Permission::new(
    "ledger.payouts_create",
    "Pay sellers in your scope their available balance through the payout partner.",
)
.scoped()
.with_risk(Risk::Money);

pub(crate) const HOLDS_RELEASE: Permission = Permission::new(
    "ledger.holds_release",
    "Release held seller money before the return window ends, in the books of your scope.",
)
.scoped()
.with_risk(Risk::Money);

pub(crate) fn all() -> Vec<Permission> {
    vec![
        READ,
        POST,
        REVERSE,
        ACCOUNTS_MANAGE,
        PERIODS_CLOSE,
        PAYMENTS_CREATE,
        REFUNDS_CREATE,
        RECONCILE,
        SETTINGS_MANAGE,
        PAYOUTS_CREATE,
        HOLDS_RELEASE,
    ]
}
