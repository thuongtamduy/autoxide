//! Sending SMS (step 37): E.164 phone numbers and a neutral sending trait (SMS brandname, D-190). A provider
//! adapter is its own module that publishes `context.provide::<dyn SmsSender>(...)`. Tests use
//! `FakeSmsSender` from the testkit.

use std::fmt;

use crate::error::Error;
use crate::tenancy::TenantId;

/// Phone number in E.164 form (`+84912345678`). Only the masked form ([`masked`](Self::masked)) may be logged.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PhoneNumber(String);

impl PhoneNumber {
    /// Normalizes `raw`: strips spaces, dots, dashes, brackets; `0...` -> `+<default country code>...`;
    /// `00...` -> `+...`; `84...` (no `+`) with default country code `84` -> `+84...`. `None` when it is not
    /// 8-15 digits.
    ///
    /// ```
    /// use autoxide_kernel::sms::PhoneNumber;
    /// assert_eq!(PhoneNumber::parse("0912 345 678", "84").unwrap().as_str(), "+84912345678");
    /// ```
    pub fn parse(raw: &str, default_country_code: &str) -> Option<Self> {
        let trimmed = raw.trim();
        let plus = trimmed.starts_with('+');
        let digits: String =
            trimmed.chars().filter(|c| !matches!(c, ' ' | '.' | '-' | '(' | ')' | '+')).collect();
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let international = if plus {
            digits
        } else if let Some(rest) = digits.strip_prefix("00") {
            rest.to_owned()
        } else if let Some(rest) = digits.strip_prefix('0') {
            format!("{default_country_code}{rest}")
        } else if digits.starts_with(default_country_code) && digits.len() > 9 {
            digits
        } else {
            return None;
        };
        if !(8..=15).contains(&international.len()) || international.starts_with('0') {
            return None;
        }
        Some(PhoneNumber(format!("+{international}")))
    }

    /// E.164 form, e.g. `+84912345678`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Country code (without `+`): matches `known` if the number starts with one of the codes in it.
    pub fn calling_code<'a>(&self, known: &[&'a str]) -> Option<&'a str> {
        known.iter().copied().find(|code| self.0[1..].starts_with(code))
    }

    /// Masked form for logs / showing to users: `+8491****678`.
    pub fn masked(&self) -> String {
        let text = &self.0;
        if text.len() <= 7 {
            return "****".to_owned();
        }
        format!("{}****{}", &text[..5], &text[text.len() - 3..])
    }
}

impl fmt::Debug for PhoneNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PhoneNumber({})", self.masked())
    }
}

/// Purpose of the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmsPurpose {
    /// One-time code.
    Otp,
    /// Transactional message (orders, delivery, ...).
    Transactional,
    /// Advertising message: sent only with prior consent, within the allowed hours (Decree 91/2020; needs verification).
    Marketing,
}

/// A message to send.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SmsMessage {
    pub tenant: TenantId,
    pub to: PhoneNumber,
    pub text: String,
    pub purpose: SmsPurpose,
    /// Code that prevents duplicate sends at the provider.
    pub reference: String,
}

impl SmsMessage {
    pub fn new(
        tenant: TenantId,
        to: PhoneNumber,
        text: impl Into<String>,
        purpose: SmsPurpose,
        reference: impl Into<String>,
    ) -> Self {
        SmsMessage { tenant, to, text: text.into(), purpose, reference: reference.into() }
    }
}

/// Provider's receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SmsReceipt {
    pub provider_reference: String,
}

impl SmsReceipt {
    pub fn new(provider_reference: impl Into<String>) -> Self {
        SmsReceipt { provider_reference: provider_reference.into() }
    }
}

/// An SMS sending provider.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait SmsSender: Send + Sync {
    fn provider(&self) -> &'static str;

    /// The `Unavailable` error is temporary (retry); `Validation` is permanent (wrong number, blocked).
    async fn send(&self, message: SmsMessage) -> Result<SmsReceipt, Error>;
}

/// Token issued by a module after the user has passed a one-time code sent to a phone number (e.g. the guest
/// order lookup token of `orders`). `auth` asks through this trait to create an account from a verified number
/// (`POST /api/v1/auth/phone-signups`, D-202) without depending on the module that issues the token. The module
/// that issues tokens publishes it with `context.provide::<dyn VerifiedPhoneTokens>(...)`; `auth` (registered
/// earlier) gets it with `StartContext::later`.
#[async_trait::async_trait]
pub trait VerifiedPhoneTokens: Send + Sync {
    /// The number that passed the one-time code for `token`, when the token is still valid and belongs to tenant `tenant`; otherwise `None`.
    async fn verified_phone(
        &self,
        tenant: &crate::tenancy::TenantContext,
        token: &str,
    ) -> Result<Option<PhoneNumber>, Error>;
}

/// `sms_disabled` (503): SMS sending is not configured.
pub fn sms_disabled() -> Error {
    Error::unavailable("sms_disabled", "Sending SMS is not set up on this server.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chuan_hoa_so_viet_nam() {
        for raw in [
            "0912 345 678",
            "0912.345.678",
            "(091) 234-5678",
            "+84 912 345 678",
            "84912345678",
            "0084912345678",
        ] {
            let parsed = PhoneNumber::parse(raw, "84").unwrap_or_else(|| panic!("{raw}"));
            assert_eq!(parsed.as_str(), "+84912345678", "{raw}");
        }
        for bad in ["", "abc", "12", "0", "+", "09123456789012345"] {
            assert!(PhoneNumber::parse(bad, "84").is_none(), "{bad}");
        }
        let number = PhoneNumber::parse("0912345678", "84").unwrap();
        assert_eq!(number.masked(), "+8491****678");
        assert_eq!(number.calling_code(&["1", "84"]), Some("84"));
        assert!(!format!("{number:?}").contains("345"));
    }
}
