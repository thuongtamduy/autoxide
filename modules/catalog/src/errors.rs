//! Lỗi của module `catalog`. `code` là hợp đồng với client và là i18n key: bản dịch tiếng Việt
//! ở `locales/vi.json`.

use autoxide_kernel::error::{Error, ErrorKind};
use uuid::Uuid;

pub(crate) fn product_not_found(id: Uuid) -> Error {
    Error::not_found("catalog.product_not_found", "There is no product `{id}` in your scope.")
        .with_param("id", id)
}

pub(crate) fn variant_not_found(id: Uuid) -> Error {
    Error::not_found("catalog.variant_not_found", "There is no variant `{id}` in your scope.")
        .with_param("id", id)
}

pub(crate) fn category_not_found(id: Uuid) -> Error {
    Error::not_found("catalog.category_not_found", "There is no category `{id}`.")
        .with_param("id", id)
}

pub(crate) fn price_list_not_found(id: Uuid) -> Error {
    Error::not_found("catalog.price_list_not_found", "There is no price list `{id}` in your scope.")
        .with_param("id", id)
}

pub(crate) fn sku_taken(sku: &str) -> Error {
    Error::conflict(
        "catalog.sku_taken",
        "The SKU `{sku}` is already used by another variant. Choose another SKU.",
    )
    .with_param("sku", sku)
}

pub(crate) fn barcode_taken(barcode: &str) -> Error {
    Error::conflict(
        "catalog.barcode_taken",
        "The barcode `{barcode}` is already used by another variant.",
    )
    .with_param("barcode", barcode)
}

pub(crate) fn slug_taken(slug: &str) -> Error {
    Error::conflict(
        "catalog.slug_taken",
        "The address `{slug}` is already used by another product or category in this language. \
         Choose another slug.",
    )
    .with_param("slug", slug)
}

pub(crate) fn option_combination_taken() -> Error {
    Error::conflict(
        "catalog.option_combination_taken",
        "The product already has a variant with this combination of option values.",
    )
}

pub(crate) fn invalid_status_transition(from: &str, to: &str) -> Error {
    Error::conflict(
        "catalog.invalid_status_transition",
        "A product cannot go from `{from}` to `{to}`.",
    )
    .with_param("from", from)
    .with_param("to", to)
}

pub(crate) fn consumer_price_rules() -> Error {
    Error::new(
        ErrorKind::Validation,
        "catalog.consumer_price_rules",
        "A price list for consumers in Viet Nam must be in VND and include tax.",
    )
}

pub(crate) fn media_not_public(id: Uuid) -> Error {
    Error::new(
        ErrorKind::Validation,
        "catalog.media_not_public",
        "Media `{id}` is not public (or does not exist): product images must be public media.",
    )
    .with_param("id", id)
}

pub(crate) fn unsupported_locale(locale: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "catalog.unsupported_locale",
        "The language `{locale}` is not one of the languages configured for products.",
    )
    .with_param("locale", locale)
}

pub(crate) fn bundle_invalid() -> Error {
    Error::new(
        ErrorKind::Validation,
        "catalog.bundle_invalid",
        "A bundle cannot contain another bundle, itself, or a service.",
    )
}

pub(crate) fn category_cycle() -> Error {
    Error::conflict(
        "catalog.category_cycle",
        "A category cannot be moved under itself or one of its own subcategories.",
    )
}

pub(crate) fn category_not_empty() -> Error {
    Error::conflict(
        "catalog.category_not_empty",
        "The category still has subcategories or products. Move or remove them first.",
    )
}

pub(crate) fn product_not_deletable() -> Error {
    Error::conflict(
        "catalog.product_not_deletable",
        "The product has been on sale before; archive it instead of deleting it.",
    )
}

pub(crate) fn price_below_floor(variant_id: Uuid, floor: i64) -> Error {
    Error::new(
        ErrorKind::Validation,
        "catalog.price_below_floor",
        "The price of variant `{variant_id}` is below the price floor of the store ({floor}).",
    )
    .with_param("variant_id", variant_id)
    .with_param("floor", floor)
}

pub(crate) fn too_many_ids(max: usize) -> Error {
    Error::new(
        ErrorKind::Validation,
        "catalog.too_many_ids",
        "Ask for at most {max} ids at a time.",
    )
    .with_param("max", max)
}

pub(crate) fn currency_mismatch() -> Error {
    Error::new(
        ErrorKind::Validation,
        "catalog.currency_mismatch",
        "A price must be in the currency of its price list.",
    )
}

pub(crate) fn variant_required() -> Error {
    Error::conflict(
        "catalog.last_variant",
        "A product must keep at least one variant; delete the product instead.",
    )
}

/// Website không có sản phẩm đang bán với slug này ở ngôn ngữ hỏi.
pub(crate) fn product_slug_not_found(slug: &str) -> Error {
    Error::not_found(
        "catalog.product_slug_not_found",
        "No product on sale on this website has the address `{slug}` in this language.",
    )
    .with_param("slug", slug)
}

/// Không có danh mục hiện được với slug này ở ngôn ngữ hỏi.
pub(crate) fn category_slug_not_found(slug: &str) -> Error {
    Error::not_found(
        "catalog.category_slug_not_found",
        "No visible category has the address `{slug}` in this language.",
    )
    .with_param("slug", slug)
}

/// Sàn mà không có `KycGate` (module `compliance` tắt): không kiểm được định danh người bán nên không mở bán.
pub(crate) fn seller_verification_unavailable() -> Error {
    Error::new(
        ErrorKind::Forbidden,
        "catalog.seller_verification_unavailable",
        "This tenant is a marketplace, but seller identity verification (module `compliance`) is not \
         enabled, so goods of a seller cannot go on sale. Enable module `compliance`, or list the goods \
         under the tenant's root.",
    )
}

/// Module cắm điều kiện mở bán (bước 46) từ chối: `reason` là mã của module đó, `detail` lời của nó.
pub(crate) fn listing_denied(reason: &str, detail: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "catalog.listing_denied",
        "This product cannot go on sale yet ({reason}): {detail}",
    )
    .with_param("reason", reason)
    .with_param("detail", detail)
}
