//! Event module `automations` phát. Ghi CÙNG transaction với thay đổi.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

use crate::permissions::AUTOMATIONS_READ;

pub(crate) const RULE_CHANGED: EventInfo = EventInfo::new(
    "automations.rule_changed",
    "An automation rule was created, changed, enabled, disabled or deleted.",
)
.activity("{actor} changed the automation rule {name}", AUTOMATIONS_READ);
pub(crate) const RUN_FAILED: EventInfo =
    EventInfo::new("automations.run_failed", "A run of an automation rule failed at a step.");
pub(crate) const RULE_PAUSED: EventInfo = EventInfo::new(
    "automations.rule_paused",
    "An automation rule stopped by itself: it ran too often, or its author lost a permission it \
     needs.",
);

/// Dữ liệu chung: `{"before": …, "after": …}` như mọi module (audit ghi "trước / sau").
#[derive(Debug, Default, Serialize)]
pub(crate) struct Change {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
}

macro_rules! event {
    ($kind:ident, $name:literal) => {
        #[derive(Debug, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $kind(pub Change);

        impl Event for $kind {
            const NAME: &'static str = $name;
        }
    };
}

event!(RuleChanged, "automations.rule_changed");
event!(RunFailed, "automations.run_failed");
event!(RulePaused, "automations.rule_paused");

pub(crate) fn all() -> Vec<EventInfo> {
    vec![RULE_CHANGED, RUN_FAILED, RULE_PAUSED]
}

/// Chủ thể của event về luật `id`.
pub(crate) fn rule_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("automations.rule:{id}"))
}
