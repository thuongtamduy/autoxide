//! Huỷ và quá hạn của đơn trả trước một phần (D-232): phần giữ lại khi người mua huỷ sau mốc chốt
//! (theo điều kiện đã chụp lúc đặt, mặc định 0 — luôn hoàn đủ), bút toán giữ lại, job
//! `orders.expire_preorder_balances` (quá hạn trả phần còn lại → huỷ hàng đặt trước theo
//! `overdue_action`). Người bán huỷ / ngừng bán / dời ngày quá giới hạn luôn hoàn đủ — các đường đó
//! không gọi phần giữ lại.

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, TenantId};
use autoxide_ledger::api::{AccountKey, LineAccount, NewEntry, NewLine};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde_json::json;
use uuid::Uuid;

use crate::advance::{self, BALANCE_OVERDUE, BALANCE_REQUESTED, Terms};
use crate::api::FundsFlow;
use crate::entity::{advance_plan, line};
use crate::events::{Change, PreorderBalanceOverdue};
use crate::preorders::LineCancel;
use crate::service::{OrdersService, now};

/// Tài khoản ghi phần giữ lại khi người mua huỷ.
pub(crate) const CANCELLATION_INCOME: &str = "orders.cancellation_income";

/// Phần giữ lại khi huỷ lúc `at`: mỗi dòng đặt trước đã qua mốc chốt giữ `retention` của phần trả
/// trước của dòng.
pub(crate) fn retention_of(terms: &Terms, lines: &[line::Model], at: DateTime<Utc>) -> i64 {
    lines
        .iter()
        .filter(|line| line.preorder_commit_at.is_some_and(|commit| commit <= at))
        .map(|line| terms.retention(line.advance_minor.unwrap_or(0)))
        .sum()
}

impl OrdersService {
    /// Phần giữ lại nếu người mua huỷ đơn `id` lúc này (0 khi không có kế hoạch trả trước).
    pub(crate) async fn retention_now(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<i64, Error> {
        let read = tenant.database().read();
        let Some(plan) = self.advance_plan(read, tenant, id).await? else { return Ok(0) };
        let terms: Terms = serde_json::from_str(&plan.terms).map_err(Error::internal)?;
        let lines = line::Entity::find()
            .filter(line::Column::OrderId.eq(id))
            .filter(line::Column::SaleMode.eq("preorder"))
            .scoped(tenant)
            .all(read)
            .await?;
        Ok(retention_of(&terms, &lines, now()))
    }

    /// Ghi phần giữ lại: Nợ `receivable.customer` / Có `orders.cancellation_income` (khoản đã thu
    /// thành thu nhập của người bán). Mã tham chiếu theo đơn: gọi lại không ghi hai lần. Chỉ khi
    /// người bán tự thu (thu hộ / chia tại cổng: `marketplace` ghi).
    pub(crate) async fn post_retention(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        keep: i64,
    ) -> Result<(), Error> {
        let Some(ledger) = &self.ledger else { return Ok(()) };
        if keep <= 0 {
            return Ok(());
        }
        let row = self.order(tenant, id).await?;
        let flow = FundsFlow::parse(&row.funds_flow).unwrap_or(FundsFlow::SelfCollect);
        if !matches!(flow, FundsFlow::SelfCollect | FundsFlow::ChannelCollect) {
            return Ok(());
        }
        let amount = Money::from_columns(keep, &row.currency)?;
        let book = OrgId::from_uuid(row.org_id);
        let receivable = crate::accounting::receivable_of(&row, book, amount.currency());
        let income = AccountKey::new(book, CANCELLATION_INCOME, amount.currency());
        let entry = NewEntry::new(
            book,
            "orders",
            "advance_retention",
            format!("orders:{id}:retention"),
            format!("Advance kept on cancelling order {}", row.number),
            vec![
                NewLine::debit(LineAccount::Key(receivable), amount),
                NewLine::credit(LineAccount::Key(income), amount),
            ],
        )
        .source_id(id.to_string());
        ledger.post(tenant, Actor::system(), entry).await.map(|_| ())
    }

    /// Job `orders.expire_preorder_balances`: kế hoạch `balance_requested` quá hạn trong `database`
    /// (mọi khách) → `balance_overdue`, event `orders.preorder_balance_overdue`, rồi huỷ hàng đặt
    /// trước theo `overdue_action` đã chụp. Trả số đơn đã xử lý.
    pub(crate) async fn expire_preorder_balances(&self, database: &Database) -> Result<u64, Error> {
        let due: Vec<(Uuid, Uuid)> = advance_plan::Entity::find()
            .select_only()
            .column(advance_plan::Column::TenantId)
            .column(advance_plan::Column::OrderId)
            .filter(advance_plan::Column::Status.eq(BALANCE_REQUESTED))
            .filter(advance_plan::Column::BalanceDueAt.lt(now()))
            .limit(crate::jobs::BATCH)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut handled = 0;
        for (tenant_id, id) in due {
            let tenant = self.databases.tenant(TenantId::from_uuid(tenant_id)).await?;
            match self.balance_overdue(&tenant, id).await {
                Ok(()) => handled += 1,
                Err(error) => {
                    tracing::warn!(order_id = %id, code = error.code(), "overdue balance not handled");
                }
            }
        }
        Ok(handled)
    }

    async fn balance_overdue(&self, tenant: &TenantContext, id: Uuid) -> Result<(), Error> {
        let (database, outbox) = (tenant.database(), &self.outbox);
        let at = now();
        let moved = database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let current = crate::lifecycle::lock(&transaction, tenant, id).await?;
                let changed = advance_plan::Entity::update_many()
                    .col_expr(advance_plan::Column::Status, Expr::value(BALANCE_OVERDUE))
                    .col_expr(advance_plan::Column::UpdatedAt, Expr::value(at))
                    .filter(advance_plan::Column::OrderId.eq(id))
                    .filter(advance_plan::Column::Status.eq(BALANCE_REQUESTED))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
                if changed.rows_affected == 0 {
                    return Ok(false);
                }
                let event = PreorderBalanceOverdue(Change::after(json!({
                    "order_id": id, "tenant_id": current.tenant_id, "org_id": current.org_id,
                    "buyer_user_id": current.buyer_user_id,
                })));
                outbox
                    .record(
                        &transaction,
                        tenant,
                        Actor::system(),
                        Some(format!("orders.order:{id}")),
                        &event,
                    )
                    .await?;
                transaction.commit().await?;
                Ok(true)
            })
            .await?;
        // Gọi lại sau khi đã chuyển `balance_overdue` (lần trước hỏng giữa chừng) vẫn huỷ tiếp.
        let plan = self.advance_plan(database.read(), tenant, id).await?;
        let Some(plan) = plan.filter(|plan| moved || plan.status == BALANCE_OVERDUE) else {
            return Ok(());
        };
        let terms: Terms = serde_json::from_str(&plan.terms).map_err(Error::internal)?;
        let lines = line::Entity::find()
            .filter(line::Column::OrderId.eq(id))
            .filter(line::Column::SaleMode.eq("preorder"))
            .scoped(tenant)
            .all(database.read())
            .await?;
        let keep = if terms.overdue_action == "cancel_full_refund" {
            0
        } else {
            retention_of(&terms, &lines, at)
        };
        let ids: Vec<Uuid> = lines.iter().map(|line| line.id).collect();
        self.cancel_preorder_lines(
            tenant,
            id,
            &ids,
            "balance_overdue",
            LineCancel { allocated_too: true, keep },
        )
        .await?;
        advance_plan::Entity::update_many()
            .col_expr(advance_plan::Column::Status, Expr::value(advance::CANCELLED))
            .col_expr(advance_plan::Column::UpdatedAt, Expr::value(now()))
            .filter(advance_plan::Column::OrderId.eq(id))
            .scoped(tenant)
            .exec(database.write())
            .await?;
        Ok(())
    }
}
