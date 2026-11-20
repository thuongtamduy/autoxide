//! Chính sách đơn theo nút (D-209, D-215): cài đặt kế thừa của bước 34 — đặt ở chuỗi, cửa hàng ghi
//! đè, đọc qua `dyn OrgDirectory` của `organizations`; chưa cài module đó thì dùng mặc định. Thay
//! bảng `orders_policies` của đặc tả bằng khuôn cài đặt kế thừa sẵn có
//! (D-20261118-step45-lifecycle).

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, OrgSetting, TenantContext};
use serde_json::Value;

use crate::api::OrderStatus;
use crate::service::OrdersService;

/// Người mua tự huỷ được tới trạng thái nào (D-209).
pub(crate) const BUYER_CANCEL_UNTIL: OrgSetting = OrgSetting::new(
    "orders.buyer_cancel_until",
    "Up to which order status buyers may cancel by themselves: `pending_payment`, `confirmed` \
     or `preparing`.",
)
.with_default("\"confirmed\"")
.with_schema(r#"{"type":"string","enum":["pending_payment","confirmed","preparing"]}"#)
.checked(|value| match value.as_str() {
    Some("pending_payment" | "confirmed" | "preparing") => Ok(value.clone()),
    _ => Err("must be `pending_payment`, `confirmed` or `preparing`".to_owned()),
});

/// Số giờ cửa hàng có để nhận đơn đã thanh toán / COD; 0: tự nhận (D-209).
pub(crate) const ACCEPTANCE_HOURS: OrgSetting = OrgSetting::new(
    "orders.acceptance_hours",
    "Hours a shop has to accept a paid or cash-on-delivery order before it is cancelled and \
     refunded (0 to 720; 0: no acceptance step).",
)
.with_default("0")
.with_schema(r#"{"type":"integer","minimum":0,"maximum":720}"#)
.checked(|value| match value.as_i64() {
    Some(hours) if (0..=720).contains(&hours) => Ok(value.clone()),
    _ => Err("must be a whole number from 0 to 720".to_owned()),
});

/// Hạn đổi trả, ngày sau khi giao (đơn hoàn tất khi hết hạn).
pub(crate) const RETURN_WINDOW_DAYS: OrgSetting = OrgSetting::new(
    "orders.return_window_days",
    "Days after delivery during which buyers may ask to return items; the order completes \
     when they end (0 to 365).",
)
.with_default("7")
.with_schema(r#"{"type":"integer","minimum":0,"maximum":365}"#)
.checked(|value| match value.as_i64() {
    Some(days) if (0..=365).contains(&days) => Ok(value.clone()),
    _ => Err("must be a whole number from 0 to 365".to_owned()),
});

/// Nhận thanh toán khi giao hàng.
pub(crate) const COD_ENABLED: OrgSetting =
    OrgSetting::new("orders.cod_enabled", "Whether buyers may pay cash on delivery.")
        .with_default("true")
        .with_schema(r#"{"type":"boolean"}"#)
        .checked(|value| match value.as_bool() {
            Some(_) => Ok(value.clone()),
            None => Err("must be `true` or `false`".to_owned()),
        });

/// Mã lý do trả hàng người mua được chọn (D-209), lấy từ danh sách `orders.return_reason`.
pub(crate) const ALLOWED_RETURN_REASONS: OrgSetting = OrgSetting::new(
    "orders.allowed_return_reasons",
    "Codes of the `orders.return_reason` list buyers may give when asking to return items.",
)
.with_default(
    r#"["changed_mind","damaged","wrong_item","not_as_described","missing_parts","other"]"#,
)
.with_schema(
    r#"{"type":"array","items":{"type":"string","pattern":"^[a-z][a-z0-9_]{0,63}$"},"maxItems":50}"#,
)
.checked(|value| match value.as_array() {
    Some(codes)
        if codes.len() <= 50
            && codes.iter().all(|code| code.as_str().is_some_and(crate::policy::is_code)) =>
    {
        Ok(value.clone())
    }
    _ => Err("must be a list of at most 50 lower_case reason codes".to_owned()),
});

/// Mã lý do trả hàng bắt buộc kèm ảnh (D-202).
pub(crate) const RETURN_PHOTO_REASONS: OrgSetting = OrgSetting::new(
    "orders.return_photo_reasons",
    "Codes of the `orders.return_reason` list for which buyers must send photos of the items.",
)
.with_default(r#"["damaged","wrong_item"]"#)
.with_schema(
    r#"{"type":"array","items":{"type":"string","pattern":"^[a-z][a-z0-9_]{0,63}$"},"maxItems":50}"#,
)
.checked(|value| match value.as_array() {
    Some(codes)
        if codes.len() <= 50
            && codes.iter().all(|code| code.as_str().is_some_and(crate::policy::is_code)) =>
    {
        Ok(value.clone())
    }
    _ => Err("must be a list of at most 50 lower_case reason codes".to_owned()),
});

/// Hết hạn giữ hàng chờ nhận tại cửa hàng thì làm gì (D-201).
pub(crate) const PICKUP_EXPIRED_ACTION: OrgSetting = OrgSetting::new(
    "orders.pickup_expired_action",
    "What happens when an order waiting in store is not collected in time: `cancel_refund` \
     (cancel, restock and refund) or `keep_waiting`.",
)
.with_default("\"cancel_refund\"")
.with_schema(r#"{"type":"string","enum":["cancel_refund","keep_waiting"]}"#)
.checked(|value| match value.as_str() {
    Some("cancel_refund" | "keep_waiting") => Ok(value.clone()),
    _ => Err("must be `cancel_refund` or `keep_waiting`".to_owned()),
});

/// Khi nào lập hoá đơn điện tử của đơn (D-215): mặc định khi giao xong — NĐ 123/2020 sửa bởi NĐ
/// 70/2025, Điều 9 (thời điểm chuyển giao quyền sở hữu; cần đối chiếu văn bản gốc).
pub(crate) const INVOICE_TIMING: OrgSetting = OrgSetting::new(
    "orders.invoice_timing",
    "When the electronic invoice of an order is issued: `on_confirm`, `on_delivered` (default: \
     when ownership passes to the buyer), `on_complete` or `on_request` (only when the buyer or \
     staff asks).",
)
.with_default("\"on_delivered\"")
.with_schema(r#"{"type":"string","enum":["on_confirm","on_delivered","on_complete","on_request"]}"#)
.checked(|value| match value.as_str() {
    Some("on_confirm" | "on_delivered" | "on_complete" | "on_request") => Ok(value.clone()),
    _ => Err("must be `on_confirm`, `on_delivered`, `on_complete` or `on_request`".to_owned()),
});

/// Loại hoá đơn của đơn bán online (theo phương pháp tính thuế của người bán).
pub(crate) const INVOICE_KIND: OrgSetting = OrgSetting::new(
    "orders.invoice_kind",
    "Kind of electronic invoice for orders: `vat_invoice` (sellers using the credit method) or \
     `sales_invoice` (direct method). Sales at the counter use `cash_register_invoice`.",
)
.with_default("\"vat_invoice\"")
.with_schema(r#"{"type":"string","enum":["vat_invoice","sales_invoice"]}"#)
.checked(|value| match value.as_str() {
    Some("vat_invoice" | "sales_invoice") => Ok(value.clone()),
    _ => Err("must be `vat_invoice` or `sales_invoice`".to_owned()),
});

/// Chênh lệch tiền két (đơn vị nhỏ nhất, giá trị tuyệt đối) quá mức này thì đóng ca cần người duyệt
/// (D-227).
pub(crate) const CASH_VARIANCE_APPROVAL_MINOR: OrgSetting = OrgSetting::new(
    "orders.cash_variance_approval_minor",
    "Cash difference at shift close (in minor units, either way) above which a second person \
     must approve the close (0 to 1000000000000).",
)
.with_default("200000")
.with_schema(r#"{"type":"integer","minimum":0,"maximum":1000000000000}"#)
.checked(|value| match value.as_i64() {
    Some(amount) if (0..=1_000_000_000_000).contains(&amount) => Ok(value.clone()),
    _ => Err("must be a whole number from 0 to 1000000000000".to_owned()),
});

/// Mã lý do dạng `snake_case` (chữ thường, số, `_`; tối đa 64 ký tự).
pub(crate) fn is_code(code: &str) -> bool {
    code.len() <= 64
        && code.starts_with(|first: char| first.is_ascii_lowercase())
        && code.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

pub(crate) fn all() -> Vec<OrgSetting> {
    vec![
        BUYER_CANCEL_UNTIL,
        ACCEPTANCE_HOURS,
        RETURN_WINDOW_DAYS,
        COD_ENABLED,
        ALLOWED_RETURN_REASONS,
        RETURN_PHOTO_REASONS,
        PICKUP_EXPIRED_ACTION,
        INVOICE_TIMING,
        INVOICE_KIND,
        CASH_VARIANCE_APPROVAL_MINOR,
    ]
}

/// Chính sách hiệu lực của một nút.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Policy {
    pub buyer_cancel_until: OrderStatus,
    pub acceptance_hours: i64,
    pub return_window_days: i64,
    pub cod_enabled: bool,
    pub allowed_return_reasons: Vec<String>,
    pub return_photo_reasons: Vec<String>,
    /// `true`: hết hạn giữ hàng thì huỷ và hoàn tiền.
    pub pickup_cancel_on_expiry: bool,
}

impl Policy {
    /// Người mua tự huỷ được đơn đang ở `status` không.
    pub(crate) fn buyer_may_cancel(&self, status: OrderStatus) -> bool {
        let rank = |status: OrderStatus| match status {
            OrderStatus::PendingPayment => Some(0),
            OrderStatus::Confirmed => Some(1),
            OrderStatus::Preparing => Some(2),
            _ => None,
        };
        match (rank(status), rank(self.buyer_cancel_until)) {
            (Some(now), Some(until)) => now <= until,
            _ => false,
        }
    }
}

impl OrdersService {
    pub(crate) async fn setting(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        setting: &OrgSetting,
    ) -> Result<Option<Value>, Error> {
        let found = match &self.orgs {
            Some(orgs) => {
                orgs.setting(tenant, org, setting.key()).await?.and_then(|found| found.value)
            }
            None => None,
        };
        Ok(found.or_else(|| setting.default_value()))
    }

    /// Chính sách hiệu lực của nút `org`.
    pub(crate) async fn policy(&self, tenant: &TenantContext, org: OrgId) -> Result<Policy, Error> {
        let cancel = self.setting(tenant, org, &BUYER_CANCEL_UNTIL).await?;
        let acceptance = self.setting(tenant, org, &ACCEPTANCE_HOURS).await?;
        let returns = self.setting(tenant, org, &RETURN_WINDOW_DAYS).await?;
        let cod = self.setting(tenant, org, &COD_ENABLED).await?;
        let reasons = self.setting(tenant, org, &ALLOWED_RETURN_REASONS).await?;
        let photos = self.setting(tenant, org, &RETURN_PHOTO_REASONS).await?;
        let pickup = self.setting(tenant, org, &PICKUP_EXPIRED_ACTION).await?;
        let codes = |value: Option<Value>| -> Vec<String> {
            value
                .as_ref()
                .and_then(Value::as_array)
                .map(|codes| codes.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default()
        };
        Ok(Policy {
            buyer_cancel_until: cancel
                .as_ref()
                .and_then(Value::as_str)
                .and_then(OrderStatus::parse)
                .unwrap_or(OrderStatus::Confirmed),
            acceptance_hours: acceptance.as_ref().and_then(Value::as_i64).unwrap_or(0),
            return_window_days: returns.as_ref().and_then(Value::as_i64).unwrap_or(7),
            cod_enabled: cod.as_ref().and_then(Value::as_bool).unwrap_or(true),
            allowed_return_reasons: codes(reasons),
            return_photo_reasons: codes(photos),
            pickup_cancel_on_expiry: pickup.as_ref().and_then(Value::as_str)
                != Some("keep_waiting"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nguoi_mua_huy_toi_trang_thai_cua_chinh_sach() {
        let policy = |until| Policy {
            buyer_cancel_until: until,
            acceptance_hours: 0,
            return_window_days: 7,
            cod_enabled: true,
            allowed_return_reasons: Vec::new(),
            return_photo_reasons: Vec::new(),
            pickup_cancel_on_expiry: true,
        };
        let confirmed = policy(OrderStatus::Confirmed);
        assert!(confirmed.buyer_may_cancel(OrderStatus::PendingPayment));
        assert!(confirmed.buyer_may_cancel(OrderStatus::Confirmed));
        assert!(!confirmed.buyer_may_cancel(OrderStatus::Preparing));
        assert!(!confirmed.buyer_may_cancel(OrderStatus::Shipping));
        assert!(policy(OrderStatus::Preparing).buyer_may_cancel(OrderStatus::Preparing));
        assert!(!policy(OrderStatus::PendingPayment).buyer_may_cancel(OrderStatus::Confirmed));
    }
}
