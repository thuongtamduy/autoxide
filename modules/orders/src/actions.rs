//! Vòng đời đơn (mục 4 của bước 45): nhân viên chuyển trạng thái, người mua tự huỷ theo chính sách
//! (`orders.buyer_cancel_until`), yêu cầu vận đơn (event `orders.fulfillment_requested` cho
//! `shipping`), hoàn tất khi hết hạn đổi trả, huỷ đơn cửa hàng không nhận kịp
//! (`orders.acceptance_hours`), hoãn hoàn tất (`OrderHolds`), hoàn tiền khi huỷ đơn đã thu. Mọi lần
//! đổi `status` đi qua `lifecycle::apply`.

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, TenantId};
use autoxide_ledger::api::NewRefund;
use autoxide_shipping::api::{Address, ShipmentItem, ShipmentRequest};
use chrono::{DateTime, Duration, Utc};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::OrderStatus;
use crate::carts::ShipTo;
use crate::entity::{completion_hold, line, order, payment};
use crate::errors;
use crate::events::{Change, FulfillmentRequested, RefundCompleted};
use crate::jobs::BATCH;
use crate::lifecycle::{Applied, Cause, Transition};
use crate::payments::{self, Method};
use crate::permissions::MANAGE;
use crate::service::OrdersService;

/// Trạng thái nhân viên được chuyển tay (xác nhận do thanh toán, hoàn tất do job).
const STAFF_TARGETS: [OrderStatus; 5] = [
    OrderStatus::Preparing,
    OrderStatus::Shipping,
    OrderStatus::Delivered,
    OrderStatus::DeliveryFailed,
    OrderStatus::Cancelled,
];

impl OrdersService {
    /// Nhân viên chuyển đơn `id` sang `to` (`orders.manage` ở nút của đơn). `expected`: `updated_at`
    /// người gọi đã đọc (`If-Match`); khác → 412.
    pub(crate) async fn staff_transition(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        to: OrderStatus,
        reason: Option<String>,
        expected: Option<DateTime<Utc>>,
    ) -> Result<order::Model, Error> {
        principal.require(&MANAGE)?;
        let row = self.order(tenant, id).await?;
        let target = OrgTarget::new(OrgId::from_uuid(row.org_id), None);
        principal.require_at(tenant.database().read(), &MANAGE, target).await?;
        if !STAFF_TARGETS.contains(&to) {
            let from = OrderStatus::parse(&row.status).unwrap_or(OrderStatus::Cancelled);
            return Err(errors::invalid_transition(from, to));
        }
        let mut transition = Transition::new(to, Cause::Staff, Actor::user(principal.user_id()));
        if let Some(reason) = reason {
            transition = transition.reason(reason);
        }
        let check = move |current: &order::Model| -> Result<bool, Error> {
            match expected {
                Some(version) if version != current.updated_at => {
                    Err(autoxide_kernel::version::changed())
                }
                _ => Ok(true),
            }
        };
        let applied = self.transition_if(tenant, id, &transition, &check).await?;
        if matches!(applied, Applied::Changed) && to == OrderStatus::Cancelled {
            self.refund_cancelled(tenant, id).await?;
        }
        self.order(tenant, id).await
    }

    /// Người mua `buyer` huỷ đơn của mình theo chính sách `orders.buyer_cancel_until` của nút.
    pub(crate) async fn buyer_cancel(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        id: Uuid,
        reason: Option<String>,
    ) -> Result<order::Model, Error> {
        let row = self.order(tenant, id).await?;
        if row.buyer_user_id != Some(buyer) {
            return Err(errors::order_not_found());
        }
        self.cancel_as_buyer(tenant, row, Actor::user(buyer), reason).await
    }

    /// Huỷ phía người mua (có tài khoản hay vãng lai): cùng chính sách, cùng service.
    pub(crate) async fn cancel_as_buyer(
        &self,
        tenant: &TenantContext,
        row: order::Model,
        actor: Actor,
        reason: Option<String>,
    ) -> Result<order::Model, Error> {
        let policy = self.policy(tenant, OrgId::from_uuid(row.org_id)).await?;
        let transition = Transition::new(OrderStatus::Cancelled, Cause::Buyer, actor)
            .reason(reason.unwrap_or_else(|| "buyer_cancelled".to_owned()));
        let check = move |current: &order::Model| -> Result<bool, Error> {
            let status = OrderStatus::parse(&current.status).unwrap_or(OrderStatus::Cancelled);
            if status == OrderStatus::Cancelled {
                return Ok(false);
            }
            if policy.buyer_may_cancel(status) {
                Ok(true)
            } else {
                Err(errors::cancel_not_allowed())
            }
        };
        let applied = self.transition_if(tenant, row.id, &transition, &check).await?;
        if matches!(applied, Applied::Changed) {
            self.refund_cancelled(tenant, row.id).await?;
        }
        self.order(tenant, row.id).await
    }

    /// Hoàn toàn bộ khoản đã thu của đơn đã huỷ qua `ledger` (mã tham chiếu theo khoản: gọi lại
    /// không hoàn hai lần). Khoản không qua đối tác (COD chưa thu) thì không có gì để hoàn.
    pub(crate) async fn refund_cancelled(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<(), Error> {
        let paid = payment::Entity::find()
            .filter(payment::Column::OrderId.eq(id))
            .filter(payment::Column::Status.eq(payments::SUCCEEDED))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let Some(ledger) = &self.ledger else { return Ok(()) };
        for row in paid {
            let Some(ledger_payment) = row.ledger_payment_id else { continue };
            let amount = Money::from_columns(row.amount_minor, &row.currency)?;
            let refund = NewRefund::new(
                ledger_payment,
                amount,
                "order_cancelled",
                format!("orders:{id}:cancel:{}", row.id),
            );
            ledger.refund(tenant, Actor::system(), refund).await?;
            self.mark_refunded(tenant, &row, amount).await?;
        }
        Ok(())
    }

    /// Ghi khoản `row` đã hoàn `amount`, cộng vào `refunded_minor` của đơn, phát
    /// `orders.refund_completed` — một transaction; khoản đã ghi hoàn thì không làm gì.
    async fn mark_refunded(
        &self,
        tenant: &TenantContext,
        row: &payment::Model,
        amount: Money,
    ) -> Result<(), Error> {
        let (database, outbox) = (tenant.database(), &self.outbox);
        let (payment_id, order_id) = (row.id, row.order_id);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let current = crate::lifecycle::lock(&transaction, tenant, order_id).await?;
                let Some(stored) =
                    payment::Entity::find_by_id(payment_id).one(&transaction).await?
                else {
                    return Ok(());
                };
                if stored.status == payments::REFUNDED {
                    return Ok(());
                }
                let now = crate::service::now();
                let mut refunded: payment::ActiveModel = stored.into();
                refunded.status = Set(payments::REFUNDED.to_owned());
                refunded.updated_at = Set(now);
                refunded.update(&transaction).await?;
                let total = current.refunded_minor + amount.minor();
                let state =
                    if total >= current.paid_minor { "refunded" } else { "partially_refunded" };
                order::Entity::update_many()
                    .col_expr(order::Column::RefundedMinor, Expr::value(total))
                    .col_expr(order::Column::PaymentStatus, Expr::value(state))
                    .col_expr(order::Column::UpdatedAt, Expr::value(now))
                    .filter(order::Column::Id.eq(order_id))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
                let event = RefundCompleted(Change::after(json!({
                    "order_id": order_id, "tenant_id": current.tenant_id, "org_id": current.org_id,
                    "channel": current.channel, "payment_id": payment_id, "amount": amount,
                    "reason": "order_cancelled",
                })));
                outbox
                    .record(
                        &transaction,
                        tenant,
                        Actor::system(),
                        Some(format!("orders.order:{order_id}")),
                        &event,
                    )
                    .await?;
                transaction.commit().await?;
                Ok(())
            })
            .await
    }

    /// Yêu cầu vận đơn cho đơn `id` (`orders.manage`): đơn `confirmed` thì nhận đơn (`preparing`)
    /// trước; phát `orders.fulfillment_requested` với `after.shipment` là `ShipmentRequest` của
    /// `shipping`. Khoá yêu cầu `order:<id>:<lần>` — gửi lại cùng lần không tạo vận đơn thứ hai.
    pub(crate) async fn request_shipment(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        note: Option<String>,
    ) -> Result<ShipmentRequest, Error> {
        principal.require(&MANAGE)?;
        let row = self.order(tenant, id).await?;
        let target = OrgTarget::new(OrgId::from_uuid(row.org_id), None);
        principal.require_at(tenant.database().read(), &MANAGE, target).await?;
        let status = OrderStatus::parse(&row.status).unwrap_or(OrderStatus::Cancelled);
        if !matches!(
            status,
            OrderStatus::Confirmed | OrderStatus::Preparing | OrderStatus::DeliveryFailed
        ) {
            return Err(errors::shipment_not_allowed(status));
        }
        let lines = line::Entity::find()
            .filter(line::Column::OrderId.eq(id))
            .filter(line::Column::RequiresShipping.eq(true))
            .scoped(tenant)
            .order_by_asc(line::Column::Id)
            .all(tenant.database().read())
            .await?;
        let request = shipment_request(&row, &lines, note)?;
        let actor = Actor::user(principal.user_id());
        let (database, outbox) = (tenant.database(), &self.outbox);
        let request_ref = &request;
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let current = crate::lifecycle::lock(&transaction, tenant, id).await?;
                // Nhận đơn khi yêu cầu giao lần đầu; giao lại sau khi giao hỏng.
                let next = match OrderStatus::parse(&current.status) {
                    Some(OrderStatus::Confirmed) => Some(OrderStatus::Preparing),
                    Some(OrderStatus::DeliveryFailed) => Some(OrderStatus::Shipping),
                    Some(OrderStatus::Preparing) => None,
                    Some(other) => return Err(errors::shipment_not_allowed(other)),
                    None => return Err(errors::order_not_found()),
                };
                if let Some(next) = next {
                    let transition = Transition::new(next, Cause::Staff, actor);
                    crate::lifecycle::apply(
                        &transaction,
                        tenant,
                        outbox,
                        current,
                        &transition,
                        |_| {},
                    )
                    .await?;
                }
                let event = FulfillmentRequested(Change::after(json!({
                    "order_id": id, "shipment": request_ref,
                })));
                outbox
                    .record(&transaction, tenant, actor, Some(format!("orders.order:{id}")), &event)
                    .await?;
                transaction.commit().await?;
                Ok(())
            })
            .await?;
        Ok(request)
    }

    /// Một lô đơn `delivered` quá hạn đổi trả của nút, không có hoãn → `completed`.
    pub(crate) async fn complete_delivered(&self, database: &Database) -> Result<u64, Error> {
        let due: Vec<(Uuid, Uuid, Uuid, Option<DateTime<Utc>>)> = order::Entity::find()
            .select_only()
            .column(order::Column::TenantId)
            .column(order::Column::Id)
            .column(order::Column::OrgId)
            .column(order::Column::DeliveredAt)
            .filter(order::Column::Status.eq(OrderStatus::Delivered.as_str()))
            .order_by_asc(order::Column::DeliveredAt)
            .limit(BATCH)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut completed = 0;
        for (tenant_id, id, org, delivered_at) in due {
            let tenant = self.databases.tenant(TenantId::from_uuid(tenant_id)).await?;
            let policy = self.policy(&tenant, OrgId::from_uuid(org)).await?;
            let ends = delivered_at.map(|at| at + Duration::days(policy.return_window_days));
            if ends.is_none_or(|ends| ends > crate::service::now()) {
                continue;
            }
            let held = completion_hold::Entity::find()
                .filter(completion_hold::Column::OrderId.eq(id))
                .scoped(&tenant)
                .one(tenant.database().read())
                .await?
                .is_some();
            if held {
                continue;
            }
            let transition =
                Transition::new(OrderStatus::Completed, Cause::System, Actor::system());
            match self.transition(&tenant, id, &transition, Some(OrderStatus::Delivered)).await {
                Ok(Applied::Changed) => completed += 1,
                Ok(Applied::Already(_)) => {}
                Err(error) => {
                    tracing::warn!(order_id = %id, code = error.code(), "order not completed")
                }
            }
        }
        Ok(completed)
    }

    /// Một lô đơn `confirmed` quá `orders.acceptance_hours` của nút mà cửa hàng chưa nhận →
    /// huỷ (`seller_not_accepted`), hoàn tiền tự động (D-209).
    pub(crate) async fn expire_unaccepted(&self, database: &Database) -> Result<u64, Error> {
        let due: Vec<(Uuid, Uuid, Uuid, Option<DateTime<Utc>>)> = order::Entity::find()
            .select_only()
            .column(order::Column::TenantId)
            .column(order::Column::Id)
            .column(order::Column::OrgId)
            .column(order::Column::ConfirmedAt)
            .filter(order::Column::Status.eq(OrderStatus::Confirmed.as_str()))
            .filter(order::Column::Channel.eq("web"))
            .order_by_asc(order::Column::ConfirmedAt)
            .limit(BATCH)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut cancelled = 0;
        for (tenant_id, id, org, confirmed_at) in due {
            let tenant = self.databases.tenant(TenantId::from_uuid(tenant_id)).await?;
            let policy = self.policy(&tenant, OrgId::from_uuid(org)).await?;
            if policy.acceptance_hours == 0 {
                continue;
            }
            let ends = confirmed_at.map(|at| at + Duration::hours(policy.acceptance_hours));
            if ends.is_none_or(|ends| ends > crate::service::now()) {
                continue;
            }
            let transition =
                Transition::new(OrderStatus::Cancelled, Cause::System, Actor::system())
                    .reason("seller_not_accepted");
            match self.transition(&tenant, id, &transition, Some(OrderStatus::Confirmed)).await {
                Ok(Applied::Changed) => {
                    cancelled += 1;
                    self.refund_cancelled(&tenant, id).await?;
                }
                Ok(Applied::Already(_)) => {}
                Err(error) => {
                    tracing::warn!(order_id = %id, code = error.code(), "unaccepted order not cancelled")
                }
            }
        }
        Ok(cancelled)
    }

    /// Việc thanh toán còn hạn của đơn chờ thanh toán (gửi lại link); lượt thu chưa tạo được lúc
    /// đặt thì tạo lại.
    pub(crate) async fn payment_link(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Vec<payment::Model>, Error> {
        principal.require(&MANAGE)?;
        let row = self.order(tenant, id).await?;
        let target = OrgTarget::new(OrgId::from_uuid(row.org_id), None);
        principal.require_at(tenant.database().read(), &MANAGE, target).await?;
        if row.status != OrderStatus::PendingPayment.as_str() {
            return Err(errors::invalid_transition(
                OrderStatus::parse(&row.status).unwrap_or(OrderStatus::Cancelled),
                OrderStatus::Confirmed,
            ));
        }
        if row.payment_due_at.is_some_and(|due| due <= crate::service::now()) {
            return Err(errors::payment_expired());
        }
        if Method::parse(&row.payment_method) != Method::Cod {
            self.start_payment(tenant, id, None).await?;
        }
        Ok(payment::Entity::find()
            .filter(payment::Column::OrderId.eq(id))
            .filter(payment::Column::Status.eq(payments::PENDING))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?)
    }
}

/// `ShipmentRequest` của `shipping` cho các dòng cần giao của đơn.
fn shipment_request(
    row: &order::Model,
    lines: &[line::Model],
    note: Option<String>,
) -> Result<ShipmentRequest, Error> {
    let ship_to: ShipTo = row
        .ship_to
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .ok_or_else(errors::delivery_required)?;
    let address: Address = ship_to.address.clone().ok_or_else(errors::delivery_required)?;
    let option: Value = row
        .shipping_option
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .ok_or_else(errors::delivery_required)?;
    let method_id = option["method_id"]
        .as_str()
        .and_then(|id| id.parse::<Uuid>().ok())
        .ok_or_else(errors::delivery_required)?;
    if lines.is_empty() {
        return Err(errors::delivery_required());
    }
    let items = lines
        .iter()
        .map(|line| {
            let snapshot: Value = serde_json::from_str(&line.snapshot).unwrap_or(Value::Null);
            ShipmentItem::new(
                line.id.to_string(),
                line.variant_id,
                &line.sku,
                &line.name,
                line.quantity,
            )
            .weight(snapshot["weight_grams"].as_i64())
        })
        .collect();
    let money = |minor: i64| Money::from_columns(minor, &row.currency);
    let declared = money(lines.iter().map(|line| line.total_minor).sum())?;
    let org = OrgId::from_uuid(row.org_id);
    let recipient = (
        ship_to.recipient_name.clone().unwrap_or_default(),
        ship_to.phone.clone().unwrap_or_default(),
    );
    let mut request = ShipmentRequest::new(
        row.id,
        row.number.clone(),
        format!("order:{}:{}", row.id, row.version),
        org,
        org,
        method_id,
        recipient,
        address,
        items,
        declared,
    )
    .quoted_fee(money(row.shipping_fee_minor - row.shipping_discount_minor)?);
    if let Some(key) = option["option_key"].as_str() {
        request = request.option_key(key);
    }
    if let Some(buyer) = row.buyer_user_id {
        request = request.buyer(buyer);
    }
    if row.payment_status == payments::COD_PENDING {
        request = request.cod(money(row.total_minor - row.paid_minor)?);
    }
    if let Some(note) = note {
        request = request.note(note);
    }
    Ok(request)
}

/// Hoãn hoàn tất đơn (tranh chấp của bước 46, yêu cầu trả hàng).
#[async_trait::async_trait]
impl crate::api::OrderHolds for OrdersService {
    async fn hold_completion(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        reference: &str,
    ) -> Result<(), Error> {
        self.order(tenant, id).await?;
        let row = completion_hold::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            order_id: Set(id),
            reference: Set(reference.to_owned()),
            created_at: Set(crate::service::now()),
        };
        completion_hold::Entity::insert(row)
            .on_conflict(
                OnConflict::columns([
                    completion_hold::Column::TenantId,
                    completion_hold::Column::OrderId,
                    completion_hold::Column::Reference,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(tenant.database().write())
            .await?;
        Ok(())
    }

    async fn release_completion(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        reference: &str,
    ) -> Result<(), Error> {
        completion_hold::Entity::delete_many()
            .filter(completion_hold::Column::OrderId.eq(id))
            .filter(completion_hold::Column::Reference.eq(reference))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        Ok(())
    }
}
