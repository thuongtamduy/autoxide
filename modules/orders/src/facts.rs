//! Trả lời tự động về trạng thái đơn cho `support` (D-230): ý định `orders.order_status` (tham số số
//! đơn). Chỉ trả lời chủ đơn — tài khoản người mua của đơn, hay số điện thoại đã xác thực trùng số
//! liên hệ của đơn; không chắc thì `None` để nhân viên trả lời. Câu trả lời là mẫu câu i18n điền dữ
//! liệu thật, không phải chữ do mô hình viết.

use autoxide_kernel::error::Error;
use autoxide_kernel::i18n::{Language, Translations};
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_support::api::{FactIntent, FactSource, Requester};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

use crate::api::OrderStatus;
use crate::checkout::Contact;
use crate::entity::order;
use crate::guests::DEFAULT_COUNTRY;

pub(crate) const ORDER_STATUS: &str = "orders.order_status";

/// Câu trả lời (khoá dịch `text:<câu>`): macro chỉ trả lại chuỗi, để `cargo xtask check` thấy câu và
/// đòi bản dịch tiếng Việt.
macro_rules! answer_text {
    ($text:literal) => {
        $text
    };
}

const ANSWER: &str = answer_text!("Order {number} is {status}.");

/// Cụm từ của từng trạng thái (dịch qua `text:`).
fn phrase(status: OrderStatus) -> &'static str {
    match status {
        OrderStatus::PendingPayment => answer_text!("waiting for payment"),
        OrderStatus::Confirmed => answer_text!("confirmed and waiting to be prepared"),
        OrderStatus::Preparing => answer_text!("being prepared"),
        OrderStatus::Shipping => answer_text!("on its way"),
        OrderStatus::Delivered => answer_text!("delivered"),
        OrderStatus::DeliveryFailed => answer_text!("not delivered yet: the last attempt failed"),
        OrderStatus::Completed => answer_text!("completed"),
        OrderStatus::Cancelled => answer_text!("cancelled"),
    }
}

pub(crate) struct OrderFacts {
    translations: Translations,
}

impl OrderFacts {
    pub(crate) fn new(translations: Translations) -> Self {
        OrderFacts { translations }
    }

    /// Người hỏi có phải chủ đơn `row` không.
    fn owns(requester: &Requester, row: &order::Model) -> bool {
        match requester {
            Requester::User(user) => row.buyer_user_id == Some(*user),
            Requester::VerifiedPhone(phone) => {
                let Ok(contact) = serde_json::from_str::<Contact>(&row.contact) else {
                    return false;
                };
                let asked = PhoneNumber::parse(phone, DEFAULT_COUNTRY);
                let own = PhoneNumber::parse(&contact.phone, DEFAULT_COUNTRY);
                matches!((asked, own), (Some(asked), Some(own)) if asked == own)
            }
            Requester::VerifiedEmail(_) => false,
        }
    }
}

#[async_trait::async_trait]
impl FactSource for OrderFacts {
    fn intents(&self) -> Vec<FactIntent> {
        vec![FactIntent {
            intent: ORDER_STATUS,
            description: answer_text!("What is the status of my order? By order number."),
            params: json!({
                "type": "object",
                "properties": {
                    "order_number": {"type": "string", "description": "Number of the order."},
                },
                "required": ["order_number"],
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
        if intent != ORDER_STATUS {
            return Ok(None);
        }
        let Some(number) =
            params["order_number"].as_str().map(str::trim).filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };
        let row = order::Entity::find()
            .filter(order::Column::Number.eq(number))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        let Some(row) = row else { return Ok(None) };
        if !Self::owns(requester, &row) {
            return Ok(None);
        }
        let Some(status) = OrderStatus::parse(&row.status) else { return Ok(None) };
        let language = Language::from_tag(locale).unwrap_or(Language::En);
        let status_text = self.translations.describe(phrase(status), language);
        Ok(Some(self.translations.describe_with(
            ANSWER,
            &[("number", &row.number), ("status", &status_text)],
            language,
        )))
    }
}

#[cfg(test)]
pub(crate) fn phrases() -> Vec<&'static str> {
    OrderStatus::ALL.iter().map(|status| phrase(*status)).collect()
}
