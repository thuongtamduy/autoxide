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
event!(PriceEffective, names::PRICE_EFFECTIVE);
event!(CategoryChanged, "catalog.category_changed");
event!(PreorderDateChanged, "catalog.preorder_date_changed");
event!(PriceListChanged, "catalog.price_list_changed");
event!(CollectionChanged, "catalog.collection_changed");
event!(ImportCompleted, "catalog.import_completed");
event!(DisplayPriceChanged, names::DISPLAY_PRICE_CHANGED);
event!(ExportCompleted, "catalog.export_completed");
event!(DownloadsIssued, "catalog.downloads_issued");
event!(DownloadsRevoked, "catalog.downloads_revoked");

pub(crate) fn all() -> Vec<EventInfo> {
    vec![
        EventInfo::new(
            "catalog.downloads_issued",
            "Download rights of a digital product were given for a paid order (`after.grants`; \
             never the download token).",
        ),
        EventInfo::new(
            "catalog.downloads_revoked",
            "Download rights of an order were revoked (order cancelled or refunded, or by staff).",
        ),
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
            names::PRICE_EFFECTIVE,
            "A scheduled price started or a temporary price ended (variants, time window).",
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
        EventInfo::new(
            names::DISPLAY_PRICE_CHANGED,
            "The public price of a product on a website changed (unit, product, price before and \
             after; `null` when not on sale there).",
        ),
        EventInfo::new(
            "catalog.import_completed",
            "A CSV import finished (status, rows read, rows written, error count).",
        ),
        EventInfo::new("catalog.export_completed", "A CSV export finished (status, rows written)."),
        EventInfo::new(
            "catalog.collection_changed",
            "A product collection was created, changed, given other products or deleted.",
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

/// Chủ thể của event về bộ sưu tập `id`.
pub(crate) fn collection_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("catalog.collection:{id}"))
}
