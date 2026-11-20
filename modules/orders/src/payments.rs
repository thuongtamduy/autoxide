//! Thanh toán của đơn (mục 3): lượt thu đi qua `ledger` (cổng thanh toán có giấy phép hay VietQR
//! về tài khoản của người bán — D-187, D-216); `orders` không giữ tiền. Hạn thanh toán là
//! `expires_at` của `ledger` (D-218). `ledger.payment_succeeded` → đơn `confirmed`; thanh toán tới
//! sau khi đơn đã huỷ → hoàn tự động (`late_payment`). Job `orders.expire_unpaid` huỷ đơn quá hạn.

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::money::Money;
use autoxide_kernel::secret::sha256;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, TenantId};
use autoxide_ledger::api::{FlowMode, NewPayment, NewRefund, NewTransferPayment};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

use crate::api::{FundsFlow, OrderStatus};
use crate::entity::{order, payment};
use crate::errors;
use crate::events::{Change, PaymentRecorded};
use crate::lifecycle::{self, Applied, Cause, Transition};
use crate::service::OrdersService;

pub(crate) const PURPOSE_ORDER: &str = "order";
pub(crate) const PENDING: &str = "pending";
pub(crate) const SUCCEEDED: &str = "succeeded";
pub(crate) const REFUNDED: &str = "refunded";
pub(crate) const REFUNDED_PARTIAL: &str = "refunded_partial";
/// Khoản đã thu (kể cả đã hoàn một phần / toàn bộ).
pub(crate) const COLLECTED: [&str; 3] = [SUCCEEDED, REFUNDED_PARTIAL, REFUNDED];
pub(crate) const UNPAID: &str = "unpaid";
pub(crate) const COD_PENDING: &str = "cod_pending";
pub(crate) const PAID: &str = "paid";
/// COD thu nhiều lần (giao nhiều lần): đã thu một phần.
pub(crate) const PARTIALLY_PAID: &str = "partially_paid";
/// Event của `ledger` khi đối tác báo đã thu.
pub(crate) const LEDGER_PAYMENT_SUCCEEDED: &str = "ledger.payment_succeeded";

/// Cách thu của một đơn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    /// Thu khi giao (hãng thu hộ).
    Cod,
    /// Chuyển khoản theo mã VietQR tới tài khoản của người bán (`ledger` khớp biến động số dư).
    Transfer,
    /// Qua cổng thanh toán (`card`, `wallet`… — phương thức của cổng).
    Gateway,
}

impl Method {
    pub(crate) fn parse(raw: &str) -> Method {
        match raw {
            "cod" => Method::Cod,
            "bank_transfer" | "vietqr_transfer" => Method::Transfer,
            _ => Method::Gateway,
        }
    }
}

pub(crate) fn funds_flow(mode: FlowMode) -> FundsFlow {
    match mode {
        FlowMode::SelfCollect => FundsFlow::SelfCollect,
        FlowMode::PartnerCollect => FundsFlow::PartnerCollect,
        FlowMode::SplitAtGateway => FundsFlow::SplitAtGateway,
    }
}

/// Mã tham chiếu của khoản thu gửi `ledger`: `orders:<order>:<purpose>:<lần>`.
pub(crate) fn reference(order: Uuid, purpose: &str, attempt: u32) -> String {
    format!("orders:{order}:{purpose}:{attempt}")
}

/// Việc người mua phải làm để trả một khoản (`payment_actions[]`, D-216).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Action {
    /// `redirect`, `qr`, `cod`, `none`.
    pub kind: String,
    pub redirect_url: Option<String>,
    pub qr_payload: Option<String>,
    pub transfer_code: Option<String>,
}

impl OrdersService {
    /// Tạo lượt thu ở `ledger` cho khoản `pending` của đơn; ghi hạn thật của `ledger` làm
    /// `payment_due_at` và việc người mua phải làm. Gọi lại khi đã có lượt thu: không làm gì.
    pub(crate) async fn start_payment(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        return_url: Option<String>,
    ) -> Result<(), Error> {
        let ledger =
            self.ledger.clone().ok_or_else(|| errors::payment_method_unavailable("gateway"))?;
        let order = self.order(tenant, order_id).await?;
        let row = payment::Entity::find()
            .filter(payment::Column::OrderId.eq(order_id))
            .filter(payment::Column::Status.eq(PENDING))
            .scoped(tenant)
            .order_by_desc(payment::Column::Id)
            .one(tenant.database().read())
            .await?
            .ok_or_else(errors::order_not_found)?;
        if row.ledger_payment_id.is_some() {
            return Ok(());
        }
        let amount = Money::from_columns(row.amount_minor, &row.currency)?;
        let org = OrgId::from_uuid(order.org_id);
        let description = format!("Order {}", order.number);
        let created =
            match Method::parse(&row.method) {
                Method::Transfer => {
                    let request = NewTransferPayment::new(
                        org,
                        amount,
                        description,
                        "orders",
                        row.reference.clone(),
                    )
                    .source_id(order_id.to_string());
                    ledger.create_transfer_payment(tenant, Actor::system(), request).await
                }
                Method::Gateway => {
                    let mut request = NewPayment::new(
                        org,
                        row.method.clone(),
                        amount,
                        description,
                        "orders",
                        row.reference.clone(),
                    )
                    .source_id(order_id.to_string());
                    // Trả góp: chọn đúng bộ nối của đối tác người mua chọn.
                    if let Some(plan) = row.installment.as_deref().and_then(|raw| {
                        serde_json::from_str::<crate::checkout::Installment>(raw).ok()
                    }) {
                        request.provider = Some(plan.partner);
                    }
                    if let Some(url) = return_url {
                        request = request.return_url(url);
                    }
                    ledger.create_payment(tenant, Actor::system(), request).await
                }
                Method::Cod => return Ok(()),
            }
            .map_err(|error| {
                if error.code() == "validation_failed" {
                    errors::payment_method_unavailable(&row.method)
                } else {
                    error
                }
            })?
            .into_inner();
        let action = Action {
            kind: if created.checkout_url.is_some() { "redirect" } else { "qr" }.to_owned(),
            redirect_url: created.checkout_url.clone(),
            qr_payload: created.qr_payload.clone(),
            transfer_code: created.transfer_code.clone(),
        };
        let mut updated: payment::ActiveModel = row.into();
        updated.ledger_payment_id = Set(Some(created.id));
        updated.gateway = Set(Some(created.provider.clone()));
        updated.action = Set(Some(serde_json::to_string(&action).map_err(Error::internal)?));
        updated.expires_at = Set(Some(created.expires_at));
        updated.updated_at = Set(crate::service::now());
        let database = tenant.database();
        let transaction = database.begin_write().await?;
        updated.update(&transaction).await?;
        order::Entity::update_many()
            .col_expr(
                order::Column::PaymentDueAt,
                sea_orm::sea_query::Expr::value(created.expires_at),
            )
            .filter(order::Column::Id.eq(order_id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Một đơn của khách, hoặc 404.
    pub(crate) async fn order(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<order::Model, Error> {
        order::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(errors::order_not_found)
    }

    /// `ledger` báo đã thu khoản `ledger_payment` của đơn: đơn `pending_payment` → `confirmed`. Gọi
    /// lại (event giao hai lần) không ghi thêm. Đơn đã huỷ → hoàn tự động.
    pub(crate) async fn payment_succeeded(
        &self,
        tenant: &TenantContext,
        ledger_payment: Uuid,
        amount: Money,
    ) -> Result<(), Error> {
        let Some(row) = payment::Entity::find()
            .filter(payment::Column::LedgerPaymentId.eq(ledger_payment))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
        else {
            return Ok(());
        };
        let (database, outbox) = (tenant.database(), &self.outbox);
        let payment_id = row.id;
        let late = database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let current = lifecycle::lock(&transaction, tenant, row.order_id).await?;
                let stored = payment::Entity::find_by_id(payment_id)
                    .one(&transaction)
                    .await?
                    .ok_or_else(errors::order_not_found)?;
                if stored.status == SUCCEEDED {
                    return Ok(false);
                }
                let now = crate::service::now();
                let purpose = stored.purpose.clone();
                let mut paid: payment::ActiveModel = stored.into();
                paid.status = Set(SUCCEEDED.to_owned());
                paid.succeeded_at = Set(Some(now));
                paid.updated_at = Set(now);
                paid.update(&transaction).await?;
                let late = current.status == OrderStatus::Cancelled.as_str();
                let paid_minor = current.paid_minor + amount.minor();
                let order_id = current.id;
                // Trả trước một phần (D-232): khoản lúc đặt → `partially_paid`; phần còn lại →
                // `paid`, đơn đã xác nhận từ trước nên không chuyển trạng thái.
                let balance = purpose == crate::advance::PURPOSE_BALANCE;
                let status_after =
                    if purpose == crate::advance::PURPOSE_ADVANCE { PARTIALLY_PAID } else { PAID };
                if !late && let Some(next) = crate::advance_flow::status_after_payment(&purpose) {
                    crate::entity::advance_plan::Entity::update_many()
                        .col_expr(
                            crate::entity::advance_plan::Column::Status,
                            sea_orm::sea_query::Expr::value(next),
                        )
                        .col_expr(
                            crate::entity::advance_plan::Column::UpdatedAt,
                            sea_orm::sea_query::Expr::value(now),
                        )
                        .filter(crate::entity::advance_plan::Column::OrderId.eq(current.id))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                }
                if late || balance {
                    // Không mở lại đơn đã huỷ: ghi đã thu, hoàn bằng job.
                    order::Entity::update_many()
                        .col_expr(
                            order::Column::PaidMinor,
                            sea_orm::sea_query::Expr::value(paid_minor),
                        )
                        .col_expr(
                            order::Column::PaymentStatus,
                            sea_orm::sea_query::Expr::value(if late {
                                current.payment_status.clone()
                            } else {
                                PAID.to_owned()
                            }),
                        )
                        .filter(order::Column::Id.eq(order_id))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                } else {
                    let transition =
                        Transition::new(OrderStatus::Confirmed, Cause::Payment, Actor::system());
                    let applied = lifecycle::apply(
                        &transaction,
                        tenant,
                        outbox,
                        current,
                        &transition,
                        |row| {
                            row.paid_minor = Set(paid_minor);
                            row.payment_status = Set(status_after.to_owned());
                        },
                    )
                    .await?;
                    if let Applied::Already(order) = applied {
                        order::Entity::update_many()
                            .col_expr(
                                order::Column::PaidMinor,
                                sea_orm::sea_query::Expr::value(paid_minor),
                            )
                            .filter(order::Column::Id.eq(order.id))
                            .scoped(tenant)
                            .exec(&transaction)
                            .await?;
                    }
                }
                let event = PaymentRecorded(Change::after(json!({
                    "order_id": order_id, "payment_id": payment_id, "amount": amount, "late": late,
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
                Ok(late)
            })
            .await?;
        if late && let Some(ledger) = &self.ledger {
            let refund = NewRefund::new(
                ledger_payment,
                amount,
                "late_payment",
                format!("orders:{}:late", row.order_id),
            );
            if let Err(error) = ledger.refund(tenant, Actor::system(), refund).await {
                tracing::warn!(order_id = %row.order_id, code = error.code(), "late payment not refunded yet");
                return Err(error);
            }
        }
        Ok(())
    }

    /// Một lô đơn `pending_payment` quá `payment_due_at` trong `database` (mọi khách) → `cancelled`;
    /// bên giữ chỗ trả lại khi nghe `orders.order_cancelled`. Trả số đơn đã huỷ.
    pub(crate) async fn expire_unpaid(
        &self,
        database: &autoxide_kernel::database::Database,
    ) -> Result<u64, Error> {
        let due: Vec<(Uuid, Uuid)> = order::Entity::find()
            .select_only()
            .column(order::Column::TenantId)
            .column(order::Column::Id)
            .filter(order::Column::Status.eq(OrderStatus::PendingPayment.as_str()))
            .filter(order::Column::PaymentDueAt.lt(crate::service::now()))
            .limit(crate::jobs::BATCH)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut cancelled = 0;
        for (tenant_id, id) in due {
            let tenant = self.databases.tenant(TenantId::from_uuid(tenant_id)).await?;
            let transition =
                Transition::new(OrderStatus::Cancelled, Cause::System, Actor::system())
                    .reason("payment_expired");
            match self.transition(&tenant, id, &transition, Some(OrderStatus::PendingPayment)).await
            {
                Ok(Applied::Changed) => cancelled += 1,
                Ok(Applied::Already(_)) => {}
                Err(error) => {
                    tracing::warn!(order_id = %id, code = error.code(), "unpaid order not expired")
                }
            }
        }
        Ok(cancelled)
    }

    /// Chuyển đơn `id` (khoá dòng, một transaction). `only_from`: chỉ khi đơn đang ở trạng thái này
    /// (job hết hạn không huỷ đơn vừa được thanh toán).
    pub(crate) async fn transition(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        transition: &Transition,
        only_from: Option<OrderStatus>,
    ) -> Result<Applied, Error> {
        let check = move |current: &order::Model| -> Result<bool, Error> {
            Ok(only_from.is_none_or(|from| from.as_str() == current.status))
        };
        self.transition_if(tenant, id, transition, &check).await
    }

    /// Chuyển đơn `id` khi `check` (chạy trên dòng đã khoá) trả `true`; `false`: không làm gì,
    /// lỗi: dừng với lỗi đó (chính sách huỷ, `If-Match`…).
    pub(crate) async fn transition_if(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        transition: &Transition,
        check: &(dyn Fn(&order::Model) -> Result<bool, Error> + Send + Sync),
    ) -> Result<Applied, Error> {
        let (database, outbox) = (tenant.database(), &self.outbox);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let current = lifecycle::lock(&transaction, tenant, id).await?;
                if !check(&current)? {
                    return Ok(Applied::Already(Box::new(current)));
                }
                let reason = transition.reason.clone();
                let applied =
                    lifecycle::apply(&transaction, tenant, outbox, current, transition, |row| {
                        if transition.to == OrderStatus::Cancelled {
                            row.cancel_reason = Set(reason);
                        }
                    })
                    .await?;
                transaction.commit().await?;
                Ok(applied)
            })
            .await
    }

    /// Token xem trạng thái công khai của khoản `payment_id` (màn QR, D-201): `<id>.<HMAC>` bằng khoá
    /// riêng của khách — không lưu, tính lại được khi gửi lại kết quả đặt hàng.
    pub(crate) async fn status_token(
        &self,
        tenant: &TenantContext,
        payment_id: Uuid,
    ) -> Result<String, Error> {
        let pepper = self.secret.derive_for(tenant.id(), "orders.payment_status").await?;
        let pepper: String = pepper.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(format!("{payment_id}.{}", sha256(&format!("{pepper}:{payment_id}"))))
    }

    /// Trạng thái thanh toán công khai theo token của màn QR (D-201); token sống bằng hạn thanh toán.
    pub(crate) async fn public_payment(
        &self,
        tenant: &TenantContext,
        token: &str,
    ) -> Result<(payment::Model, order::Model), Error> {
        let token = token.trim();
        let id = token
            .split_once('.')
            .and_then(|(id, _)| id.parse::<Uuid>().ok())
            .ok_or_else(errors::payment_token_invalid)?;
        if self.status_token(tenant, id).await? != token {
            return Err(errors::payment_token_invalid());
        }
        let row = payment::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .filter(|row| {
                row.status != PENDING || row.expires_at.is_none_or(|at| at > crate::service::now())
            })
            .ok_or_else(errors::payment_token_invalid)?;
        let order = self.order(tenant, row.order_id).await?;
        Ok((row, order))
    }
}

/// Nghe `ledger.payment_succeeded` (chỉ khoản của `orders`).
pub(crate) struct LedgerEvents(pub Arc<OrdersService>);

#[async_trait::async_trait]
impl EventHandler for LedgerEvents {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let payload = &event.payload;
        if payload["source_module"].as_str() != Some("orders") {
            return Ok(());
        }
        let Some(id) = payload["payment_id"].as_str().and_then(|id| id.parse::<Uuid>().ok()) else {
            return Ok(());
        };
        let Ok(amount) = serde_json::from_value::<Money>(payload["amount"].clone()) else {
            tracing::warn!(event_id = %event.id, "ledger.payment_succeeded without an amount; ignored");
            return Ok(());
        };
        self.0.payment_succeeded(&event.tenant, id, amount).await
    }
}

/// Việc người mua cho khoản `row` (đọc từ `action`, không có thì theo phương thức).
pub(crate) fn action_of(row: &payment::Model) -> Value {
    row.action.as_deref().and_then(|raw| serde_json::from_str(raw).ok()).unwrap_or_else(|| {
        match Method::parse(&row.method) {
            Method::Cod => json!({"kind": "cod"}),
            _ => json!({"kind": "none"}),
        }
    })
}
