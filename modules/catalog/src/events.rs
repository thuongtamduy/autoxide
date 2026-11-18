//! Event module `catalog` phát. Cùng hình dạng `{"before": …, "after": …}` với mọi module; ghi
//! CÙNG transaction với thay đổi (luật kiến trúc 3). Tên event là hằng trong `api::events` để
//! module khác nghe đúng tên.

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

event!(ProductCreated, names::PRODUCT_CREATED);
event!(ProductUpdated, names::PRODUCT_UPDATED);
event!(ProductStatusChanged, names::PRODUCT_STATUS_CHANGED);
event!(ProductArchived, names::PRODUCT_ARCHIVED);
event!(ProductDeleted, names::PRODUCT_DELETED);
event!(VariantCreated, names::VARIANT_CREATED);
event!(VariantUpdated, names::VARIANT_UPDATED);
event!(VariantDeleted, names::VARIANT_DELETED);
event!(PriceChanged, names::PRICE_CHANGED);
event!(CategoryChanged, "catalog.category_changed");
event!(PreorderDateChanged, "catalog.preorder_date_changed");
event!(PriceListChanged, "catalog.price_list_changed");

pub(crate) fn all() -> Vec<EventInfo> {
    vec![
        EventInfo::new(names::PRODUCT_CREATED, "A product was created."),
        EventInfo::new(names::PRODUCT_UPDATED, "A product's details, texts or images changed."),
        EventInfo::new(
            names::PRODUCT_STATUS_CHANGED,
            "A product's sale status changed (draft, active, inactive, archived).",
        ),
        EventInfo::new(names::PRODUCT_ARCHIVED, "A product was archived."),
        EventInfo::new(names::PRODUCT_DELETED, "A product that was never on sale was deleted."),
        EventInfo::new(names::VARIANT_CREATED, "A product variant was created."),
        EventInfo::new(
            names::VARIANT_UPDATED,
            "A variant's SKU, barcode, sale mode, size or weight changed.",
        ),
        EventInfo::new(names::VARIANT_DELETED, "A product variant was deleted."),
        EventInfo::new(
            names::PRICE_CHANGED,
            "Prices were added to a price list (price list, variant, new price, start).",
        ),
        EventInfo::new(
            "catalog.category_changed",
            "A category was created, changed, moved or deleted.",
        ),
        EventInfo::new(
            "catalog.price_list_changed",
            "A price list was created or changed (name, priority, customer group, on / off).",
        ),
        EventInfo::new(
            "catalog.preorder_date_changed",
            "The expected availability date of a pre-order variant changed.",
        ),
    ]
}

/// Chủ thể (subject) của event về sản phẩm `id`.
pub(crate) fn product_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("catalog.product:{id}"))
}

/// Chủ thể của event về bảng giá `id`.
pub(crate) fn price_list_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("catalog.price_list:{id}"))
}

/// Chủ thể của event về danh mục `id`.
pub(crate) fn category_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("catalog.category:{id}"))
}
