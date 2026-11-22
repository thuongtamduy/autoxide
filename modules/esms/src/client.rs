//! Gọi eSMS: gửi tin OTP / chăm sóc khách hàng và thử kết nối bằng hàm lấy số dư. Request, trường
//! và mã trả về theo đúng tài liệu ghi ở đầu `lib.rs`.

use std::time::Duration;

use autoxide_kernel::connections::{ConnectionProbe, ConnectionStatus};
use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::provider::{ProviderAccounts, ProviderReply, Target, hex, hmac_sha256, send};
use autoxide_kernel::sms::{PhoneNumber, SmsMessage, SmsPurpose, SmsReceipt, SmsSender};
use autoxide_kernel::tenancy::TenantContext;
use serde_json::{Value, json};

use crate::account::{Account, CONNECTION};
use crate::api::PROVIDER;

/// Đường dẫn theo tài liệu (kể cả dấu `/` cuối của hàm gửi).
const SEND_PATH: &str = "/MainService.svc/json/SendMultipleMessage_V4_post_json/";
const BALANCE_PATH: &str = "/MainService.svc/json/GetBalance_json";

/// `SmsType` của tin chăm sóc khách hàng (gồm cả mã một lần).
const CUSTOMER_CARE: &str = "2";
/// `RequestId` dài tối đa 50 ký tự; eSMS chặn trùng trong 24 giờ.
const REQUEST_ID_MAX: usize = 50;
/// Bên gọi (`compliance`…) tự đặt hạn chờ ngắn hơn; đây là trần của mọi lời gọi.
const TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct Esms {
    accounts: ProviderAccounts,
    client: reqwest::Client,
    api_url: String,
}

impl Esms {
    pub(crate) fn new(accounts: ProviderAccounts, api_url: String) -> Result<Self, Error> {
        // `Target::Fixed`: địa chỉ do người vận hành đặt, không phải do khách chọn.
        let client = autoxide_kernel::provider::http_client(TIMEOUT, Target::Fixed)?;
        Ok(Esms { accounts, client, api_url })
    }

    /// POST thân JSON. reqwest của workspace không bật feature `json` nên tự tuần tự hoá.
    async fn post(
        &self,
        path: &str,
        body: &Value,
        purpose: &'static str,
    ) -> Result<ProviderReply, Error> {
        let request = self
            .client
            .post(format!("{}{path}", self.api_url))
            .header("content-type", "application/json")
            .body(serde_json::to_vec(body).map_err(Error::internal)?);
        send(request, PROVIDER, purpose).await.map_err(|_| unavailable())
    }

    async fn deliver(
        &self,
        account: &Account,
        phone: &str,
        message: &SmsMessage,
    ) -> Result<SmsReceipt, Error> {
        let request_id = request_id(&message.reference);
        let mut body = json!({
            "ApiKey": account.api_key.as_str(),
            "SecretKey": account.secret_key.as_str(),
            "Brandname": account.brandname,
            "Phone": phone,
            "Content": message.text,
            "SmsType": CUSTOMER_CARE,
            "IsUnicode": if message.text.is_ascii() { "0" } else { "1" },
            "RequestId": request_id,
        });
        if account.sandbox {
            body["Sandbox"] = json!("1");
        }
        let reply = self.post(SEND_PATH, &body, "send_sms").await?;
        let answer = parse(&reply)?;
        match code_of(&answer, "CodeResult").as_deref() {
            Some("100") => Ok(SmsReceipt::new(
                answer["SMSID"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .map_or_else(|| format!("request:{request_id}"), str::to_owned),
            )),
            // Trùng `RequestId`: lần gọi trước của chính tin này đã được eSMS nhận (bên gọi thử
            // lại sau khi mất câu trả lời). Thân 124 không có SMSID nên biên nhận là mã của ta.
            Some("124") => Ok(SmsReceipt::new(format!("request:{request_id}"))),
            code => Err(rejected(code)),
        }
    }
}

#[async_trait::async_trait]
impl SmsSender for Esms {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    async fn send(&self, message: SmsMessage) -> Result<SmsReceipt, Error> {
        // Hàm quảng cáo của eSMS đòi ít nhất 30 số mỗi request và chờ duyệt: không khớp hợp đồng
        // một tin một số. Không gửi tin quảng cáo qua đường chăm sóc khách hàng (NĐ 91/2020/NĐ-CP,
        // xem D-20261121-esms-adapter).
        if message.purpose == SmsPurpose::Marketing {
            return Err(marketing_unsupported());
        }
        let phone = local_number(&message.to).ok_or_else(phone_unsupported)?;
        let tenant = self.accounts.tenant(message.tenant).await?;
        let account = Account::read(&self.accounts, &tenant).await?;
        let outcome = self.deliver(&account, &phone, &message).await;
        // Người quản trị thấy lỗi cuối của kết nối mà không đọc log; câu lỗi không chứa secret, số
        // điện thoại hay nội dung tin.
        let error = outcome.as_ref().err().map(Error::message);
        ConnectionStatus.record(&tenant, CONNECTION, error.as_deref()).await;
        if let Err(error) = &outcome {
            tracing::warn!(phone = %message.to.masked(), code = error.code(), "esms refused a message");
        }
        outcome
    }
}

// Thử kết nối từ trang cài đặt (`POST /api/v1/kernel/settings/{name}/test`): hàm lấy số dư là lời
// gọi rẻ nhất có xác thực, key nằm trong thân (hàm lấy danh sách brandname đặt key trong URL nên
// không dùng). Số dư không được trả ra.
#[async_trait::async_trait]
impl ConnectionProbe for Esms {
    fn connection(&self) -> &'static str {
        CONNECTION
    }

    async fn test(&self, tenant: &TenantContext) -> Result<(), Error> {
        let account = Account::read(&self.accounts, tenant).await?;
        let body = json!({
            "ApiKey": account.api_key.as_str(),
            "SecretKey": account.secret_key.as_str(),
        });
        let reply = self.post(BALANCE_PATH, &body, "get_balance").await?;
        // Hàm này trả `CodeResponse`, hàm gửi trả `CodeResult`.
        match code_of(&parse(&reply)?, "CodeResponse").as_deref() {
            Some("100") => Ok(()),
            code => Err(rejected(code)),
        }
    }
}

/// Số nội địa như ví dụ của tài liệu (`0901888484`). Tài liệu không nêu dạng cho số ngoài Việt Nam
/// nên chỉ nhận `+84…`.
fn local_number(phone: &PhoneNumber) -> Option<String> {
    phone.as_str().strip_prefix("+84").map(|rest| format!("0{rest}"))
}

/// `reference` của bên gọi làm `RequestId`; dài quá 50 ký tự thì thay bằng mã băm ổn định của nó
/// (cùng `reference` luôn ra cùng mã, nên gọi lại vẫn bị eSMS chặn trùng).
fn request_id(reference: &str) -> String {
    if !reference.is_empty() && reference.len() <= REQUEST_ID_MAX && reference.is_ascii() {
        return reference.to_owned();
    }
    let mut digest = hex(&hmac_sha256(b"esms.request_id", reference.as_bytes()));
    digest.truncate(REQUEST_ID_MAX);
    digest
}

/// Thân JSON của câu trả lời. HTTP 5xx / 429 hay thân không đọc được: lỗi tạm thời.
fn parse(reply: &ProviderReply) -> Result<Value, Error> {
    if reply.status >= 500 || reply.status == 429 {
        return Err(unavailable());
    }
    match reply.json() {
        Some(answer @ Value::Object(_)) => Ok(answer),
        _ if reply.is_success() => Err(unavailable()),
        _ => Err(rejected(None)),
    }
}

/// Mã trả về: tài liệu ghi kiểu string, ví dụ có chỗ là số — nhận cả hai.
fn code_of(answer: &Value, field: &str) -> Option<String> {
    match &answer[field] {
        Value::String(code) => Some(code.trim().to_owned()),
        Value::Number(code) => Some(code.to_string()),
        _ => None,
    }
}

/// Mã lỗi của eSMS (bảng mã lỗi trong tài liệu) → `code` ổn định của Autoxide. Câu lỗi của eSMS
/// (`ErrorMessage`) không được chép ra.
fn rejected(code: Option<&str>) -> Error {
    let shown = code.unwrap_or("unknown");
    let error = match code {
        Some("101") => Error::new(
            ErrorKind::Validation,
            "esms.credentials_rejected",
            "eSMS refused this tenant's ApiKey / SecretKey (eSMS code {code}). Set \
             `esms.connection.sms.api_key` and `esms.connection.sms.secret_key` again.",
        ),
        // Nạp tiền xong là gửi được: lỗi tạm thời, bên gọi thử lại.
        Some("103") => Error::unavailable(
            "esms.insufficient_balance",
            "The eSMS account has not enough balance to send (eSMS code {code}). Top it up, then \
             try again.",
        ),
        Some("104") => Error::new(
            ErrorKind::Validation,
            "esms.brandname_rejected",
            "eSMS does not accept this brandname (eSMS code {code}): it is not registered or not \
             active for customer-care messages. Fix `esms.connection.sms.brandname` or register \
             the brandname with eSMS.",
        ),
        Some("146") => Error::new(
            ErrorKind::Validation,
            "esms.template_rejected",
            "The message does not match a customer-care template registered for this brandname \
             (eSMS code {code}). Register the template with eSMS.",
        ),
        Some("177") => Error::new(
            ErrorKind::Validation,
            "esms.carrier_not_registered",
            "The brandname is not registered with the mobile network of this phone number (eSMS \
             code {code}). Register it with eSMS.",
        ),
        Some("99" | "108") => Error::new(
            ErrorKind::Validation,
            "esms.phone_rejected",
            "eSMS refused this phone number: wrong format or blocked (eSMS code {code}).",
        ),
        Some("201") => Error::new(
            ErrorKind::Validation,
            "esms.content_blocked",
            "eSMS refused the message because it contains a blocked keyword (eSMS code {code}).",
        ),
        Some("140") => Error::new(
            ErrorKind::Validation,
            "esms.ip_not_allowed",
            "eSMS does not allow this server's IP address to send yet (eSMS code {code}). Ask \
             eSMS to add it to the allowed list.",
        ),
        Some("143") => Error::new(
            ErrorKind::Validation,
            "esms.account_inactive",
            "The eSMS account is not activated or is locked (eSMS code {code}). Contact eSMS.",
        ),
        _ => Error::new(
            ErrorKind::Validation,
            "esms.request_rejected",
            "eSMS refused the request (eSMS code {code}). See the eSMS table of error codes.",
        ),
    };
    error.with_param("code", shown)
}

fn unavailable() -> Error {
    Error::unavailable("esms.unavailable", "eSMS cannot be reached right now; try again later.")
}

fn marketing_unsupported() -> Error {
    Error::new(
        ErrorKind::Validation,
        "esms.marketing_unsupported",
        "The eSMS adapter only sends one-time codes and customer-care messages; advertising \
         messages are not sent through it.",
    )
}

fn phone_unsupported() -> Error {
    Error::new(
        ErrorKind::Validation,
        "esms.phone_unsupported",
        "The eSMS adapter only sends to Vietnamese mobile numbers (+84).",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn so_noi_dia_va_request_id() {
        let phone = PhoneNumber::parse("0901 888 484", "84").unwrap();
        assert_eq!(local_number(&phone).as_deref(), Some("0901888484"));
        let foreign = PhoneNumber::parse("+14155550100", "84").unwrap();
        assert_eq!(local_number(&foreign), None);

        assert_eq!(request_id("c82cd356-bf49-4113-9466-65a7f6359c96").len(), 36);
        let long = "x".repeat(80);
        let hashed = request_id(&long);
        assert_eq!(hashed.len(), 50);
        assert_eq!(hashed, request_id(&long), "cùng reference ra cùng RequestId");
        assert_ne!(hashed, request_id(&"y".repeat(80)));
    }
}
