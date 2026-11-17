//! Bên gửi giả cho Zalo ZNS, web push và đẩy di động (bước 37, D-348): giữ mọi tin trong bộ nhớ; đánh
//! dấu một endpoint / token là "đã mất" để thử nhánh nhà cung cấp trả 404 / 410.

use std::collections::HashSet;
use std::sync::Mutex;

use autoxide_kernel::error::Error;
use autoxide_kernel::push::{
    DeviceToken, MobilePushSender, PushContent, PushDelivery, WebPushSender, WebPushSubscription,
};
use autoxide_kernel::tenancy::TenantId;
use autoxide_kernel::zns::{ZnsMessage, ZnsSender};

/// Gửi ZNS giả: giữ mọi tin.
#[derive(Default)]
pub struct FakeZnsSender {
    pub sent: Mutex<Vec<ZnsMessage>>,
}

impl FakeZnsSender {
    pub fn new() -> Self {
        FakeZnsSender::default()
    }

    /// Số tin đã gửi.
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

/// Web push giả: giữ (endpoint, nội dung) đã gửi; endpoint trong `gone` trả [`PushDelivery::Gone`].
#[derive(Default)]
pub struct FakeWebPushSender {
    pub sent: Mutex<Vec<(String, PushContent)>>,
    gone: Mutex<HashSet<String>>,
}

impl FakeWebPushSender {
    pub fn new() -> Self {
        FakeWebPushSender::default()
    }

    /// Từ nay endpoint `endpoint` trả "đã mất" (như HTTP 410 của dịch vụ đẩy).
    pub fn mark_gone(&self, endpoint: &str) {
        if let Ok(mut gone) = self.gone.lock() {
            gone.insert(endpoint.to_owned());
        }
    }

    /// Số tin đã gửi thành công.
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

/// Đẩy di động giả: giữ (token, nội dung) đã gửi; token trong `gone` trả [`PushDelivery::Gone`].
#[derive(Default)]
pub struct FakeMobilePushSender {
    pub sent: Mutex<Vec<(String, PushContent)>>,
    gone: Mutex<HashSet<String>>,
}

impl FakeMobilePushSender {
    pub fn new() -> Self {
        FakeMobilePushSender::default()
    }

    /// Từ nay token `token` trả "hết hiệu lực".
    pub fn mark_gone(&self, token: &str) {
        if let Ok(mut gone) = self.gone.lock() {
            gone.insert(token.to_owned());
        }
    }

    /// Số tin đã gửi thành công.
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
