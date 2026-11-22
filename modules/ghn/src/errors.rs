//! Lỗi của bộ nối GHN. `code` là hợp đồng và là i18n key (bản dịch ở `locales/vi.json`). Lỗi chung
//! của mọi hãng (`shipping.carrier_*`) lấy từ `autoxide_shipping::api::errors`.

use autoxide_kernel::error::{Error, ErrorKind};

fn invalid(code: &'static str, message: &'static str) -> Error {
    Error::new(ErrorKind::Validation, code, message)
}

/// `ghn.account_incomplete` (422): tài khoản hãng thiếu `token` hay `shop_id`.
pub(crate) fn account_incomplete(field: &str) -> Error {
    invalid(
        "ghn.account_incomplete",
        "The GHN carrier account has no `{field}`. Put the token in `credentials` and `shop_id` \
         in `settings` of the carrier account.",
    )
    .with_param("field", field)
}

/// `ghn.setting_invalid` (422): một cài đặt tuỳ chọn của tài khoản sai giá trị.
pub(crate) fn setting_invalid(field: &str, problem: &str) -> Error {
    invalid(
        "ghn.setting_invalid",
        "Setting `{field}` of the GHN carrier account is not valid: {problem}.",
    )
    .with_param("field", field)
    .with_param("problem", problem)
}

/// `ghn.shop_not_found` (422): token không quản lý cửa hàng này, hoặc cửa hàng bị khoá.
pub(crate) fn shop_not_found(shop_id: i64) -> Error {
    invalid(
        "ghn.shop_not_found",
        "This GHN token does not manage shop `{shop_id}`, or the shop is not active. Check the \
         ShopId in the GHN dashboard.",
    )
    .with_param("shop_id", shop_id)
}

/// `ghn.permission_missing` (422): GHN trả 403 — tài khoản chưa được cấp quyền cho API này.
pub(crate) fn permission_missing() -> Error {
    invalid(
        "ghn.permission_missing",
        "The GHN account is not allowed to do this. Ask GHN to grant the permission.",
    )
}

/// `ghn.currency_unsupported` (422).
pub(crate) fn currency_unsupported(currency: &str) -> Error {
    invalid(
        "ghn.currency_unsupported",
        "GHN only takes amounts in VND; this request uses {currency}.",
    )
    .with_param("currency", currency)
}

/// `ghn.country_unsupported` (422).
pub(crate) fn country_unsupported(country: &str) -> Error {
    invalid(
        "ghn.country_unsupported",
        "GHN only delivers inside Vietnam; this address is in {country}.",
    )
    .with_param("country", country)
}

/// `ghn.service_unknown` (422).
pub(crate) fn service_unknown(service: &str) -> Error {
    invalid(
        "ghn.service_unknown",
        "GHN has no service `{service}`. Use `2` (light goods, under 20 kg) or `5` (heavy goods \
         or several parcels).",
    )
    .with_param("service", service)
}

/// `ghn.direction_unsupported` (422).
pub(crate) fn direction_unsupported() -> Error {
    invalid(
        "ghn.direction_unsupported",
        "The GHN connector cannot book a pickup of returned goods from a buyer yet. Create the \
         return in the GHN dashboard.",
    )
}

/// `ghn.phone_invalid` (422): `PHONE_INVALID`.
pub(crate) fn phone_invalid() -> Error {
    invalid(
        "ghn.phone_invalid",
        "GHN refused the sender or recipient phone number. Correct it and try again.",
    )
}

/// `ghn.address_not_recognized` (422): `WARD_IS_INVALID`.
pub(crate) fn address_not_recognized() -> Error {
    invalid(
        "ghn.address_not_recognized",
        "GHN does not recognise the address (province or ward name). Check the address and try \
         again.",
    )
}

/// `ghn.route_not_supported` (422): `ROUTE_NOT_FOUND_SERVICE`, `SERVICE_NOT_FOUND_CONFIG_FEE`.
pub(crate) fn route_not_supported() -> Error {
    invalid(
        "ghn.route_not_supported",
        "GHN does not serve this route with the chosen service. Choose another service or \
         carrier.",
    )
}

/// `ghn.order_not_found` (422): GHN không có đơn này trong tài khoản.
pub(crate) fn order_not_found() -> Error {
    invalid("ghn.order_not_found", "GHN does not find this order in the account.")
}

/// `ghn.webhook_body` (400): thân webhook không đọc được.
pub(crate) fn webhook_body() -> Error {
    Error::new(
        ErrorKind::BadRequest,
        "ghn.webhook_body",
        "The webhook body is not the JSON the GHN documentation describes.",
    )
}
