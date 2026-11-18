//! Event module `promotions` phát. Cùng hình dạng `{"before": …, "after": …}` với mọi module; ghi
//! CÙNG transaction với thay đổi (luật kiến trúc 3). Tên event là hằng trong `api::events`.
//! Payload không chứa `buyer_keys`.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

use crate::api::events as names;

/// Dữ liệu chung của mọi event của module.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Change {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
}

impl Change {
    pub(crate) fn after(after: Value) -> Self {
        Change { before: None, after: Some(after) }
    }
}

/// Một loại event: tên + dữ liệu `Change`.
macro_rules! event {
    ($kind:ident, $name:expr) => {
        #[derive(Debug, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $kind(pub Change);

        impl Event for $kind {
            const NAME: &'static str = $name;
        }
    };
}

event!(PromotionCreated, names::PROMOTION_CREATED);
event!(PromotionUpdated, names::PROMOTION_UPDATED);
event!(PromotionActivated, names::PROMOTION_ACTIVATED);
event!(PromotionStarted, names::PROMOTION_STARTED);
event!(PromotionPaused, names::PROMOTION_PAUSED);
event!(PromotionEnded, names::PROMOTION_ENDED);
event!(PromotionCancelled, names::PROMOTION_CANCELLED);
event!(CodesGenerated, names::CODES_GENERATED);
event!(RedemptionConfirmed, names::REDEMPTION_CONFIRMED);
event!(RedemptionReleased, names::REDEMPTION_RELEASED);
event!(NoticeSubmitted, names::NOTICE_SUBMITTED);
event!(VoucherIssued, names::VOUCHER_ISSUED);
event!(VoucherUsed, names::VOUCHER_USED);
event!(BudgetExhausted, names::BUDGET_EXHAUSTED);
event!(LegalDataImported, "promotions.legal_data_imported");

pub(crate) fn all() -> Vec<EventInfo> {
    vec![
        EventInfo::new(names::PROMOTION_CREATED, "A promotion was created as a draft."),
        EventInfo::new(
            names::PROMOTION_UPDATED,
            "A promotion's rules, texts, targets or limits changed.",
        ),
        EventInfo::new(
            names::PROMOTION_ACTIVATED,
            "A promotion was turned on (scheduled or running).",
        ),
        EventInfo::new(names::PROMOTION_STARTED, "A scheduled promotion reached its start time."),
        EventInfo::new(names::PROMOTION_PAUSED, "A running promotion was paused."),
        EventInfo::new(
            names::PROMOTION_ENDED,
            "A promotion ended (end time reached or ended early).",
        ),
        EventInfo::new(names::PROMOTION_CANCELLED, "A promotion was cancelled."),
        EventInfo::new(names::CODES_GENERATED, "A batch of coupon or claim codes was generated."),
        EventInfo::new(
            names::REDEMPTION_CONFIRMED,
            "The promotion usage held for an order was confirmed.",
        ),
        EventInfo::new(
            names::REDEMPTION_RELEASED,
            "The promotion usage held for an order was given back.",
        ),
        EventInfo::new(
            names::NOTICE_SUBMITTED,
            "A promotion notice was recorded as submitted to an authority.",
        ),
        EventInfo::new(names::VOUCHER_ISSUED, "A voucher was put into a user's wallet."),
        EventInfo::new(names::VOUCHER_USED, "A voucher was used for an order."),
        EventInfo::new(names::BUDGET_EXHAUSTED, "A promotion used up its budget."),
        EventInfo::new(
            names::PARTICIPATION_REVIEWED,
            "A seller's application to join a platform promotion was approved or rejected.",
        ),
        EventInfo::new(
            names::AUTOMATIC_PRICES_CHANGED,
            "The automatic sale price of some variants changed (for public product lists).",
        ),
        EventInfo::new(
            "promotions.legal_data_imported",
            "A promotion law data pack (limits, notice rules) was imported.",
        ),
    ]
}

/// Chủ thể (subject) của event về chương trình `id`.
pub(crate) fn promotion_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("promotions.promotion:{id}"))
}
