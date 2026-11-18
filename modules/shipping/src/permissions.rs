//! Permission của module `shipping`: `<module>.<việc>`. Handler REST gọi `principal.require(&…)`
//! rồi kiểm phạm vi theo nút của bản ghi (`require_at` / `require_write_at`, bước 34); tool MCP
//! khai `.requires(…)`. Sổ địa chỉ của chính người mua không cần permission: mọi tài khoản đã
//! đăng nhập (kể cả tài khoản người mua, vốn không có permission — bước 34) chỉ chạm địa chỉ của
//! mình.

use autoxide_kernel::access::Permission;

pub(crate) const READ: Permission = Permission::new(
    "shipping.read",
    "Quote shipping options for a cart in your scope (agents, staff placing orders).",
)
.scoped();

pub(crate) const ADDRESSES_READ: Permission =
    Permission::new("shipping.addresses.read", "See the address books of buyers (customer care).");

pub(crate) const SETTINGS_MANAGE: Permission = Permission::new(
    "shipping.settings.manage",
    "Manage shipping zones, methods, rates, pickup points and delivery calendars in your scope.",
)
.scoped();

pub(crate) const CARRIERS_MANAGE: Permission = Permission::new(
    "shipping.carriers.manage",
    "Connect carrier accounts in your scope and test them; credentials are never shown back.",
)
.scoped();

pub(crate) const SHIPMENTS_READ: Permission = Permission::new(
    "shipping.shipments.read",
    "See the shipments in your scope with parcels, items and tracking history.",
)
.scoped();

pub(crate) const SHIPMENTS_MANAGE: Permission = Permission::new(
    "shipping.shipments.manage",
    "Create, book, split, cancel shipments in your scope and update their status by hand.",
)
.scoped();

pub(crate) const COD_MANAGE: Permission = Permission::new(
    "shipping.cod.manage",
    "Import cash-on-delivery statements of carriers and resolve their discrepancies.",
);

pub(crate) fn all() -> Vec<Permission> {
    vec![
        READ,
        ADDRESSES_READ,
        SETTINGS_MANAGE,
        CARRIERS_MANAGE,
        SHIPMENTS_READ,
        SHIPMENTS_MANAGE,
        COD_MANAGE,
    ]
}
