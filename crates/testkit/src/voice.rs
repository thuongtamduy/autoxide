//! A fake voice provider for tests (step 61, D-232): keeps every command in memory, has no HTTP client
//! (never touches the network), signs webhooks with a fixed key; a recording is a few fixed bytes.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use autoxide_kernel::error::Error;
use autoxide_kernel::storage::ByteStream;
use autoxide_kernel::tenancy::TenantId;
use autoxide_kernel::voice::{
    BrowserSession, BrowserSessionRequest, CallStatus, OutboundCall, ProviderCall,
    VoiceCapabilities, VoiceNotification, VoiceProvider, provider_unavailable, signature_invalid,
};
use axum::http::HeaderMap;
use bytes::Bytes;
use serde_json::{Value, json};
use uuid::Uuid;

/// Name of the fake provider (webhook path `/voice/webhooks/fake/…`).
pub const FAKE_VOICE_PROVIDER: &str = "fake";
/// Content of every fake recording.
pub const FAKE_RECORDING: &[u8] = b"fake-recording";
const SECRET: &str = "fake-voice-secret";
const SIGNATURE: &str = "x-fake-voice-signature";
const TIMESTAMP: &str = "x-fake-voice-timestamp";
/// Replay-protection window of the signature, in seconds.
const WINDOW: i64 = 300;

/// A fake voice provider.
pub struct FakeVoiceProvider {
    recording_notice: AtomicBool,
    unavailable: AtomicUsize,
    sessions: Mutex<Vec<BrowserSessionRequest>>,
    /// `call_id` → (call command, call code on the provider side).
    calls: Mutex<HashMap<Uuid, (OutboundCall, String)>>,
    /// Recordings currently at the provider → still present (not deleted).
    recordings: Mutex<HashMap<String, bool>>,
    events: AtomicUsize,
}

impl Default for FakeVoiceProvider {
    fn default() -> Self {
        FakeVoiceProvider {
            recording_notice: AtomicBool::new(true),
            unavailable: AtomicUsize::new(0),
            sessions: Mutex::new(Vec::new()),
            calls: Mutex::new(HashMap::new()),
            recordings: Mutex::new(HashMap::new()),
            events: AtomicUsize::new(0),
        }
    }
}

impl FakeVoiceProvider {
    /// Whether the provider can (`true`, the default) or cannot play the recorded announcement.
    pub fn with_recording_notice(self, supported: bool) -> Self {
        self.recording_notice.store(supported, Ordering::SeqCst);
        self
    }

    /// The next `n` calls to the provider report `voice.provider_unavailable`.
    pub fn fail_next(&self, n: usize) {
        self.unavailable.store(n, Ordering::SeqCst);
    }

    fn take_failure(&self) -> Result<(), Error> {
        let failed = self
            .unavailable
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1))
            .is_ok();
        if failed { Err(provider_unavailable()) } else { Ok(()) }
    }

    /// WebRTC sessions issued.
    pub fn sessions(&self) -> Vec<BrowserSessionRequest> {
        lock(&self.sessions).clone()
    }

    /// Outbound calls placed (once per `call_id`).
    pub fn calls(&self) -> Vec<OutboundCall> {
        lock(&self.calls).values().map(|(call, _)| call.clone()).collect()
    }

    /// Provider-side code of call `call_id`.
    pub fn provider_call_id(&self, call_id: Uuid) -> Option<String> {
        lock(&self.calls).get(&call_id).map(|(_, id)| id.clone())
    }

    /// Whether recording `id` is still at the provider.
    pub fn has_recording(&self, id: &str) -> bool {
        lock(&self.recordings).get(id).copied().unwrap_or(false)
    }

    fn sign(timestamp: i64, body: &[u8]) -> String {
        autoxide_kernel::secret::sha256(&format!(
            "{SECRET}|{timestamp}|{}",
            String::from_utf8_lossy(body)
        ))
    }

    /// A signed webhook (header, body) reporting call `call_id` moving to `status`; `recording`: has a
    /// recording (code `rec-<call_id>`, call completed). Every call is a new event.
    pub fn emit(
        &self,
        call_id: Uuid,
        status: CallStatus,
        duration_seconds: Option<u32>,
        recording: bool,
    ) -> (Vec<(String, String)>, Vec<u8>) {
        let number = self.events.fetch_add(1, Ordering::SeqCst);
        let provider_call_id =
            self.provider_call_id(call_id).unwrap_or_else(|| format!("fake-call-{call_id}"));
        let recording_id = recording.then(|| format!("rec-{call_id}"));
        if let Some(id) = &recording_id {
            lock(&self.recordings).insert(id.clone(), true);
        }
        let body = json!({"events": [{
            "event_id": format!("evt-{number}-{call_id}"),
            "provider_call_id": provider_call_id,
            "call_id": call_id,
            "status": status.as_str(),
            "agent_identity": lock(&self.calls).get(&call_id).map(|(call, _)| call.agent_identity.clone()),
            "duration_seconds": duration_seconds,
            "recording_id": recording_id,
        }]});
        Self::signed(&body, Timestamp::now().as_unix_seconds())
    }

    /// Body `body` signed with timestamp `timestamp` (to test replay protection, resending the same event).
    pub fn signed(body: &Value, timestamp: i64) -> (Vec<(String, String)>, Vec<u8>) {
        let bytes = body.to_string().into_bytes();
        let headers = vec![
            (SIGNATURE.to_owned(), Self::sign(timestamp, &bytes)),
            (TIMESTAMP.to_owned(), timestamp.to_string()),
        ];
        (headers, bytes)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn text(value: &Value, key: &str) -> Result<String, Error> {
    value[key].as_str().map(str::to_owned).ok_or_else(signature_invalid)
}

#[async_trait::async_trait]
impl VoiceProvider for FakeVoiceProvider {
    fn provider(&self) -> &'static str {
        FAKE_VOICE_PROVIDER
    }

    fn capabilities(&self) -> VoiceCapabilities {
        VoiceCapabilities {
            browser_calls: true,
            outbound: true,
            recording: true,
            recording_notice: self.recording_notice.load(Ordering::SeqCst),
        }
    }

    async fn browser_session(
        &self,
        _tenant: TenantId,
        request: &BrowserSessionRequest,
    ) -> Result<BrowserSession, Error> {
        self.take_failure()?;
        lock(&self.sessions).push(request.clone());
        Ok(BrowserSession {
            token: format!("fake-webrtc-{}", Uuid::now_v7()),
            expires_at: Timestamp::now() + SignedDuration::from_mins(10),
        })
    }

    async fn place_call(
        &self,
        _tenant: TenantId,
        call: &OutboundCall,
    ) -> Result<ProviderCall, Error> {
        self.take_failure()?;
        let mut calls = lock(&self.calls);
        let (_, id) = calls
            .entry(call.call_id)
            .or_insert_with(|| (call.clone(), format!("fake-call-{}", call.call_id)));
        Ok(ProviderCall { provider_call_id: id.clone(), status: CallStatus::Queued })
    }

    fn verify_webhook(
        &self,
        _tenant: TenantId,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<Vec<VoiceNotification>, Error> {
        let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
        let (Some(signature), Some(timestamp)) = (header(SIGNATURE), header(TIMESTAMP)) else {
            return Err(signature_invalid());
        };
        let timestamp: i64 = timestamp.parse().map_err(|_| signature_invalid())?;
        if (Timestamp::now().as_unix_seconds() - timestamp).abs() > WINDOW {
            return Err(signature_invalid());
        }
        if Self::sign(timestamp, body) != signature {
            return Err(signature_invalid());
        }
        let body: Value = serde_json::from_slice(body).map_err(|_| signature_invalid())?;
        let at = Timestamp::from_unix_seconds(timestamp).unwrap_or_else(Timestamp::now);
        let items = body["events"].as_array().ok_or_else(signature_invalid)?;
        items
            .iter()
            .map(|item| {
                Ok(VoiceNotification {
                    provider_event_id: text(item, "event_id")?,
                    provider_call_id: text(item, "provider_call_id")?,
                    call_id: item["call_id"].as_str().and_then(|raw| raw.parse().ok()),
                    status: CallStatus::parse(&text(item, "status")?)
                        .ok_or_else(signature_invalid)?,
                    agent_identity: item["agent_identity"].as_str().map(str::to_owned),
                    occurred_at: at,
                    duration_seconds: item["duration_seconds"]
                        .as_u64()
                        .and_then(|seconds| u32::try_from(seconds).ok()),
                    provider_recording_id: item["recording_id"].as_str().map(str::to_owned),
                })
            })
            .collect()
    }

    async fn fetch_recording(
        &self,
        _tenant: TenantId,
        provider_recording_id: &str,
    ) -> Result<ByteStream, Error> {
        self.take_failure()?;
        if !self.has_recording(provider_recording_id) {
            return Err(Error::not_found(
                "voice.recording_not_found",
                "The voice provider has no recording `{id}`.",
            )
            .with_param("id", provider_recording_id));
        }
        let chunk: Result<Bytes, Error> = Ok(Bytes::from_static(FAKE_RECORDING));
        Ok(Box::pin(futures_util::stream::iter(vec![chunk])))
    }

    async fn delete_recording(
        &self,
        _tenant: TenantId,
        provider_recording_id: &str,
    ) -> Result<(), Error> {
        self.take_failure()?;
        lock(&self.recordings).insert(provider_recording_id.to_owned(), false);
        Ok(())
    }
}
