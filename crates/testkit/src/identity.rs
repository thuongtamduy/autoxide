//! Nhà cung cấp xác thực danh tính và gửi SMS giả cho test.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::HashMap;
use std::sync::Mutex;

use autoxide_kernel::error::Error;
use autoxide_kernel::identity::{
    IdentityVerifier, OutcomeStatus, VerificationMethod, VerificationOutcome, VerificationRequest,
    VerificationSession, callback_invalid,
};
use autoxide_kernel::sms::{SmsMessage, SmsReceipt, SmsSender};
use axum::http::HeaderMap;
use serde_json::{Value, json};

/// Tên của nhà cung cấp giả.
pub const FAKE_IDENTITY_PROVIDER: &str = "fake";
const SECRET: &str = "fake-identity-secret";
const SIGNATURE: &str = "x-fake-signature";

/// Xác thực danh tính giả: lượt mới chờ (`Pending`) tới khi test đặt kết quả bằng
/// [`complete`](Self::complete) rồi gửi callback đã ký ([`callback`](Self::callback)).
#[derive(Default)]
pub struct FakeIdentityVerifier {
    /// Lượt đã bắt đầu, theo mã ở nhà cung cấp.
    pub started: Mutex<Vec<VerificationRequest>>,
    outcomes: Mutex<HashMap<String, VerificationOutcome>>,
}

impl FakeIdentityVerifier {
    pub fn new() -> Self {
        FakeIdentityVerifier::default()
    }

    /// Mã ở nhà cung cấp của lượt thử `attempt_id`.
    pub fn reference(attempt_id: uuid::Uuid) -> String {
        format!("fake_kyc_{}", attempt_id.simple())
    }

    /// Đặt kết quả cho lượt `reference`: đã xác thực với tên và số giấy tờ, hay bị từ chối.
    pub fn complete(&self, reference: &str, verified: bool, name: &str, document: &str) {
        let outcome = if verified {
            VerificationOutcome::new(reference, OutcomeStatus::Verified)
                .verified_name(name)
                .document("cccd", document)
                .assurance("vneid_level_2")
        } else {
            VerificationOutcome::new(
                reference,
                OutcomeStatus::Rejected { reason_code: "document_unreadable".to_owned() },
            )
        };
        if let Ok(mut outcomes) = self.outcomes.lock() {
            outcomes.insert(reference.to_owned(), outcome);
        }
    }

    /// Callback đã ký (header, thân) báo kết quả của lượt `reference`.
    pub fn callback(reference: &str) -> (Vec<(String, String)>, Vec<u8>) {
        let body = json!({"reference": reference}).to_string().into_bytes();
        let signature = autoxide_kernel::secret::sha256(&format!(
            "{SECRET}|{}",
            String::from_utf8_lossy(&body)
        ));
        (vec![(SIGNATURE.to_owned(), signature)], body)
    }

    fn outcome(&self, reference: &str) -> VerificationOutcome {
        self.outcomes
            .lock()
            .ok()
            .and_then(|outcomes| outcomes.get(reference).cloned())
            .unwrap_or_else(|| VerificationOutcome::new(reference, OutcomeStatus::Pending))
    }
}

#[async_trait::async_trait]
impl IdentityVerifier for FakeIdentityVerifier {
    fn provider(&self) -> &'static str {
        FAKE_IDENTITY_PROVIDER
    }

    fn methods(&self) -> Vec<VerificationMethod> {
        vec![
            VerificationMethod::Vneid,
            VerificationMethod::Ekyc,
            VerificationMethod::BusinessRegistry,
        ]
    }

    async fn start(&self, request: VerificationRequest) -> Result<VerificationSession, Error> {
        let reference = Self::reference(request.attempt_id);
        if let Ok(mut started) = self.started.lock() {
            started.push(request);
        }
        Ok(VerificationSession::Redirect {
            url: format!("https://kyc.example.test/{reference}"),
            provider_reference: reference,
            expires_at: Timestamp::now() + SignedDuration::from_mins(30),
        })
    }

    async fn fetch(&self, provider_reference: &str) -> Result<VerificationOutcome, Error> {
        Ok(self.outcome(provider_reference))
    }

    async fn verify_callback(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<VerificationOutcome, Error> {
        let signature = headers
            .get(SIGNATURE)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(callback_invalid)?;
        let expected =
            autoxide_kernel::secret::sha256(&format!("{SECRET}|{}", String::from_utf8_lossy(body)));
        if signature != expected {
            return Err(callback_invalid());
        }
        let body: Value = serde_json::from_slice(body).map_err(|_| callback_invalid())?;
        let reference = body["reference"].as_str().ok_or_else(callback_invalid)?;
        Ok(self.outcome(reference))
    }
}

/// Gửi SMS giả: giữ mọi tin trong bộ nhớ.
#[derive(Default)]
pub struct FakeSmsSender {
    pub sent: Mutex<Vec<SmsMessage>>,
}

impl FakeSmsSender {
    pub fn new() -> Self {
        FakeSmsSender::default()
    }

    /// Nội dung tin cuối cùng gửi tới số `to` (E.164).
    pub fn last_text(&self, to: &str) -> Option<String> {
        self.sent
            .lock()
            .ok()?
            .iter()
            .rev()
            .find(|message| message.to.as_str() == to)
            .map(|message| message.text.clone())
    }
}

#[async_trait::async_trait]
impl SmsSender for FakeSmsSender {
    fn provider(&self) -> &'static str {
        "fake"
    }

    async fn send(&self, message: SmsMessage) -> Result<SmsReceipt, Error> {
        let reference = format!("fake_sms_{}", message.reference);
        if let Ok(mut sent) = self.sent.lock() {
            sent.push(message);
        }
        Ok(SmsReceipt::new(reference))
    }
}
