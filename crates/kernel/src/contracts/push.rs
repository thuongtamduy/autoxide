//! Thông báo đẩy (bước 37, D-348): trait trung lập cho web push (trình duyệt, RFC 8030) và đẩy tới ứng
//! dụng di động (D-220). Bộ cài đặt thật — web push (VAPID RFC 8292, mã hoá nội dung RFC 8291),
//! Firebase Cloud Messaging, Apple Push Notification service — là module riêng cần khoá của nhà cung
//! cấp, công bố `context.provide::<dyn WebPushSender>(…)` / `dyn MobilePushSender` và đăng ký TRƯỚC
//! module `notifications`. Test dùng `FakeWebPushSender` / `FakeMobilePushSender` của testkit.

use crate::error::Error;
use crate::tenancy::TenantId;

/// Kết quả một lần đẩy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushDelivery {
    /// Nhà cung cấp đã nhận; kèm mã tin của họ.
    Sent { provider_reference: String },
    /// Đăng ký / token không còn dùng được (web push trả 404 / 410, token di động hết hiệu lực): bên
    /// gọi xoá nó, không gửi lại.
    Gone,
}

/// Nội dung hiển thị của một thông báo đẩy.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PushContent {
    pub title: String,
    pub body: String,
    /// Link mở khi bấm vào thông báo.
    pub link: Option<String>,
    /// Mã chống gửi trùng ở nhà cung cấp.
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

/// Đăng ký web push của một trình duyệt (`PushSubscription` của trình duyệt).
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebPushSubscription {
    pub endpoint: String,
    /// Khoá công khai P-256 của trình duyệt (base64url).
    pub p256dh: String,
    /// Secret xác thực 16 byte (base64url).
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

/// Bên gửi web push.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait WebPushSender: Send + Sync {
    fn provider(&self) -> &'static str;

    /// Khoá công khai VAPID (base64url) cho trình duyệt đăng ký nhận.
    fn public_key(&self) -> String;

    /// Lỗi `Unavailable` là tạm thời (thử lại); `Validation` là vĩnh viễn.
    async fn send(
        &self,
        tenant: TenantId,
        subscription: &WebPushSubscription,
        content: &PushContent,
    ) -> Result<PushDelivery, Error>;
}

/// Token của một thiết bị di động.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeviceToken {
    /// `ios` hay `android`.
    pub platform: String,
    pub token: String,
    /// Mã ứng dụng (bundle id / package name).
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

/// Bên đẩy thông báo tới ứng dụng di động (FCM, APNs…).
#[async_trait::async_trait]
pub trait MobilePushSender: Send + Sync {
    fn provider(&self) -> &'static str;

    /// Lỗi `Unavailable` là tạm thời (thử lại); `Validation` là vĩnh viễn.
    async fn send(
        &self,
        tenant: TenantId,
        device: &DeviceToken,
        content: &PushContent,
    ) -> Result<PushDelivery, Error>;
}

/// `web_push_disabled` (503): chưa cấu hình web push.
pub fn web_push_disabled() -> Error {
    Error::unavailable("web_push_disabled", "Web push notifications are not set up on this server.")
}

/// `mobile_push_disabled` (503): chưa cấu hình đẩy tới ứng dụng di động.
pub fn mobile_push_disabled() -> Error {
    Error::unavailable(
        "mobile_push_disabled",
        "Push notifications to mobile apps are not set up on this server.",
    )
}
