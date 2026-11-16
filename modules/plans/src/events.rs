//! Event module `plans` phát. Mọi event ghi ở database điều khiển, dưới khách nền tảng; event về
//! một khách mang `tenant_id` trong thân và chủ thể `plans.subscription:<tenant_id>`.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

macro_rules! events {
    ($($konst:ident, $kind:ident, $name:literal, $description:literal;)*) => {
        $(
            pub(crate) const $konst: EventInfo = EventInfo::new($name, $description);

            #[derive(Debug, Serialize)]
            pub(crate) struct $kind(pub Value);

            impl Event for $kind {
                const NAME: &'static str = $name;
            }
        )*

        pub(crate) fn all() -> Vec<EventInfo> {
            vec![$($konst),*]
        }
    };
}

events! {
    PLAN_CREATED, PlanCreated, "plans.plan_created", "A plan was created (`plan`).";
    PLAN_UPDATED, PlanUpdated, "plans.plan_updated", "A plan or its prices changed (`before`, `after`). Tenants on the plan get the new entitlements within the cache time.";
    PLAN_RETIRED, PlanRetired, "plans.plan_retired", "A plan is no longer sold (`plan`); tenants on it keep it.";
    SUBSCRIPTION_CREATED, SubscriptionCreated, "plans.subscription_created", "A tenant got its first subscription (`tenant_id`, `subscription`).";
    SUBSCRIPTION_STATUS_CHANGED, SubscriptionStatusChanged, "plans.subscription_status_changed", "The status of a subscription changed (`tenant_id`, `from`, `to`).";
    SUBSCRIPTION_PLAN_CHANGED, SubscriptionPlanChanged, "plans.subscription_plan_changed", "A tenant moved to another plan or price, or changed its add-ons (`tenant_id`, `before`, `after`).";
    OVERRIDE_SET, OverrideSet, "plans.override_set", "An override was set for a tenant (`tenant_id`, `override`).";
    OVERRIDE_REMOVED, OverrideRemoved, "plans.override_removed", "An override of a tenant was removed (`tenant_id`, `override`).";
    ENTITLEMENTS_CHANGED, EntitlementsChanged, "plans.entitlements_changed", "What a tenant may use changed because of its subscription, add-ons or overrides (`tenant_id`). Read `GET /api/v1/plans/entitlements` again.";
}
