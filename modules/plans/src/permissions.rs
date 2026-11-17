//! Permission của module `plans`. "Vận hành" (gói, thuê bao của mọi khách) chỉ có tác dụng khi giữ
//! ở khách nền tảng; "khách" (`billing`) là việc của quản trị viên mỗi khách. Không permission nào
//! `scoped()`: thuê bao là việc của cả khách.

use autoxide_kernel::access::{Permission, Risk};

pub(crate) const PLANS_READ: Permission = Permission::new(
    "plans.plans.read",
    "Platform staff: see every plan, including drafts and retired plans, with their prices.",
);

pub(crate) const PLANS_MANAGE: Permission = Permission::new(
    "plans.plans.manage",
    "Platform staff: create and change plans and their prices, mark the default plan, retire plans.",
)
.with_risk(Risk::Money);

pub(crate) const SUBSCRIPTIONS_READ: Permission = Permission::new(
    "plans.subscriptions.read",
    "Platform staff: see the subscription, usage, overrides, billing periods and billing profile \
     of every tenant.",
);

pub(crate) const SUBSCRIPTIONS_MANAGE: Permission = Permission::new(
    "plans.subscriptions.manage",
    "Platform staff: put a tenant on a plan, add add-ons, set overrides, cancel or suspend a \
     subscription, record a payment made outside the system.",
)
.with_risk(Risk::Money);

pub(crate) const BILLING_READ: Permission = Permission::new(
    "plans.billing.read",
    "See the subscription of your tenant, its usage, billing periods and billing profile.",
);

pub(crate) const BILLING_MANAGE: Permission = Permission::new(
    "plans.billing.manage",
    "Change, cancel or resume the subscription of your tenant, and set its billing profile.",
)
.with_risk(Risk::Money);

pub(crate) const PROVISION: Permission = Permission::new(
    "plans.provision",
    "Platform staff: create a tenant on a plan (tenant, subscription, sample data, invitation of its \
     administrator) and run a failed provisioning again.",
)
.with_risk(Risk::Money);

pub(crate) fn all() -> Vec<Permission> {
    vec![
        PLANS_READ,
        PLANS_MANAGE,
        SUBSCRIPTIONS_READ,
        SUBSCRIPTIONS_MANAGE,
        BILLING_READ,
        BILLING_MANAGE,
        PROVISION,
    ]
}
