//! SPIKE: a generic type through the shadow derive gives the same schema as utoipa's own derive.
#![allow(clippy::unwrap_used, dead_code)]

use autoxide_spike_macros::Schema;
use serde::Serialize;
use utoipa::{PartialSchema, ToSchema};

/// One page.
#[derive(Serialize, Schema)]
#[serde(rename_all = "camelCase")]
struct Wrapped<T> {
    /// The items.
    data_items: Vec<T>,
    #[deprecated]
    old_total: Option<i64>,
    #[serde(flatten)]
    extra: Extra,
}

/// One page.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = Wrapped)]
struct Direct<T> {
    /// The items.
    data_items: Vec<T>,
    #[deprecated]
    old_total: Option<i64>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Serialize, Schema)]
struct Extra {
    /// Next cursor.
    #[schema(example = "abc")]
    cursor: Option<String>,
}

/// An item.
#[derive(Serialize, Schema)]
enum Kind {
    /// First.
    A,
    B {
        value: u32,
    },
}

#[test]
fn generic_schema_is_identical() {
    let shadow = serde_json::to_value(<Wrapped<Kind> as PartialSchema>::schema()).unwrap();
    let direct = serde_json::to_value(<Direct<Kind> as PartialSchema>::schema()).unwrap();
    assert_eq!(shadow, direct);
    assert_eq!(<Wrapped<Kind> as ToSchema>::name(), <Direct<Kind> as ToSchema>::name());
    let mut a = Vec::new();
    let mut b = Vec::new();
    <Wrapped<Kind> as ToSchema>::schemas(&mut a);
    <Direct<Kind> as ToSchema>::schemas(&mut b);
    assert_eq!(serde_json::to_value(&a).unwrap(), serde_json::to_value(&b).unwrap());
    assert!(!a.is_empty(), "nested schemas are forwarded");
}
