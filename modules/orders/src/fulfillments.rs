//! Giao hàng nhiều lần (D-204, D-20261119-step45-pickup-preorder): mỗi lần yêu cầu vận đơn là một
//! dòng `orders_fulfillments` (khoá `fulfillment:<id>` gửi `shipping`) với các đơn vị đã sẵn sàng —
//! hàng có sẵn đi ngay, hàng đặt trước chỉ khi `inventory` đã chia đủ hàng về
//! (`inventory.preorder_allocated`). Đơn chỉ `delivered` khi mọi đơn vị cần giao (trừ phần đã huỷ)
//! nằm trong lần giao đã tới tay người mua.

use std::collections::HashMap;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_shipping::api::{Address, ShipmentItem, ShipmentRequest};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::OrderStatus;
use crate::carts::ShipTo;
use crate::entity::{fulfillment, line, order};
use crate::errors;
use crate::events::{Change, FulfillmentRequested};
use crate::lifecycle::{self, Cause, Transition};
use crate::payments;
use crate::permissions::MANAGE;
use crate::service::OrdersService;

pub(crate) const REQUESTED: &str = "requested";
pub(crate) const SHIPPING: &str = "shipping";
pub(crate) const DELIVERED: &str = "delivered";
pub(crate) const FAILED: &str = "failed";

/// Một dòng của một lần giao (JSON trong cột `lines`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ShippedLine {
    pub line_id: Uuid,
    pub quantity: i64,
}

fn lines_of(row: &fulfillment::Model) -> Vec<ShippedLine> {
    serde_json::from_str(&row.lines).unwrap_or_default()
}

/// Số đơn vị của mỗi dòng nằm trong các lần giao có trạng thái thuộc `statuses`.
fn shipped(rows: &[fulfillment::Model], statuses: &[&str]) -> HashMap<Uuid, i64> {
    let mut total = HashMap::new();
    for row in rows.iter().filter(|row| statuses.contains(&row.status.as_str())) {
        for line in lines_of(row) {
            *total.entry(line.line_id).or_insert(0) += line.quantity;
        }
    }
    total
}

/// Đơn vị còn phải giao của dòng (trừ đã huỷ, trừ đã nằm trong lần giao chưa hỏng).
fn remaining(line: &line::Model, sent: &HashMap<Uuid, i64>) -> i64 {
    line.quantity - line.cancelled_quantity - sent.get(&line.id).copied().unwrap_or(0)
}

/// Dòng đã có hàng: hàng có sẵn, hay hàng đặt trước đã được chia đủ.
fn ready(line: &line::Model) -> bool {
    line.sale_mode != "preorder"
        || line.allocated_quantity >= line.quantity - line.cancelled_quantity
}

/// Giá trị hàng của `quantity` đơn vị dòng `line` (phần tiền đã trả, theo tỉ lệ, làm tròn xuống).
fn goods_value(line: &line::Model, quantity: i64) -> Result<i64, Error> {
    if line.quantity <= 0 || quantity <= 0 {
        return Ok(0);
    }
    let value = i128::from(line.total_minor) * i128::from(quantity.min(line.quantity))
        / i128::from(line.quantity);
    i64::try_from(value).map_err(Error::internal)
}

/// Tiền hàng của phần đã huỷ của các dòng (hàng đặt trước người bán ngừng bán, người mua huỷ khi
/// bị dời ngày) — không còn phải thu.
pub(crate) fn cancelled_value(lines: &[line::Model]) -> Result<i64, Error> {
    let mut total = 0;
    for line in lines {
        total += goods_value(line, line.cancelled_quantity)?;
    }
    Ok(total)
}

/// Mọi đơn vị cần giao đã tới tay người mua (lần giao `delivered`).
pub(crate) fn all_delivered(lines: &[line::Model], rows: &[fulfillment::Model]) -> bool {
    let delivered = shipped(rows, &[DELIVERED]);
    lines.iter().filter(|line| line.requires_shipping).all(|line| remaining(line, &delivered) <= 0)
}

pub(crate) async fn fulfillments_of(
    connection: &impl ConnectionTrait,
    tenant: &TenantContext,
    order_id: Uuid,
) -> Result<Vec<fulfillment::Model>, Error> {
    Ok(fulfillment::Entity::find()
        .filter(fulfillment::Column::OrderId.eq(order_id))
        .scoped(tenant)
        .order_by_asc(fulfillment::Column::Id)
        .all(connection)
        .await?)
}

impl OrdersService {
    /// Yêu cầu vận đơn cho các đơn vị đã sẵn sàng của đơn `id` (`orders.manage`): đơn `confirmed`
    /// thì nhận đơn (`preparing`) trước; phát `orders.fulfillment_requested` với `after.shipment` là
    /// `ShipmentRequest` của `shipping`. Không có gì mới để giao mà còn lần giao đang chờ: gửi lại
    /// đúng yêu cầu đó (cùng khoá — không tạo vận đơn thứ hai).
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
            OrderStatus::Confirmed
                | OrderStatus::Preparing
                | OrderStatus::Shipping
                | OrderStatus::DeliveryFailed
        ) {
            return Err(errors::shipment_not_allowed(status));
        }
        let actor = Actor::user(principal.user_id());
        let (database, outbox, note) = (tenant.database(), &self.outbox, &note);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let current = lifecycle::lock(&transaction, tenant, id).await?;
                let lines: Vec<line::Model> = line::Entity::find()
                    .filter(line::Column::OrderId.eq(id))
                    .filter(line::Column::RequiresShipping.eq(true))
                    .scoped(tenant)
                    .order_by_asc(line::Column::Id)
                    .all(&transaction)
                    .await?;
                let rows = fulfillments_of(&transaction, tenant, id).await?;
                let sent = shipped(&rows, &[REQUESTED, SHIPPING, DELIVERED]);
                // Trả trước một phần (D-232): hàng đặt trước chỉ đi khi phần còn lại đã trả.
                let plan = self.advance_plan(&transaction, tenant, id).await?;
                let blocked = crate::advance_flow::preorder_blocked(plan.as_ref());
                let held_back = blocked
                    && lines.iter().any(|line| {
                        line.sale_mode == "preorder" && ready(line) && remaining(line, &sent) > 0
                    });
                let picked: Vec<(line::Model, i64)> = lines
                    .iter()
                    .filter(|line| ready(line))
                    .filter(|line| !(blocked && line.sale_mode == "preorder"))
                    .filter_map(|line| {
                        let left = remaining(line, &sent);
                        (left > 0).then(|| (line.clone(), left))
                    })
                    .collect();
                if picked.is_empty() {
                    // Gửi lại yêu cầu còn chờ (agent retry), không thì không có gì để giao.
                    if let Some(pending) = rows.iter().rev().find(|row| row.status == REQUESTED) {
                        let items: Vec<(line::Model, i64)> = lines_of(pending)
                            .into_iter()
                            .filter_map(|shipped| {
                                lines
                                    .iter()
                                    .find(|line| line.id == shipped.line_id)
                                    .map(|line| (line.clone(), shipped.quantity))
                            })
                            .collect();
                        let cod = cod_for(&current, &lines, &rows, &items, Some(pending.id))?;
                        return shipment_request(&current, &items, &pending.request_key, note, cod);
                    }
                    if lines.is_empty() {
                        return Err(errors::delivery_required());
                    }
                    if held_back {
                        let left =
                            current.total_minor - current.paid_minor + current.refunded_minor;
                        let left = autoxide_kernel::money::Money::from_columns(
                            left.max(0),
                            &current.currency,
                        )?;
                        return Err(errors::balance_unpaid(left.to_string()));
                    }
                    return Err(errors::nothing_to_ship());
                }
                let next = match OrderStatus::parse(&current.status) {
                    Some(OrderStatus::Confirmed) => Some(OrderStatus::Preparing),
                    Some(OrderStatus::DeliveryFailed) => Some(OrderStatus::Shipping),
                    Some(OrderStatus::Preparing | OrderStatus::Shipping) => None,
                    Some(other) => return Err(errors::shipment_not_allowed(other)),
                    None => return Err(errors::order_not_found()),
                };
                let fulfillment_id = Uuid::now_v7();
                let key = format!("fulfillment:{fulfillment_id}");
                let cod = cod_for(&current, &lines, &rows, &picked, None)?;
                let request = shipment_request(&current, &picked, &key, note, cod)?;
                if let Some(next) = next {
                    let transition = Transition::new(next, Cause::Staff, actor);
                    lifecycle::apply(
                        &transaction,
                        tenant,
                        outbox,
                        current.clone(),
                        &transition,
                        |_| {},
                    )
                    .await?;
                }
                let now = crate::service::now();
                let shipped_lines: Vec<ShippedLine> = picked
                    .iter()
                    .map(|(line, quantity)| ShippedLine { line_id: line.id, quantity: *quantity })
                    .collect();
                fulfillment::ActiveModel {
                    id: Set(fulfillment_id),
                    tenant_id: Set(tenant.id().as_uuid()),
                    order_id: Set(id),
                    request_key: Set(key),
                    lines: Set(serde_json::to_string(&shipped_lines).map_err(Error::internal)?),
                    status: Set(REQUESTED.to_owned()),
                    cod_collected_minor: Set(None),
                    created_at: Set(now),
                    updated_at: Set(now),
                }
                .insert(&transaction)
                .await?;
                let event = FulfillmentRequested(Change::after(json!({
                    "order_id": id, "tenant_id": current.tenant_id, "org_id": current.org_id,
                    "channel": current.channel, "fulfillment_id": fulfillment_id,
                    "shipment": request,
                })));
                outbox
                    .record(&transaction, tenant, actor, Some(format!("orders.order:{id}")), &event)
                    .await?;
                transaction.commit().await?;
                Ok(request)
            })
            .await
    }

    /// Ghi trạng thái lần giao mang khoá `request_key` theo vận đơn (trong transaction của
    /// `follow_shipment`); trả `true` khi mọi đơn vị cần giao đã tới tay người mua.
    pub(crate) async fn note_fulfillment(
        &self,
        transaction: &impl ConnectionTrait,
        tenant: &TenantContext,
        order_id: Uuid,
        request_key: &str,
        to: OrderStatus,
    ) -> Result<bool, Error> {
        let status = match to {
            OrderStatus::Shipping => SHIPPING,
            OrderStatus::Delivered => DELIVERED,
            OrderStatus::DeliveryFailed => FAILED,
            _ => return Ok(false),
        };
        fulfillment::Entity::update_many()
            .col_expr(fulfillment::Column::Status, status.into())
            .col_expr(fulfillment::Column::UpdatedAt, crate::service::now().into())
            .filter(fulfillment::Column::OrderId.eq(order_id))
            .filter(fulfillment::Column::RequestKey.eq(request_key))
            // Đã giao thì không lùi lại (event tới sai thứ tự).
            .filter(fulfillment::Column::Status.ne(DELIVERED))
            .scoped(tenant)
            .exec(transaction)
            .await?;
        let lines = line::Entity::find()
            .filter(line::Column::OrderId.eq(order_id))
            .scoped(tenant)
            .all(transaction)
            .await?;
        let rows = fulfillments_of(transaction, tenant, order_id).await?;
        // Đơn cũ chưa có lần giao nào (yêu cầu trước khi có bảng này): theo vận đơn như trước.
        Ok(rows.is_empty() || all_delivered(&lines, &rows))
    }
}

/// Tiền hãng thu hộ cho lần giao `items` của đơn COD: lần cuối (không còn đơn vị nào chưa giao)
/// thu phần còn lại của đơn, lần trước thu giá trị hàng của lần đó. `None`: không thu hộ.
fn cod_for(
    row: &order::Model,
    lines: &[line::Model],
    rows: &[fulfillment::Model],
    items: &[(line::Model, i64)],
    this: Option<Uuid>,
) -> Result<Option<Money>, Error> {
    if row.payment_status != payments::COD_PENDING && row.payment_status != payments::PARTIALLY_PAID
    {
        return Ok(None);
    }
    let money = |minor: i64| Money::from_columns(minor, &row.currency);
    let others: Vec<fulfillment::Model> =
        rows.iter().filter(|other| Some(other.id) != this).cloned().collect();
    let mut sent = shipped(&others, &[REQUESTED, SHIPPING, DELIVERED]);
    for (line, quantity) in items {
        *sent.entry(line.id).or_insert(0) += quantity;
    }
    let last = lines.iter().all(|line| remaining(line, &sent) <= 0);
    if last {
        // Phần còn lại của đơn trừ tiền hàng của các lần giao khác chưa thu.
        let pending = shipped(&others, &[REQUESTED, SHIPPING]);
        let mut owed = row.total_minor - row.paid_minor - cancelled_value(lines)?;
        for line in lines {
            owed -= goods_value(line, pending.get(&line.id).copied().unwrap_or(0))?;
        }
        return Ok(Some(money(owed.max(0))?));
    }
    let mut value = 0;
    for (line, quantity) in items {
        value += goods_value(line, *quantity)?;
    }
    Ok(Some(money(value)?))
}

/// `ShipmentRequest` của `shipping` cho `items` (dòng, số đơn vị) của đơn.
fn shipment_request(
    row: &order::Model,
    items: &[(line::Model, i64)],
    key: &str,
    note: &Option<String>,
    cod: Option<Money>,
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
    if items.is_empty() {
        return Err(errors::delivery_required());
    }
    let money = |minor: i64| Money::from_columns(minor, &row.currency);
    let mut declared = 0;
    let mut shipment_items = Vec::new();
    for (line, quantity) in items {
        let snapshot: Value = serde_json::from_str(&line.snapshot).unwrap_or(Value::Null);
        shipment_items.push(
            ShipmentItem::new(
                line.id.to_string(),
                line.variant_id,
                &line.sku,
                &line.name,
                *quantity,
            )
            .weight(snapshot["weight_grams"].as_i64()),
        );
        declared += goods_value(line, *quantity)?;
    }
    let org = OrgId::from_uuid(row.org_id);
    let recipient = (
        ship_to.recipient_name.clone().unwrap_or_default(),
        ship_to.phone.clone().unwrap_or_default(),
    );
    let mut request = ShipmentRequest::new(
        row.id,
        row.number.clone(),
        key,
        org,
        org,
        method_id,
        recipient,
        address,
        shipment_items,
        money(declared)?,
    )
    .quoted_fee(money(row.shipping_fee_minor - row.shipping_discount_minor)?);
    if let Some(key) = option["option_key"].as_str() {
        request = request.option_key(key);
    }
    if let Some(buyer) = row.buyer_user_id {
        request = request.buyer(buyer);
    }
    if let Some(cod) = cod {
        request = request.cod(cod);
    }
    if let Some(note) = note {
        request = request.note(note.clone());
    }
    Ok(request)
}
