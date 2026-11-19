//! Permission của module `organizations`. Ba permission về nút áp theo độ sâu (`scoped`): người có
//! `subtree` ở một vùng chỉ thấy và sửa các nút trong vùng đó. Với nút, độ sâu `own` hiểu như
//! `unit` (nút không có người sở hữu).

use autoxide_kernel::access::Permission;

pub(crate) const UNITS_READ: Permission = Permission::new(
    "organizations.units.read",
    "See the organization units in your scope and their unit types.",
)
.scoped();

pub(crate) const UNITS_WRITE: Permission = Permission::new(
    "organizations.units.write",
    "Create, rename and move organization units in your scope. A new unit waits for approval \
     unless you can also manage units there.",
)
.scoped();

pub(crate) const UNITS_MANAGE: Permission = Permission::new(
    "organizations.units.manage",
    "Approve, suspend, archive and reactivate organization units in your scope. A suspended or \
     archived unit and everything below it can no longer be written to.",
)
.scoped();

pub(crate) const TYPES_MANAGE: Permission = Permission::new(
    "organizations.types.manage",
    "Create, rename and delete the unit types of the tenant (store, warehouse, region…).",
);

pub(crate) const SETTINGS_WRITE: Permission = Permission::new(
    "organizations.settings.write",
    "Set and clear the settings of the organization units in your scope (time zone, currency, \
     language…); units below inherit them unless they set their own.",
)
.scoped();

pub(crate) const PROFILE_MANAGE: Permission = Permission::new(
    "organizations.profile.manage",
    "Edit the public profile of the organization units in your scope: whether websites show \
     them, display name, address, opening hours, pickup, cover image.",
)
.scoped();

/// Nhóm nút cho báo cáo (bước 38, D-232). Không `scoped`: chỉ cấp ở độ sâu `tenant`, vì nhóm vắt
/// qua nhiều nhánh. Nhóm không cấp quyền gì.
pub(crate) const GROUPS_MANAGE: Permission = Permission::new(
    "organizations.groups.manage",
    "Create, rename and delete unit groups (any units of the tenant, e.g. \"stores in the \
     north\") and choose their units. Groups only filter and group reports; they grant nothing.",
);

pub(crate) fn all() -> Vec<Permission> {
    vec![
        UNITS_READ,
        UNITS_WRITE,
        UNITS_MANAGE,
        TYPES_MANAGE,
        SETTINGS_WRITE,
        PROFILE_MANAGE,
        GROUPS_MANAGE,
    ]
}
