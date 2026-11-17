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

pub(crate) const PAYMENT_CREATED: EventInfo = EventInfo::new(
    "ledger.payment_created",
    "A payment was created (`payment_id`, `org_id`, `provider`, `amount`, `source_module`, \
     `source_id`, `reference`).",
);

pub(crate) const PAYMENT_SUCCEEDED: EventInfo = EventInfo::new(
    "ledger.payment_succeeded",
    "The partner collected a payment (`payment_id`, `org_id`, `provider`, `provider_ref`, `amount`, \
     `source_module`, `source_id`, `reference`).",
);

pub(crate) const PAYMENT_FAILED: EventInfo = EventInfo::new(
    "ledger.payment_failed",
    "A payment failed or expired (`payment_id`, `org_id`, `reason`, `source_module`, `source_id`, \
     `reference`).",
);

pub(crate) const PAYMENT_REFUNDED: EventInfo = EventInfo::new(
    "ledger.payment_refunded",
    "A refund was paid back (`payment_id`, `refund_id`, `amount`, `source_module`, `source_id`).",
);

pub(crate) const EXCEPTION_OPENED: EventInfo = EventInfo::new(
    "ledger.exception_opened",
    "A difference needs a person (`exception_id`, `kind`, `subject`).",
);

pub(crate) const EXCEPTION_RESOLVED: EventInfo = EventInfo::new(
    "ledger.exception_resolved",
    "A difference was resolved (`exception_id`, `kind`, `subject`, `resolution`).",
);

pub(crate) const PAYOUT_SENT: EventInfo = EventInfo::new(
    "ledger.payout_sent",
    "The payout partner paid a seller (`payout_id`, `org_id`, `payee_org_id`, `amount`, `provider`).",
);

pub(crate) const PAYOUT_FAILED: EventInfo = EventInfo::new(
    "ledger.payout_failed",
    "The payout partner refused a payout; the amount is back on the seller's available balance \
     (`payout_id`, `org_id`, `payee_org_id`, `amount`, `provider`, `reason`).",
);

pub(crate) const HOLD_RELEASED: EventInfo = EventInfo::new(
    "ledger.hold_released",
    "Held seller money became available (`hold_id`, `org_id`, `seller_org_id`, `amount`, \
     `source_module`, `source_id`).",
);

/// Một kiểu event (thân JSON) cho mỗi tên.
macro_rules! event {
    ($kind:ident, $name:literal) => {
        #[derive(Debug, Serialize)]
        pub(crate) struct $kind(pub Value);

        impl Event for $kind {
            const NAME: &'static str = $name;
        }
    };
}

event!(PaymentCreated, "ledger.payment_created");
event!(PaymentSucceeded, "ledger.payment_succeeded");
event!(PaymentFailed, "ledger.payment_failed");
event!(PaymentRefunded, "ledger.payment_refunded");
event!(ExceptionOpened, "ledger.exception_opened");
event!(ExceptionResolved, "ledger.exception_resolved");
event!(PayoutSent, "ledger.payout_sent");
event!(PayoutFailed, "ledger.payout_failed");
event!(HoldReleased, "ledger.hold_released");

pub(crate) fn all() -> Vec<EventInfo> {
    vec![
        ENTRY_POSTED,
        ENTRY_REVERSED,
        PERIOD_CLOSED,
        PAYMENT_CREATED,
        PAYMENT_SUCCEEDED,
        PAYMENT_FAILED,
        PAYMENT_REFUNDED,
        EXCEPTION_OPENED,
        EXCEPTION_RESOLVED,
        PAYOUT_SENT,
        PAYOUT_FAILED,
        HOLD_RELEASED,
    ]
}
