//! Mã VietQR (bước 36 mục 9, D-194): chuỗi theo chuẩn EMVCo MPM mà NAPAS dùng cho chuyển khoản
//! liên ngân hàng tới tài khoản (dịch vụ `QRIBFTTA`), nội dung chuyển khoản mang mã tham chiếu của
//! thanh toán. Riêng Việt Nam nên ở module, không ở kernel. Tự viết (vài chục dòng TLV + CRC), không
//! thêm thư viện.
//!
//! Căn cứ: đặc tả mã QR của NAPAS dựa trên EMVCo Merchant-Presented Mode; TCCS 03:2018/NHNN — cần
//! đối chiếu bản mới nhất. Test đối chiếu với mã mẫu đã công bố (CRC đúng từng ký tự).

use autoxide_kernel::money::{Currency, Money};
use uuid::Uuid;

/// GUID của NAPAS cho VietQR.
const NAPAS_GUID: &str = "A000000727";
/// Dịch vụ chuyển nhanh tới tài khoản.
const SERVICE_TO_ACCOUNT: &str = "QRIBFTTA";
/// Mã tiền VND theo ISO 4217 (số).
const CURRENCY_VND: &str = "704";

/// Tiền tố của mã chuyển khoản (chữ hoa, không dấu: ngân hàng hay bỏ dấu, cắt, chèn khoảng trắng).
pub(crate) const TRANSFER_PREFIX: &str = "AX";
/// Số ký tự sau tiền tố.
pub(crate) const TRANSFER_BODY_LEN: usize = 10;

/// CRC-16/CCITT-FALSE (đa thức 0x1021, khởi đầu 0xFFFF, không đảo bit) — trường `63` của EMV QR.
pub(crate) fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for byte in data {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

/// Một trường TLV: mã 2 chữ số, độ dài 2 chữ số (theo byte), giá trị. `None` khi quá 99 byte.
fn tlv(tag: &str, value: &str) -> Option<String> {
    let len = value.len();
    (len <= 99).then(|| format!("{tag}{len:02}{value}"))
}

/// Chuỗi VietQR động (có số tiền) chuyển `amount` (VND) tới tài khoản `account` của ngân hàng `bin`,
/// nội dung `memo`. `None` khi trường quá dài hay tiền không phải VND.
pub(crate) fn payload(bin: &str, account: &str, amount: Money, memo: &str) -> Option<String> {
    if amount.currency() != Currency::VND || !amount.is_positive() {
        return None;
    }
    let beneficiary = format!("{}{}", tlv("00", bin)?, tlv("01", account)?);
    let merchant = format!(
        "{}{}{}",
        tlv("00", NAPAS_GUID)?,
        tlv("01", &beneficiary)?,
        tlv("02", SERVICE_TO_ACCOUNT)?
    );
    let mut text = String::new();
    text.push_str(&tlv("00", "01")?);
    text.push_str(&tlv("01", "12")?);
    text.push_str(&tlv("38", &merchant)?);
    text.push_str(&tlv("53", CURRENCY_VND)?);
    text.push_str(&tlv("54", &amount.minor().to_string())?);
    text.push_str(&tlv("58", "VN")?);
    text.push_str(&tlv("62", &tlv("08", memo)?)?);
    text.push_str("6304");
    let crc = crc16(text.as_bytes());
    text.push_str(&format!("{crc:04X}"));
    Some(text)
}

/// Bảng chữ base32 (RFC 4648) — chỉ chữ hoa và số 2–7, không lẫn `0`/`O`, `1`/`I`.
const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Mã chuyển khoản của thanh toán `id`: `AX` + 10 ký tự base32 lấy từ phần ngẫu nhiên (10 byte cuối)
/// của UUIDv7 — 50 bit, unique index (khách, mã) chặn trùng.
pub(crate) fn transfer_code(id: Uuid) -> String {
    let bytes = id.as_bytes();
    let mut bits: u64 = 0;
    for byte in &bytes[9..16] {
        bits = (bits << 8) | u64::from(*byte);
    }
    let mut code = String::from(TRANSFER_PREFIX);
    for index in (0..TRANSFER_BODY_LEN).rev() {
        let digit = (bits >> (index * 5)) & 0x1F;
        code.push(char::from(BASE32[usize::try_from(digit).unwrap_or(0)]));
    }
    code
}

/// Các mã chuyển khoản có thể có trong nội dung `memo` ngân hàng ghi: bỏ mọi ký tự không phải chữ
/// hay số (khoảng trắng, dấu chấm ngân hàng chèn), không phân biệt hoa thường.
pub(crate) fn codes_in(memo: &str) -> Vec<String> {
    let clean: Vec<char> =
        memo.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).collect();
    let width = TRANSFER_PREFIX.len() + TRANSFER_BODY_LEN;
    let mut found = Vec::new();
    for start in 0..clean.len() {
        if clean.len() - start < width {
            break;
        }
        let candidate: String = clean[start..start + width].iter().collect();
        if candidate.starts_with(TRANSFER_PREFIX)
            && candidate[TRANSFER_PREFIX.len()..].bytes().all(|b| BASE32.contains(&b))
            && !found.contains(&candidate)
        {
            found.push(candidate);
        }
    }
    found
}

/// Một ngân hàng trong danh sách BIN (tên, mã viết tắt ở file dữ liệu cho người đọc).
#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct Bank {
    pub bin: String,
}

#[derive(Debug, serde::Deserialize)]
struct BankList {
    banks: Vec<Bank>,
}

/// Danh sách BIN ngân hàng (dữ liệu, `data/vn-banks.json`).
pub(crate) fn banks() -> Vec<Bank> {
    serde_json::from_str::<BankList>(include_str!("../data/vn-banks.json"))
        .map(|list| list.banks)
        .unwrap_or_default()
}

pub(crate) fn bank(bin: &str) -> Option<Bank> {
    banks().into_iter().find(|bank| bank.bin == bin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_dung_gia_tri_kiem_chuan() {
        // Giá trị kiểm của CRC-16/CCITT-FALSE.
        assert_eq!(crc16(b"123456789"), 0x29B1);
        // Mẫu của đặc tả EMVCo MPM (có chữ Trung Quốc UTF-8): CRC A13A.
        let sample = "00020101021229300012D156000000000510A93FO3230Q31280012D15600000001030812345678520441115802CN5914BEST TRANSPORT6007BEIJING64200002ZH0104最佳运输0202北京540523.7253031565502016233030412340603***0708A60086670902ME91320016A0112233449988770708123456786304A13A";
        let (body, crc) = sample.split_at(sample.len() - 4);
        assert_eq!(format!("{:04X}", crc16(body.as_bytes())), crc);
    }

    #[test]
    fn vietqr_payload_khop_mau_napas() {
        // Mã VietQR động đã công bố (ACB 970416, tài khoản 257678859, 10.000 đ, "Chuyen tien"):
        // dựng lại phải trùng từng ký tự, kể cả CRC.
        let sample = "00020101021238530010A0000007270123000697041601092576788590208QRIBFTTA53037045405100005802VN62150811Chuyen tien630453E6";
        let built =
            payload("970416", "257678859", Money::new(10_000, Currency::VND), "Chuyen tien");
        assert_eq!(built.as_deref(), Some(sample));
        let other = "00020101021238530010A0000007270123000697041601092245284790208QRIBFTTA53037045405100005802VN62340107NPS68690819TRANSFER TO SOMEONE6304AC13";
        let (body, crc) = other.split_at(other.len() - 4);
        assert_eq!(format!("{:04X}", crc16(body.as_bytes())), crc);
        assert!(payload("970416", "1", Money::new(1, Currency::USD), "x").is_none());
    }

    #[test]
    fn ma_chuyen_khoan_tim_lai_duoc_trong_noi_dung() {
        let id = Uuid::now_v7();
        let code = transfer_code(id);
        assert_eq!(code.len(), 12);
        assert!(code.starts_with("AX"));
        // Ngân hàng chèn khoảng trắng, đổi chữ thường, thêm chữ khác.
        let spaced =
            format!("MBVCB.123 {} chuyen tien", code.to_lowercase().replacen('x', "x ", 1));
        assert_eq!(codes_in(&spaced), vec![code.clone()]);
        assert!(codes_in("thanh toan don hang").is_empty());
        assert!(bank("970436").is_some());
        assert!(bank("000000").is_none());
    }
}
