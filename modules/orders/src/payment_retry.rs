//! Người mua tự mở lại lượt thu (bước 70 P8): lần khởi tạo lúc đặt hỏng (vd MoMo thiếu
//! `return_url`) hay link đã hết hạn thì người mua — vãng lai bằng token xem đơn (`Order-Token`)
//! hoặc đã đăng nhập — tự mở lại, không phải nhờ nhân viên (`POST /{id}/payment-link`). Cùng một
//! logic với đường của nhân viên (`OrdersService::reopen_payments`): chỉ đơn còn chờ thanh toán
//! trước hạn, hay phần còn lại đã được yêu cầu.
//!
//! Chặn lạm dụng: mỗi đơn tối đa [`RETRY_PER_HOUR`] lần mở lại một giờ (mỗi lần có thể gọi đối tác
//! thanh toán). Gọi lại cùng `Idempotency-Key` không tính thêm lượt.

use autoxide_kernel::error::Error;
use autoxide_kernel::reference::fingerprint;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::throttle::{Outcome, Throttle};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{order, payment};
use crate::errors;
use crate::guests::GuestAccess;
use crate::service::OrdersService;

/// Số lần một đơn được người mua mở lại lượt thu trong một giờ.
pub(crate) const RETRY_PER_HOUR: u32 = 10;

/// Tên thao tác trong `orders_guest_requests` (chống trùng theo `Idempotency-Key`).
const ACTION: &str = "payment_link";

impl OrdersService {
    /// `POST /public/orders/{id}/payment-link`: người mua vãng lai mở lại lượt thu của đơn token mở.
    pub(crate) async fn guest_payment_link(
        &self,
        tenant: &TenantContext,
        access: &GuestAccess,
        id: Uuid,
        return_url: Option<String>,
        reference: &str,
    ) -> Result<Vec<payment::Model>, Error> {
        let row = self.guest_order(tenant, access, id).await?;
        self.buyer_reopen(tenant, &row, return_url, reference).await
    }

    /// `POST /me/orders/{id}/payment-link`: người mua đăng nhập, đơn của chính mình.
    pub(crate) async fn my_payment_link(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        id: Uuid,
        return_url: Option<String>,
        reference: &str,
    ) -> Result<Vec<payment::Model>, Error> {
        let row = self.order(tenant, id).await?;
        if row.buyer_user_id != Some(buyer) {
            return Err(errors::order_not_found());
        }
        self.buyer_reopen(tenant, &row, return_url, reference).await
    }

    async fn buyer_reopen(
        &self,
        tenant: &TenantContext,
        row: &order::Model,
        return_url: Option<String>,
        reference: &str,
    ) -> Result<Vec<payment::Model>, Error> {
        let print = fingerprint(&json!({ "return_url": return_url }))?;
        if self.guest_replayed(tenant, row.id, ACTION, reference, &print).await? {
            return self.pending_payments(tenant, row.id).await;
        }
        let hour = std::time::Duration::from_secs(3600);
        let limit = Throttle::new("orders", RETRY_PER_HOUR, hour)?;
        if let Outcome::Limited { retry_after } =
            limit.hit(tenant, &format!("payment-link:{}", row.id)).await?
        {
            return Err(errors::payment_retry_too_many(retry_after));
        }
        let payments = self.reopen_payments(tenant, row, return_url).await?;
        Self::remember_guest_request(
            tenant.database().write(),
            tenant,
            row.id,
            ACTION,
            reference,
            &print,
        )
        .await?;
        Ok(payments)
    }
}
