//! Lần thử thanh toán mới sau một lượt thu hỏng (UT-6 L4). Khoản `pending` của đơn trỏ tới lượt thu ở
//! `ledger` đã `failed` / `expired` (đối tác từ chối, link hết hạn) thì mở lại (`payment-link`) không
//! được trả lại lượt cũ: khoản cũ thành `superseded`, khoản mới mang lần thử n+1 (mã tham chiếu mới
//! `orders:<đơn>:<mục đích>:<n+1>` — `ledger` tạo lượt thu mới, không trả lại lượt cũ), nên người mua
//! nhận `payment_id`, `status_token` và `ledger_payment_id` mới.
//!
//! Lượt cũ đã kết thúc ở `ledger` (và ở đối tác: hợp đồng `PaymentGateway` không có lệnh huỷ lượt thu,
//! đối tác tự đóng lượt hỏng / hết hạn). Nếu đối tác vẫn báo đã thu lượt cũ (hiếm, sai thứ tự thông báo)
//! thì tiền đó được hoàn qua lệnh hoàn của đối tác (`Ledger::refund`, D-187: core không giữ tiền), theo
//! mã tham chiếu ổn định — event giao lại không hoàn hai lần — và ghi `orders.payment_recorded`
//! (`superseded: true`) một lần để audit.
//!
//! Đọc / ghi qua handle `tenant.db()` của kernel (luật raw-database).

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_ledger::api::NewRefund;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::json;
use uuid::Uuid;

use crate::entity::payment;
use crate::events::{Change, PaymentRecorded};
use crate::payments::{self, PENDING};
use crate::service::OrdersService;

/// Khoản đã được thay bằng một lần thử mới (lượt thu của nó hỏng hay hết hạn ở `ledger`).
pub(crate) const SUPERSEDED: &str = "superseded";
/// Trạng thái kết thúc không thu được của một lượt thu ở `ledger`.
const LEDGER_ENDED: [&str; 2] = ["failed", "expired"];
/// Lý do hoàn ghi ở `ledger`.
const REFUND_REASON: &str = "superseded_payment";

impl OrdersService {
    /// Khoản `pending` mới nhất của đơn `order_id` trỏ tới lượt thu đã hỏng / hết hạn ở `ledger`: thay
    /// bằng lần thử mới (chưa có lượt thu; [`OrdersService::start_payment`] tạo sau). Không có gì để
    /// thay thì không làm gì. Hai lần gọi cùng lúc chỉ một lần thay (khoá dòng đơn, cập nhật có điều
    /// kiện `status = pending`).
    pub(crate) async fn replace_ended_attempt(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
    ) -> Result<(), Error> {
        let Some(ledger) = self.ledger.clone() else {
            return Ok(());
        };
        let db = tenant.db();
        let Some(row) = payment::Entity::find()
            .filter(payment::Column::OrderId.eq(order_id))
            .filter(payment::Column::Status.eq(PENDING))
            .filter(payment::Column::LedgerPaymentId.is_not_null())
            // Phần còn lại của đơn trả trước có lần thử riêng (`reopen_balance`).
            .filter(payment::Column::Purpose.ne(crate::advance::PURPOSE_BALANCE))
            .scoped(tenant)
            .order_by_desc(payment::Column::Id)
            .one(&db.read())
            .await?
        else {
            return Ok(());
        };
        let Some(ledger_payment) = row.ledger_payment_id else {
            return Ok(());
        };
        // Gọi `ledger` ngoài transaction ghi.
        let ended = ledger
            .payment(tenant, ledger_payment)
            .await?
            .is_some_and(|found| LEDGER_ENDED.contains(&found.status.as_str()));
        if !ended {
            return Ok(());
        }
        let transaction = db.begin_write().await?;
        let current = crate::lifecycle::lock(&transaction, tenant, order_id).await?;
        let at = crate::service::now();
        let moved = payment::Entity::update_many()
            .col_expr(payment::Column::Status, Expr::value(SUPERSEDED))
            .col_expr(payment::Column::UpdatedAt, Expr::value(at))
            .filter(payment::Column::Id.eq(row.id))
            .filter(payment::Column::Status.eq(PENDING))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if moved.rows_affected == 0 {
            // Một lần gọi khác vừa thay (hay khoản vừa được thu): không làm gì thêm.
            transaction.rollback().await?;
            return Ok(());
        }
        let attempts = payment::Entity::find()
            .filter(payment::Column::OrderId.eq(order_id))
            .filter(payment::Column::Purpose.eq(row.purpose.clone()))
            .filter(payment::Column::Reference.starts_with(attempt_prefix(order_id, &row.purpose)))
            .scoped(tenant)
            .count(&transaction)
            .await?;
        let attempt = u32::try_from(attempts).unwrap_or(u32::MAX).saturating_add(1);
        payment::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(row.tenant_id),
            order_id: Set(order_id),
            purpose: Set(row.purpose.clone()),
            method: Set(row.method.clone()),
            gateway: Set(None),
            ledger_payment_id: Set(None),
            external_id: Set(None),
            reference: Set(payments::reference(order_id, &row.purpose, attempt)),
            amount_minor: Set(row.amount_minor),
            currency: Set(row.currency.clone()),
            tendered_minor: Set(row.tendered_minor),
            status: Set(PENDING.to_owned()),
            action: Set(None),
            expires_at: Set(current.payment_due_at),
            succeeded_at: Set(None),
            installment: Set(row.installment.clone()),
            created_at: Set(at),
            updated_at: Set(at),
        }
        .insert(&transaction)
        .await?;
        transaction.commit().await?;
        tracing::info!(order_id = %order_id, attempt, "payment attempt replaced after the previous one ended");
        Ok(())
    }

    /// `ledger` báo đã thu lượt `ledger_payment` của khoản `row` đã bị thay (`superseded`): không cộng
    /// vào đơn; ghi audit một lần rồi hoàn qua đối tác. Gọi lại (event giao lại, lần hoàn trước lỗi)
    /// thì chỉ gửi lại lệnh hoàn cùng mã tham chiếu — `ledger` không hoàn hai lần.
    pub(crate) async fn refund_superseded(
        &self,
        tenant: &TenantContext,
        row: &payment::Model,
        ledger_payment: Uuid,
        amount: Money,
    ) -> Result<(), Error> {
        let db = tenant.db();
        let transaction = db.begin_write().await?;
        crate::lifecycle::lock(&transaction, tenant, row.order_id).await?;
        let stored = payment::Entity::find_by_id(row.id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(crate::errors::order_not_found)?;
        if stored.succeeded_at.is_none() {
            // `succeeded_at` trên khoản `superseded`: đối tác đã thu lượt cũ (đánh dấu để chỉ ghi audit
            // một lần); trạng thái giữ `superseded` nên tiền này không tính vào đơn.
            let at = crate::service::now();
            let mut paid: payment::ActiveModel = stored.into();
            paid.succeeded_at = Set(Some(at));
            paid.updated_at = Set(at);
            paid.update(&transaction).await?;
            let event = PaymentRecorded(Change::after(json!({
                "order_id": row.order_id, "payment_id": row.id, "amount": amount,
                "late": true, "superseded": true,
            })));
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::system(),
                    Some(format!("orders.order:{}", row.order_id)),
                    &event,
                )
                .await?;
        }
        transaction.commit().await?;
        let Some(ledger) = &self.ledger else {
            return Ok(());
        };
        let refund =
            NewRefund::new(ledger_payment, amount, REFUND_REASON, superseded_reference(row.id));
        if let Err(error) = ledger.refund(tenant, Actor::system(), refund).await {
            tracing::warn!(order_id = %row.order_id, code = error.code(), "superseded payment not refunded yet");
            return Err(error);
        }
        Ok(())
    }
}

/// Phần đầu chung của mã tham chiếu mọi lần thử của một mục đích (`orders:<đơn>:<mục đích>:`).
fn attempt_prefix(order: Uuid, purpose: &str) -> String {
    format!("orders:{order}:{purpose}:")
}

/// Mã tham chiếu ổn định của lệnh hoàn lượt thu đã bị thay của khoản `payment`.
fn superseded_reference(payment: Uuid) -> String {
    format!("orders:payment:{payment}:superseded")
}
