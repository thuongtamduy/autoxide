//! Permission của module `tax`. Đọc thuế suất, tính thuế, xem hồ sơ thuế theo phạm vi nút; đặt hồ
//! sơ thuế theo nút; nhập gói dữ liệu là việc của nhân sự nền tảng.

use autoxide_kernel::access::{Permission, Risk};

pub(crate) const READ: Permission = Permission::new(
    "tax.read",
    "See tax jurisdictions, categories and rates, compute taxes, and see the tax profiles of the \
     units in your scope.",
)
.scoped();

pub(crate) const PROFILES_MANAGE: Permission = Permission::new(
    "tax.profiles_manage",
    "Set the tax profile (tax code, regime, business lines) of the units in your scope.",
)
.scoped()
.with_risk(Risk::Money);

pub(crate) const PACKS_MANAGE: Permission = Permission::new(
    "tax.packs_manage",
    "Platform staff: import tax data packs (rates and rules for every tenant).",
)
.with_risk(Risk::Money);

pub(crate) fn all() -> Vec<Permission> {
    vec![READ, PROFILES_MANAGE, PACKS_MANAGE]
}
