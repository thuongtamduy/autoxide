//! Permission của module `promotions` (D-222): `<module>.<việc>`. Handler REST gọi
//! `principal.require(&…)` rồi kiểm phạm vi theo nút của chương trình (`require_at` /
//! `require_write_at`, bước 34); tool MCP khai `.requires(…)`.

use autoxide_kernel::access::Permission;

pub(crate) const READ: Permission = Permission::new(
    "promotions.read",
    "See the promotions, codes, redemptions and statistics in your scope; try carts against them.",
)
.scoped();

pub(crate) const WRITE: Permission = Permission::new(
    "promotions.write",
    "Create and change draft promotions in your scope (texts, targets, flash-sale items, tiers).",
)
.scoped();

pub(crate) const PUBLISH: Permission = Permission::new(
    "promotions.publish",
    "Turn the promotions in your scope on, pause, resume, end or cancel them; release stuck holds.",
)
.scoped();

pub(crate) const CODES_MANAGE: Permission = Permission::new(
    "promotions.codes.manage",
    "Generate, list and export the coupon and claim codes of the promotions in your scope.",
)
.scoped();

pub(crate) const COMPLIANCE: Permission = Permission::new(
    "promotions.compliance",
    "Export promotion notices for the authorities and record that they were submitted.",
)
.scoped();

pub(crate) const USE: Permission = Permission::new(
    "promotions.use",
    "As a buyer: see your voucher wallet, save vouchers, enter claim codes, ask for flash-sale \
     reminders.",
);

pub(crate) const VOUCHERS_GRANT: Permission = Permission::new(
    "promotions.vouchers.grant",
    "Put a voucher of a promotion in your scope into a user's wallet (with a reason).",
)
.scoped();

pub(crate) const FUNDING_MANAGE: Permission = Permission::new(
    "promotions.funding.manage",
    "Set who pays for the discount of a promotion (platform, brand, payment partner shares).",
)
.scoped();

pub(crate) const PARTICIPATE: Permission = Permission::new(
    "promotions.participate",
    "As a seller: apply to join a platform promotion with your own items.",
)
.scoped();

pub(crate) fn all() -> Vec<Permission> {
    vec![
        READ,
        WRITE,
        PUBLISH,
        CODES_MANAGE,
        COMPLIANCE,
        USE,
        VOUCHERS_GRANT,
        FUNDING_MANAGE,
        PARTICIPATE,
    ]
}
