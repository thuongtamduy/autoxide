//! Luồng trả trước một phần (D-232, phép tính ở [`crate::advance`]): công bố trên giỏ, lập kế hoạch lúc
//! đặt, khoản trả trước thành công → `partially_paid`, hàng về đủ → yêu cầu phần còn lại (link thanh
//! toán qua `ledger`, hạn `balance_due_hours`), người mua mở lại link hết hạn, phần còn lại thành công →
//! `paid`. Quá hạn và huỷ có giữ lại ở `advance_cancel.rs`.

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::Money;
use autoxide_kernel::secret::sha256;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::json;
use uuid::Uuid;

use crate::advance::{
    self, ADVANCE_PAID, AWAITING_ADVANCE, BALANCE_PAID, BALANCE_REQUESTED, PAYMENT_LINK,
    PURPOSE_BALANCE, Split, SplitLine, Terms,
};
use crate::entity::{advance_plan, line, order, payment};
use crate::errors;
use crate::events::{Change, PreorderBalanceRequested};
use crate::payments;
use crate::policy::PREORDER_ADVANCE;
use crate::pricing::Quote;
use crate::service::{OrdersService, now};

/// Kế hoạch trả trước của một giỏ / lượt đặt.
#[derive(Debug, Clone)]
pub(crate) struct Planned {
    pub terms: Terms,
    pub split: Split,
    /// Mốc chốt của từng dòng của báo giá (cùng thứ tự).
    pub commit_at: Vec<Option<Timestamp>>,
}

fn split_lines(quote: &Quote) -> Vec<SplitLine> {
    quote
        .lines
        .iter()
        .map(|line| {
            let preorder = line.gift_of.is_none()
                && line.variant.as_ref().is_some_and(|variant| {
                    variant.sale_mode == autoxide_catalog::api::SaleMode::Preorder
                });
            let gross = if line.problem.is_none() {
                line.gross
            } else {
                Money::zero(quote.total.currency())
            };
            SplitLine { gross, preorder }
        })
        .collect()
}

impl OrdersService {
    /// Điều kiện trả trước của nút `org` (`None`: hàng đặt trước trả đủ lúc đặt).
    pub(crate) async fn advance_terms(
        &self,
        tenant: &TenantContext,
        org: OrgId,
    ) -> Result<Option<Terms>, Error> {
        let value = self.setting(tenant, org, &PREORDER_ADVANCE).await?;
        Ok(value.filter(|value| !value.is_null()).and_then(|value| Terms::parse(&value).ok()))
    }

    /// Kế hoạch trả trước cho báo giá `quote` ở nút `org`; `None` khi không áp dụng.
    pub(crate) async fn plan_for(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        quote: &Quote,
    ) -> Result<Option<Planned>, Error> {
        if !quote.has_preorder() {
            return Ok(None);
        }
        let Some(terms) = self.advance_terms(tenant, org).await? else { return Ok(None) };
        let Some(split) = advance::split(&split_lines(quote), quote.total, &terms)? else {
            return Ok(None);
        };
        let lines = split_lines(quote);
        let commit_at = quote
            .lines
            .iter()
            .zip(&lines)
            .map(|(quoted, line)| {
                let variant = quoted.variant.as_ref().filter(|_| line.preorder)?;
                terms.commit_at(variant.preorder_ends_at, variant.preorder_available_on)
            })
            .collect();
        Ok(Some(Planned { terms, split, commit_at }))
    }

    /// Kế hoạch trả trước của đơn, nếu có.
    pub(crate) async fn advance_plan(
        &self,
        connection: &impl ConnectionTrait,
        tenant: &TenantContext,
        order_id: Uuid,
    ) -> Result<Option<advance_plan::Model>, Error> {
        Ok(advance_plan::Entity::find()
            .filter(advance_plan::Column::OrderId.eq(order_id))
            .scoped(tenant)
            .one(connection)
            .await?)
    }

    /// Hàng đặt trước của đơn đã về đủ (mọi dòng đặt trước còn hiệu lực đã chia hàng): kế hoạch
    /// `advance_paid` → `balance_requested`, tạo khoản phần còn lại và mở link thanh toán. Gọi lại
    /// không tạo thêm.
    pub(crate) async fn request_balance(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
    ) -> Result<(), Error> {
        let database = tenant.database();
        let Some(plan) = self.advance_plan(database.read(), tenant, order_id).await? else {
            return Ok(());
        };
        if plan.status != ADVANCE_PAID {
            return Ok(());
        }
        let lines = line::Entity::find()
            .filter(line::Column::OrderId.eq(order_id))
            .filter(line::Column::SaleMode.eq("preorder"))
            .scoped(tenant)
            .all(database.read())
            .await?;
        let ready = lines
            .iter()
            .all(|line| line.allocated_quantity >= line.quantity - line.cancelled_quantity);
        if !ready {
            return Ok(());
        }
        let terms: Terms = serde_json::from_str(&plan.terms).map_err(Error::internal)?;
        let outbox = &self.outbox;
        let opened = database
            .write_with_retry(move || {
                let terms = terms.clone();
                async move {
                    let transaction = database.begin_write().await?;
                    let current = crate::lifecycle::lock(&transaction, tenant, order_id).await?;
                    let at = now();
                    let due = at + SignedDuration::from_hours(terms.balance_due_hours);
                    let moved = advance_plan::Entity::update_many()
                        .col_expr(advance_plan::Column::Status, Expr::value(BALANCE_REQUESTED))
                        .col_expr(advance_plan::Column::BalanceAttempt, Expr::value(1))
                        .col_expr(advance_plan::Column::BalanceRequestedAt, Expr::value(at))
                        .col_expr(advance_plan::Column::BalanceDueAt, Expr::value(due))
                        .col_expr(advance_plan::Column::UpdatedAt, Expr::value(at))
                        .filter(advance_plan::Column::OrderId.eq(order_id))
                        .filter(advance_plan::Column::Status.eq(ADVANCE_PAID))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                    if moved.rows_affected == 0 {
                        return Ok(false);
                    }
                    let balance = current.total_minor - current.paid_minor + current.refunded_minor;
                    balance_row(tenant, &current, balance.max(0), 1, &current.payment_method)
                        .insert(&transaction)
                        .await?;
                    let event = PreorderBalanceRequested(Change::after(json!({
                        "order_id": order_id, "tenant_id": current.tenant_id,
                        "org_id": current.org_id, "buyer_user_id": current.buyer_user_id,
                        "amount": Money::from_columns(balance.max(0), &current.currency)?,
                        "due_at": due,
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
                    Ok(true)
                }
            })
            .await?;
        if opened {
            self.start_payment(tenant, order_id, None).await?;
        }
        Ok(())
    }

    /// `POST /me/orders/{id}/payments` (người mua) / `POST /{id}/payment-link` (nhân viên): mở lại
    /// link trả phần còn lại khi lần trước đã hết hạn và chưa quá hạn trả. Link cũ còn hạn thì trả
    /// lại đúng link đó. Trả khoản đang chờ.
    pub(crate) async fn reopen_balance(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        return_url: Option<String>,
    ) -> Result<payment::Model, Error> {
        let database = tenant.database();
        let plan = self
            .advance_plan(database.read(), tenant, order_id)
            .await?
            .filter(|plan| plan.status == BALANCE_REQUESTED)
            .ok_or_else(errors::balance_not_requested)?;
        let at = now();
        if plan.balance_due_at.is_some_and(|due| due <= at) {
            return Err(errors::balance_overdue());
        }
        let latest = self.latest_balance(database.read(), tenant, order_id).await?;
        let reusable = latest.as_ref().filter(|row| {
            row.status == payments::PENDING && row.expires_at.is_none_or(|expires| expires > at)
        });
        if reusable.is_none() {
            let attempt = plan.balance_attempt;
            let previous = latest.as_ref().map(|row| row.method.clone());
            database
                .write_with_retry(|| async {
                    let transaction = database.begin_write().await?;
                    let current = crate::lifecycle::lock(&transaction, tenant, order_id).await?;
                    // Khoá dòng kế hoạch bằng `UPDATE … WHERE balance_attempt = <cũ>`: hai lần bấm cùng
                    // lúc chỉ một lần mở link mới.
                    let moved = advance_plan::Entity::update_many()
                        .col_expr(advance_plan::Column::BalanceAttempt, Expr::value(attempt + 1))
                        .col_expr(advance_plan::Column::UpdatedAt, Expr::value(at))
                        .filter(advance_plan::Column::OrderId.eq(order_id))
                        .filter(advance_plan::Column::BalanceAttempt.eq(attempt))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                    if moved.rows_affected == 0 {
                        return Ok(());
                    }
                    payment::Entity::update_many()
                        .col_expr(payment::Column::Status, Expr::value("expired"))
                        .col_expr(payment::Column::UpdatedAt, Expr::value(at))
                        .filter(payment::Column::OrderId.eq(order_id))
                        .filter(payment::Column::Purpose.eq(PURPOSE_BALANCE))
                        .filter(payment::Column::Status.eq(payments::PENDING))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                    let balance = current.total_minor - current.paid_minor + current.refunded_minor;
                    let method = previous.clone().unwrap_or_else(|| current.payment_method.clone());
                    balance_row(tenant, &current, balance.max(0), attempt + 1, &method)
                        .insert(&transaction)
                        .await?;
                    transaction.commit().await?;
                    Ok(())
                })
                .await?;
        }
        self.start_payment(tenant, order_id, return_url).await?;
        self.latest_balance(database.read(), tenant, order_id)
            .await?
            .ok_or_else(errors::balance_not_requested)
    }

    async fn latest_balance(
        &self,
        connection: &impl ConnectionTrait,
        tenant: &TenantContext,
        order_id: Uuid,
    ) -> Result<Option<payment::Model>, Error> {
        Ok(payment::Entity::find()
            .filter(payment::Column::OrderId.eq(order_id))
            .filter(payment::Column::Purpose.eq(PURPOSE_BALANCE))
            .scoped(tenant)
            .order_by_desc(payment::Column::Id)
            .one(connection)
            .await?)
    }

    /// Người mua mở lại link trả phần còn lại của đơn của mình.
    pub(crate) async fn buyer_reopen_balance(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        order_id: Uuid,
    ) -> Result<payment::Model, Error> {
        let row = self.order(tenant, order_id).await?;
        if row.buyer_user_id != Some(buyer) {
            return Err(errors::order_not_found());
        }
        self.reopen_balance(tenant, order_id, None).await
    }
}

/// Khoản `pending` của phần còn lại, lần thử `attempt`.
fn balance_row(
    tenant: &TenantContext,
    order: &order::Model,
    amount: i64,
    attempt: i32,
    method: &str,
) -> payment::ActiveModel {
    let at = now();
    // Phần còn lại không thu bằng COD (người mua đã chọn trả online lúc đặt).
    let method = if method == "cod" { "gateway" } else { method };
    payment::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        order_id: Set(order.id),
        purpose: Set(PURPOSE_BALANCE.to_owned()),
        method: Set(method.to_owned()),
        gateway: Set(None),
        ledger_payment_id: Set(None),
        external_id: Set(None),
        reference: Set(payments::reference(
            order.id,
            PURPOSE_BALANCE,
            u32::try_from(attempt).unwrap_or(1),
        )),
        amount_minor: Set(amount),
        currency: Set(order.currency.clone()),
        tendered_minor: Set(None),
        status: Set(payments::PENDING.to_owned()),
        action: Set(None),
        expires_at: Set(None),
        succeeded_at: Set(None),
        installment: Set(None),
        created_at: Set(at),
        updated_at: Set(at),
    }
}

/// Dòng `orders_advance_plans` của đơn mới.
pub(crate) fn plan_row(
    tenant: &TenantContext,
    order_id: Uuid,
    planned: &Planned,
) -> Result<advance_plan::ActiveModel, Error> {
    let terms = serde_json::to_string(&planned.terms).map_err(Error::internal)?;
    let at = now();
    Ok(advance_plan::ActiveModel {
        tenant_id: Set(tenant.id().as_uuid()),
        order_id: Set(order_id),
        currency: Set(planned.split.upfront.currency().code().to_owned()),
        advance_minor: Set(planned.split.advance.minor()),
        upfront_minor: Set(planned.split.upfront.minor()),
        balance_minor: Set(planned.split.balance.minor()),
        balance_method: Set(PAYMENT_LINK.to_owned()),
        terms_sha256: Set(sha256(&terms)),
        terms: Set(terms),
        status: Set(AWAITING_ADVANCE.to_owned()),
        balance_attempt: Set(0),
        balance_requested_at: Set(None),
        balance_due_at: Set(None),
        created_at: Set(at),
        updated_at: Set(at),
    })
}

/// Trạng thái kế hoạch sau khi khoản `purpose` thu xong; `None`: không đổi.
pub(crate) fn status_after_payment(purpose: &str) -> Option<&'static str> {
    match purpose {
        advance::PURPOSE_ADVANCE => Some(ADVANCE_PAID),
        PURPOSE_BALANCE => Some(BALANCE_PAID),
        _ => None,
    }
}

/// Hàng đặt trước của đơn có kế hoạch chưa trả đủ thì chưa giao được.
pub(crate) fn preorder_blocked(plan: Option<&advance_plan::Model>) -> bool {
    plan.is_some_and(|plan| plan.status != BALANCE_PAID && plan.status != advance::CANCELLED)
}
