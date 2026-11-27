//! Hoàn tất một thanh toán giả trong sandbox (UT-4, D-20261127-sandbox-payment-simulation).
//!
//! Trong sandbox, kernel bọc mọi `PaymentGateway`: lượt thu được "tạo" ở bộ giả (mã `SANDBOX-…`), không
//! tới đối tác thật, nên không đối tác nào gửi webhook về và lượt thu đứng `pending`. Lệnh này đóng vai
//! đối tác: dựng đúng một `GatewayNotification` như bộ nối dựng từ webhook, lưu và xử lý qua đúng đường
//! của webhook ([`LedgerService::record_notification`] → job `ProcessGatewayEvent` → ghi thu / hỏng, bút
//! toán, event `ledger.payment_succeeded` cho module nguồn). Chỉ bỏ bước kiểm chữ ký — không có đối tác
//! nào ký.
//!
//! D-187 (core không giữ tiền): không có tiền thật nào; lệnh chỉ trả lời thay đối tác cho lượt thu chưa
//! từng tới đối tác thật. Bốn điều kiện, thiếu một là từ chối:
//!
//! 1. Người vận hành bật `AUTOXIDE_LEDGER_SANDBOX_SIMULATION` (mặc định tắt).
//! 2. Khách hiện tại là sandbox (chế độ một khách không có sandbox nên không bao giờ dùng được).
//! 3. Lượt thu được tạo khi khách đã là sandbox (cờ `sandbox` ghi lúc tạo) VÀ mã ở đối tác do bộ giả của
//!    kernel cấp (`SANDBOX-…`) — một mã giả không lọt qua chỉ nhờ tiền tố.
//! 4. Lượt thu còn `pending`.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::money::Money;
use autoxide_kernel::payment::{GatewayNotification, NotificationKind};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::tenancy::sandbox::{SANDBOX_PREFIX, sandbox_only};
use autoxide_kernel::time::Timestamp;
use serde_json::json;
use uuid::Uuid;

use crate::api::Payment;
use crate::payments::{describe_payment, payment_state};
use crate::permissions::RECONCILE;
use crate::service::{LedgerService, money};

/// Kết quả đối tác giả báo về.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Succeeded,
    Failed,
}

impl Outcome {
    /// `succeeded` / `failed`; giá trị khác → lỗi của ô `outcome` vào `problems`.
    fn parse(raw: &str, problems: &mut Vec<FieldError>) -> Option<Outcome> {
        match raw {
            "succeeded" => Some(Outcome::Succeeded),
            "failed" => Some(Outcome::Failed),
            _ => {
                problems.push(FieldError::new(
                    "outcome",
                    "invalid",
                    "must be `succeeded` or `failed`",
                ));
                None
            }
        }
    }
}

/// Số đối tác giả báo đã thu: REST gửi `Money`, tool MCP gửi số đơn vị nhỏ (tiền tệ của lượt thu).
#[derive(Debug, Clone, Copy)]
pub(crate) enum Reported {
    Money(Money),
    Minor(i64),
}

/// `ledger.simulation_disabled` (404): người vận hành chưa bật giả lập.
fn simulation_disabled() -> Error {
    Error::not_found(
        "ledger.simulation_disabled",
        "Simulated payments are turned off on this server; the operator sets \
         `AUTOXIDE_LEDGER_SANDBOX_SIMULATION=true` to allow them inside sandboxes.",
    )
}

/// `ledger.payment_not_simulated` (409): lượt thu không do bộ giả của sandbox tạo.
fn not_simulated() -> Error {
    Error::conflict(
        "ledger.payment_not_simulated",
        "This payment was not created by the sandbox's simulated partner (bank transfer payments are not \
         supported yet), so its result cannot be simulated.",
    )
}

impl LedgerService {
    /// Đối tác giả báo kết quả `outcome` cho lượt thu `id` của sandbox (`amount`: số đã thu; bỏ trống
    /// thì đúng số phải thu — số khác đi đường `needs_review` như thông báo thật). Xử lý ngay qua đường
    /// của webhook rồi trả lượt thu; module nguồn (vd `orders`) nhận event như thường.
    pub(crate) async fn simulate_payment(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        outcome: &str,
        amount: Option<Reported>,
    ) -> Result<Payment, Error> {
        // Quyền trước mọi kiểm khác: người không có quyền không biết tính năng bật hay tắt.
        principal.require(&RECONCILE)?;
        if !self.config.sandbox_simulation {
            return Err(simulation_disabled());
        }
        if !tenant.is_sandbox() {
            return Err(sandbox_only());
        }
        let row = self.payment_for(tenant, principal, &RECONCILE, id).await?;
        let simulated = row
            .provider_ref
            .as_deref()
            .is_some_and(|reference| reference.starts_with(SANDBOX_PREFIX));
        if !row.sandbox || !simulated {
            return Err(not_simulated());
        }
        if row.status != "pending" {
            return Err(payment_state(&row.status));
        }
        // Gom mọi lỗi của đầu vào, báo một lần (D-083).
        let mut problems = Vec::new();
        let outcome = Outcome::parse(outcome, &mut problems);
        let due = money(row.amount_minor, &row.amount_currency)?;
        let received = match amount {
            None => due,
            Some(Reported::Money(amount)) => amount,
            Some(Reported::Minor(minor)) => Money::new(minor, due.currency()),
        };
        if received.currency() != due.currency() || received.minor() <= 0 {
            problems.push(FieldError::new(
                "amount",
                "invalid",
                "must be a positive amount in the currency of the payment",
            ));
        }
        let Some(outcome) = outcome.filter(|_| problems.is_empty()) else {
            return Err(Error::validation(problems));
        };
        let kind = match outcome {
            Outcome::Succeeded => NotificationKind::PaymentSucceeded,
            Outcome::Failed => NotificationKind::PaymentFailed { reason: "simulated".to_owned() },
        };
        let provider_ref = row.provider_ref.clone().unwrap_or_default();
        let notification = GatewayNotification::new(
            format!("sandbox:{}", Uuid::now_v7()),
            kind,
            provider_ref,
            received,
            Timestamp::now(),
        )
        .payment_id(row.id);
        let body = json!({
            "simulated": true, "payment_id": row.id, "outcome": match outcome {
                Outcome::Succeeded => "succeeded",
                Outcome::Failed => "failed",
            },
            "amount": {"minor": received.minor(), "currency": received.currency().code()},
        });
        let stored = self
            .record_notification(tenant, &row.provider, &notification, body.to_string())
            .await?;
        if let Some(event_id) = stored {
            // Xử lý ngay (cùng hàm job dùng; job chạy sau thấy đã xử lý thì bỏ qua): người thử thấy kết
            // quả trong câu trả lời. Lỗi ở đây thì job thử lại như với webhook.
            if let Err(error) = self.process_gateway_event(tenant, event_id).await {
                tracing::warn!(code = error.code(), "simulated notification left to the job");
            }
        }
        tracing::info!(payment_id = %row.id, sandbox = %tenant.id(), "simulated partner notification");
        let read = tenant.db().read();
        let row = Self::find_payment(&read, tenant, row.id)
            .await?
            .ok_or_else(crate::payments::payment_not_found)?;
        describe_payment(&row)
    }
}
