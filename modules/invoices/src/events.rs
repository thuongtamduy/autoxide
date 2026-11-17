//! Event module `invoices` phát. Payload chỉ ID, số, số tiền, trạng thái — không tên, email, mã số
//! thuế của người mua.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

pub(crate) const DOCUMENT_ISSUED: EventInfo = EventInfo::new(
    "invoices.document_issued",
    "A document was issued (`document_id`, `org_id`, `kind`, `symbol`, `number`, `total`, \
     `source_module`, `source_id`).",
);

pub(crate) const DOCUMENT_ACCEPTED: EventInfo = EventInfo::new(
    "invoices.document_accepted",
    "The tax authority accepted an e-invoice (`document_id`, `authority_code`).",
);

pub(crate) const DOCUMENT_REJECTED: EventInfo = EventInfo::new(
    "invoices.document_rejected",
    "An e-invoice was refused (`document_id`, `reason`); fix it and issue it again.",
);

pub(crate) const DOCUMENT_CANCELLED: EventInfo = EventInfo::new(
    "invoices.document_cancelled",
    "An issued document was cancelled (`document_id`, `source_module`, `source_id`).",
);

pub(crate) const DOCUMENT_CORRECTED: EventInfo = EventInfo::new(
    "invoices.document_corrected",
    "An issued document was adjusted or replaced (`document_id`, `relation`, `by_document_id`).",
);

pub(crate) const EXPORT_READY: EventInfo = EventInfo::new(
    "invoices.export_ready",
    "An accounting export file is ready to download (`export_id`, `profile`, `format`, `rows`).",
);

macro_rules! event {
    ($kind:ident, $name:literal) => {
        #[derive(Debug, Serialize)]
        pub(crate) struct $kind(pub Value);

        impl Event for $kind {
            const NAME: &'static str = $name;
        }
    };
}

event!(DocumentIssued, "invoices.document_issued");
event!(DocumentAccepted, "invoices.document_accepted");
event!(DocumentRejected, "invoices.document_rejected");
event!(DocumentCancelled, "invoices.document_cancelled");
event!(DocumentCorrected, "invoices.document_corrected");
event!(ExportReady, "invoices.export_ready");

pub(crate) fn all() -> Vec<EventInfo> {
    vec![
        DOCUMENT_ISSUED,
        DOCUMENT_ACCEPTED,
        DOCUMENT_REJECTED,
        DOCUMENT_CANCELLED,
        DOCUMENT_CORRECTED,
        EXPORT_READY,
    ]
}
