//! Push notifications (step 37, D-348): neutral traits for web push (browsers, RFC 8030) and push to mobile
//! apps (D-220). The real implementations — web push (VAPID RFC 8292, payload encryption RFC 8291),
//! Firebase Cloud Messaging, Apple Push Notification service — are separate modules that need the provider's
//! keys, publish `context.provide::<dyn WebPushSender>(…)` / `dyn MobilePushSender`, and register BEFORE the
//! `notifications` module. Tests use `FakeWebPushSender` / `FakeMobilePushSender` from the testkit.

use crate::error::Error;
use crate::tenancy::TenantId;

/// Result of one push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushDelivery {
    /// The provider accepted it; carries the provider's message id.
    Sent { provider_reference: String },
    /// The subscription / token is no longer usable (web push returned 404 / 410, or the mobile token
    /// expired): the caller deletes it and does not resend.
    Gone,
}

/// Display content of a push notification.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PushContent {
    pub title: String,
    pub body: String,
    /// Link opened when the notification is clicked.
    pub link: Option<String>,
    /// Id that prevents duplicate delivery at the provider.
    pub reference: String,
}

impl PushContent {
    pub fn new(
        title: impl Into<String>,
        body: impl Into<String>,
        link: Option<String>,
        reference: impl Into<String>,
    ) -> Self {
        PushContent { title: title.into(), body: body.into(), link, reference: reference.into() }
    }
}

/// A web push subscription of one browser (the browser's `PushSubscription`).
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebPushSubscription {
    pub endpoint: String,
    /// The browser's P-256 public key (base64url).
    pub p256dh: String,
    /// 16-byte authentication secret (base64url).
    pub auth: String,
}

impl WebPushSubscription {
    pub fn new(
        endpoint: impl Into<String>,
        p256dh: impl Into<String>,
        auth: impl Into<String>,
    ) -> Self {
        WebPushSubscription { endpoint: endpoint.into(), p256dh: p256dh.into(), auth: auth.into() }
    }
}

impl std::fmt::Debug for WebPushSubscription {
    // Không in secret `auth`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebPushSubscription")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

/// Sender of web push.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait WebPushSender: Send + Sync {
    fn provider(&self) -> &'static str;

    /// VAPID public key (base64url) for browsers to subscribe with.
    fn public_key(&self) -> String;

    /// The `Unavailable` error is temporary (retry); `Validation` is permanent.
    ///
    /// `subscription.endpoint` was written by a browser user, so the sender must treat it as an untrusted
    /// address: connect only to public IPs, resolving the host right before connecting
    /// ([`crate::net::resolve_target`] or an HTTP client built with [`crate::net::PublicOnly`]) — never
    /// to a private, loopback or link-local address (cloud metadata). `notifications` already refuses
    /// such endpoints when saving, but a domain name can point anywhere later.
    async fn send(
        &self,
        tenant: TenantId,
        subscription: &WebPushSubscription,
        content: &PushContent,
    ) -> Result<PushDelivery, Error>;
}

/// Token of one mobile device.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeviceToken {
    /// `ios` or `android`.
    pub platform: String,
    pub token: String,
    /// Application id (bundle id / package name).
    pub app_id: String,
}

impl DeviceToken {
    pub fn new(
        platform: impl Into<String>,
        token: impl Into<String>,
        app_id: impl Into<String>,
    ) -> Self {
        DeviceToken { platform: platform.into(), token: token.into(), app_id: app_id.into() }
    }
}

impl std::fmt::Debug for DeviceToken {
    // Không in token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceToken")
            .field("platform", &self.platform)
            .field("app_id", &self.app_id)
            .finish_non_exhaustive()
    }
}

/// Sender of push notifications to mobile apps (FCM, APNs…).
#[async_trait::async_trait]
pub trait MobilePushSender: Send + Sync {
    fn provider(&self) -> &'static str;

    /// The `Unavailable` error is temporary (retry); `Validation` is permanent.
    async fn send(
        &self,
        tenant: TenantId,
        device: &DeviceToken,
        content: &PushContent,
    ) -> Result<PushDelivery, Error>;
}

/// `web_push_disabled` (503): web push is not configured.
pub fn web_push_disabled() -> Error {
    Error::unavailable("web_push_disabled", "Web push notifications are not set up on this server.")
}

/// `mobile_push_disabled` (503): push to mobile apps is not configured.
pub fn mobile_push_disabled() -> Error {
    Error::unavailable(
        "mobile_push_disabled",
        "Push notifications to mobile apps are not set up on this server.",
    )
}
