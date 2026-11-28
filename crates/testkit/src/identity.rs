//! Fake identity-verification and SMS providers for tests.

use autoxide_kernel::time::{Date, Timestamp};
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

/// Name of the fake provider.
pub const FAKE_IDENTITY_PROVIDER: &str = "fake";
const SECRET: &str = "fake-identity-secret";
const SIGNATURE: &str = "x-fake-signature";

/// Fake identity verification: a new attempt waits (`Pending`) until the test sets the outcome with
/// [`complete`](Self::complete) and then sends the signed callback ([`callback`](Self::callback)).
#[derive(Default)]
pub struct FakeIdentityVerifier {
    /// Attempts started, keyed by the provider-side reference.
    pub started: Mutex<Vec<VerificationRequest>>,
    outcomes: Mutex<HashMap<String, VerificationOutcome>>,
}

impl FakeIdentityVerifier {
    pub fn new() -> Self {
        FakeIdentityVerifier::default()
    }

    /// Provider-side reference of attempt `attempt_id`.
    pub fn reference(attempt_id: uuid::Uuid) -> String {
        format!("fake_kyc_{}", attempt_id.simple())
    }

    /// Sets the outcome of attempt `reference`: verified with a name and document number, or rejected.
    ///
    /// A verified outcome also carries the seller details of Vietnam's Decree 248/2026 Article 18 (date of
    /// birth, address, legal representative, issuing country and end date of the ID card), so a test seller is
    /// complete whatever rule is in force; use [`complete_with`](Self::complete_with) for another outcome.
    pub fn complete(&self, reference: &str, verified: bool, name: &str, document: &str) {
        let outcome = if verified {
            let (birth, until) = (Date::new(1990, 1, 1), Date::new(2040, 12, 31));
            let mut outcome = VerificationOutcome::new(reference, OutcomeStatus::Verified)
                .verified_name(name)
                .document("cccd", document)
                .assurance("vneid_level_2")
                .address("1 Test Street, Hanoi")
                .representative(name);
            if let (Some(birth), Some(until)) = (birth, until) {
                outcome = outcome.birth_date(birth).document_validity("VN", until);
            }
            outcome
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

    /// Sets any outcome for the attempt of `outcome.provider_reference`.
    pub fn complete_with(&self, outcome: VerificationOutcome) {
        if let Ok(mut outcomes) = self.outcomes.lock() {
            outcomes.insert(outcome.provider_reference.clone(), outcome);
        }
    }

    /// Signed callback (header, body) reporting the outcome of attempt `reference`.
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

/// Fake SMS sender: keeps every message in memory.
#[derive(Default)]
pub struct FakeSmsSender {
    pub sent: Mutex<Vec<SmsMessage>>,
}

impl FakeSmsSender {
    pub fn new() -> Self {
        FakeSmsSender::default()
    }

    /// Body of the last message sent to number `to` (E.164).
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
