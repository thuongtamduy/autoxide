//! Máy trạng thái của đơn (đặc tả 45 mục "Máy trạng thái đơn"). Bảng chuyển hợp lệ là hằng
//! [`TRANSITIONS`]; MỌI lần đổi `status` đi qua [`apply`]: kiểm bảng, ghi `orders_status_history`,
//! ghi event (outbox) — cùng transaction với thay đổi của bên gọi. Không có chỗ nào khác gán cột
//! `status` của đơn (test `status_is_only_changed_by_lifecycle` đọc mã nguồn để kiểm).
//!
//! Bên gọi khoá dòng đơn trước ([`lock`]): huỷ và thanh toán cùng lúc thì bên sau thấy trạng thái
//! mới và làm nhánh đúng. Chuyển tới đúng trạng thái đang có là thành công, không làm gì (event giao
//! lại, nhận lại thanh toán).

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Timestamp;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{OrderStatus, hold_reference};
use crate::entity::{line, order, status_history};
use crate::errors;
use crate::events::{
    Change, OrderCancelled, OrderCompleted, OrderConfirmed, OrderDelivered, OrderPlaced,
    OrderStatusChanged,
};

use OrderStatus::{
    Cancelled, Completed, Confirmed, Delivered, DeliveryFailed, PendingPayment, Preparing, Shipping,
};

/// Các chuyển hợp lệ (từ, sang).
pub(crate) const TRANSITIONS: [(OrderStatus, OrderStatus); 13] = [
    (PendingPayment, Confirmed),
    (PendingPayment, Cancelled),
    (Confirmed, Preparing),
    (Confirmed, Delivered),
    (Confirmed, Cancelled),
    (Preparing, Shipping),
    (Preparing, Cancelled),
    (Shipping, Delivered),
    (Shipping, DeliveryFailed),
    (DeliveryFailed, Shipping),
    (DeliveryFailed, Cancelled),
    (Delivered, Completed),
    // Event "đã giao" của hãng tới trước "đã lấy hàng" (đặc tả 45 "Nhận lại event").
    (Preparing, Delivered),
];

pub(crate) fn allowed(from: OrderStatus, to: OrderStatus) -> bool {
    TRANSITIONS.contains(&(from, to))
}

/// Ai / cái gì gây ra một chuyển.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cause {
    Buyer,
    Staff,
    System,
    Payment,
    Shipping,
}

impl Cause {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Cause::Buyer => "buyer",
            Cause::Staff => "staff",
            Cause::Shipping => "shipping",
            Cause::System => "system",
            Cause::Payment => "payment",
        }
    }
}

/// Một lần chuyển.
#[derive(Debug, Clone)]
pub(crate) struct Transition {
    pub to: OrderStatus,
    pub cause: Cause,
    pub actor: Actor,
    pub reason: Option<String>,
}

impl Transition {
    pub(crate) fn new(to: OrderStatus, cause: Cause, actor: Actor) -> Self {
        Transition { to, cause, actor, reason: None }
    }

    #[must_use]
    pub(crate) fn reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

/// Kết quả của [`apply`].
#[derive(Debug, Clone)]
pub(crate) enum Applied {
    /// Đã chuyển.
    Changed,
    /// Đơn đã ở trạng thái đích: không làm gì.
    Already(Box<order::Model>),
}

/// Đọc và khoá dòng đơn `id` của khách `tenant` tới hết `transaction` (transaction GHI).
pub(crate) async fn lock(
    transaction: &impl ConnectionTrait,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<order::Model, Error> {
    rows::lock_one(
        transaction,
        order::Entity::find_by_id(id).filter(order::Column::TenantId.eq(tenant.id().as_uuid())),
    )
    .await?
    .ok_or_else(errors::order_not_found)
}

/// Chuyển đơn `current` (đã khoá bằng [`lock`]) theo `transition`; `edit` đổi thêm cột khác của
/// đơn trong cùng lần ghi (tiền đã thu, lý do huỷ…).
pub(crate) async fn apply(
    transaction: &impl ConnectionTrait,
    tenant: &TenantContext,
    outbox: &Outbox,
    current: order::Model,
    transition: &Transition,
    edit: impl FnOnce(&mut order::ActiveModel),
) -> Result<Applied, Error> {
    let from = OrderStatus::parse(&current.status)
        .ok_or_else(|| Error::internal(format!("unknown order status `{}`", current.status)))?;
    if from == transition.to {
        return Ok(Applied::Already(Box::new(current)));
    }
    if !allowed(from, transition.to) {
        return Err(errors::invalid_transition(from, transition.to));
    }
    let now = crate::service::now();
    let mut row: order::ActiveModel = current.clone().into();
    // Chỗ DUY NHẤT gán cột `status` của đơn.
    row.status = Set(transition.to.as_str().to_owned());
    row.version = Set(current.version + 1);
    row.updated_at = Set(now);
    stamp(&mut row, transition.to, now);
    edit(&mut row);
    let updated = row.update(transaction).await?;
    record_history(transaction, tenant, updated.id, Some(from), transition, now).await?;
    emit(transaction, tenant, outbox, &updated, Some(from), transition).await?;
    Ok(Applied::Changed)
}

/// Tạo đơn mới ở trạng thái `transition.to` (`pending_payment` hay `confirmed`): chèn dòng đơn,
/// ghi lịch sử và event `orders.order_placed` (+ `orders.order_confirmed` khi tạo thẳng ở
/// `confirmed`). Dòng đơn (`orders_lines`) phải chèn TRƯỚC để event mang đủ dòng.
pub(crate) async fn create(
    transaction: &impl ConnectionTrait,
    tenant: &TenantContext,
    outbox: &Outbox,
    mut row: order::ActiveModel,
    transition: &Transition,
) -> Result<order::Model, Error> {
    let now = crate::service::now();
    row.status = Set(transition.to.as_str().to_owned());
    stamp(&mut row, transition.to, now);
    let created = row.insert(transaction).await?;
    record_history(transaction, tenant, created.id, None, transition, now).await?;
    let after = payload(transaction, &created).await?;
    let subject = Some(format!("orders.order:{}", created.id));
    let change = Change { before: None, after: Some(after) };
    outbox
        .record(
            transaction,
            tenant,
            transition.actor,
            subject.clone(),
            &OrderPlaced(change.clone()),
        )
        .await?;
    if transition.to == Confirmed {
        outbox
            .record(transaction, tenant, transition.actor, subject, &OrderConfirmed(change))
            .await?;
    }
    Ok(created)
}

/// Ghi mốc thời gian theo trạng thái đích.
fn stamp(row: &mut order::ActiveModel, to: OrderStatus, now: Timestamp) {
    match to {
        Confirmed => row.confirmed_at = Set(Some(now)),
        Delivered => {
            row.delivered_at = Set(Some(now));
            // Hàng đã tới tay người mua (kho ghi `sale` theo `orders.order_delivered`, UT-1).
            row.fulfillment_status = Set("fulfilled".to_owned());
        }
        Completed => row.completed_at = Set(Some(now)),
        Cancelled => row.cancelled_at = Set(Some(now)),
        PendingPayment | Preparing | Shipping | DeliveryFailed => {}
    }
}

/// Dòng lịch sử của một lần chuyển (`from` = `None`: đơn mới).
pub(crate) async fn record_history(
    transaction: &impl ConnectionTrait,
    tenant: &TenantContext,
    order_id: Uuid,
    from: Option<OrderStatus>,
    transition: &Transition,
    at: Timestamp,
) -> Result<(), Error> {
    status_history::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        order_id: Set(order_id),
        from_status: Set(from.map(|status| status.as_str().to_owned())),
        to_status: Set(transition.to.as_str().to_owned()),
        cause: Set(transition.cause.as_str().to_owned()),
        actor_id: Set(transition.actor.user_id),
        reason: Set(transition.reason.clone()),
        at: Set(at),
    }
    .insert(transaction)
    .await?;
    Ok(())
}

/// Thân event của đơn: định danh, trạng thái, tiền, dòng (biến thể, số lượng, kho).
pub(crate) async fn payload(
    transaction: &impl ConnectionTrait,
    order: &order::Model,
) -> Result<Value, Error> {
    let lines = line::Entity::find()
        .filter(line::Column::TenantId.eq(order.tenant_id))
        .filter(line::Column::OrderId.eq(order.id))
        .order_by_asc(line::Column::Id)
        .all(transaction)
        .await?;
    // Tiền người mua trả cho hàng hoá (sau giảm, gồm thuế, không phí giao) — cơ sở tính tổng chi và
    // điểm của `customers` (D-226).
    let goods: i64 = lines.iter().map(|line| line.total_minor).sum();
    // … không gồm phần trả bằng điểm / thẻ quà tặng (bước 47).
    let tendered = crate::tenders::tendered_minor(transaction, order.tenant_id, order.id).await?;
    let goods_paid = (goods - tendered).max(0);
    let lines: Vec<Value> = lines
        .iter()
        .map(|line| {
            json!({"line_id": line.id, "line_key": line.line_key, "variant_id": line.variant_id,
                "product_id": line.product_id, "seller_org_id": line.seller_org_id,
                "quantity": line.quantity, "warehouse_org_id": line.warehouse_org_id,
                "net": {"amount": line.total_minor.to_string(), "currency": order.currency},
                "tax": {"amount": line.tax_minor.to_string(), "currency": order.currency}})
        })
        .collect();
    let money = |minor: i64| json!({"amount": minor.to_string(), "currency": order.currency});
    Ok(json!({
        "order_id": order.id,
        "tenant_id": order.tenant_id,
        "org_id": order.org_id,
        "channel": order.channel,
        "number": order.number,
        "status": order.status,
        "payment_status": order.payment_status,
        "payment_method": order.payment_method,
        "funds_flow": order.funds_flow,
        "buyer_user_id": order.buyer_user_id,
        // Khoá liên hệ của số người mua (HMAC theo khách, không phải số): `customers` nhận diện
        // khách quen không có tài khoản.
        "buyer_key": order.buyer_key,
        "placed_at": order.placed_at,
        "hold_reference": hold_reference(order.id),
        "total": money(order.total_minor),
        "paid": money(order.paid_minor),
        "goods_paid": money(goods_paid),
        "lines": lines,
    }))
}

/// Event của một lần chuyển: `orders.order_status_changed` luôn, cộng event riêng của trạng thái đích.
pub(crate) async fn emit(
    transaction: &impl ConnectionTrait,
    tenant: &TenantContext,
    outbox: &Outbox,
    order: &order::Model,
    from: Option<OrderStatus>,
    transition: &Transition,
) -> Result<(), Error> {
    let mut after = payload(transaction, order).await?;
    after["cause"] =
        json!(transition.reason.clone().unwrap_or_else(|| transition.cause.as_str().to_owned()));
    let before = from.map(|from| json!({"order_id": order.id, "status": from.as_str()}));
    let change = Change { before, after: Some(after) };
    let subject = Some(format!("orders.order:{}", order.id));
    let actor = transition.actor;
    outbox
        .record(transaction, tenant, actor, subject.clone(), &OrderStatusChanged(change.clone()))
        .await?;
    match transition.to {
        Confirmed => {
            outbox.record(transaction, tenant, actor, subject, &OrderConfirmed(change)).await
        }
        Cancelled => {
            outbox.record(transaction, tenant, actor, subject, &OrderCancelled(change)).await
        }
        Delivered => {
            // Phần mỗi dòng chưa đi theo lần giao lẻ nào (kho xuất đúng phần này, kèm serial đã ghi).
            let mut change = change;
            if let Some(after) = change.after.as_mut() {
                after["delivered_lines"] = json!(
                    crate::fulfillments::undelivered_lines(transaction, tenant, order.id).await?
                );
            }
            outbox.record(transaction, tenant, actor, subject, &OrderDelivered(change)).await
        }
        Completed => {
            outbox.record(transaction, tenant, actor, subject, &OrderCompleted(change)).await
        }
        PendingPayment | Preparing | Shipping | DeliveryFailed => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_in_table_is_allowed_others_rejected() {
        for from in OrderStatus::ALL {
            for to in OrderStatus::ALL {
                let expected = TRANSITIONS.contains(&(from, to));
                assert_eq!(allowed(from, to), expected, "{from} -> {to}");
            }
        }
        // Đơn đã huỷ hay hoàn tất không đi đâu nữa.
        for to in OrderStatus::ALL {
            assert!(!allowed(Cancelled, to));
            assert!(!allowed(Completed, to));
        }
        assert!(allowed(PendingPayment, Confirmed));
        assert!(!allowed(PendingPayment, Shipping));
    }

    /// Có khối `order::ActiveModel { … }` nào gán `status: Set(` không.
    fn literal_sets_status(text: &str) -> bool {
        text.match_indices("order::ActiveModel {").any(|(start, found)| {
            let body = &text[start + found.len()..];
            let mut depth = 1;
            let end = body
                .char_indices()
                .find(|(_, c)| {
                    match c {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {}
                    }
                    depth == 0
                })
                .map_or(body.len(), |(index, _)| index);
            body[..end].lines().any(|line| line.trim_start().starts_with("status: Set("))
        })
    }

    #[test]
    fn status_is_only_changed_by_lifecycle() {
        // Đọc mã nguồn của crate: chỉ file này gán cột `status` của đơn (`row.status = Set(`).
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs")
                    && !path.ends_with("lifecycle.rs")
                {
                    let text = std::fs::read_to_string(&path).unwrap();
                    // Đổi cột `status` của `orders_orders` bằng câu UPDATE, hay dựng dòng đơn
                    // với `status`.
                    let assigns = text.contains("col_expr(order::Column::Status")
                        || literal_sets_status(&text);
                    if assigns {
                        offenders.push(path.display().to_string());
                    }
                }
            }
        }
        assert!(offenders.is_empty(), "order status assigned outside lifecycle.rs: {offenders:?}");
    }
}

/// Thử phá state machine của đơn (D-20261123-property-fuzzing). Bảng nhỏ nên kiểm vét cạn các tính chất
/// của đồ thị; dãy lệnh ngẫu nhiên (đúng như người mua, nhân viên, hãng và cổng thanh toán gửi lệch thứ tự)
/// thì sinh bằng proptest.
#[cfg(test)]
mod properties {
    use super::*;
    use crate::policy::Policy;
    use proptest::prelude::*;

    const TERMINAL: [OrderStatus; 2] = [Cancelled, Completed];

    /// Các trạng thái đi tới được từ `start` qua các chuyển hợp lệ.
    fn reachable(start: OrderStatus) -> Vec<OrderStatus> {
        let mut seen = vec![start];
        let mut index = 0;
        while let Some(&from) = seen.get(index) {
            for (source, to) in TRANSITIONS {
                if source == from && !seen.contains(&to) {
                    seen.push(to);
                }
            }
            index += 1;
        }
        seen
    }

    #[test]
    fn do_thi_trang_thai_dung_hinh() {
        for (from, to) in TRANSITIONS {
            assert_ne!(from, to, "vòng về chính nó: {from}");
            assert!(!TERMINAL.contains(&from), "đi ra từ trạng thái cuối: {from} -> {to}");
        }
        // Mọi trạng thái đi tới được từ lúc tạo đơn (chờ thanh toán hay đã xác nhận).
        let from_start = reachable(PendingPayment);
        for status in OrderStatus::ALL {
            assert!(from_start.contains(&status), "không tới được {status}");
            // Không đơn nào kẹt mãi: từ mọi trạng thái đều tới được một trạng thái cuối.
            assert!(
                reachable(status).iter().any(|status| TERMINAL.contains(status)),
                "{status} không tới được trạng thái cuối"
            );
        }
        // Đã giao thì không còn huỷ (đơn đã giao đi qua trả hàng, không qua huỷ).
        for status in [Delivered, Completed] {
            assert!(!reachable(status).contains(&Cancelled), "{status} -> cancelled");
        }
    }

    #[test]
    fn nguoi_mua_chi_huy_duoc_khi_chuyen_huy_hop_le() {
        for until in OrderStatus::ALL {
            let policy = Policy {
                buyer_cancel_until: until,
                acceptance_hours: 24,
                return_window_days: 7,
                cod_enabled: true,
                allowed_return_reasons: Vec::new(),
                return_photo_reasons: Vec::new(),
                pickup_cancel_on_expiry: false,
            };
            for status in OrderStatus::ALL {
                if policy.buyer_may_cancel(status) {
                    assert!(allowed(status, Cancelled), "{until}: {status}");
                    assert!(!matches!(status, Shipping | Delivered | DeliveryFailed | Completed));
                }
            }
        }
    }

    proptest! {
        #[test]
        fn day_lenh_bat_ky_khong_ra_khoi_trang_thai_cuoi(
            first in prop_oneof![Just(PendingPayment), Just(Confirmed)],
            commands in proptest::collection::vec(proptest::sample::select(OrderStatus::ALL.to_vec()), 0..40),
        ) {
            // Mô hình của `apply`: cùng trạng thái thì không làm gì; chuyển không hợp lệ bị từ chối.
            let mut status = first;
            let mut history = vec![status];
            for to in commands {
                if to != status && allowed(status, to) {
                    prop_assert!(!TERMINAL.contains(&status));
                    status = to;
                    history.push(status);
                }
            }
            // Đã thu tiền (rời `pending_payment`) thì không quay lại chờ thanh toán.
            let left = history.iter().position(|status| *status != PendingPayment);
            if let Some(left) = left {
                prop_assert!(!history[left..].contains(&PendingPayment));
            }
            // Mỗi bước kế tiếp đều nằm trong bảng.
            for pair in history.windows(2) {
                prop_assert!(TRANSITIONS.contains(&(pair[0], pair[1])));
            }
        }
    }
}
