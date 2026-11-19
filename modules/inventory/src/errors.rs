//! Lỗi của module `inventory`. `code` là hợp đồng với client và là i18n key: bản dịch tiếng Việt
//! ở `locales/vi.json`.

use autoxide_kernel::error::{Error, ErrorKind};
use uuid::Uuid;

pub(crate) fn level_not_found(id: Uuid) -> Error {
    Error::not_found("inventory.level_not_found", "There is no stock level `{id}` in your scope.")
        .with_param("id", id)
}

/// Không đủ hàng: `lines` liệt kê từng dòng thiếu (`<dòng>: còn <số>`).
pub(crate) fn insufficient_stock(lines: &str) -> Error {
    Error::conflict(
        "inventory.insufficient_stock",
        "Not enough stock for {lines}. Lower the quantities or pick another warehouse.",
    )
    .with_param("lines", lines.to_owned())
}

pub(crate) fn not_a_warehouse(org: Uuid) -> Error {
    Error::new(
        ErrorKind::Validation,
        "inventory.not_a_warehouse",
        "The unit `{org_id}` is not a warehouse or a store, so it cannot hold stock.",
    )
    .with_param("org_id", org)
}

pub(crate) fn currency_mismatch(expected: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "inventory.currency_mismatch",
        "The unit cost must be in {currency}, the currency of the stock already in this warehouse.",
    )
    .with_param("currency", expected.to_owned())
}

pub(crate) fn variant_not_found(id: Uuid) -> Error {
    Error::not_found("inventory.variant_not_found", "There is no product variant `{id}`.")
        .with_param("id", id)
}

pub(crate) fn not_tracked(id: Uuid) -> Error {
    Error::new(
        ErrorKind::Validation,
        "inventory.not_tracked",
        "The variant `{id}` does not track stock (digital goods, services and bundles); there is \
         nothing to count or move.",
    )
    .with_param("id", id)
}

pub(crate) fn adjustment_not_found(id: Uuid) -> Error {
    Error::not_found(
        "inventory.adjustment_not_found",
        "There is no stock adjustment `{id}` in your scope.",
    )
    .with_param("id", id)
}

pub(crate) fn no_source(org: Uuid) -> Error {
    Error::conflict(
        "inventory.no_source",
        "No warehouse serves the unit `{org_id}` yet: list its warehouses with `PUT \
         /api/v1/inventory/sources/{org_id}`.",
    )
    .with_param("org_id", org)
}
