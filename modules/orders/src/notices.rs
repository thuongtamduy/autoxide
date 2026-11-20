//! Thư xác nhận và cập nhật trạng thái đơn cho người mua qua `notifications` (đặc tả 45 mục "Hoá đơn
//! và chứng từ của người mua", "Event … tự nghe `orders.order_*` để gửi thông báo"): tin giao dịch,
//! mẫu i18n (`<khoá>.title|body|short` trong `locales/vi.json`), ngôn ngữ của đơn. `orders` nghe event
//! của chính mình rồi gọi `dyn Notifier` NGOÀI transaction; khoá chống trùng là ID event, nên event
//! giao lại không gửi tin thứ hai.
//!
//! Người nhận: tài khoản của người mua; vãng lai thì email liên hệ, không có email thì số điện thoại.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler, Subscription};
use autoxide_kernel::money::Money;
use autoxide_kernel::sms::PhoneNumber;
use autoxide_notifications::api::{
    Category, Channel, Notification, NotificationKind, NotificationTemplate, Notifier, Recipient,
};
use serde_json::json;
use uuid::Uuid;

use crate::api::events as names;
use crate::checkout::Contact;
use crate::guests::DEFAULT_COUNTRY;
use crate::service::OrdersService;

const CHANNELS: &[Channel] =
    &[Channel::Email, Channel::Sms, Channel::Zns, Channel::InApp, Channel::MobilePush];

/// Đã nhận đơn (thư xác nhận).
pub(crate) const PLACED: NotificationKind = NotificationKind::new(
    Category::Transactional,
    CHANNELS,
    NotificationTemplate::new(
        "orders.order_placed",
        "We received your order {number}",
        "Thank you! We received your order {number} of {total}. We will let you know when it \
         ships.",
        "Order {number} received",
    ),
)
.sample(r#"{"number": "2026-000123", "total": "250.000 ₫", "link": "https://shop.example.com/orders/2026-000123"}"#);

/// Đã thanh toán / đã xác nhận.
pub(crate) const CONFIRMED: NotificationKind = NotificationKind::new(
    Category::Transactional,
    CHANNELS,
    NotificationTemplate::new(
        "orders.order_confirmed",
        "Order {number} is confirmed",
        "Your order {number} is confirmed and will be prepared soon.",
        "Order {number} confirmed",
    ),
)
.sample(r#"{"number": "2026-000123", "link": "https://shop.example.com/orders/2026-000123"}"#);

/// Đã giao cho hãng vận chuyển.
pub(crate) const SHIPPED: NotificationKind = NotificationKind::new(
    Category::Transactional,
    CHANNELS,
    NotificationTemplate::new(
        "orders.order_shipped",
        "Order {number} is on its way",
        "Your order {number} was handed to the carrier.",
        "Order {number} shipped",
    ),
)
.sample(r#"{"number": "2026-000123", "link": "https://shop.example.com/orders/2026-000123"}"#);

/// Đã giao xong.
pub(crate) const DELIVERED: NotificationKind = NotificationKind::new(
    Category::Transactional,
    CHANNELS,
    NotificationTemplate::new(
        "orders.order_delivered",
        "Order {number} was delivered",
        "Your order {number} was delivered. Thank you for shopping with us.",
        "Order {number} delivered",
    ),
)
.sample(r#"{"number": "2026-000123", "link": "https://shop.example.com/orders/2026-000123"}"#);

/// Đã huỷ.
pub(crate) const CANCELLED: NotificationKind = NotificationKind::new(
    Category::Transactional,
    CHANNELS,
    NotificationTemplate::new(
        "orders.order_cancelled",
        "Order {number} was cancelled",
        "Your order {number} was cancelled. Any amount paid is refunded to how you paid.",
        "Order {number} cancelled",
    ),
)
.sample(r#"{"number": "2026-000123", "link": "https://shop.example.com/orders/2026-000123"}"#);

pub(crate) fn declare(notifier: &dyn Notifier) -> Result<(), Error> {
    for kind in [PLACED, CONFIRMED, SHIPPED, DELIVERED, CANCELLED] {
        notifier.declare(kind)?;
    }
    Ok(())
}

pub(crate) fn subscriptions(service: &Arc<OrdersService>) -> Vec<Subscription> {
    if service.notifier.is_none() {
        return Vec::new();
    }
    let on = |kind: NotificationKind| Arc::new(Notice { service: service.clone(), kind });
    vec![
        Subscription::new(names::ORDER_PLACED, on(PLACED)),
        Subscription::new(names::ORDER_CONFIRMED, on(CONFIRMED)),
        Subscription::new(names::ORDER_STATUS_CHANGED, on(SHIPPED)),
        Subscription::new(names::ORDER_DELIVERED, on(DELIVERED)),
        Subscription::new(names::ORDER_CANCELLED, on(CANCELLED)),
    ]
}

struct Notice {
    service: Arc<OrdersService>,
    kind: NotificationKind,
}

#[async_trait::async_trait]
impl EventHandler for Notice {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(notifier) = &self.service.notifier else { return Ok(()) };
        let after = &event.payload["after"];
        let Some(id) = after["order_id"].as_str().and_then(|id| id.parse::<Uuid>().ok()) else {
            return Ok(());
        };
        // `order_status_changed` có cho mọi chuyển: chỉ gửi khi sang `shipping`.
        if self.kind.key() == SHIPPED.key() && after["status"].as_str() != Some("shipping") {
            return Ok(());
        }
        let row = self.service.order(&event.tenant, id).await?;
        // Đơn tạo thẳng ở `confirmed` (COD, quầy): thư "đã nhận đơn" là đủ.
        if self.kind.key() == CONFIRMED.key() && row.payment_status != "paid" {
            return Ok(());
        }
        // Bán tại quầy: khách cầm hoá đơn tại chỗ, không gửi tin.
        if row.channel == "pos" {
            return Ok(());
        }
        let contact: Option<Contact> = serde_json::from_str(&row.contact).ok();
        let recipient = match (row.buyer_user_id, contact) {
            (Some(user), _) => Recipient::User(user),
            (None, Some(Contact { email: Some(email), .. })) => Recipient::Email(email),
            (None, Some(contact)) => match PhoneNumber::parse(&contact.phone, DEFAULT_COUNTRY) {
                Some(phone) => Recipient::Phone(phone.as_str().to_owned()),
                None => return Ok(()),
            },
            (None, None) => return Ok(()),
        };
        let total = Money::from_columns(row.total_minor, &row.currency)?;
        let params = json!({"number": row.number, "total": total.to_string()});
        let notification =
            Notification::new(self.kind.key(), recipient, params, format!("orders:{}", event.id))
                .locale(row.locale.clone());
        if let Err(error) = notifier.notify(event.tenant.id(), notification).await {
            tracing::warn!(order_id = %id, code = error.code(), "order notice not sent");
        }
        Ok(())
    }
}
