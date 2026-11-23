//! Contract of the `notifications` module for other modules (step 37, D-348). Other modules obtain
//! `context.require::<dyn Notifier>()` at startup (registered AFTER `notifications`), declare their
//! notification kinds with [`Notifier::declare`] right in `Module::start`, then send with
//! [`Notifier::notify`] from an event handler or job — NOT inside their own write transaction.
//!
//! Transactional and marketing messages are kept apart (Decree 91/2020/ND-CP on anti-spam messages and email;
//! Personal Data Protection Law 91/2025/QH15 — verify against the original texts): each kind belongs to
//! exactly one [`Category`] group; marketing messages are sent only when the `compliance` consent ledger
//! holds a consent still in effect, within the time windows allowed by the rule `notifications.send_windows`,
//! with an advertising label and an unsubscribe link.

use autoxide_kernel::error::Error;
pub use autoxide_kernel::template::Variable;
use autoxide_kernel::tenancy::TenantId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

/// Fake notification sender for other modules' tests (feature `testing`).
#[cfg(feature = "testing")]
pub mod testing;

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

    /// Channel in the `compliance` consent ledger (`email`, `sms`, `zns`, `push`, `in_app`).
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

/// Default template of a notification kind: the English text in code (placeholders `{name}`); the translation
/// lives in the declaring crate's `locales/<language>.json` under the keys `<key>.title`, `<key>.body`,
/// `<key>.short` (`cargo xtask check` verifies completeness). `short` is for SMS, ZNS and push notifications
/// (no HTML).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationTemplate {
    pub key: &'static str,
    pub title: &'static str,
    pub body: &'static str,
    pub short: &'static str,
}

impl NotificationTemplate {
    /// `key`: `<module>.<kind>`, also the key of the notification kind.
    pub const fn new(
        key: &'static str,
        title: &'static str,
        body: &'static str,
        short: &'static str,
    ) -> Self {
        NotificationTemplate { key, title, body, short }
    }
}

/// A notification kind declared by a module.
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
    /// Marketing messages: the purpose key in the consent ledger; `None` → `marketing`.
    pub purpose: Option<&'static str>,
    /// A message inviting opt-in to marketing: each address receives it only once per channel.
    pub invitation: bool,
    /// Variable schema for tenant-authored templates (D-206): the template variables that may be used
    /// (`{{ code }}`). Empty → derived from the `{name}` placeholders of the default template (all optional),
    /// plus `link`.
    pub variables: &'static [Variable],
    /// Sample data for preview (JSON object); `None` → generated from the schema (`"[name]"`).
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

    /// Declares the variable schema for tenant-authored templates (D-206), e.g. `Variable::required("link", "…")`.
    pub const fn variables(mut self, variables: &'static [Variable]) -> Self {
        self.variables = variables;
        self
    }

    /// Sample data (JSON object) for preview and test sends.
    pub const fn sample(mut self, sample: &'static str) -> Self {
        self.sample = Some(sample);
        self
    }

    /// Sets the purpose key in the consent ledger (marketing messages).
    pub const fn purpose(mut self, purpose: &'static str) -> Self {
        self.purpose = Some(purpose);
        self
    }

    /// Marks an opt-in invitation: sent at most once per (channel, address).
    pub const fn invitation(mut self) -> Self {
        self.invitation = true;
        self
    }

    pub const fn key(&self) -> &'static str {
        self.template.key
    }

    /// Consent purpose of the marketing message.
    pub fn consent_purpose(&self) -> &'static str {
        self.purpose.unwrap_or("marketing")
    }
}

/// Recipient.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Recipient {
    /// A tenant user: email from the account, in-app, push.
    User(Uuid),
    /// An email address that has no account (or none yet).
    Email(String),
    /// A phone number in E.164 (`+84912345678`).
    Phone(String),
    /// Every active administrator of the tenant (platform notifications to the tenant).
    TenantAdmins,
}

/// A notification to send.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    /// Key of the declared kind (`orders.order_shipped`).
    pub kind: String,
    pub recipient: Recipient,
    /// Values to fill into the template (`{"code": "DH001"}`) — display text only, no sensitive data.
    pub params: Value,
    /// Link opened on click (a page of the UI).
    pub link: Option<String>,
    /// Language (`vi`, `en`); `None`: the user's preferred language, English if none.
    pub locale: Option<String>,
    /// Required: calling again with the same key does not create a second message (e.g. the ID of the event that caused it).
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

/// Sends a notification.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait Notifier: Send + Sync {
    /// Declares a notification kind; call in `Module::start`. Error on a duplicate key, a key without the
    /// module prefix, or no channels.
    fn declare(&self, kind: NotificationKind) -> Result<(), Error>;

    /// Records the messages (one row per channel) + the send job in `notifications`' own transaction. Do NOT
    /// call inside another module's write transaction. The same `dedupe_key` → returns the existing messages
    /// and creates no more. A kind not declared → `notifications.kind_unknown`.
    async fn notify(&self, tenant: TenantId, notification: Notification) -> Result<Outcome, Error>;
}
