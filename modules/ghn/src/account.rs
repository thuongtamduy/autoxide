//! Đọc tài khoản GHN của khách từ tài khoản hãng của `shipping` (`CarrierAccount`): `token` nằm
//! trong `credentials` (mã hoá, không bao giờ trả ra), `shop_id` và các tuỳ chọn nằm trong
//! `settings`. Không có giá trị nào lấy từ biến môi trường hay từ khách khác.

use autoxide_kernel::error::Error;
use autoxide_shipping::api::CarrierAccount;
use serde_json::Value;

use crate::errors;

/// Hướng dẫn giao hàng GHN nhận (`required_note`).
const REQUIRED_NOTES: [&str; 3] = ["KHONGCHOXEMHANG", "CHOXEMHANGKHONGTHU", "CHOTHUHANG"];

/// Khổ giấy in vận đơn → đoạn đường dẫn của trang in.
const LABEL_SIZES: [(&str, &str); 3] =
    [("a5", "printA5"), ("80x80", "print80x80"), ("52x70", "print52x70")];

/// Tài khoản GHN đã đọc. Không `Debug`: có token.
pub(crate) struct GhnAccount {
    pub token: String,
    pub shop_id: i64,
    pub sandbox: bool,
    /// `required_note` của đơn; mặc định không cho xem hàng.
    pub required_note: String,
    /// Ai trả phí vận chuyển cho GHN: 1 cửa hàng, 2 người nhận.
    pub payment_type_id: i64,
    /// Mô tả hàng gửi GHN (`content`).
    pub content: String,
    /// Đoạn đường dẫn của trang in (`printA5`…).
    pub label_page: &'static str,
}

/// `shop_id` viết số hay chuỗi số đều nhận.
fn whole_number(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `shop_id` của tài khoản: ở `settings`, hoặc (cho tiện) ở `credentials`.
pub(crate) fn shop_id(account: &CarrierAccount) -> Result<i64, Error> {
    account
        .settings
        .get("shop_id")
        .and_then(whole_number)
        .or_else(|| account.credentials.get("shop_id").and_then(|raw| raw.trim().parse().ok()))
        .filter(|id| *id > 0)
        .ok_or_else(|| errors::account_incomplete("shop_id"))
}

pub(crate) fn read(account: &CarrierAccount) -> Result<GhnAccount, Error> {
    let token = account
        .credentials
        .get("token")
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| errors::account_incomplete("token"))?
        .to_owned();
    let shop_id = shop_id(account)?;
    let text = |name: &str| account.settings.get(name).and_then(Value::as_str).map(str::trim);

    let required_note = match text("required_note") {
        None | Some("") => REQUIRED_NOTES[0].to_owned(),
        Some(note) if REQUIRED_NOTES.contains(&note) => note.to_owned(),
        Some(_) => {
            return Err(errors::setting_invalid(
                "required_note",
                "use KHONGCHOXEMHANG, CHOXEMHANGKHONGTHU or CHOTHUHANG",
            ));
        }
    };
    let payment_type_id = match account.settings.get("payment_type_id") {
        None | Some(Value::Null) => 1,
        Some(value) => match whole_number(value) {
            Some(id @ (1 | 2)) => id,
            _ => {
                return Err(errors::setting_invalid(
                    "payment_type_id",
                    "use 1 (the shop pays the fee) or 2 (the recipient pays)",
                ));
            }
        },
    };
    let label_page = match text("label_size") {
        None | Some("") => LABEL_SIZES[0].1,
        Some(size) => LABEL_SIZES
            .iter()
            .find(|(name, _)| *name == size)
            .map(|(_, page)| *page)
            .ok_or_else(|| errors::setting_invalid("label_size", "use a5, 80x80 or 52x70"))?,
    };
    let content = match text("content") {
        None | Some("") => "Hàng hóa".to_owned(),
        // GHN: tối đa 2000 ký tự.
        Some(content) => content.chars().take(2000).collect(),
    };
    Ok(GhnAccount {
        token,
        shop_id,
        sandbox: account.sandbox,
        required_note,
        payment_type_id,
        content,
        label_page,
    })
}
