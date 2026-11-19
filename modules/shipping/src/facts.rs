//! Trả lời tự động về hành trình vận đơn cho `support` (bước 44 mục 8, D-230): ý định
//! `shipping.tracking` (mã vận đơn hay số đơn). Chỉ trả lời chủ đơn — tài khoản người mua của vận
//! đơn, hay số điện thoại đã xác thực trùng số người nhận; không chắc (không thấy, nhiều vận đơn
//! khác trạng thái, không chứng minh được là chủ) thì `None` để nhân viên trả lời. Câu trả lời là
//! mẫu câu i18n của module điền dữ liệu thật, không phải chữ do mô hình viết.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::i18n::{Language, Translations};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_support::api::{FactIntent, FactSource, Requester};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{Value, json};

use crate::api::ShipmentStatus;
use crate::entity::shipment;

pub(crate) const TRACKING: &str = "shipping.tracking";

/// Câu trả lời (khoá dịch `text:<câu>` trong `locales/vi.json`).
const ANSWER: &str = "Shipment {tracking} of order {order} is {status} (updated {at} UTC).";
const ANSWER_NO_TRACKING: &str = "The shipment of order {order} is {status} (updated {at} UTC).";

/// Cụm từ của từng trạng thái (dịch qua `text:`).
fn phrase(status: ShipmentStatus) -> &'static str {
    match status {
        ShipmentStatus::Pending => "being prepared",
        ShipmentStatus::Booking | ShipmentStatus::ReadyToShip => {
            "waiting for the carrier to pick it up"
        }
        ShipmentStatus::PickedUp => "picked up by the carrier",
        ShipmentStatus::InTransit => "on its way",
        ShipmentStatus::OutForDelivery => "out for delivery today",
        ShipmentStatus::Delivered => "delivered",
        ShipmentStatus::DeliveryFailed => "not delivered yet: the last attempt failed",
        ShipmentStatus::Returning => "on its way back to the shop",
        ShipmentStatus::Returned => "returned to the shop",
        ShipmentStatus::ReadyForPickup => "ready for pickup at the store",
        ShipmentStatus::PickedUpByBuyer => "collected at the store",
        ShipmentStatus::Cancelled => "cancelled",
        ShipmentStatus::Lost => "reported lost by the carrier; the shop will contact you",
    }
}

/// Mọi cụm từ (cho test giữ đủ bản dịch).
#[cfg(test)]
pub(crate) fn phrases() -> Vec<&'static str> {
    ShipmentStatus::ALL.iter().map(|status| phrase(*status)).collect()
}

/// Chữ số của một số điện thoại, đổi `+84…` thành `0…` để so.
fn phone_key(raw: &str) -> String {
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    match digits.strip_prefix("84") {
        Some(rest) if raw.trim_start().starts_with('+') => format!("0{rest}"),
        _ => digits,
    }
}

pub(crate) struct ShippingFacts {
    pub translations: Translations,
}

impl ShippingFacts {
    pub(crate) fn new(translations: Translations) -> Arc<Self> {
        Arc::new(ShippingFacts { translations })
    }
}

#[async_trait::async_trait]
impl FactSource for ShippingFacts {
    fn intents(&self) -> Vec<FactIntent> {
        vec![FactIntent {
            intent: TRACKING,
            description: "Where is my parcel? The status of a shipment, by tracking number or order number.",
            params: json!({
                "type": "object",
                "properties": {
                    "tracking_number": {"type": "string", "description": "Tracking number of the carrier."},
                    "order_number": {"type": "string", "description": "Number of the order."},
                },
            }),
        }]
    }

    async fn answer(
        &self,
        tenant: &TenantContext,
        requester: &Requester,
        intent: &str,
        params: &Value,
        locale: &str,
    ) -> Result<Option<String>, Error> {
        if intent != TRACKING {
            return Ok(None);
        }
        let text =
            |key: &str| params[key].as_str().map(str::trim).filter(|value| !value.is_empty());
        let mut condition = Condition::any();
        if let Some(tracking) = text("tracking_number") {
            condition = condition.add(shipment::Column::TrackingNumber.eq(tracking));
        }
        if let Some(order) = text("order_number") {
            condition = condition.add(shipment::Column::OrderNumber.eq(order));
        }
        if condition.is_empty() {
            return Ok(None);
        }
        let rows = shipment::Entity::find()
            .filter(condition)
            .scoped(tenant)
            .order_by_desc(shipment::Column::Id)
            .all(tenant.database().read())
            .await?;
        // Chỉ vận đơn của chính người hỏi.
        let mine: Vec<&shipment::Model> = rows
            .iter()
            .filter(|row| match requester {
                Requester::User(user) => row.buyer_id == Some(*user),
                Requester::VerifiedPhone(phone) => {
                    phone_key(&row.recipient_phone) == phone_key(phone)
                }
                Requester::VerifiedEmail(_) => false,
            })
            .collect();
        let [row] = mine.as_slice() else {
            // Không thấy, hay nhiều vận đơn (giao một phần): để nhân viên trả lời.
            return Ok(None);
        };
        let Some(status) = ShipmentStatus::parse(&row.status) else { return Ok(None) };
        let language = Language::from_tag(locale).unwrap_or(Language::En);
        let status_text = self.translations.describe(phrase(status), language);
        let at = row.status_at.format("%Y-%m-%d %H:%M").to_string();
        let answer = match &row.tracking_number {
            Some(tracking) => self.translations.describe_with(
                ANSWER,
                &[
                    ("tracking", tracking),
                    ("order", &row.order_number),
                    ("status", &status_text),
                    ("at", &at),
                ],
                language,
            ),
            None => self.translations.describe_with(
                ANSWER_NO_TRACKING,
                &[("order", &row.order_number), ("status", &status_text), ("at", &at)],
                language,
            ),
        };
        Ok(Some(answer))
    }
}

#[cfg(test)]
mod tests {
    use super::phone_key;

    #[test]
    fn so_dien_thoai_so_theo_chu_so() {
        assert_eq!(phone_key("+84912345678"), "0912345678");
        assert_eq!(phone_key("0912 345 678"), "0912345678");
    }
}
