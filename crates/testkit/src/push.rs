//! Fake senders for Zalo ZNS, web push and mobile push (step 37, D-348): keep every message in memory; mark
//! an endpoint / token as "gone" to exercise the branch where the provider returns 404 / 410.

use std::collections::HashSet;
use std::sync::Mutex;

use autoxide_kernel::error::Error;
use autoxide_kernel::push::{
    DeviceToken, MobilePushSender, PushContent, PushDelivery, WebPushSender, WebPushSubscription,
};
use autoxide_kernel::tenancy::TenantId;
use autoxide_kernel::zns::{ZnsMessage, ZnsSender};

/// A fake ZNS sender: keeps every message.
#[derive(Default)]
pub struct FakeZnsSender {
    pub sent: Mutex<Vec<ZnsMessage>>,
}

impl FakeZnsSender {
    pub fn new() -> Self {
        FakeZnsSender::default()
    }

    /// Number of messages sent.
    pub fn count(&self) -> usize {
        self.sent.lock().map_or(0, |sent| sent.len())
    }
}

#[async_trait::async_trait]
impl ZnsSender for FakeZnsSender {
    fn provider(&self) -> &'static str {
        "fake"
    }

    async fn send(&self, message: ZnsMessage) -> Result<String, Error> {
        let reference = format!("fake_zns_{}", message.reference);
        if let Ok(mut sent) = self.sent.lock() {
            sent.push(message);
        }
        Ok(reference)
    }
}

/// A fake web push: keeps the (endpoint, content) sent; an endpoint in `gone` returns [`PushDelivery::Gone`].
#[derive(Default)]
pub struct FakeWebPushSender {
    pub sent: Mutex<Vec<(String, PushContent)>>,
    gone: Mutex<HashSet<String>>,
}

impl FakeWebPushSender {
    pub fn new() -> Self {
        FakeWebPushSender::default()
    }

    /// From now on endpoint `endpoint` returns "gone" (like HTTP 410 from the push service).
    pub fn mark_gone(&self, endpoint: &str) {
        if let Ok(mut gone) = self.gone.lock() {
            gone.insert(endpoint.to_owned());
        }
    }

    /// Number of messages sent successfully.
    pub fn count(&self) -> usize {
        self.sent.lock().map_or(0, |sent| sent.len())
    }
}

#[async_trait::async_trait]
impl WebPushSender for FakeWebPushSender {
    fn provider(&self) -> &'static str {
        "fake"
    }

    fn public_key(&self) -> String {
        "BFakeVapidPublicKey".to_owned()
    }

    async fn send(
        &self,
        _tenant: TenantId,
        subscription: &WebPushSubscription,
        content: &PushContent,
    ) -> Result<PushDelivery, Error> {
        if self.gone.lock().is_ok_and(|gone| gone.contains(&subscription.endpoint)) {
            return Ok(PushDelivery::Gone);
        }
        if let Ok(mut sent) = self.sent.lock() {
            sent.push((subscription.endpoint.clone(), content.clone()));
        }
        Ok(PushDelivery::Sent { provider_reference: format!("fake_push_{}", content.reference) })
    }
}

/// A fake mobile push: keeps the (token, content) sent; a token in `gone` returns [`PushDelivery::Gone`].
#[derive(Default)]
pub struct FakeMobilePushSender {
    pub sent: Mutex<Vec<(String, PushContent)>>,
    gone: Mutex<HashSet<String>>,
}

impl FakeMobilePushSender {
    pub fn new() -> Self {
        FakeMobilePushSender::default()
    }

    /// From now on token `token` returns "expired".
    pub fn mark_gone(&self, token: &str) {
        if let Ok(mut gone) = self.gone.lock() {
            gone.insert(token.to_owned());
        }
    }

    /// Number of messages sent successfully.
    pub fn count(&self) -> usize {
        self.sent.lock().map_or(0, |sent| sent.len())
    }
}

#[async_trait::async_trait]
impl MobilePushSender for FakeMobilePushSender {
    fn provider(&self) -> &'static str {
        "fake"
    }

    async fn send(
        &self,
        _tenant: TenantId,
        device: &DeviceToken,
        content: &PushContent,
    ) -> Result<PushDelivery, Error> {
        if self.gone.lock().is_ok_and(|gone| gone.contains(&device.token)) {
            return Ok(PushDelivery::Gone);
        }
        if let Ok(mut sent) = self.sent.lock() {
            sent.push((device.token.clone(), content.clone()));
        }
        Ok(PushDelivery::Sent { provider_reference: format!("fake_mobile_{}", content.reference) })
    }
}
