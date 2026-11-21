//! Khoá người mua cho hạn mức khuyến mãi (`Cart.buyer_keys`, đặc tả 43 "Chống lạm dụng" và 45
//! "Giỏ: coupon, voucher") và cờ `first_order`.
//!
//! Một người có thể có nhiều khoá: `user:<id>` khi đăng nhập, `phone:<HMAC số điện thoại>` và
//! `address:<HMAC địa chỉ giao chuẩn hoá>`. `promotions` cộng lượt vào **mọi** khoá, nên đổi tài
//! khoản hay mua vãng lai với cùng số điện thoại / địa chỉ vẫn bị đếm chung. Bản thô không rời
//! module này: chỉ HMAC bằng khoá riêng của khách (D-194).

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_shipping::api::Address;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QuerySelect};
use sha2::Sha256;
use uuid::Uuid;

use crate::api::OrderStatus;
use crate::entity::order;
use crate::service::OrdersService;

/// Nhãn dẫn khoá HMAC của địa chỉ (khác nhãn của khoá liên hệ trong kernel).
const ADDRESS_KEY_PURPOSE: &str = "orders.address-key";

/// Khoá người mua của một giỏ và cờ đơn đầu.
#[derive(Debug, Clone, Default)]
pub(crate) struct BuyerIdentity {
    pub keys: Vec<String>,
    pub first_order: bool,
}

/// Dòng địa chỉ chuẩn hoá: chữ thường, mọi ký tự không phải chữ / số thành một khoảng trắng.
fn normalized_address(address: &Address) -> String {
    let line: String = address
        .line
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    // Byte 0 ngăn cách các phần: hai địa chỉ khác nhau không ghép thành cùng một chuỗi.
    format!(
        "{}\0{}\0{}\0{line}",
        address.country.trim().to_uppercase(),
        address.province_code.trim(),
        address.commune_code.trim()
    )
}

impl OrdersService {
    /// Khoá của địa chỉ giao dạng `address:<HMAC>`.
    async fn address_buyer_key(
        &self,
        tenant: &TenantContext,
        address: &Address,
    ) -> Result<String, Error> {
        let key = self.secret.derive_for(tenant.id(), ADDRESS_KEY_PURPOSE).await?;
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&key)
            .map_err(|error| Error::internal(format!("hmac key: {error}")))?;
        mac.update(normalized_address(address).as_bytes());
        Ok(format!("address:{}", URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())))
    }

    /// Khoá người mua và cờ đơn đầu từ người đăng nhập `buyer`, số điện thoại nhận hàng `phone`
    /// và địa chỉ giao `destination` của giỏ.
    ///
    /// `first_order`: chưa có đơn nào (trừ đơn đã huỷ) của tài khoản hay của số điện thoại này
    /// trong khách. Không biết người mua là ai (vãng lai chưa nhập số) thì coi là **không** phải
    /// đơn đầu — chương trình "đơn đầu" chỉ áp khi kiểm được.
    pub(crate) async fn buyer_identity(
        &self,
        tenant: &TenantContext,
        buyer: Option<Uuid>,
        phone: Option<&str>,
        destination: Option<&Address>,
    ) -> Result<BuyerIdentity, Error> {
        let mut keys = Vec::new();
        if let Some(buyer) = buyer {
            keys.push(format!("user:{buyer}"));
        }
        let phone_key = match phone {
            Some(raw) => self.buyer_key(tenant, raw).await?,
            None => None,
        };
        if let Some(key) = &phone_key {
            keys.push(format!("phone:{key}"));
        }
        if let Some(address) = destination {
            keys.push(self.address_buyer_key(tenant, address).await?);
        }
        let first_order = if buyer.is_some() || phone_key.is_some() {
            let mut who = Condition::any();
            if let Some(buyer) = buyer {
                who = who.add(order::Column::BuyerUserId.eq(buyer));
            }
            if let Some(key) = &phone_key {
                who = who.add(order::Column::BuyerKey.eq(key.clone()));
            }
            let earlier: Option<Uuid> = order::Entity::find()
                .select_only()
                .column(order::Column::Id)
                .filter(who)
                .filter(order::Column::Status.ne(OrderStatus::Cancelled.as_str()))
                .scoped(tenant)
                .into_tuple()
                .one(tenant.database().read())
                .await?;
            earlier.is_none()
        } else {
            false
        };
        Ok(BuyerIdentity { keys, first_order })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(line: &str) -> Address {
        serde_json::from_value(serde_json::json!({
            "country": "vn", "province_code": "01", "province_name": "Ha Noi",
            "commune_code": "00004", "commune_name": "Ba Dinh", "line": line,
        }))
        .unwrap()
    }

    #[test]
    fn dia_chi_chuan_hoa_bo_hoa_thuong_dau_cau_va_khoang_trang() {
        assert_eq!(
            normalized_address(&address("  12,  Trang   TIỀN ")),
            normalized_address(&address("12 trang tiền"))
        );
        assert_ne!(
            normalized_address(&address("12 Trang Tien")),
            normalized_address(&address("14 Trang Tien"))
        );
    }
}
