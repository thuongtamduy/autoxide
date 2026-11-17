//! Event module `ledger` phát. Payload chỉ ID, số tiền, ngày — không tên, email, số điện thoại.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

pub(crate) const ENTRY_POSTED: EventInfo = EventInfo::new(
    "ledger.entry_posted",
    "An entry was posted (`entry_id`, `book_id`, `org_id`, `effective_date`, `source_module`, \
     `source_kind`, `source_id`, `reference`, `total`).",
);

pub(crate) const ENTRY_REVERSED: EventInfo = EventInfo::new(
    "ledger.entry_reversed",
    "An entry was reversed (`entry_id` of the reversal, `reversed_entry_id`, `book_id`, `org_id`, \
     `effective_date`).",
);

pub(crate) const PERIOD_CLOSED: EventInfo = EventInfo::new(
    "ledger.period_closed",
    "An accounting period was closed (`period_id`, `book_id`, `org_id`, `starts_on`, `ends_on`).",
);

#[derive(Debug, Serialize)]
pub(crate) struct EntryPosted(pub Value);

impl Event for EntryPosted {
    const NAME: &'static str = "ledger.entry_posted";
}

#[derive(Debug, Serialize)]
pub(crate) struct EntryReversed(pub Value);

impl Event for EntryReversed {
    const NAME: &'static str = "ledger.entry_reversed";
}

#[derive(Debug, Serialize)]
pub(crate) struct PeriodClosed(pub Value);

impl Event for PeriodClosed {
    const NAME: &'static str = "ledger.period_closed";
}

pub(crate) fn all() -> Vec<EventInfo> {
    vec![ENTRY_POSTED, ENTRY_REVERSED, PERIOD_CLOSED]
}
