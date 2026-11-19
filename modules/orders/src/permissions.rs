//! Permission của module `orders`: `<module>.<việc>`. Handler REST gọi `principal.require(&…)` rồi
//! kiểm phạm vi theo nút của bản ghi (`require_at` / `require_write_at`, bước 34); tool MCP khai
//! `.requires(…)`.
//!
//! Người mua KHÔNG cần permission (D-20261118-step45-orders-api): tài khoản người mua của khách
//! (`Realm::Customer`, bước 34) đăng nhập được nhưng không có permission nào, nên đường `/me/…`
//! chỉ cần đăng nhập và chỉ chạm giỏ / đơn của chính người đó — như sổ địa chỉ của `shipping`.
//! Người mua vãng lai dùng `Cart-Token` / token xem đơn.

use autoxide_kernel::access::Permission;

pub(crate) const READ: Permission = Permission::new(
    "orders.read",
    "See orders, carts and abandoned carts in your scope, with lines, payments and history.",
)
.scoped();

pub(crate) const MANAGE: Permission = Permission::new(
    "orders.manage",
    "Move orders in your scope through their lifecycle, request shipments, record manual \
     payments, resend payment links, hand over store pickups.",
)
.scoped();

pub(crate) const REFUND: Permission = Permission::new(
    "orders.refund",
    "Refund orders in your scope (through the payment partner when the payment went through it).",
)
.scoped();

pub(crate) const RETURNS_MANAGE: Permission = Permission::new(
    "orders.returns.manage",
    "Approve, reject, receive and refund return requests in your scope.",
)
.scoped();

pub(crate) const POS_SELL: Permission = Permission::new(
    "orders.pos.sell",
    "Sell at the counter of a store in your scope (point of sale).",
)
.scoped();

pub(crate) const POS_SHIFT: Permission = Permission::new(
    "orders.pos.shift",
    "Open and close cash register shifts of stores in your scope.",
)
.scoped();

pub(crate) const REVIEWS_MODERATE: Permission = Permission::new(
    "orders.reviews.moderate",
    "Reply to, hide (with an allowed reason) and reopen product reviews in your scope; review AI \
     summary drafts.",
)
.scoped();

pub(crate) const RISK_MANAGE: Permission = Permission::new(
    "orders.risk.manage",
    "See cash-on-delivery statistics of buyers and manage cash-on-delivery rules and block lists \
     in your scope.",
)
.scoped();

pub(crate) const WARRANTY_MANAGE: Permission = Permission::new(
    "orders.warranty.manage",
    "Look up warranties and handle warranty claims in your scope.",
)
.scoped();

pub(crate) const REPORTS_READ: Permission =
    Permission::new("orders.reports.read", "See sales reports of your scope.").scoped();

pub(crate) const SETTINGS_MANAGE: Permission = Permission::new(
    "orders.settings.manage",
    "Manage order policies of nodes in your scope: returns, cancellation, cash on delivery, \
     invoices, pre-orders, store pickup.",
)
.scoped();

pub(crate) fn all() -> Vec<Permission> {
    vec![
        READ,
        MANAGE,
        REFUND,
        RETURNS_MANAGE,
        POS_SELL,
        POS_SHIFT,
        REVIEWS_MODERATE,
        RISK_MANAGE,
        WARRANTY_MANAGE,
        REPORTS_READ,
        SETTINGS_MANAGE,
    ]
}
