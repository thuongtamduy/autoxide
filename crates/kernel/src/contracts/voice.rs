//! Contract with voice providers (step 61, D-232): calling the shop from a browser (WebRTC), calling the buyer
//! back, call status webhooks, call recordings. A provider's adapter (Stringee, Twilio, ...) is its own module
//! that depends only on the kernel, is registered BEFORE the module that uses it (`conversations`) and
//! contributes with `context.contribute::<dyn VoiceProvider>(Arc::new(...))?`. Every function takes the tenant:
//! the adapter reads the tenant's connection settings from the per-tenant secret store itself (step 33). Tests
//! use `FakeVoiceProvider` from `autoxide-testkit`.
//!
//! The full phone number goes only to the provider; the caller does not write it to logs, events or errors (use
//! [`PhoneNumber::masked`](crate::sms::PhoneNumber::masked)).
use crate::time::Timestamp;
use uuid::Uuid;

use crate::error::{Error, ErrorKind};
use crate::sms::PhoneNumber;
use crate::storage::ByteStream;
use crate::tenancy::TenantId;

/// What the provider can do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VoiceCapabilities {
    /// The buyer can call from a browser / app (the provider's WebRTC SDK).
    pub browser_calls: bool,
    /// Outbound calls to a phone number (calling the buyer back).
    pub outbound: bool,
    /// Call recording.
    pub recording: bool,
    /// Can play a recording announcement at the start of the call. Without it, recording cannot be enabled.
    pub recording_notice: bool,
}

/// Who needs a WebRTC session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    /// Buyer: can only call `hotline`.
    Customer,
    /// Staff: receive calls to the hotline and make outbound calls; the identity is `identity`.
    Agent,
}

/// Request for a short-lived WebRTC session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserSessionRequest {
    pub role: SessionRole,
    /// Identity in the session (user id): the webhook reports the right person who picked up.
    pub identity: String,
    /// The shop's public number.
    pub hotline: PhoneNumber,
    /// Our own call (the buyer calls) so the provider can report back; `None` for a staff session.
    pub call_id: Option<Uuid>,
    /// Recording announcement (translated) when the call is recorded.
    pub recording_notice: Option<String>,
}

/// WebRTC session for the provider's SDK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserSession {
    /// Short-lived token handed to the SDK; not stored.
    pub token: String,
    pub expires_at: Timestamp,
}

/// An outbound call: connects the staff member's phone with the buyer's number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundCall {
    /// Our own code: the provider uses it to de-duplicate (calling again with the same code -> the same call).
    pub call_id: Uuid,
    /// Number the buyer sees: the shop's hotline, never a staff number.
    pub from: PhoneNumber,
    pub to: PhoneNumber,
    pub agent_identity: String,
    pub record: bool,
    /// Recording announcement (translated); required when `record`.
    pub recording_notice: Option<String>,
}

/// A call on the provider's side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCall {
    pub provider_call_id: String,
    pub status: CallStatus,
}

/// Call status. Order of progress: `Queued` -> `Ringing` -> `InProgress` -> a final status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallStatus {
    Queued,
    Ringing,
    InProgress,
    Completed,
    NoAnswer,
    Busy,
    Failed,
    Cancelled,
}

impl CallStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CallStatus::Queued => "queued",
            CallStatus::Ringing => "ringing",
            CallStatus::InProgress => "in_progress",
            CallStatus::Completed => "completed",
            CallStatus::NoAnswer => "no_answer",
            CallStatus::Busy => "busy",
            CallStatus::Failed => "failed",
            CallStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "queued" => CallStatus::Queued,
            "ringing" => CallStatus::Ringing,
            "in_progress" => CallStatus::InProgress,
            "completed" => CallStatus::Completed,
            "no_answer" => CallStatus::NoAnswer,
            "busy" => CallStatus::Busy,
            "failed" => CallStatus::Failed,
            "cancelled" => CallStatus::Cancelled,
            _ => return None,
        })
    }

    /// Final status: no longer changes.
    pub fn is_final(self) -> bool {
        self.rank() == 3
    }

    /// Progress rank of a status: a webhook arriving out of order must not move the rank backwards.
    pub fn rank(self) -> u8 {
        match self {
            CallStatus::Queued => 0,
            CallStatus::Ringing => 1,
            CallStatus::InProgress => 2,
            _ => 3,
        }
    }
}

/// A status notification from a webhook whose signature has been verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceNotification {
    /// Event code on the provider's side: prevents duplicate handling.
    pub provider_event_id: String,
    pub provider_call_id: String,
    /// Our own code when the provider reports back (`OutboundCall::call_id`, `BrowserSessionRequest::call_id`).
    pub call_id: Option<Uuid>,
    pub status: CallStatus,
    pub agent_identity: Option<String>,
    pub occurred_at: Timestamp,
    pub duration_seconds: Option<u32>,
    pub provider_recording_id: Option<String>,
}

/// A voice provider.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait VoiceProvider: Send + Sync {
    /// Stable name (`stringee`, `twilio`, `fake`, ...): part of the webhook path and on every call.
    fn provider(&self) -> &'static str;

    fn capabilities(&self) -> VoiceCapabilities;

    /// Short-lived token for the WebRTC SDK.
    async fn browser_session(
        &self,
        tenant: TenantId,
        request: &BrowserSessionRequest,
    ) -> Result<BrowserSession, Error>;

    /// Places an outbound call. Calling again with the same `call_id` must return the same call (no extra call).
    async fn place_call(
        &self,
        tenant: TenantId,
        call: &OutboundCall,
    ) -> Result<ProviderCall, Error>;

    /// Verifies the signature over the **raw body** (constant-time comparison) and the replay window, and
    /// only then reads the content. A wrong signature yields [`signature_invalid`].
    fn verify_webhook(
        &self,
        tenant: TenantId,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<Vec<VoiceNotification>, Error>;

    /// Downloads the recording.
    async fn fetch_recording(
        &self,
        tenant: TenantId,
        provider_recording_id: &str,
    ) -> Result<ByteStream, Error>;

    /// Deletes the recording at the provider (after it has been saved locally). Deleting what is already deleted succeeds.
    async fn delete_recording(
        &self,
        tenant: TenantId,
        provider_recording_id: &str,
    ) -> Result<(), Error>;
}

/// `voice.signature_invalid` (401): the webhook signature is wrong or outside the time window.
pub fn signature_invalid() -> Error {
    Error::new(
        ErrorKind::Unauthenticated,
        "voice.signature_invalid",
        "The signature of the voice provider notification is not valid.",
    )
}

/// `voice.provider_unavailable` (503): the provider does not respond.
pub fn provider_unavailable() -> Error {
    Error::unavailable(
        "voice.provider_unavailable",
        "The voice provider is not answering. Try again later.",
    )
}

#[cfg(test)]
mod tests {
    use super::CallStatus;

    #[test]
    fn trang_thai_cuoi_va_bac_tien() {
        for status in [CallStatus::Completed, CallStatus::NoAnswer, CallStatus::Busy] {
            assert!(status.is_final());
            assert_eq!(CallStatus::parse(status.as_str()), Some(status));
        }
        assert!(CallStatus::Ringing.rank() < CallStatus::InProgress.rank());
        assert!(!CallStatus::Queued.is_final());
    }
}
