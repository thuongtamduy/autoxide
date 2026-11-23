//! Webhook trạng thái đơn của GHN (trang "Order Status Callback"): thân JSON khoá PascalCase.
//!
//! GHN KHÔNG ký webhook. Thứ chứng minh request là của đúng tài khoản: (1) địa chỉ webhook mang
//! token ngẫu nhiên của tài khoản hãng (`shipping` lưu băm, tìm tài khoản bằng nó trước khi gọi
//! hàm này) và (2) `ShopID` trong thân phải là cửa hàng của tài khoản đó. Sai `ShopID` →
//! `shipping.webhook_signature` (401), không đọc tiếp.

use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::time::Timestamp;
use autoxide_shipping::api::{CarrierAccount, CarrierUpdate, errors as carrier};
use serde::Deserialize;

use crate::{account, errors, status};

/// Các trường dùng tới. GHN luôn gửi đủ trường (trường trống mang giá trị không); `default` để
/// thiếu trường không làm hỏng cả webhook.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Callback {
    #[serde(rename = "ShopID")]
    shop_id: i64,
    #[serde(default)]
    time: String,
    #[serde(default)]
    order_code: String,
    #[serde(default, rename = "Type")]
    kind: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    reason_code: String,
    #[serde(default)]
    total_fee: i64,
}

/// Loại callback làm đổi trạng thái đơn. Các loại khác (`update_weight`, `update_cod`,
/// `update_fee`, `update_payment_type`, `cod`, `update_partial_return`) không đổi trạng thái: nhận
/// (2xx) nhưng không sinh cập nhật — tiền thu hộ đối soát qua bảng kê COD, không qua webhook.
const STATUS_TYPES: [&str; 2] = ["create", "switch_status"];

pub(crate) fn parse(account: &CarrierAccount, body: &[u8]) -> Result<Vec<CarrierUpdate>, Error> {
    let shop_id = account::shop_id(account)?;
    let callback: Callback = serde_json::from_slice(body).map_err(|_| errors::webhook_body())?;
    if callback.shop_id != shop_id {
        return Err(carrier::webhook_signature());
    }
    if callback.order_code.is_empty() || !STATUS_TYPES.contains(&callback.kind.as_str()) {
        return Ok(Vec::new());
    }
    let Some(mapped) = status::map(&callback.status) else {
        tracing::warn!(carrier_status = %callback.status, "GHN status without a shipment status; update skipped");
        return Ok(Vec::new());
    };
    // GHN: chống trùng theo OrderCode + Type + Time. `Time` sai dạng thì khoá vẫn ổn định (lấy
    // nguyên văn), chỉ thời điểm rơi về lúc nhận.
    let event_id =
        format!("{}:{}:{}:{}", callback.order_code, callback.kind, callback.status, callback.time);
    let occurred_at = Timestamp::parse_rfc3339(&callback.time).unwrap_or_else(|_| Timestamp::now());
    let note = match (callback.reason_code.is_empty(), callback.reason.is_empty()) {
        (true, true) => String::new(),
        (false, true) => callback.reason_code,
        (true, false) => callback.reason,
        (false, false) => format!("{}: {}", callback.reason_code, callback.reason),
    };
    let fee = (callback.total_fee > 0).then(|| Money::new(callback.total_fee, Currency::VND));
    Ok(vec![
        CarrierUpdate::new(event_id, callback.order_code, mapped, occurred_at)
            .carrier_status(callback.status)
            .fee(fee)
            .note(note),
    ])
}

/// Thử phá webhook của GHN (D-20261123-property-fuzzing): thân do bất kỳ ai biết địa chỉ webhook gửi.
#[cfg(test)]
mod properties {
    use super::*;
    use autoxide_shipping::api::CarrierCredentials;
    use proptest::prelude::*;
    use serde_json::json;

    fn account(shop_id: i64) -> CarrierAccount {
        CarrierAccount::new(
            Default::default(),
            "ghn",
            true,
            json!({ "shop_id": shop_id }),
            CarrierCredentials::default(),
        )
    }

    proptest! {
        #[test]
        fn than_bat_ky_khong_panic(body in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = parse(&account(885), &body);
        }

        #[test]
        fn cua_hang_khac_luon_bi_tu_choi(
            shop in 1i64..,
            other in any::<i64>(),
            status in "[a-z_]{0,20}",
            fee in any::<i64>(),
        ) {
            prop_assume!(shop != other);
            let body = json!({
                "ShopID": other, "OrderCode": "GHN1", "Type": "switch_status",
                "Status": status, "TotalFee": fee, "Time": "2026-11-23T08:00:00Z",
            });
            prop_assert!(parse(&account(shop), body.to_string().as_bytes()).is_err());
        }

        #[test]
        fn cap_nhat_dung_cua_hang_khong_co_phi_am(
            shop in 1i64..,
            status in prop_oneof![Just("delivered".to_owned()), Just("picking".to_owned()), "[a-z_]{0,20}"],
            fee in any::<i64>(),
            time in "\\PC{0,40}",
            code in "\\PC{0,40}",
        ) {
            let body = json!({
                "ShopID": shop, "OrderCode": code, "Type": "switch_status",
                "Status": status, "TotalFee": fee, "Time": time,
            });
            let updates = parse(&account(shop), body.to_string().as_bytes())
                .map_err(|error| TestCaseError::fail(error.message()))?;
            for update in updates {
                prop_assert!(update.fee.is_none_or(Money::is_positive));
                prop_assert_eq!(update.tracking_number, code.clone());
            }
        }
    }
}
