//! Hoàn tiền của đơn — MỘT đường cho mọi nguồn: huỷ đơn đã thu, trả hàng, nhân viên hoàn tay
//! (`POST /{id}/refunds`, tool `refund_order`), tranh chấp của bước 46 (`OrderRefunds`).
//!
//! 1. Ghi lượt hoàn `pending` vào `orders_refunds` trong transaction có khoá dòng đơn: tổng các lượt
//!    chưa hỏng không vượt tiền đã thu của đơn, cũng không vượt phần còn lại của khoản được hoàn
//!    (mười lệnh hoàn song song không hoàn quá). Cùng `reference` + cùng nội dung → lượt cũ.
//! 2. Khoản thu qua cổng (`ledger`): gọi `Ledger::refund` (mã `orders:refund:<id>`). Cổng hoàn
//!    ngay → xong; cổng báo sau → chờ event `ledger.payment_refunded`. Khoản thu ngoài cổng (COD,
//!    tiền mặt): `manual` — nhân viên trả tiền ngoài hệ thống, lượt hoàn ghi nhận việc đó.
//! 3. Xong: cộng `refunded_minor` của đơn, phát `orders.refund_completed` — cùng transaction. Bút
//!    toán đảo do `accounting.rs` ghi khi nghe event này (D-20261119-step45-returns).
//!
//! Core không giữ tiền của người mua (D-187): tiền hoàn đi qua đối tác thu (`ledger`) hay do người
//! bán tự trả.

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::money::Money;
use autoxide_kernel::reference::{self, Reference, Submitted};
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_ledger::api::NewRefund;
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::entity::{order, payment, refund};
use crate::errors;
use crate::events::{Change, RefundCompleted};
use crate::payments;
use crate::service::OrdersService;

pub(crate) const PENDING: &str = "pending";
pub(crate) const SUCCEEDED: &str = "succeeded";
pub(crate) const FAILED: &str = "failed";
/// Tiền trả ngoài cổng (tiền mặt, chuyển khoản của người bán) — đã xong khi ghi.
pub(crate) const MANUAL: &str = "manual";
/// Event của `ledger` khi cổng đã hoàn xong (lượt hoàn báo sau).
pub(crate) const LEDGER_PAYMENT_REFUNDED: &str = "ledger.payment_refunded";

/// Mã tham chiếu tối đa 100 ký tự (giới hạn của kernel): dài hơn thì thay bằng băm của nó — cùng
/// nội dung luôn ra cùng mã.
pub(crate) fn short_reference(raw: &str) -> String {
    if raw.len() <= autoxide_kernel::reference::MAX_LENGTH {
        raw.to_owned()
    } else {
        format!("orders:{}", autoxide_kernel::secret::sha256(raw))
    }
}

/// Một lệnh hoàn.
#[derive(Debug, Clone)]
pub(crate) struct RefundRequest {
    pub order_id: Uuid,
    pub amount: Money,
    /// Chữ ngắn, không dữ liệu cá nhân (gửi cả sang cổng).
    pub reason: String,
    pub reference: String,
    pub return_id: Option<Uuid>,
    /// Hoàn đúng khoản này (huỷ đơn hoàn từng khoản); `None`: khoản còn nhiều nhất.
    pub payment_id: Option<Uuid>,
    pub created_by: Option<Uuid>,
}

impl OrdersService {
    /// Hoàn tiền theo `request` (xem đầu file). Trả lượt hoàn sau khi đã gửi cổng.
    pub(crate) async fn refund(
        &self,
        tenant: &TenantContext,
        request: RefundRequest,
    ) -> Result<Submitted<refund::Model>, Error> {
        let mut problems = Vec::new();
        let reference = Reference::checked(&request.reference, "reference", &mut problems);
        if !request.amount.is_positive() {
            problems.push(FieldError::new("amount", "range", "must be greater than zero"));
        }
        let reason = request.reason.trim().to_owned();
        if !(1..=200).contains(&reason.chars().count()) {
            problems.push(FieldError::new("reason", "length", "must be 1 to 200 characters long"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let Some(reference) = reference else {
            return Err(Error::internal("a checked reference went missing"));
        };
        let fingerprint = reference::fingerprint(&json!({
            "order": request.order_id, "amount": request.amount, "reason": reason,
            "return": request.return_id, "payment": request.payment_id,
        }))?;
        let database = tenant.database();
        let (reference, fingerprint, request, reason) =
            (&reference, &fingerprint, &request, &reason);
        let submitted = reference::submit_once(database, move || async move {
            let transaction = database.begin_write().await?;
            let stored = refund::Entity::find()
                .filter(refund::Column::Reference.eq(reference.as_str()))
                .scoped(tenant)
                .one(&transaction)
                .await?
                .map(|row| {
                    let fingerprint = row.reference_fingerprint.clone();
                    (row, fingerprint)
                });
            if let Some(existing) = reference::replayed(stored, reference, fingerprint)? {
                return Ok(Submitted::Replayed(existing));
            }
            // Khoá dòng đơn: các lệnh hoàn của cùng đơn chạy nối đuôi.
            let current = crate::lifecycle::lock(&transaction, tenant, request.order_id).await?;
            if request.amount.currency().code() != current.currency {
                return Err(Error::validation(vec![FieldError::new(
                    "amount",
                    "currency",
                    "must be in the currency of the order",
                )]));
            }
            let earlier = refund::Entity::find()
                .filter(refund::Column::OrderId.eq(current.id))
                .filter(refund::Column::Status.ne(FAILED))
                .scoped(tenant)
                .all(&transaction)
                .await?;
            let committed: i64 = earlier.iter().map(|row| row.amount_minor).sum();
            let left_on_order = current.paid_minor - committed;
            let collected = payment::Entity::find()
                .filter(payment::Column::OrderId.eq(current.id))
                .filter(payment::Column::Status.is_in(payments::COLLECTED))
                .scoped(tenant)
                .order_by_asc(payment::Column::Id)
                .all(&transaction)
                .await?;
            let left_of = |row: &payment::Model| {
                let used: i64 = earlier
                    .iter()
                    .filter(|refund| refund.payment_id == row.id)
                    .map(|refund| refund.amount_minor)
                    .sum();
                row.amount_minor - used
            };
            let chosen = match request.payment_id {
                Some(id) => collected.iter().find(|row| row.id == id),
                None => collected.iter().max_by_key(|row| left_of(row)),
            };
            let left = chosen.map_or(0, |row| left_of(row).min(left_on_order));
            let Some(chosen) = chosen.filter(|_| request.amount.minor() <= left) else {
                let remaining = Money::from_columns(left.max(0), &current.currency)?;
                return Err(errors::refund_exceeds_paid(remaining));
            };
            let at = crate::service::now();
            let saved = refund::Entity::insert(refund::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                order_id: Set(current.id),
                return_id: Set(request.return_id),
                payment_id: Set(chosen.id),
                reference: Set(reference.as_str().to_owned()),
                reference_fingerprint: Set(fingerprint.clone()),
                amount_minor: Set(request.amount.minor()),
                currency: Set(current.currency.clone()),
                status: Set(PENDING.to_owned()),
                ledger_refund_id: Set(None),
                reason: Set(reason.clone()),
                created_by: Set(request.created_by),
                created_at: Set(at),
                updated_at: Set(at),
            })
            .exec_with_returning(&transaction)
            .await?;
            transaction.commit().await?;
            Ok(Submitted::Created(saved))
        })
        .await?;
        let created = matches!(submitted, Submitted::Created(_));
        let row = self.send(tenant, submitted.into_inner()).await?;
        Ok(if created { Submitted::Created(row) } else { Submitted::Replayed(row) })
    }

    /// Gửi lượt hoàn `pending` chưa gửi: qua cổng nếu khoản thu qua cổng, không thì `manual`. Gửi
    /// lại (lệnh cùng `reference` sau lỗi mạng) gọi lại cổng với cùng mã — cổng không hoàn hai lần.
    async fn send(
        &self,
        tenant: &TenantContext,
        row: refund::Model,
    ) -> Result<refund::Model, Error> {
        if row.status != PENDING || row.ledger_refund_id.is_some() {
            return Ok(row);
        }
        let paid = payment::Entity::find_by_id(row.payment_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| Error::internal("the refunded payment vanished"))?;
        let amount = Money::from_columns(row.amount_minor, &row.currency)?;
        match (&self.ledger, paid.ledger_payment_id) {
            (Some(ledger), Some(ledger_payment)) => {
                let intent = NewRefund::new(
                    ledger_payment,
                    amount,
                    row.reason.clone(),
                    format!("orders:refund:{}", row.id),
                );
                let answer = match ledger.refund(tenant, Actor::system(), intent).await {
                    Ok(answer) => answer.into_inner(),
                    // Cổng / sổ từ chối hẳn (không hỗ trợ hoàn, vượt số còn lại): lượt hoàn hỏng,
                    // trả lại phần đã giữ chỗ. Lỗi tạm (5xx) giữ `pending` để gửi lại.
                    Err(error) if error.kind().status() < 500 => {
                        self.finish(tenant, row.id, FAILED, None).await?;
                        return Err(error);
                    }
                    Err(error) => return Err(error),
                };
                match answer.status.as_str() {
                    "succeeded" => self.finish(tenant, row.id, SUCCEEDED, Some(answer.id)).await?,
                    "failed" => self.finish(tenant, row.id, FAILED, Some(answer.id)).await?,
                    _ => self.note_ledger_refund(tenant, row.id, answer.id).await?,
                }
            }
            _ => self.finish(tenant, row.id, MANUAL, None).await?,
        }
        refund::Entity::find_by_id(row.id)
            .scoped(tenant)
            .one(tenant.database().write())
            .await?
            .ok_or_else(|| Error::internal("a refund vanished"))
    }

    async fn note_ledger_refund(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        ledger_refund: Uuid,
    ) -> Result<(), Error> {
        refund::Entity::update_many()
            .col_expr(refund::Column::LedgerRefundId, Expr::value(Some(ledger_refund)))
            .col_expr(refund::Column::UpdatedAt, Expr::value(crate::service::now()))
            .filter(refund::Column::Id.eq(id))
            .filter(refund::Column::LedgerRefundId.is_null())
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        Ok(())
    }

    /// Kết thúc lượt hoàn `id` (còn `pending`) với `status`. Thành công (`succeeded` / `manual`):
    /// cộng vào tiền đã hoàn của đơn và khoản, phát `orders.refund_completed` — một transaction.
    /// Gọi lại vô hại.
    async fn finish(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        status: &'static str,
        ledger_refund: Option<Uuid>,
    ) -> Result<(), Error> {
        let (database, outbox) = (tenant.database(), &self.outbox);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let Some(found) =
                    refund::Entity::find_by_id(id).scoped(tenant).one(&transaction).await?
                else {
                    return Ok(());
                };
                let current = crate::lifecycle::lock(&transaction, tenant, found.order_id).await?;
                let Some(row) =
                    rows::lock_one(&transaction, refund::Entity::find_by_id(id).scoped(tenant))
                        .await?
                else {
                    return Ok(());
                };
                if row.status != PENDING {
                    return Ok(());
                }
                let now = crate::service::now();
                let mut done: refund::ActiveModel = row.clone().into();
                done.status = Set(status.to_owned());
                if ledger_refund.is_some() {
                    done.ledger_refund_id = Set(ledger_refund);
                }
                done.updated_at = Set(now);
                done.update(&transaction).await?;
                if status == FAILED {
                    transaction.commit().await?;
                    return Ok(());
                }
                let refunded_on_payment: i64 = refund::Entity::find()
                    .filter(refund::Column::PaymentId.eq(row.payment_id))
                    .filter(refund::Column::Status.is_in([SUCCEEDED, MANUAL]))
                    .scoped(tenant)
                    .all(&transaction)
                    .await?
                    .iter()
                    .map(|refund| refund.amount_minor)
                    .sum();
                if let Some(paid) = payment::Entity::find_by_id(row.payment_id)
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                {
                    let state = if refunded_on_payment >= paid.amount_minor {
                        payments::REFUNDED
                    } else {
                        payments::REFUNDED_PARTIAL
                    };
                    let mut updated: payment::ActiveModel = paid.into();
                    updated.status = Set(state.to_owned());
                    updated.updated_at = Set(now);
                    updated.update(&transaction).await?;
                }
                let total = current.refunded_minor + row.amount_minor;
                let state =
                    if total >= current.paid_minor { "refunded" } else { "partially_refunded" };
                order::Entity::update_many()
                    .col_expr(order::Column::RefundedMinor, Expr::value(total))
                    .col_expr(order::Column::PaymentStatus, Expr::value(state))
                    .col_expr(order::Column::UpdatedAt, Expr::value(now))
                    .filter(order::Column::Id.eq(current.id))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
                let event = RefundCompleted(Change::after(json!({
                    "order_id": current.id, "tenant_id": current.tenant_id,
                    "org_id": current.org_id, "channel": current.channel, "refund_id": row.id,
                    "return_id": row.return_id, "payment_id": row.payment_id,
                    "amount": Money::from_columns(row.amount_minor, &row.currency)?,
                    "manual": status == MANUAL, "reason": row.reason,
                })));
                outbox
                    .record(
                        &transaction,
                        tenant,
                        Actor::system(),
                        Some(format!("orders.order:{}", current.id)),
                        &event,
                    )
                    .await?;
                transaction.commit().await?;
                Ok(())
            })
            .await
    }

    /// `ledger` báo đã hoàn xong lượt `ledger_refund` (lượt hoàn báo sau của cổng).
    pub(crate) async fn ledger_refunded(
        &self,
        tenant: &TenantContext,
        ledger_refund: Uuid,
    ) -> Result<(), Error> {
        let found = refund::Entity::find()
            .filter(refund::Column::LedgerRefundId.eq(ledger_refund))
            .scoped(tenant)
            .one(tenant.database().write())
            .await?;
        match found {
            Some(row) => self.finish(tenant, row.id, SUCCEEDED, None).await,
            None => Ok(()),
        }
    }

    /// Hoàn toàn bộ phần còn lại của mọi khoản đã thu của đơn đã huỷ (mã tham chiếu theo khoản:
    /// gọi lại không hoàn hai lần).
    pub(crate) async fn refund_cancelled(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<(), Error> {
        self.refund_cancelled_keeping(tenant, id, 0).await
    }

    /// Như [`refund_cancelled`](Self::refund_cancelled) nhưng giữ lại `keep` của khoản trả trước
    /// (D-232: người mua huỷ sau mốc chốt). Mã tham chiếu như nhau: gọi lại không hoàn hai lần.
    pub(crate) async fn refund_cancelled_keeping(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        keep: i64,
    ) -> Result<(), Error> {
        let database = tenant.database();
        let collected = payment::Entity::find()
            .filter(payment::Column::OrderId.eq(id))
            .filter(payment::Column::Status.is_in(payments::COLLECTED))
            .scoped(tenant)
            .all(database.read())
            .await?;
        for row in collected {
            let reference = format!("orders:{id}:cancel:{}", row.id);
            let earlier = refund::Entity::find()
                .filter(refund::Column::PaymentId.eq(row.id))
                .filter(refund::Column::Status.ne(FAILED))
                .scoped(tenant)
                .all(database.read())
                .await?;
            if let Some(same) = earlier.iter().find(|refund| refund.reference == reference) {
                // Lần gọi trước đã ghi: gửi lại nếu chưa gửi được.
                self.send(tenant, same.clone()).await?;
                continue;
            }
            let used: i64 = earlier.iter().map(|refund| refund.amount_minor).sum();
            let kept = if row.purpose == crate::advance::PURPOSE_ADVANCE { keep.max(0) } else { 0 };
            let left = row.amount_minor - used - kept;
            if left <= 0 {
                continue;
            }
            let request = RefundRequest {
                order_id: id,
                amount: Money::from_columns(left, &row.currency)?,
                reason: "order_cancelled".to_owned(),
                reference,
                return_id: None,
                payment_id: Some(row.id),
                created_by: None,
            };
            self.refund(tenant, request).await?;
        }
        Ok(())
    }
}

/// Nghe `ledger.payment_refunded` (chỉ khoản của `orders`).
pub(crate) struct LedgerRefunds(pub Arc<OrdersService>);

#[async_trait::async_trait]
impl EventHandler for LedgerRefunds {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let payload = &event.payload;
        if payload["source_module"].as_str() != Some("orders") {
            return Ok(());
        }
        let Some(id) = payload["refund_id"].as_str().and_then(|id| id.parse::<Uuid>().ok()) else {
            return Ok(());
        };
        self.0.ledger_refunded(&event.tenant, id).await
    }
}

#[async_trait::async_trait]
impl crate::api::OrderRefunds for OrdersService {
    async fn refund(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        amount: Money,
        reason: &str,
        reference: &str,
    ) -> Result<Submitted<Uuid>, Error> {
        let request = RefundRequest {
            order_id: id,
            amount,
            reason: reason.to_owned(),
            reference: reference.to_owned(),
            return_id: None,
            payment_id: None,
            created_by: None,
        };
        Ok(match OrdersService::refund(self, tenant, request).await? {
            Submitted::Created(row) => Submitted::Created(row.id),
            Submitted::Replayed(row) => Submitted::Replayed(row.id),
        })
    }
}

impl OrdersService {
    /// Nhân viên hoàn tay (`POST /{id}/refunds`, tool `refund_order`): cần `orders.refund` ở nút
    /// bán của đơn.
    pub(crate) async fn staff_refund(
        &self,
        tenant: &TenantContext,
        principal: &autoxide_kernel::access::Principal,
        order_id: Uuid,
        amount: Money,
        reason: String,
        reference: String,
    ) -> Result<Submitted<refund::Model>, Error> {
        principal.require(&crate::permissions::REFUND)?;
        let row = self.order(tenant, order_id).await?;
        let target = autoxide_kernel::access::OrgTarget::new(
            autoxide_kernel::tenancy::OrgId::from_uuid(row.org_id),
            None,
        );
        principal.require_at(tenant.database().read(), &crate::permissions::REFUND, target).await?;
        let request = RefundRequest {
            order_id,
            amount,
            reason,
            reference,
            return_id: None,
            payment_id: None,
            created_by: Some(principal.user_id()),
        };
        self.refund(tenant, request).await
    }

    /// Các lượt hoàn của đơn `order_id` (nhân viên có `orders.read` ở nút bán), cũ trước.
    pub(crate) async fn order_refunds(
        &self,
        tenant: &TenantContext,
        principal: &autoxide_kernel::access::Principal,
        order_id: Uuid,
    ) -> Result<Vec<refund::Model>, Error> {
        self.staff_order(tenant, principal, order_id).await?;
        Ok(refund::Entity::find()
            .filter(refund::Column::OrderId.eq(order_id))
            .scoped(tenant)
            .order_by_asc(refund::Column::Id)
            .all(tenant.database().read())
            .await?)
    }
}
