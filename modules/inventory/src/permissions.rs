//! Permission của module `inventory`: `<module>.<việc>`. Mọi permission theo nút (bước 34): độ
//! sâu `unit` / `subtree` = kho của nút / nhánh; `tenant` = mọi kho của khách. Handler REST gọi
//! `principal.require(&…)` rồi service kiểm phạm vi ở nút kho (`require_at` / `require_write_at`);
//! tool MCP khai `.requires(…)`.

use autoxide_kernel::access::Permission;

pub(crate) const READ: Permission = Permission::new(
    "inventory.read",
    "See stock levels, the movement ledger, lots and the stock valuation of warehouses in your scope.",
)
.scoped();

pub(crate) const ADJUST: Permission = Permission::new(
    "inventory.adjust",
    "Adjust stock with a reason, apply stock counts and release stuck reservations in your scope.",
)
.scoped();

pub(crate) const RECEIVE: Permission = Permission::new(
    "inventory.receive",
    "Receive goods into warehouses in your scope (goods receipts).",
)
.scoped();

pub(crate) const TRANSFER: Permission = Permission::new(
    "inventory.transfer",
    "Ship stock transfers from, and receive them into, warehouses in your scope.",
)
.scoped();

pub(crate) const COUNT: Permission = Permission::new(
    "inventory.count",
    "Open stock counts and enter counted quantities in your scope.",
)
.scoped();

pub(crate) const RESERVATIONS_READ: Permission = Permission::new(
    "inventory.reservations.read",
    "See stock reservations of orders sold by units in your scope.",
)
.scoped();

pub(crate) const SETTINGS_MANAGE: Permission = Permission::new(
    "inventory.settings.manage",
    "Set reorder points, which warehouses serve a store and how a store picks warehouses.",
)
.scoped();

pub(crate) const PURCHASING_READ: Permission = Permission::new(
    "inventory.purchasing.read",
    "See suppliers, purchase orders and supplier returns in your scope.",
)
.scoped();

pub(crate) const PURCHASING_MANAGE: Permission = Permission::new(
    "inventory.purchasing.manage",
    "Manage suppliers, purchase orders and supplier returns in your scope.",
)
.scoped();

pub(crate) fn all() -> Vec<Permission> {
    vec![
        READ,
        ADJUST,
        RECEIVE,
        TRANSFER,
        COUNT,
        RESERVATIONS_READ,
        SETTINGS_MANAGE,
        PURCHASING_READ,
        PURCHASING_MANAGE,
    ]
}
