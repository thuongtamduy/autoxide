//! Bút toán doanh thu của đơn (đặc tả 45 "Chỗ dễ sai", D-20261118-step45-lifecycle). Tiền người mua
//! trả đã được `ledger` ghi khi thu (Nợ `clearing.gateway` / Có `receivable.customer`) và `shipping`
//! ghi khi hãng thu COD (Có `receivable.customer` theo đơn). Khi đơn `delivered` và chế độ dòng tiền
//! là tự thu / kênh thu, `orders` ghi doanh thu:
//!
//! | Nợ | Có |
//! |---|---|
//! | `receivable.customer` (tổng đơn) | `revenue.sales` (tiền hàng chưa thuế), `tax.vat_output` (thuế), `revenue.shipping` (phí giao chưa thuế) |
//!
//! Thu hộ / chia tại cổng: `marketplace` (bước 46) ghi phần chia khi nghe `orders.order_completed`
//! — `orders` không ghi doanh thu của người bán. Mã tham chiếu `orders:<id>:revenue`: event giao lại
//! không ghi hai lần.
//!
//! Hoàn tiền (`orders.refund_completed`, D-20261119-step45-returns): cổng hoàn thì `ledger` ghi Nợ
//! `receivable.customer` / Có `clearing.gateway`; hoàn ngoài cổng (`manual`) thì `orders` ghi Nợ
//! `receivable.customer` / Có `cash.on_hand` (người bán trả bằng tiền của mình). Đơn đã ghi doanh thu
//! thì thêm bút toán đảo doanh thu — Nợ `revenue.sales`, `tax.vat_output`, `revenue.shipping` (chia
//! số hoàn theo tỉ lệ của bút toán doanh thu) / Có `receivable.customer`. Hoàn trước khi giao: bút
//! toán đảo ghi ngay sau bút toán doanh thu, khi đơn được giao.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Subscription};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_ledger::api::{AccountKey, Counterparty, LineAccount, NewEntry, NewLine};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::api::{FundsFlow, events as names};
use crate::entity::{line, order, refund};
use crate::payments::Method;
use crate::refunds;
use crate::service::OrdersService;

pub(crate) fn subscriptions(service: &Arc<OrdersService>) -> Vec<Subscription> {
    vec![
        Subscription::new(names::ORDER_DELIVERED, Arc::new(Revenue(service.clone()))),
        Subscription::new(names::REFUND_COMPLETED, Arc::new(Refunded(service.clone()))),
    ]
}

struct Refunded(Arc<OrdersService>);

#[async_trait::async_trait]
impl EventHandler for Refunded {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(refund) =
            event.payload["after"]["refund_id"].as_str().and_then(|id| id.parse::<Uuid>().ok())
        else {
            return Ok(());
        };
        self.0.post_refund(&event.tenant, refund).await
    }
}

/// Tiền doanh thu của đơn chia theo tài khoản: (mục đích, số tiền), bỏ dòng 0.
fn revenue_parts(row: &order::Model, lines: &[line::Model]) -> Vec<(&'static str, i64)> {
    let goods_gross: i64 = lines.iter().map(|line| line.total_minor).sum();
    let goods_tax: i64 = lines.iter().map(|line| line.tax_minor).sum();
    let shipping_gross = row.total_minor - goods_gross;
    let shipping_tax = row.tax_minor - goods_tax;
    [
        ("revenue.sales", goods_gross - goods_tax),
        ("tax.vat_output", row.tax_minor),
        ("revenue.shipping", shipping_gross - shipping_tax),
    ]
    .into_iter()
    .filter(|(_, minor)| *minor != 0)
    .collect()
}

/// Đơn có bút toán doanh thu do `orders` ghi không (đã giao, tự thu / kênh thu).
fn has_revenue(row: &order::Model) -> bool {
    let flow = FundsFlow::parse(&row.funds_flow).unwrap_or(FundsFlow::SelfCollect);
    matches!(flow, FundsFlow::SelfCollect | FundsFlow::ChannelCollect)
        && row.total_minor != 0
        && row.delivered_at.is_some()
}

/// Tài khoản phải thu của đơn (COD: theo đơn, vì `shipping` ghi phần đã thu vào đó).
pub(crate) fn receivable_of(row: &order::Model, book: OrgId, currency: Currency) -> AccountKey {
    let receivable = AccountKey::new(book, "receivable.customer", currency);
    if Method::parse(&row.payment_method) == Method::Cod {
        receivable.with_counterparty(Counterparty::new("orders.order", row.id.to_string()))
    } else {
        receivable
    }
}

struct Revenue(Arc<OrdersService>);

#[async_trait::async_trait]
impl EventHandler for Revenue {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(order) =
            event.payload["after"]["order_id"].as_str().and_then(|id| id.parse::<Uuid>().ok())
        else {
            return Ok(());
        };
        self.0.post_revenue(&event.tenant, order).await
    }
}

impl OrdersService {
    /// Ghi doanh thu của đơn đã giao (chỉ tự thu / kênh thu; gọi lại không ghi thêm), rồi bút
    /// toán đảo của các lượt hoàn đã xong trước đó.
    pub(crate) async fn post_revenue(&self, tenant: &TenantContext, id: Uuid) -> Result<(), Error> {
        let Some(ledger) = &self.ledger else { return Ok(()) };
        let row = self.order(tenant, id).await?;
        let flow = FundsFlow::parse(&row.funds_flow).unwrap_or(FundsFlow::SelfCollect);
        if !matches!(flow, FundsFlow::SelfCollect | FundsFlow::ChannelCollect)
            || row.total_minor == 0
        {
            return Ok(());
        }
        let lines = line::Entity::find()
            .filter(line::Column::OrderId.eq(id))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let money = |minor: i64| Money::from_columns(minor, &row.currency);
        let book = OrgId::from_uuid(row.org_id);
        let currency = money(0)?.currency();
        let receivable = receivable_of(&row, book, currency);
        let mut entry_lines =
            vec![NewLine::debit(LineAccount::Key(receivable), money(row.total_minor)?)];
        for (purpose, minor) in revenue_parts(&row, &lines) {
            let key = AccountKey::new(book, purpose, currency);
            entry_lines.push(NewLine::credit(LineAccount::Key(key), money(minor)?));
        }
        let entry = NewEntry::new(
            book,
            "orders",
            "revenue",
            format!("orders:{id}:revenue"),
            format!("Revenue of order {}", row.number),
            entry_lines,
        )
        .source_id(id.to_string());
        ledger.post(tenant, Actor::system(), entry).await?;
        let done = refund::Entity::find()
            .filter(refund::Column::OrderId.eq(id))
            .filter(refund::Column::Status.is_in([refunds::SUCCEEDED, refunds::MANUAL]))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        for refunded in done {
            self.post_refund(tenant, refunded.id).await?;
        }
        Ok(())
    }

    /// Bút toán của lượt hoàn `id` đã xong (xem đầu file). Gọi lại không ghi thêm.
    pub(crate) async fn post_refund(&self, tenant: &TenantContext, id: Uuid) -> Result<(), Error> {
        let Some(ledger) = &self.ledger else { return Ok(()) };
        let Some(refunded) =
            refund::Entity::find_by_id(id).scoped(tenant).one(tenant.database().read()).await?
        else {
            return Ok(());
        };
        if ![refunds::SUCCEEDED, refunds::MANUAL].contains(&refunded.status.as_str()) {
            return Ok(());
        }
        let row = self.order(tenant, refunded.order_id).await?;
        let amount = Money::from_columns(refunded.amount_minor, &refunded.currency)?;
        let book = OrgId::from_uuid(row.org_id);
        let receivable = receivable_of(&row, book, amount.currency());
        let order_id = row.id;
        if refunded.status == refunds::MANUAL {
            let cash = AccountKey::new(book, "cash.on_hand", amount.currency());
            let entry = NewEntry::new(
                book,
                "orders",
                "refund",
                format!("orders:{order_id}:refund:{id}:paid"),
                format!("Refund of order {} paid by the seller", row.number),
                vec![
                    NewLine::debit(LineAccount::Key(receivable.clone()), amount),
                    NewLine::credit(LineAccount::Key(cash), amount),
                ],
            )
            .source_id(order_id.to_string());
            ledger.post(tenant, Actor::system(), entry).await?;
        }
        if !has_revenue(&row) {
            return Ok(());
        }
        let lines = line::Entity::find()
            .filter(line::Column::OrderId.eq(order_id))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let parts = revenue_parts(&row, &lines);
        let ratios: Vec<u64> =
            parts.iter().map(|(_, minor)| u64::try_from(*minor).unwrap_or(0)).collect();
        if ratios.iter().all(|ratio| *ratio == 0) {
            return Ok(());
        }
        let shares = amount.allocate(&ratios)?;
        let mut entry_lines = Vec::new();
        for ((purpose, _), share) in parts.iter().zip(shares) {
            if share.minor() != 0 {
                let key = AccountKey::new(book, *purpose, amount.currency());
                entry_lines.push(NewLine::debit(LineAccount::Key(key), share));
            }
        }
        entry_lines.push(NewLine::credit(LineAccount::Key(receivable), amount));
        let entry = NewEntry::new(
            book,
            "orders",
            "sales_return",
            format!("orders:{order_id}:refund:{id}:revenue"),
            format!("Refund of order {}", row.number),
            entry_lines,
        )
        .source_id(order_id.to_string());
        ledger.post(tenant, Actor::system(), entry).await?;
        Ok(())
    }
}
