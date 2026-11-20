//! Permission của module `customers`: `<module>.<việc>`. Handler REST gọi `principal.require(&…)` rồi
//! kiểm phạm vi theo nút của hồ sơ (`require_at`, bước 34); tool MCP khai `.requires(…)`.
//!
//! Người mua KHÔNG cần permission (như `orders`, D-20261118-step45-orders-api): tài khoản người mua
//! (`Realm::Customer`) đăng nhập được nhưng không có permission nào, nên đường `/me` chỉ cần đăng
//! nhập và chỉ chạm hồ sơ của chính người đó. Bỏ `customers.self` của đặc tả
//! (D-20261120-step47-customers).

use autoxide_kernel::access::Permission;

pub(crate) const READ: Permission = Permission::new(
    "customers.read",
    "See customer profiles in your scope with masked phone and email, purchase history and groups.",
)
.scoped();

pub(crate) const MANAGE: Permission = Permission::new(
    "customers.manage",
    "Register customers at the counter by phone number and edit customer profiles in your scope.",
)
.scoped();

pub(crate) const CONTACT_REVEAL: Permission = Permission::new(
    "customers.contact.reveal",
    "See the full phone number and email of customers in your scope; every look is audited.",
)
.scoped();

pub(crate) const SEGMENTS_MANAGE: Permission = Permission::new(
    "customers.segments.manage",
    "Create, change and delete customer groups and their rules, and add or remove members.",
)
.scoped();

pub(crate) const LOYALTY_MANAGE: Permission = Permission::new(
    "customers.loyalty.manage",
    "Set the loyalty program: earning and redeeming rules, membership tiers and voucher rewards.",
);

pub(crate) const LOYALTY_ADJUST: Permission = Permission::new(
    "customers.loyalty.adjust",
    "Add or remove loyalty points of customers in your scope by hand, with a reference and a reason.",
)
.scoped();

pub(crate) const ENGAGEMENT_MANAGE: Permission = Permission::new(
    "customers.engagement.manage",
    "Set abandoned cart reminders and referral rewards.",
);

pub(crate) const STORED_VALUE_MANAGE: Permission = Permission::new(
    "customers.stored_value.manage",
    "Issue, look up and void gift cards and change customers' store credit (only when gift cards or store credit are turned on).",
);

pub(crate) fn all() -> Vec<Permission> {
    vec![
        READ,
        MANAGE,
        CONTACT_REVEAL,
        SEGMENTS_MANAGE,
        LOYALTY_MANAGE,
        LOYALTY_ADJUST,
        ENGAGEMENT_MANAGE,
        STORED_VALUE_MANAGE,
    ]
}
