//! Trả một phần (hay cả) đơn bằng giá trị nội bộ — điểm thưởng, thẻ quà tặng, số dư — qua `Tender`
//! do module sau cắm vào (bước 47, D-20261120-step47-loyalty).
//!
//! - Lúc đặt hàng: sau khi giữ tồn và khuyến mại, giữ từng tender theo thứ tự người mua gửi với
//!   `reference = orders:<id>` và trần là phần còn phải trả; lỗi thì trả mọi chỗ đã giữ.
//! - Khi tạo đơn: mỗi tender là một khoản thanh toán `succeeded` phương thức `tender.<loại>`; khoản
//!   thu ở đối tác chỉ còn phần còn lại. Trả đủ bằng tender thì đơn vào thẳng `confirmed`.
//! - Xác nhận / trả chỗ giữ: bên cài nghe `orders.order_confirmed` / `orders.order_cancelled`.
//! - Event của đơn: `goods_paid` trừ phần trả bằng tender (cơ sở điểm D-226 không tính phần trả bằng
//!   điểm / thẻ).

use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::TenantContext;
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::api::hold_reference;
use crate::entity::payment;
use crate::payments;
use crate::service::OrdersService;

/// Tiền tố phương thức của khoản trả bằng tender.
pub(crate) const METHOD_PREFIX: &str = "tender.";

/// Một tender người mua chọn lúc đặt hàng.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct TenderRequest {
    /// `points`, `gift_card`, `store_credit`.
    pub kind: String,
    /// Dữ liệu riêng của loại, vd `{"points": 500}`.
    pub input: Value,
}

/// Một tender đã giữ được.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HeldTender {
    pub kind: String,
    pub amount_minor: i64,
}

fn unavailable(kind: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "orders.tender_unavailable",
        "The shop does not accept `{kind}` as a way to pay.",
    )
    .with_param("kind", kind)
}

impl OrdersService {
    /// Giữ các tender; trả các phần đã giữ (cả khi lỗi giữa chừng: phần đã giữ nằm trong `held`
    /// để bên gọi trả lại).
    #[expect(clippy::too_many_arguments, reason = "giữ tender cần chừng đó ngữ cảnh của lượt đặt")]
    pub(crate) async fn hold_tenders(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        buyer: Option<Uuid>,
        requests: &[TenderRequest],
        total: Money,
        until: DateTime<Utc>,
        held: &mut Vec<HeldTender>,
    ) -> Result<(), Error> {
        let mut problems = Vec::new();
        for (index, request) in requests.iter().enumerate() {
            if requests[..index].iter().any(|other| other.kind == request.kind) {
                problems.push(FieldError::new(
                    format!("tenders[{index}].kind"),
                    "duplicate",
                    "is given twice",
                ));
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let reference = hold_reference(order_id);
        let mut remaining = total;
        for request in requests {
            let tender =
                self.extensions.tender(&request.kind).ok_or_else(|| unavailable(&request.kind))?;
            if !remaining.is_positive() {
                break;
            }
            let amount =
                tender.hold(tenant, &reference, buyer, &request.input, remaining, until).await?;
            // Bên cài không được giữ quá trần; giữ 0 thì không ghi gì.
            let amount =
                Money::new(amount.minor().clamp(0, remaining.minor()), remaining.currency());
            held.push(HeldTender { kind: request.kind.clone(), amount_minor: amount.minor() });
            remaining = remaining.checked_sub(amount)?;
        }
        Ok(())
    }

    /// Trả mọi tender đã giữ của đơn (lỗi chỉ ghi log; hạn giữ của bên cài là lưới an toàn).
    pub(crate) async fn release_tenders(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        held: &[HeldTender],
    ) {
        let reference = hold_reference(order_id);
        for part in held {
            if let Some(tender) = self.extensions.tender(&part.kind)
                && let Err(error) = tender.release(tenant, &reference).await
            {
                tracing::warn!(order_id = %order_id, kind = %part.kind, code = error.code(), "tender hold not released");
            }
        }
    }

    /// Mọi tender cắm vào (trả lại khi không biết lượt đặt đã giữ tới đâu).
    pub(crate) fn all_tenders(&self) -> Vec<HeldTender> {
        self.extensions
            .tender_kinds()
            .into_iter()
            .map(|kind| HeldTender { kind: kind.to_owned(), amount_minor: 0 })
            .collect()
    }
}

/// Khoản thanh toán `succeeded` của một tender đã giữ.
pub(crate) fn payment_row(
    tenant: &TenantContext,
    order_id: Uuid,
    currency: &str,
    part: &HeldTender,
    at: DateTime<Utc>,
) -> payment::ActiveModel {
    payment::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        order_id: Set(order_id),
        purpose: Set(payments::PURPOSE_ORDER.to_owned()),
        method: Set(format!("{METHOD_PREFIX}{}", part.kind)),
        gateway: Set(None),
        ledger_payment_id: Set(None),
        external_id: Set(None),
        reference: Set(format!("{}:{}", hold_reference(order_id), part.kind)),
        amount_minor: Set(part.amount_minor),
        currency: Set(currency.to_owned()),
        tendered_minor: Set(None),
        status: Set(payments::SUCCEEDED.to_owned()),
        action: Set(None),
        expires_at: Set(None),
        succeeded_at: Set(Some(at)),
        installment: Set(None),
        created_at: Set(at),
        updated_at: Set(at),
    }
}

/// Tiền đơn đã trả bằng tender.
pub(crate) async fn tendered_minor<C: ConnectionTrait>(
    connection: &C,
    tenant_id: Uuid,
    order_id: Uuid,
) -> Result<i64, Error> {
    let rows = payment::Entity::find()
        .filter(payment::Column::TenantId.eq(tenant_id))
        .filter(payment::Column::OrderId.eq(order_id))
        .filter(payment::Column::Method.starts_with(METHOD_PREFIX))
        .filter(payment::Column::Status.eq(payments::SUCCEEDED))
        .all(connection)
        .await?;
    Ok(rows.iter().map(|row| row.amount_minor).sum())
}
