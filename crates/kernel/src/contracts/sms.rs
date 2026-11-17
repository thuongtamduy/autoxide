//! Gửi SMS (bước 37): số điện thoại chuẩn E.164 và trait gửi tin trung lập (SMS brandname, D-190). Bộ
//! nối nhà cung cấp là module riêng công bố `context.provide::<dyn SmsSender>(…)`. Test dùng
//! `FakeSmsSender` của testkit.

use std::fmt;

use crate::error::Error;
use crate::tenancy::TenantId;

/// Số điện thoại dạng E.164 (`+84912345678`). Chỉ dạng che ([`masked`](Self::masked)) được ghi log.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PhoneNumber(String);

impl PhoneNumber {
    /// Chuẩn hoá `raw`: bỏ khoảng trắng, dấu chấm, gạch, ngoặc; `0…` → `+<mã nước mặc định>…`; `00…` →
    /// `+…`; `84…` (không `+`) với mã nước mặc định `84` → `+84…`. `None` khi không phải 8–15 chữ số.
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

    /// Dạng E.164, vd `+84912345678`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Mã nước (không `+`): khớp với `known` nếu số bắt đầu bằng một mã trong đó.
    pub fn calling_code<'a>(&self, known: &[&'a str]) -> Option<&'a str> {
        known.iter().copied().find(|code| self.0[1..].starts_with(code))
    }

    /// Dạng che để ghi log / hiện cho người dùng: `+8491****678`.
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

/// Mục đích của tin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmsPurpose {
    /// Mã một lần.
    Otp,
    /// Tin giao dịch (đơn hàng, giao hàng…).
    Transactional,
    /// Tin quảng cáo: chỉ gửi khi đã đồng ý trước, trong khung giờ cho phép (NĐ 91/2020 — cần đối chiếu).
    Marketing,
}

/// Một tin cần gửi.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SmsMessage {
    pub tenant: TenantId,
    pub to: PhoneNumber,
    pub text: String,
    pub purpose: SmsPurpose,
    /// Mã chống gửi trùng ở nhà cung cấp.
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

/// Biên nhận của nhà cung cấp.
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

/// Một nhà cung cấp gửi SMS.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait SmsSender: Send + Sync {
    fn provider(&self) -> &'static str;

    /// Lỗi `Unavailable` là tạm thời (thử lại); `Validation` là vĩnh viễn (số sai, bị chặn).
    async fn send(&self, message: SmsMessage) -> Result<SmsReceipt, Error>;
}

/// `sms_disabled` (503): chưa cấu hình gửi SMS.
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
