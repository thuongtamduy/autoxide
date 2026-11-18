//! Lỗi của module `analytics`. `code` là hợp đồng với client và là i18n key (bản dịch ở
//! `locales/vi.json`).

use autoxide_kernel::error::{Error, ErrorKind};

/// `analytics.site_unknown` (404): khách không có website (nút) đó.
pub(crate) fn site_unknown(site: uuid::Uuid) -> Error {
    Error::not_found(
        "analytics.site_unknown",
        "There is no website (organization unit) `{site}` in this tenant. Leave `site` out for the \
         website of the whole tenant.",
    )
    .with_param("site", site)
}

/// `analytics.range_too_long` (422).
pub(crate) fn range_too_long(max: i64) -> Error {
    Error::new(
        ErrorKind::Validation,
        "analytics.range_too_long",
        "Ask for at most {max} days at once: move `from` or `to` closer together.",
    )
    .with_param("max", max)
}

/// `analytics.range_reversed` (422).
pub(crate) fn range_reversed() -> Error {
    Error::new(
        ErrorKind::Validation,
        "analytics.range_reversed",
        "`from` must not be after `to`.",
    )
}

/// `analytics.funnel_unknown` (404).
pub(crate) fn funnel_unknown(key: &str) -> Error {
    Error::not_found(
        "analytics.funnel_unknown",
        "There is no funnel `{key}`; see `GET /api/v1/analytics/funnels` for the declared ones.",
    )
    .with_param("key", key)
}

/// `analytics.kind_invalid` (422): loại đối tượng sai dạng.
pub(crate) fn kind_invalid() -> Error {
    Error::new(
        ErrorKind::Validation,
        "analytics.kind_invalid",
        "`kind` must be an object kind like `catalog.product`: lowercase letters, digits and `_`, \
         with one dot.",
    )
}
