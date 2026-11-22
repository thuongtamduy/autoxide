//! Hợp đồng của module `notifications` cho module khác (bước 37, D-348). Module khác lấy lúc khởi động
//! `context.require::<dyn Notifier>()` (đăng ký SAU `notifications`), khai loại thông báo của mình bằng
//! [`Notifier::declare`] ngay trong `Module::start`, rồi gửi bằng [`Notifier::notify`] từ handler event
//! hay job — KHÔNG trong transaction ghi của mình.
//!
//! Tách tin giao dịch và tin quảng cáo (NĐ 91/2020/NĐ-CP về chống tin nhắn, thư điện tử rác; Luật Bảo
//! vệ dữ liệu cá nhân 91/2025/QH15 — cần đối chiếu văn bản gốc): mỗi loại thuộc đúng một nhóm
//! [`Category`]; tin quảng cáo chỉ gửi khi sổ đồng ý của `compliance` có đồng ý còn hiệu lực, trong
//! khung giờ quy tắc `notifications.send_windows` cho phép, kèm nhãn quảng cáo và link huỷ nhận.

use autoxide_kernel::error::Error;
pub use autoxide_kernel::template::Variable;
use autoxide_kernel::tenancy::TenantId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

/// Group of a notification kind: decides whether preferences and consent apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = NotificationCategory)]
pub enum Category {
    /// Account security (sign-in, password, MFA): always sent, cannot be turned off.
    Security,
    /// Caused by something the person did or asked for (order, booking, invoice): on unless the
    /// person turns it off.
    Transactional,
    /// Advertising: only with a recorded consent, inside the allowed hours, with an advertising
    /// label and an unsubscribe link.
    Marketing,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Security => "security",
            Category::Transactional => "transactional",
            Category::Marketing => "marketing",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        [Category::Security, Category::Transactional, Category::Marketing]
            .into_iter()
            .find(|category| category.as_str() == raw)
    }
}

/// Where a notification goes.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[schema(as = NotificationChannel)]
pub enum Channel {
    Email,
    Sms,
    /// Zalo ZNS (approved templates of a Zalo Official Account).
    Zns,
    /// Browser push (RFC 8030).
    WebPush,
    /// The in-app inbox (`GET /api/v1/notifications/me/inbox`).
    InApp,
    /// Push to a mobile app (FCM, APNs).
    MobilePush,
}

impl Channel {
    pub const ALL: [Channel; 6] = [
        Channel::Email,
        Channel::Sms,
        Channel::Zns,
        Channel::WebPush,
        Channel::InApp,
        Channel::MobilePush,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Email => "email",
            Channel::Sms => "sms",
            Channel::Zns => "zns",
            Channel::WebPush => "web_push",
            Channel::InApp => "in_app",
            Channel::MobilePush => "mobile_push",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Channel::ALL.into_iter().find(|channel| channel.as_str() == raw)
    }

    /// Kênh trong sổ đồng ý của `compliance` (`email`, `sms`, `zns`, `push`, `in_app`).
    pub fn consent_channel(self) -> &'static str {
        match self {
            Channel::Email => "email",
            Channel::Sms => "sms",
            Channel::Zns => "zns",
            Channel::WebPush | Channel::MobilePush => "push",
            Channel::InApp => "in_app",
        }
    }
}

/// Mẫu mặc định của một loại thông báo: câu tiếng Anh trong code (chỗ điền `{tên}`), bản dịch ở
/// `locales/<ngôn ngữ>.json` của crate khai báo với khoá `<key>.title`, `<key>.body`, `<key>.short`
/// (`cargo xtask check` kiểm đủ). `short` cho SMS, ZNS và thông báo đẩy (không HTML).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationTemplate {
    pub key: &'static str,
    pub title: &'static str,
    pub body: &'static str,
    pub short: &'static str,
}

impl NotificationTemplate {
    /// `key`: `<module>.<loại>`, cũng là khoá của loại thông báo.
    pub const fn new(
        key: &'static str,
        title: &'static str,
        body: &'static str,
        short: &'static str,
    ) -> Self {
        NotificationTemplate { key, title, body, short }
    }
}

/// Một loại thông báo do module khai.
///
/// ```
/// use autoxide_notifications::api::{Category, Channel, NotificationKind, NotificationTemplate};
///
/// const SHIPPED: NotificationKind = NotificationKind::new(
///     Category::Transactional,
///     &[Channel::Email, Channel::InApp],
///     NotificationTemplate::new(
///         "orders.order_shipped",
///         "Order {code} is on its way",
///         "Your order {code} was handed to the carrier.",
///         "Order {code} shipped",
///     ),
/// );
/// assert_eq!(SHIPPED.key(), "orders.order_shipped");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationKind {
    pub category: Category,
    pub channels: &'static [Channel],
    pub template: NotificationTemplate,
    /// Tin quảng cáo: khoá mục đích trong sổ đồng ý; `None` → `marketing`.
    pub purpose: Option<&'static str>,
    /// Tin mời đăng ký nhận quảng cáo: mỗi địa chỉ chỉ nhận một lần trên mỗi kênh.
    pub invitation: bool,
    /// Schema biến cho mẫu khách tự soạn (D-206): biến mẫu được dùng (`{{ code }}`). Rỗng → suy ra từ
    /// chỗ điền `{tên}` của mẫu mặc định (đều không bắt buộc), cộng `link`.
    pub variables: &'static [Variable],
    /// Dữ liệu mẫu để xem trước (JSON object); `None` → sinh từ schema (`"[tên]"`).
    pub sample: Option<&'static str>,
}

impl NotificationKind {
    pub const fn new(
        category: Category,
        channels: &'static [Channel],
        template: NotificationTemplate,
    ) -> Self {
        NotificationKind {
            category,
            channels,
            template,
            purpose: None,
            invitation: false,
            variables: &[],
            sample: None,
        }
    }

    /// Khai schema biến cho mẫu khách tự soạn (D-206), vd `Variable::required("link", "…")`.
    pub const fn variables(mut self, variables: &'static [Variable]) -> Self {
        self.variables = variables;
        self
    }

    /// Dữ liệu mẫu (JSON object) cho xem trước và gửi thử.
    pub const fn sample(mut self, sample: &'static str) -> Self {
        self.sample = Some(sample);
        self
    }

    /// Đặt khoá mục đích trong sổ đồng ý (tin quảng cáo).
    pub const fn purpose(mut self, purpose: &'static str) -> Self {
        self.purpose = Some(purpose);
        self
    }

    /// Đánh dấu tin mời đăng ký: gửi tối đa một lần cho mỗi (kênh, địa chỉ).
    pub const fn invitation(mut self) -> Self {
        self.invitation = true;
        self
    }

    pub const fn key(&self) -> &'static str {
        self.template.key
    }

    /// Mục đích đồng ý của tin quảng cáo.
    pub fn consent_purpose(&self) -> &'static str {
        self.purpose.unwrap_or("marketing")
    }
}

/// Người nhận.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Recipient {
    /// Một người dùng của khách: email từ tài khoản, in-app, push.
    User(Uuid),
    /// Một địa chỉ email chưa (hay không) có tài khoản.
    Email(String),
    /// Một số điện thoại E.164 (`+84912345678`).
    Phone(String),
    /// Mọi quản trị viên còn hoạt động của khách (thông báo của nền tảng tới khách).
    TenantAdmins,
}

/// Một thông báo cần gửi.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    /// Khoá của loại đã khai (`orders.order_shipped`).
    pub kind: String,
    pub recipient: Recipient,
    /// Giá trị điền vào mẫu (`{"code": "DH001"}`) — chỉ chữ hiển thị, không dữ liệu nhạy cảm.
    pub params: Value,
    /// Link mở khi bấm (trang của giao diện).
    pub link: Option<String>,
    /// Ngôn ngữ (`vi`, `en`); `None`: ngôn ngữ ưa thích của người dùng, không có thì tiếng Anh.
    pub locale: Option<String>,
    /// Bắt buộc: gọi lại cùng khoá thì không tạo tin thứ hai (vd ID của event gây ra tin).
    pub dedupe_key: String,
}

impl Notification {
    pub fn new(
        kind: impl Into<String>,
        recipient: Recipient,
        params: Value,
        dedupe_key: impl Into<String>,
    ) -> Self {
        Notification {
            kind: kind.into(),
            recipient,
            params,
            link: None,
            locale: None,
            dedupe_key: dedupe_key.into(),
        }
    }

    pub fn link(mut self, link: impl Into<String>) -> Self {
        self.link = Some(link.into());
        self
    }

    pub fn locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = Some(locale.into());
        self
    }
}

/// Why a channel was not used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = NotificationSkipReason)]
pub enum SkipReason {
    /// Advertising without a recorded consent.
    NoConsent,
    /// The consent was withdrawn before sending.
    ConsentWithdrawn,
    /// The person turned this kind or channel off.
    PreferenceOff,
    /// No address on this channel (no email, phone number, push subscription…).
    NoAddress,
    /// The server has no provider for this channel, or the shop has not set up its account at the
    /// provider (`provider.not_configured`).
    ChannelDisabled,
    /// Advertising that could not go out within 24 hours of the allowed hours.
    QuietHoursExpired,
    /// The single invitation to this address was already sent.
    InvitationUsed,
    /// Too many notifications to this recipient in the last hour.
    Throttled,
}

impl SkipReason {
    pub fn as_str(self) -> &'static str {
        match self {
            SkipReason::NoConsent => "no_consent",
            SkipReason::ConsentWithdrawn => "consent_withdrawn",
            SkipReason::PreferenceOff => "preference_off",
            SkipReason::NoAddress => "no_address",
            SkipReason::ChannelDisabled => "channel_disabled",
            SkipReason::QuietHoursExpired => "quiet_hours_expired",
            SkipReason::InvitationUsed => "invitation_used",
            SkipReason::Throttled => "throttled",
        }
    }
}

/// A channel that was not used, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationSkipped)]
pub struct Skipped {
    pub channel: Channel,
    pub reason: SkipReason,
}

/// What `notify` did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationOutcome)]
pub struct Outcome {
    /// Messages queued for delivery (or already queued with the same `dedupe_key`).
    pub message_ids: Vec<Uuid>,
    /// Channels not used.
    pub skipped: Vec<Skipped>,
}

/// Gửi thông báo.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait Notifier: Send + Sync {
    /// Khai một loại thông báo; gọi trong `Module::start`. Khai trùng khoá, khoá không mang tiền tố
    /// module hay không có kênh nào → lỗi.
    fn declare(&self, kind: NotificationKind) -> Result<(), Error>;

    /// Ghi tin (mỗi kênh một dòng) + job gửi trong transaction riêng của `notifications`. KHÔNG gọi
    /// trong transaction ghi của module khác. Cùng `dedupe_key` → trả lại các tin đã có, không tạo
    /// thêm. Loại chưa khai → `notifications.kind_unknown`.
    async fn notify(&self, tenant: TenantId, notification: Notification) -> Result<Outcome, Error>;
}
