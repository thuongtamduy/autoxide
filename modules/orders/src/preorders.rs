//! Hàng đặt trước (D-204, D-20261119-step45-pickup-preorder): nghe `inventory.preorder_allocated`
//! (hàng về được chia cho dòng → giao được), `catalog.preorder_date_changed` (ngày giao dời muộn hơn
//! ngày đã hứa → báo người mua, cho huỷ miễn phí) và biến thể đặt trước bị tắt / xoá (người bán ngừng
//! bán → huỷ phần chưa có hàng, hoàn tiền tự động).

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Subscription};
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::NaiveDate;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::OrderStatus;
use crate::entity::{line, order};
use crate::events::{Change, PreorderCancelled, PreorderDelayed};
use crate::lifecycle::{self, Applied, Cause, Transition};
use crate::refunds::RefundRequest;
use crate::service::OrdersService;

/// `inventory` chia hàng về cho một dòng đặt trước.
pub(crate) const PREORDER_ALLOCATED: &str = autoxide_inventory::api::events::PREORDER_ALLOCATED;
/// `catalog` dời ngày giao dự kiến của biến thể đặt trước (`before` / `after`: biến thể).
pub(crate) const PREORDER_DATE_CHANGED: &str = "catalog.preorder_date_changed";
const VARIANT_UPDATED: &str = autoxide_catalog::api::events::VARIANT_UPDATED;
const VARIANT_DELETED: &str = autoxide_catalog::api::events::VARIANT_DELETED;

/// Trạng thái đơn còn chờ hàng đặt trước.
const WAITING: [&str; 5] =
    ["pending_payment", "confirmed", "preparing", "shipping", "delivery_failed"];

pub(crate) fn subscriptions(service: &Arc<OrdersService>) -> Vec<Subscription> {
    let listener = Arc::new(PreorderEvents(service.clone()));
    [PREORDER_ALLOCATED, PREORDER_DATE_CHANGED, VARIANT_UPDATED, VARIANT_DELETED]
        .into_iter()
        .map(|name| Subscription::optional(name, listener.clone()))
        .collect()
}

struct PreorderEvents(Arc<OrdersService>);

fn uuid_at(value: &Value, key: &str) -> Option<Uuid> {
    value[key].as_str().and_then(|raw| raw.parse().ok())
}

#[async_trait::async_trait]
impl EventHandler for PreorderEvents {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let (before, after) = (&event.payload["before"], &event.payload["after"]);
        let tenant = &event.tenant;
        match event.name.as_str() {
            PREORDER_ALLOCATED => {
                let (Some(order), Some(key), Some(total)) = (
                    uuid_at(after, "order_id"),
                    after["line_key"].as_str(),
                    after["allocated_quantity"].as_i64(),
                ) else {
                    return Ok(());
                };
                self.0
                    .preorder_allocated(
                        tenant,
                        order,
                        key,
                        total,
                        uuid_at(after, "warehouse_org_id"),
                    )
                    .await
            }
            PREORDER_DATE_CHANGED => {
                let (Some(variant), Some(day)) = (
                    uuid_at(after, "id"),
                    after["preorder_available_on"].as_str().and_then(|day| day.parse().ok()),
                ) else {
                    return Ok(());
                };
                self.0.preorder_delayed(tenant, variant, day).await
            }
            VARIANT_UPDATED => {
                let stopped = before["sale_mode"] == "preorder" && after["active"] == false;
                match (stopped, uuid_at(after, "id")) {
                    (true, Some(variant)) => self.0.preorder_stopped(tenant, variant).await,
                    _ => Ok(()),
                }
            }
            VARIANT_DELETED => {
                let was_preorder = before["sale_mode"] == "preorder";
                match (was_preorder, uuid_at(before, "id")) {
                    (true, Some(variant)) => self.0.preorder_stopped(tenant, variant).await,
                    _ => Ok(()),
                }
            }
            _ => Ok(()),
        }
    }
}

/// Đơn vị đặt trước của dòng chưa có hàng và chưa huỷ.
fn unallocated(line: &line::Model) -> i64 {
    (line.quantity - line.cancelled_quantity - line.allocated_quantity).max(0)
}

/// Dòng đặt trước bị dời muộn hơn ngày đã hứa mà còn phần chưa có hàng.
pub(crate) fn delayed(line: &line::Model) -> bool {
    line.sale_mode == "preorder"
        && unallocated(line) > 0
        && match (line.preorder_delayed_to, line.preorder_available_on) {
            (Some(new), Some(promised)) => new > promised,
            (Some(_), None) => true,
            _ => false,
        }
}

impl OrdersService {
    async fn preorder_allocated(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        line_key: &str,
        allocated: i64,
        warehouse: Option<Uuid>,
    ) -> Result<(), Error> {
        let mut update = line::Entity::update_many()
            .col_expr(line::Column::AllocatedQuantity, Expr::value(allocated))
            .filter(line::Column::OrderId.eq(order_id))
            .filter(line::Column::LineKey.eq(line_key))
            // Event cũ tới sau không lùi số đã chia.
            .filter(line::Column::AllocatedQuantity.lt(allocated));
        if let Some(warehouse) = warehouse {
            update = update.col_expr(line::Column::WarehouseOrgId, Expr::value(Some(warehouse)));
        }
        update.scoped(tenant).exec(tenant.database().write()).await?;
        // Trả trước một phần (D-232): hàng đặt trước về đủ → yêu cầu phần còn lại.
        self.request_balance(tenant, order_id).await
    }

    /// Dòng đặt trước đang chờ của biến thể `variant`.
    async fn waiting_lines(
        &self,
        tenant: &TenantContext,
        variant: Uuid,
    ) -> Result<Vec<line::Model>, Error> {
        let waiting = sea_orm::sea_query::Query::select()
            .column(order::Column::Id)
            .from(order::Entity)
            .and_where(order::Column::TenantId.eq(tenant.id().as_uuid()))
            .and_where(order::Column::Status.is_in(WAITING))
            .to_owned();
        let lines = line::Entity::find()
            .filter(line::Column::VariantId.eq(variant))
            .filter(line::Column::SaleMode.eq("preorder"))
            .filter(line::Column::OrderId.in_subquery(waiting))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        Ok(lines.into_iter().filter(|line| unallocated(line) > 0).collect())
    }

    /// Ngày giao của biến thể đặt trước đổi thành `day`: dòng đang chờ nhận ngày mới; muộn hơn
    /// ngày đã hứa (chụp lúc đặt) → `orders.preorder_delayed` (người mua được huỷ miễn phí).
    async fn preorder_delayed(
        &self,
        tenant: &TenantContext,
        variant: Uuid,
        day: NaiveDate,
    ) -> Result<(), Error> {
        for line in self.waiting_lines(tenant, variant).await? {
            if line.preorder_delayed_to == Some(day) {
                continue;
            }
            let row = self.order(tenant, line.order_id).await?;
            let later = line.preorder_available_on.is_none_or(|promised| day > promised);
            let (database, outbox) = (tenant.database(), &self.outbox);
            let line = &line;
            let row = &row;
            database
                .write_with_retry(move || async move {
                    let transaction = database.begin_write().await?;
                    line::Entity::update_many()
                        .col_expr(line::Column::PreorderDelayedTo, Expr::value(Some(day)))
                        .filter(line::Column::Id.eq(line.id))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                    if later {
                        let event = PreorderDelayed(Change::after(json!({
                            "order_id": row.id, "tenant_id": row.tenant_id, "org_id": row.org_id,
                            "channel": row.channel, "number": row.number,
                            "buyer_user_id": row.buyer_user_id, "line_id": line.id,
                            "variant_id": line.variant_id, "sku": line.sku,
                            "promised_on": line.preorder_available_on, "expected_on": day,
                            "free_cancellation": true,
                        })));
                        outbox
                            .record(&transaction, tenant, Actor::system(), subject(row.id), &event)
                            .await?;
                    }
                    transaction.commit().await?;
                    Ok(())
                })
                .await?;
        }
        Ok(())
    }

    /// Người bán ngừng bán đặt trước biến thể `variant`: đơn chỉ còn chờ hàng này thì huỷ cả đơn
    /// và hoàn tiền; đơn có phần khác thì huỷ phần chưa có hàng và hoàn phần tiền của nó.
    async fn preorder_stopped(&self, tenant: &TenantContext, variant: Uuid) -> Result<(), Error> {
        for line in self.waiting_lines(tenant, variant).await? {
            self.cancel_preorder_lines(
                tenant,
                line.order_id,
                &[line.id],
                "preorder_stopped",
                LineCancel::default(),
            )
            .await?;
        }
        Ok(())
    }

    /// Huỷ phần đặt trước chưa có hàng của các dòng `line_ids` của đơn `order_id` (lý do `reason`)
    /// và hoàn tiền tương ứng. Đơn chưa giao gì và không còn gì khác để giao → huỷ cả đơn.
    pub(crate) async fn cancel_preorder_lines(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        line_ids: &[Uuid],
        reason: &str,
        cancel: LineCancel,
    ) -> Result<(), Error> {
        let (database, outbox) = (tenant.database(), &self.outbox);
        let outcome = database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let current = lifecycle::lock(&transaction, tenant, order_id).await?;
                if !WAITING.contains(&current.status.as_str()) {
                    return Ok(Outcome::Nothing);
                }
                let lines = line::Entity::find()
                    .filter(line::Column::OrderId.eq(order_id))
                    .scoped(tenant)
                    .all(&transaction)
                    .await?;
                let rows =
                    crate::fulfillments::fulfillments_of(&transaction, tenant, order_id).await?;
                let cancelling: Vec<(&line::Model, i64)> = lines
                    .iter()
                    .filter(|line| line_ids.contains(&line.id) && line.sale_mode == "preorder")
                    .map(|line| {
                        let units = if cancel.allocated_too {
                            line.quantity - line.cancelled_quantity
                        } else {
                            unallocated(line)
                        };
                        (line, units)
                    })
                    .filter(|(_, units)| *units > 0)
                    .collect();
                if cancelling.is_empty() {
                    return Ok(Outcome::Nothing);
                }
                // Còn gì khác để giao (hay đã giao) thì chỉ huỷ phần này.
                let others_left = lines.iter().any(|line| {
                    line.requires_shipping
                        && line.quantity
                            - line.cancelled_quantity
                            - cancelling
                                .iter()
                                .find(|(cancelled, _)| cancelled.id == line.id)
                                .map_or(0, |(_, units)| *units)
                            > 0
                });
                let cancellable = matches!(
                    OrderStatus::parse(&current.status),
                    Some(
                        OrderStatus::PendingPayment
                            | OrderStatus::Confirmed
                            | OrderStatus::Preparing
                    )
                );
                if !others_left && rows.is_empty() && cancellable {
                    let transition =
                        Transition::new(OrderStatus::Cancelled, Cause::System, Actor::system())
                            .reason(reason);
                    let note = transition.reason.clone();
                    let applied = lifecycle::apply(
                        &transaction,
                        tenant,
                        outbox,
                        current,
                        &transition,
                        |row| {
                            row.cancel_reason = sea_orm::Set(note);
                        },
                    )
                    .await?;
                    transaction.commit().await?;
                    return Ok(if matches!(applied, Applied::Changed) {
                        Outcome::OrderCancelled
                    } else {
                        Outcome::Nothing
                    });
                }
                let mut value = 0;
                // Đơn trả trước một phần chưa trả phần còn lại: chỉ phần trả trước của dòng là đã thu.
                let mut advance_value = 0;
                let plan = self.advance_plan(&transaction, tenant, order_id).await?;
                let partial = crate::advance_flow::preorder_blocked(plan.as_ref());
                let mut cancelled = Vec::new();
                for (line, units) in &cancelling {
                    line::Entity::update_many()
                        .col_expr(
                            line::Column::CancelledQuantity,
                            Expr::value(line.cancelled_quantity + units),
                        )
                        .filter(line::Column::Id.eq(line.id))
                        .filter(line::Column::CancelledQuantity.eq(line.cancelled_quantity))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                    let share = i128::from(line.total_minor) * i128::from(*units)
                        / i128::from(line.quantity.max(1));
                    value += i64::try_from(share).map_err(Error::internal)?;
                    let advance = i128::from(line.advance_minor.unwrap_or(0)) * i128::from(*units)
                        / i128::from(line.quantity.max(1));
                    advance_value += i64::try_from(advance).map_err(Error::internal)?;
                    cancelled.push(json!({
                        "line_id": line.id, "variant_id": line.variant_id, "quantity": units,
                        "line_key": line.line_key,
                    }));
                }
                let event = PreorderCancelled(Change::after(json!({
                    "order_id": current.id, "tenant_id": current.tenant_id,
                    "org_id": current.org_id, "channel": current.channel,
                    "buyer_user_id": current.buyer_user_id, "reason": reason,
                    "lines": cancelled, "amount": Money::from_columns(value, &current.currency)?,
                })));
                outbox
                    .record(&transaction, tenant, Actor::system(), subject(order_id), &event)
                    .await?;
                transaction.commit().await?;
                let owed = if partial { advance_value } else { value };
                let refundable = (current.paid_minor - current.refunded_minor).min(owed);
                Ok(Outcome::LinesCancelled { refund: refundable, currency: current.currency })
            })
            .await?;
        match outcome {
            Outcome::OrderCancelled => {
                self.refund_cancelled_keeping(tenant, order_id, cancel.keep).await?;
                self.post_retention(tenant, order_id, cancel.keep).await
            }
            Outcome::LinesCancelled { refund, currency } if refund > 0 => {
                let refund = (refund - cancel.keep).max(0);
                self.post_retention(tenant, order_id, cancel.keep.min(refund + cancel.keep))
                    .await?;
                if refund == 0 {
                    return Ok(());
                }
                let ids: Vec<String> = line_ids.iter().map(Uuid::to_string).collect();
                let request = RefundRequest {
                    order_id,
                    amount: Money::from_columns(refund, &currency)?,
                    reason: reason.to_owned(),
                    reference: crate::refunds::short_reference(&format!(
                        "orders:{order_id}:{reason}:{}",
                        ids.join(",")
                    )),
                    return_id: None,
                    payment_id: None,
                    created_by: None,
                };
                self.refund(tenant, request).await.map(|_| ())
            }
            _ => Ok(()),
        }
    }
}

/// Cách huỷ dòng đặt trước.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LineCancel {
    /// Huỷ cả phần đã chia hàng (quá hạn trả phần còn lại), không chỉ phần chưa có hàng.
    pub allocated_too: bool,
    /// Giữ lại chừng này của phần trả trước (người mua huỷ sau mốc chốt, theo điều kiện đã chụp).
    pub keep: i64,
}

enum Outcome {
    Nothing,
    OrderCancelled,
    LinesCancelled { refund: i64, currency: String },
}

fn subject(order_id: Uuid) -> Option<String> {
    Some(format!("orders.order:{order_id}"))
}
