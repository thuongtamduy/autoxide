//! `PaymentGateway` của MoMo: tạo giao dịch, kiểm IPN, truy vấn trạng thái, hoàn tiền. Mọi lời gọi
//! đọc tài khoản của ĐÚNG khách được truyền vào; không có tài khoản chung nào để rơi về.

use std::time::Duration;

use autoxide_kernel::connections::{ConnectionProbe, ConnectionStatus};
use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::payment::{
    self, GatewayCapabilities, GatewayNotification, GatewayPayment, GatewayPaymentStatus,
    GatewayRefund, NotificationKind, PaymentGateway, PaymentIntent, RefundIntent, RefundStatus,
    StatementPage,
};
use autoxide_kernel::provider::{
    Environment, ProviderAccounts, Target, WebhookAck, constant_time_eq, hex, hmac_sha256,
    http_client, not_configured, send,
};
use autoxide_kernel::tenancy::{SettingValue, TenantContext, TenantId};
use autoxide_kernel::time::{Date, Timestamp};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::account::{
    ACCESS_KEY, CONNECTION, ENVIRONMENT, MomoConfig, PARTNER_CODE, RETURN_URL, SECRET_KEY,
    valid_return_url,
};
use crate::api::PROVIDER;

/// Đường nhận IPN (không có định danh của khách ở cuối); route là của `ledger`.
pub(crate) const WEBHOOK_PATH: &str = "/api/v1/ledger/webhooks/momo";

/// Tài liệu MoMo: hạn giờ tối thiểu khi gọi API là 30 giây.
const TIMEOUT: Duration = Duration::from_secs(30);
/// Tài liệu MoMo (thanh toán một lần bằng ví): số tiền từ 1.000 đến 50.000.000 VND.
const MIN_AMOUNT: i64 = 1_000;
const MAX_AMOUNT: i64 = 50_000_000;
/// Ngôn ngữ của câu `message` MoMo trả về.
const LANG: &str = "vi";

const CREATE: &str = "/v2/gateway/api/create";
const QUERY: &str = "/v2/gateway/api/query";
const REFUND: &str = "/v2/gateway/api/refund";

// Mã kết quả (`resultCode`) theo trang "List Result Codes" của MoMo.
const OK: i64 = 0;
/// Hệ thống bảo trì — thử lại sau.
const MAINTENANCE: i64 = 10;
/// Truy cập bị từ chối (cấu hình tài khoản doanh nghiệp).
const ACCESS_DENIED: i64 = 11;
/// Xác thực doanh nghiệp thất bại (sai khoá).
const AUTH_FAILED: i64 = 13;
const DUPLICATE_REQUEST: i64 = 40;
const DUPLICATE_ORDER: i64 = 41;
/// `orderId` không hợp lệ hoặc không tìm thấy.
const ORDER_NOT_FOUND: i64 = 42;
/// Một giao dịch tương tự đang được xử lý.
const BEING_PROCESSED: i64 = 43;
/// Giao dịch đã khởi tạo, chờ người mua xác nhận.
const WAITING_FOR_BUYER: i64 = 1000;
/// URL / mã QR đã hết hạn.
const EXPIRED: i64 = 1005;
/// Hoàn tiền lỗi trong lúc xử lý — thử lại sau.
const REFUND_RETRY: i64 = 1080;
const PROCESSING: i64 = 7000;
const PROCESSING_AT_ISSUER: i64 = 7002;
/// Đã cấp quyền trước, chờ xác nhận (luồng hai bước; bộ nối không dùng nhưng vẫn coi là chưa xong).
const PRE_AUTHORIZED: i64 = 8000;
const AUTHORIZED: i64 = 9000;

/// Kết quả chưa phải cuối cùng: giao dịch còn có thể thành công.
fn not_final(code: i64) -> bool {
    matches!(
        code,
        WAITING_FOR_BUYER | PROCESSING | PROCESSING_AT_ISSUER | PRE_AUTHORIZED | AUTHORIZED
    )
}

/// Tài khoản MoMo của một khách, đã đọc.
struct Account {
    partner_code: String,
    access_key: SettingValue,
    secret_key: SettingValue,
    base_url: String,
}

impl Account {
    /// Chữ ký của MoMo: HMAC-SHA256 (hex) trên `key=value` nối bằng `&`, khoá theo thứ tự a-z,
    /// bắt đầu bằng `accessKey` — người gọi truyền `fields` (không kể `accessKey`) ĐÃ theo thứ tự.
    fn sign(&self, fields: &[(&str, &str)]) -> String {
        let mut raw = format!("accessKey={}", self.access_key.as_str());
        for (name, value) in fields {
            raw.push('&');
            raw.push_str(name);
            raw.push('=');
            raw.push_str(value);
        }
        hex(&hmac_sha256(self.secret_key.as_str().as_bytes(), raw.as_bytes()))
    }
}

pub(crate) struct MomoGateway {
    accounts: ProviderAccounts,
    config: MomoConfig,
    client: reqwest::Client,
    status: ConnectionStatus,
}

impl MomoGateway {
    pub(crate) fn new(
        accounts: ProviderAccounts,
        config: MomoConfig,
        status: ConnectionStatus,
    ) -> Result<Self, Error> {
        // `Target::Fixed`: địa chỉ của MoMo do người vận hành đặt, không phải khách chọn.
        let client = http_client(TIMEOUT, Target::Fixed)?;
        Ok(MomoGateway { accounts, config, client, status })
    }

    async fn account(&self, tenant: &TenantContext) -> Result<Account, Error> {
        let partner_code = self.accounts.require(tenant, &PARTNER_CODE).await?;
        let access_key = self.accounts.require(tenant, &ACCESS_KEY).await?;
        let secret_key = self.accounts.require(tenant, &SECRET_KEY).await?;
        let environment = self.accounts.get(tenant, &ENVIRONMENT).await?;
        // Giá trị lạ (không qua được bộ kiểm lúc lưu) thì từ chối, không đoán môi trường.
        let environment = Environment::parse(environment.as_ref().map(|value| value.as_str()))
            .ok_or_else(|| not_configured(ENVIRONMENT.name()))?;
        let base_url =
            environment.pick(&self.config.api_url, &self.config.sandbox_api_url).to_owned();
        Ok(Account {
            partner_code: partner_code.as_str().trim().to_owned(),
            access_key,
            secret_key,
            base_url,
        })
    }

    /// URL nhận IPN của khách này — MoMo đòi gửi `ipnUrl` trong từng lần tạo giao dịch.
    async fn ipn_url(&self, tenant: &TenantContext) -> Result<String, Error> {
        self.accounts.webhook_url(tenant, WEBHOOK_PATH).await
    }

    /// Gửi `body` tới `path` và trả JSON MoMo trả lời. Không bao giờ gọi trong transaction ghi.
    async fn call(
        &self,
        tenant: &TenantContext,
        account: &Account,
        path: &str,
        purpose: &'static str,
        body: &Value,
    ) -> Result<Value, Error> {
        let request = self
            .client
            .post(format!("{}{path}", account.base_url))
            .header("content-type", "application/json; charset=UTF-8")
            .body(serde_json::to_vec(body).map_err(Error::internal)?);
        let Ok(reply) = send(request, PROVIDER, purpose).await else {
            self.status.record(tenant, CONNECTION, Some("MoMo did not answer.")).await;
            return Err(payment::provider_unavailable());
        };
        // MoMo trả `resultCode` trong thân; đọc thân trước, mã HTTP chỉ dùng khi thân không có gì.
        match reply.json().filter(|answer| result_code(answer).is_some()) {
            Some(answer) => Ok(answer),
            None if reply.status >= 500 => Err(payment::provider_unavailable()),
            None => Err(payment::provider_rejected(format!(
                "MoMo answered HTTP {} without a result code",
                reply.status
            ))),
        }
    }

    /// Đổi `resultCode` khác 0 của một lời gọi thành lỗi, và ghi tình trạng kết nối của khách.
    async fn refused(&self, tenant: &TenantContext, answer: &Value) -> Error {
        let code = result_code(answer).unwrap_or(-1);
        match code {
            ACCESS_DENIED | AUTH_FAILED => {
                let error = account_rejected(code);
                self.status.record(tenant, CONNECTION, Some(&error.message())).await;
                error
            }
            MAINTENANCE | BEING_PROCESSED | PROCESSING | PROCESSING_AT_ISSUER | REFUND_RETRY => {
                payment::provider_unavailable()
            }
            _ => payment::provider_rejected(describe(code, answer)),
        }
    }

    async fn query(
        &self,
        tenant: &TenantContext,
        account: &Account,
        order_id: &str,
        purpose: &'static str,
    ) -> Result<Value, Error> {
        // Mỗi lần hỏi là một request mới: `requestId` mới, `orderId` là giao dịch cần hỏi.
        let request_id = Uuid::now_v7().to_string();
        let signature = account.sign(&[
            ("orderId", order_id),
            ("partnerCode", &account.partner_code),
            ("requestId", &request_id),
        ]);
        let body = json!({
            "partnerCode": account.partner_code,
            "requestId": request_id,
            "orderId": order_id,
            "lang": LANG,
            "signature": signature,
        });
        self.call(tenant, account, QUERY, purpose, &body).await
    }
}

#[async_trait::async_trait]
impl PaymentGateway for MomoGateway {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    fn capabilities(&self) -> GatewayCapabilities {
        GatewayCapabilities::new(&["wallet"]).with_refunds(true)
    }

    async fn create_payment(
        &self,
        tenant: TenantId,
        intent: &PaymentIntent,
    ) -> Result<GatewayPayment, Error> {
        let tenant = self.accounts.tenant(tenant).await?;
        let account = self.account(&tenant).await?;
        let amount = vnd_amount(intent.amount)?;
        if !(MIN_AMOUNT..=MAX_AMOUNT).contains(&amount) {
            return Err(amount_out_of_range());
        }
        let redirect_url = match intent.return_url.as_deref().filter(|url| valid_return_url(url)) {
            Some(url) => url.to_owned(),
            None => self
                .accounts
                .get(&tenant, &RETURN_URL)
                .await?
                .map(|value| value.as_str().trim().to_owned())
                .ok_or_else(return_url_required)?,
        };
        let ipn_url = self.ipn_url(&tenant).await?;
        // `orderId` = `requestId` = ID thanh toán trong sổ: MoMo từ chối `orderId` trùng (mã 41),
        // nên gọi lại không bao giờ tạo giao dịch thứ hai.
        let order_id = intent.payment_id.to_string();
        let order_info = match intent.description.trim() {
            "" => format!("Payment {order_id}"),
            description => description.to_owned(),
        };
        let (amount_text, extra_data, request_type) = (amount.to_string(), "", "captureWallet");
        let signature = account.sign(&[
            ("amount", &amount_text),
            ("extraData", extra_data),
            ("ipnUrl", &ipn_url),
            ("orderId", &order_id),
            ("orderInfo", &order_info),
            ("partnerCode", &account.partner_code),
            ("redirectUrl", &redirect_url),
            ("requestId", &order_id),
            ("requestType", request_type),
        ]);
        let body = json!({
            "partnerCode": account.partner_code,
            "requestId": order_id,
            "amount": amount,
            "orderId": order_id,
            "orderInfo": order_info,
            "redirectUrl": redirect_url,
            "ipnUrl": ipn_url,
            "requestType": request_type,
            "extraData": extra_data,
            "lang": LANG,
            "signature": signature,
        });
        let answer = self.call(&tenant, &account, CREATE, "create_payment", &body).await?;
        match result_code(&answer) {
            Some(OK) => {}
            // Lần gọi trước đã tới MoMo nhưng câu trả lời (có link thanh toán) bị mất: MoMo không
            // cấp lại link cho cùng `orderId`. Người mua chưa thể trả (chưa ai có link), nên cách
            // an toàn là tạo một thanh toán mới.
            Some(DUPLICATE_REQUEST | DUPLICATE_ORDER) => return Err(order_already_created()),
            _ => return Err(self.refused(&tenant, &answer).await),
        }
        self.status.record(&tenant, CONNECTION, None).await;
        let mut created = GatewayPayment::new(order_id);
        created.checkout_url = text(&answer, "payUrl").filter(|url| !url.is_empty());
        created.qr_payload = text(&answer, "qrCodeUrl").filter(|code| !code.is_empty());
        Ok(created)
    }

    async fn refund(
        &self,
        tenant: TenantId,
        intent: &RefundIntent,
    ) -> Result<GatewayRefund, Error> {
        let tenant = self.accounts.tenant(tenant).await?;
        let account = self.account(&tenant).await?;
        let amount = vnd_amount(intent.amount)?;
        // Hoàn tiền cần `transId` (mã giao dịch của MoMo) của lượt thu; sổ chỉ giữ `orderId`, nên
        // hỏi MoMo trước.
        let paid =
            self.query(&tenant, &account, &intent.payment_provider_ref, "refund_lookup").await?;
        if result_code(&paid) != Some(OK) {
            return Err(self.refused(&tenant, &paid).await);
        }
        let Some(trans_id) = paid.get("transId").and_then(Value::as_i64) else {
            return Err(payment::provider_rejected("MoMo did not return the transaction id"));
        };
        // MoMo đòi `orderId` MỚI cho lượt hoàn: dùng ID lượt hoàn trong sổ (gọi lại không hoàn hai lần).
        let order_id = intent.refund_id.to_string();
        let description = match intent.reason.trim() {
            "" => format!("Refund {order_id}"),
            reason => reason.to_owned(),
        };
        let (amount_text, trans_text) = (amount.to_string(), trans_id.to_string());
        let signature = account.sign(&[
            ("amount", &amount_text),
            ("description", &description),
            ("orderId", &order_id),
            ("partnerCode", &account.partner_code),
            ("requestId", &order_id),
            ("transId", &trans_text),
        ]);
        let body = json!({
            "partnerCode": account.partner_code,
            "orderId": order_id,
            "requestId": order_id,
            "amount": amount,
            "transId": trans_id,
            "lang": LANG,
            "description": description,
            "signature": signature,
        });
        let answer = self.call(&tenant, &account, REFUND, "refund", &body).await?;
        match result_code(&answer) {
            Some(OK) => {
                self.status.record(&tenant, CONNECTION, None).await;
                Ok(GatewayRefund::new(order_id, RefundStatus::Succeeded))
            }
            // Lệnh hoàn này đã tới MoMo ở lần gọi trước mà không rõ kết quả: không gửi lệnh mới
            // (tránh hoàn hai lần), không tự coi là xong — để người đối soát xem ở MoMo.
            Some(DUPLICATE_REQUEST | DUPLICATE_ORDER) => Err(refund_already_sent()),
            _ => Err(self.refused(&tenant, &answer).await),
        }
    }

    async fn payment_status(
        &self,
        tenant: TenantId,
        provider_ref: &str,
    ) -> Result<GatewayPaymentStatus, Error> {
        let tenant = self.accounts.tenant(tenant).await?;
        let account = self.account(&tenant).await?;
        let answer = self.query(&tenant, &account, provider_ref, "payment_status").await?;
        let code = result_code(&answer).unwrap_or(-1);
        Ok(match code {
            OK => {
                let amount = answer.get("amount").and_then(Value::as_i64).ok_or_else(|| {
                    payment::provider_rejected("MoMo did not return the paid amount")
                })?;
                GatewayPaymentStatus::Succeeded { amount: Money::new(amount, Currency::VND) }
            }
            EXPIRED => GatewayPaymentStatus::Expired,
            code if not_final(code) => GatewayPaymentStatus::Pending,
            // Lỗi của tài khoản hay của hệ thống không nói gì về giao dịch: báo lỗi, không báo
            // "thất bại".
            ACCESS_DENIED | AUTH_FAILED | MAINTENANCE | BEING_PROCESSED => {
                return Err(self.refused(&tenant, &answer).await);
            }
            code => GatewayPaymentStatus::Failed { reason: describe(code, &answer) },
        })
    }

    async fn verify_webhook(
        &self,
        tenant: TenantId,
        _headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<GatewayNotification, Error> {
        let tenant = self.accounts.tenant(tenant).await?;
        let account = self.account(&tenant).await?;
        // Chữ ký của MoMo nằm TRONG thân JSON và ký trên giá trị từng trường, nên phải đọc JSON
        // trước; không trường nào được tin cho tới khi chữ ký khớp.
        let notice: Value =
            serde_json::from_slice(body).map_err(|_| payment::signature_invalid())?;
        let field = |name: &str| text(&notice, name).unwrap_or_default();
        let names = [
            "amount",
            "extraData",
            "message",
            "orderId",
            "orderInfo",
            "orderType",
            "partnerCode",
            "payType",
            "requestId",
            "responseTime",
            "resultCode",
            "transId",
        ];
        let values: Vec<String> = names.iter().map(|name| field(name)).collect();
        let pairs: Vec<(&str, &str)> =
            names.iter().copied().zip(values.iter().map(String::as_str)).collect();
        let wanted = account.sign(&pairs);
        let given = field("signature").to_ascii_lowercase();
        // So sánh thời gian hằng: không lộ chữ ký đúng qua thời gian trả lời.
        if !constant_time_eq(wanted.as_bytes(), given.as_bytes())
            || field("partnerCode") != account.partner_code
        {
            return Err(payment::signature_invalid());
        }
        let (order_id, trans_id) = (field("orderId"), field("transId"));
        let malformed = || payment::provider_rejected("the MoMo notification is incomplete");
        let code = result_code(&notice).ok_or_else(malformed)?;
        let amount = notice.get("amount").and_then(Value::as_i64).ok_or_else(malformed)?;
        if order_id.is_empty() {
            return Err(malformed());
        }
        let kind = match code {
            OK => NotificationKind::PaymentSucceeded,
            // Chưa phải kết quả cuối: không ghi gì, MoMo sẽ báo lại khi có kết quả.
            code if not_final(code) => return Err(notification_not_final(code)),
            code => NotificationKind::PaymentFailed { reason: describe(code, &notice) },
        };
        // `responseTime` là mili giây từ 1970. Không dùng cửa sổ thời gian để chống phát lại: MoMo
        // không ký thời điểm gửi, và một thông báo phát lại mang cùng `event_id` nên chỉ được xử
        // lý một lần.
        let occurred_at = notice
            .get("responseTime")
            .and_then(Value::as_i64)
            .and_then(|millis| millis.checked_mul(1000))
            .and_then(Timestamp::from_unix_micros)
            .unwrap_or_else(Timestamp::now);
        let mut notification = GatewayNotification::new(
            format!("{order_id}:{trans_id}:{code}"),
            kind,
            order_id.clone(),
            Money::new(amount, Currency::VND),
            occurred_at,
        );
        if let Ok(payment_id) = Uuid::parse_str(&order_id) {
            notification = notification.payment_id(payment_id);
        }
        Ok(notification)
    }

    // Tài liệu MoMo (trang "Payment Notification"): trả HTTP 204 trong 15 giây.
    fn webhook_ack(&self) -> Option<WebhookAck> {
        Some(WebhookAck::no_content())
    }

    async fn statement(
        &self,
        _tenant: TenantId,
        _from: Date,
        _to: Date,
        _cursor: Option<String>,
    ) -> Result<StatementPage, Error> {
        // API thanh toán của MoMo không có sao kê; đối soát theo sao kê làm ở cổng doanh nghiệp.
        Err(payment::not_supported(PROVIDER))
    }
}

#[async_trait::async_trait]
impl ConnectionProbe for MomoGateway {
    fn connection(&self) -> &'static str {
        CONNECTION
    }

    /// Hỏi trạng thái một `orderId` không có: MoMo nhận chữ ký thì trả "không tìm thấy" (mã 42);
    /// sai khoá hay sai môi trường thì trả mã khác.
    async fn test(&self, tenant: &TenantContext) -> Result<(), Error> {
        let account = self.account(tenant).await?;
        let order_id = format!("probe-{}", Uuid::now_v7());
        let answer = self.query(tenant, &account, &order_id, "test_connection").await?;
        match result_code(&answer) {
            Some(ORDER_NOT_FOUND | OK) => Ok(()),
            Some(code) if not_final(code) => Ok(()),
            Some(code @ (ACCESS_DENIED | AUTH_FAILED)) => Err(account_rejected(code)),
            _ => Err(self.refused(tenant, &answer).await),
        }
    }
}

// ---- Đọc câu trả lời -----------------------------------------------------------------------------

/// `resultCode` của một câu trả lời / thông báo (MoMo gửi số; chấp nhận cả chuỗi số).
fn result_code(answer: &Value) -> Option<i64> {
    match answer.get("resultCode")? {
        Value::Number(number) => number.as_i64(),
        Value::String(raw) => raw.trim().parse().ok(),
        _ => None,
    }
}

/// Giá trị trường `name` dưới dạng chữ — đúng dạng MoMo dùng khi ký (số viết thập phân).
fn text(answer: &Value, name: &str) -> Option<String> {
    match answer.get(name)? {
        Value::String(raw) => Some(raw.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

/// Câu ngắn cho người đọc lỗi: mã của MoMo và câu MoMo kèm theo (cắt ngắn).
fn describe(code: i64, answer: &Value) -> String {
    let message: String = text(answer, "message").unwrap_or_default().chars().take(200).collect();
    if message.is_empty() {
        format!("MoMo result code {code}")
    } else {
        format!("MoMo result code {code} ({message})")
    }
}

/// Số tiền VND nguyên của `amount`; tiền tệ khác → lỗi.
fn vnd_amount(amount: Money) -> Result<i64, Error> {
    if amount.currency() == Currency::VND {
        Ok(amount.minor())
    } else {
        Err(Error::new(
            ErrorKind::Validation,
            "momo.currency_not_supported",
            "MoMo only takes payments in VND, not {currency}.",
        )
        .with_param("currency", amount.currency().code()))
    }
}

// ---- Lỗi -----------------------------------------------------------------------------------------

/// `momo.amount_out_of_range` (422).
fn amount_out_of_range() -> Error {
    Error::new(
        ErrorKind::Validation,
        "momo.amount_out_of_range",
        "A MoMo wallet payment must be between {min} and {max} VND.",
    )
    .with_param("min", MIN_AMOUNT)
    .with_param("max", MAX_AMOUNT)
}

/// `momo.return_url_required` (422).
fn return_url_required() -> Error {
    Error::new(
        ErrorKind::Validation,
        "momo.return_url_required",
        "MoMo needs the page the buyer returns to after paying. Send `return_url` with the \
         payment, or set `momo.connection.account.return_url` for this tenant.",
    )
}

/// `momo.account_rejected` (422): MoMo không nhận bộ khoá của khách.
fn account_rejected(code: i64) -> Error {
    Error::new(
        ErrorKind::Validation,
        "momo.account_rejected",
        "MoMo refused this tenant's account (result code {code}). Check the Partner Code, Access \
         Key, Secret Key and environment under `momo.connection.account`.",
    )
    .with_param("code", code)
}

/// `momo.order_already_created` (409).
fn order_already_created() -> Error {
    Error::conflict(
        "momo.order_already_created",
        "MoMo already has this payment from an earlier attempt and does not hand out its payment \
         link again. Create a new payment with a new reference.",
    )
}

/// `momo.refund_already_sent` (409).
fn refund_already_sent() -> Error {
    Error::conflict(
        "momo.refund_already_sent",
        "MoMo already received this refund in an earlier attempt, so it was not sent again. Check \
         its result in the MoMo for Business portal.",
    )
}

/// `momo.notification_not_final` (422): thông báo chưa mang kết quả cuối.
fn notification_not_final(code: i64) -> Error {
    Error::new(
        ErrorKind::Validation,
        "momo.notification_not_final",
        "This MoMo notification (result code {code}) is not a final result; nothing was recorded.",
    )
    .with_param("code", code)
}
