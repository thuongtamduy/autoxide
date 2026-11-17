//! Event module `tax` phát. `tax.pack_imported` ghi ở khách nền tảng (dữ liệu dùng chung);
//! `tax.profile_changed` ở khách có hồ sơ.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

pub(crate) const PACK_IMPORTED: EventInfo = EventInfo::new(
    "tax.pack_imported",
    "A tax data pack was imported (`code`, `version`, `rates`).",
);

pub(crate) const PROFILE_CHANGED: EventInfo = EventInfo::new(
    "tax.profile_changed",
    "The tax profile of an organization unit was set (`org_id`, `regime`, `created`).",
);

#[derive(Debug, Serialize)]
pub(crate) struct PackImported(pub Value);

impl Event for PackImported {
    const NAME: &'static str = "tax.pack_imported";
}

#[derive(Debug, Serialize)]
pub(crate) struct ProfileChanged(pub Value);

impl Event for ProfileChanged {
    const NAME: &'static str = "tax.profile_changed";
}

pub(crate) fn all() -> Vec<EventInfo> {
    vec![PACK_IMPORTED, PROFILE_CHANGED]
}
