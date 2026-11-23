//! Sending Zalo ZNS messages (step 37, D-348): a neutral trait for template messages approved by Zalo. The Zalo
//! adapter is its own module (needs the Zalo Official Account key) that publishes
//! `context.provide::<dyn ZnsSender>(...)` and is registered BEFORE the `notifications` module. Tests use
//! `FakeZnsSender` from the testkit.

use serde_json::Value;

use crate::error::Error;
use crate::sms::PhoneNumber;
use crate::tenancy::TenantId;

/// A ZNS message to send: the approved template code and the values of the template's parameters.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ZnsMessage {
    pub tenant: TenantId,
    pub to: PhoneNumber,
    /// ZNS template code approved by Zalo.
    pub template_id: String,
    /// Values of the template's parameters (`{"order_code": "..."}`).
    pub params: Value,
    /// Code that prevents duplicate sends at the provider.
    pub reference: String,
}

impl ZnsMessage {
    pub fn new(
        tenant: TenantId,
        to: PhoneNumber,
        template_id: impl Into<String>,
        params: Value,
        reference: impl Into<String>,
    ) -> Self {
        ZnsMessage {
            tenant,
            to,
            template_id: template_id.into(),
            params,
            reference: reference.into(),
        }
    }
}

/// A provider that sends ZNS messages.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait ZnsSender: Send + Sync {
    fn provider(&self) -> &'static str;

    /// Returns the message code at the provider. The `Unavailable` error is temporary (retry); `Validation` is
    /// permanent (the number does not use Zalo, the template is not approved).
    async fn send(&self, message: ZnsMessage) -> Result<String, Error>;
}

/// `zns_disabled` (503): Zalo ZNS sending is not configured.
pub fn zns_disabled() -> Error {
    Error::unavailable("zns_disabled", "Sending Zalo ZNS messages is not set up on this server.")
}
