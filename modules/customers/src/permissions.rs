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

pub(crate) fn all() -> Vec<Permission> {
    vec![READ, MANAGE, CONTACT_REVEAL, SEGMENTS_MANAGE]
}
