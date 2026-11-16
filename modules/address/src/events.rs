//! Event module `address` phát.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;

pub(crate) const DATASET_IMPORTED: EventInfo = EventInfo::new(
    "address.dataset_imported",
    "A new official list of administrative units was imported (`dataset`: source, effective date, counts, units opened and closed).",
);

#[derive(Debug, Serialize)]
pub(crate) struct DatasetImported {
    pub dataset: serde_json::Value,
}

impl Event for DatasetImported {
    const NAME: &'static str = "address.dataset_imported";
}

pub(crate) fn all() -> Vec<EventInfo> {
    vec![DATASET_IMPORTED]
}
