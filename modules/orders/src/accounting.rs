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

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Subscription};
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_ledger::api::{AccountKey, Counterparty, LineAccount, NewEntry, NewLine};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::api::{FundsFlow, events as names};
use crate::entity::line;
use crate::payments::Method;
use crate::service::OrdersService;

pub(crate) fn subscriptions(service: &Arc<OrdersService>) -> Vec<Subscription> {
    vec![Subscription::new(names::ORDER_DELIVERED, Arc::new(Revenue(service.clone())))]
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
    /// Ghi doanh thu của đơn đã giao (chỉ tự thu / kênh thu; gọi lại không ghi thêm).
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
        let goods_gross: i64 = lines.iter().map(|line| line.total_minor).sum();
        let goods_tax: i64 = lines.iter().map(|line| line.tax_minor).sum();
        let shipping_gross = row.total_minor - goods_gross;
        let shipping_tax = row.tax_minor - goods_tax;
        let book = OrgId::from_uuid(row.org_id);
        let currency = money(0)?.currency();
        let mut receivable = AccountKey::new(book, "receivable.customer", currency);
        // COD: `shipping` ghi phần đã thu vào tài khoản phải thu theo đơn.
        if Method::parse(&row.payment_method) == Method::Cod {
            receivable =
                receivable.with_counterparty(Counterparty::new("orders.order", id.to_string()));
        }
        let mut entry_lines =
            vec![NewLine::debit(LineAccount::Key(receivable), money(row.total_minor)?)];
        for (purpose, minor) in [
            ("revenue.sales", goods_gross - goods_tax),
            ("tax.vat_output", row.tax_minor),
            ("revenue.shipping", shipping_gross - shipping_tax),
        ] {
            if minor != 0 {
                let key = AccountKey::new(book, purpose, currency);
                entry_lines.push(NewLine::credit(LineAccount::Key(key), money(minor)?));
            }
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
        Ok(())
    }
}
