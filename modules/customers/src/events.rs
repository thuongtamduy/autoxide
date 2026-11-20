//! Event module `customers` phát. Cùng hình dạng `{"before": …, "after": …}` với mọi module; ghi CÙNG
//! transaction với thay đổi (luật kiến trúc 3). Payload không mang số điện thoại hay email rõ.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

pub(crate) const PROFILE_CREATED: &str = "customers.profile_created";
pub(crate) const PROFILE_UPDATED: &str = "customers.profile_updated";
pub(crate) const PROFILE_LINKED: &str = "customers.profile_linked";
pub(crate) const CONTACT_REVEALED: &str = "customers.contact_revealed";
pub(crate) const GROUP_CHANGED: &str = "customers.group_changed";
pub(crate) const GROUP_MEMBERSHIP_CHANGED: &str = "customers.group_membership_changed";
pub(crate) const POINTS_EARNED: &str = "customers.points_earned";
pub(crate) const POINTS_REDEEMED: &str = "customers.points_redeemed";
pub(crate) const POINTS_EXPIRED: &str = "customers.points_expired";
pub(crate) const POINTS_ADJUSTED: &str = "customers.points_adjusted";
pub(crate) const TIER_CHANGED: &str = "customers.tier_changed";

/// Dữ liệu chung của mọi event của module.
#[derive(Debug, Clone, Default, Serialize)]
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

event!(ProfileCreated, PROFILE_CREATED);
event!(ProfileUpdated, PROFILE_UPDATED);
event!(ProfileLinked, PROFILE_LINKED);
event!(ContactRevealed, CONTACT_REVEALED);
event!(GroupChanged, GROUP_CHANGED);
event!(GroupMembershipChanged, GROUP_MEMBERSHIP_CHANGED);
event!(PointsEarned, POINTS_EARNED);
event!(PointsRedeemed, POINTS_REDEEMED);
event!(PointsExpired, POINTS_EXPIRED);
event!(PointsAdjusted, POINTS_ADJUSTED);
event!(TierChanged, TIER_CHANGED);

pub(crate) fn all() -> Vec<EventInfo> {
    vec![
        EventInfo::new(
            PROFILE_CREATED,
            "A customer profile was created (first order, registration at the counter or the \
             buyer's first visit).",
        ),
        EventInfo::new(PROFILE_UPDATED, "A customer profile was changed."),
        EventInfo::new(
            PROFILE_LINKED,
            "A customer profile kept by phone number was linked to the buyer's account after the \
             buyer proved the number.",
        ),
        EventInfo::new(
            CONTACT_REVEALED,
            "A staff member looked at the full phone number and email of a customer (audit).",
        ),
        EventInfo::new(GROUP_CHANGED, "A customer group was created, changed or deleted."),
        EventInfo::new(
            GROUP_MEMBERSHIP_CHANGED,
            "A customer joined or left customer groups (`after.joined`, `after.left`).",
        ),
        EventInfo::new(
            POINTS_EARNED,
            "A customer earned loyalty points (a new lot with its own expiry).",
        ),
        EventInfo::new(POINTS_REDEEMED, "Loyalty points paid part of a confirmed order."),
        EventInfo::new(POINTS_EXPIRED, "Unused loyalty points of a lot expired."),
        EventInfo::new(
            POINTS_ADJUSTED,
            "Loyalty points were added or removed by staff or taken back after a refund.",
        ),
        EventInfo::new(TIER_CHANGED, "A customer moved to another membership tier (or lost it)."),
    ]
}
