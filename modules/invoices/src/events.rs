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

/// Số liệu cho báo cáo gộp (bước 38): tổng tiền và số chứng từ đã lập, theo loại chứng từ.
pub(crate) const ISSUED_TOTAL: autoxide_kernel::report::FactKind =
    autoxide_kernel::report::FactKind::new(
        "invoices.issued_total",
        "Total of the documents issued, in the currency of each document; dimension `kind` is \
         the kind of document (e.g. `vat_invoice`).",
        autoxide_kernel::report::Measure::Amount,
        &["kind"],
    );

pub(crate) const ISSUED_COUNT: autoxide_kernel::report::FactKind =
    autoxide_kernel::report::FactKind::new(
        "invoices.issued_count",
        "Number of documents issued; dimension `kind` is the kind of document.",
        autoxide_kernel::report::Measure::Quantity,
        &["kind"],
    );

pub(crate) fn facts() -> Vec<autoxide_kernel::report::FactSource> {
    vec![autoxide_kernel::report::FactSource::new(
        DOCUMENT_ISSUED.name(),
        vec![ISSUED_TOTAL, ISSUED_COUNT],
        issued_facts,
    )]
}

/// `invoices.document_issued` → tổng tiền và một chứng từ, theo loại.
fn issued_facts(
    event: &autoxide_kernel::event::Delivered,
) -> Result<Vec<autoxide_kernel::report::Fact>, autoxide_kernel::error::Error> {
    use autoxide_kernel::report::{Fact, money_field, org_field};
    let org = org_field(&event.payload);
    let kind = event.payload.get("kind").and_then(Value::as_str).unwrap_or("unknown").to_owned();
    let mut facts = Vec::new();
    if let Some(total) = money_field(&event.payload, "total")? {
        facts.push(
            Fact::new(ISSUED_TOTAL.kind, event.occurred_at)
                .org(org)
                .amount(total)
                .dimension("kind", kind.clone()),
        );
    }
    facts.push(
        Fact::new(ISSUED_COUNT.kind, event.occurred_at)
            .org(org)
            .quantity(1)
            .dimension("kind", kind),
    );
    Ok(facts)
}
