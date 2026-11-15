//! Tên loại authenticator theo AAGUID (D-272). Dữ liệu ở `data/passkey-authenticators.json`: chỉ
//! cặp "mã → tên sản phẩm" do từng hãng công bố, đối chiếu với danh sách cộng đồng
//! `passkeydeveloper/passkey-authenticator-aaguids` (không lấy biểu tượng). Mã không có trong bảng:
//! không có tên — giao diện hiện tên người dùng đặt.

use std::collections::HashMap;
use std::sync::LazyLock;

use uuid::Uuid;

static NAMES: LazyLock<HashMap<Uuid, String>> = LazyLock::new(|| {
    // Tệp đi kèm mã nguồn, test kiểm nó đọc được; hỏng thì chỉ mất tên, không mất passkey.
    serde_json::from_str(include_str!("../../data/passkey-authenticators.json")).unwrap_or_default()
});

/// Tên loại authenticator của `aaguid`, khi biết.
pub(crate) fn name(aaguid: Uuid) -> Option<&'static str> {
    NAMES.get(&aaguid).map(String::as_str)
}

/// AAGUID lấy từ dữ liệu đăng ký: `None` khi không đủ 16 byte hay toàn số 0 (authenticator không
/// khai, vd khi không xin attestation).
pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Uuid> {
    Uuid::from_slice(bytes).ok().filter(|aaguid| !aaguid.is_nil())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bang_ten_doc_duoc_va_nhan_ra_loai_quen() {
        assert!(NAMES.len() > 20);
        let hello = Uuid::parse_str("08987058-cadc-4b81-b6e1-30de50dcbe96").unwrap();
        assert_eq!(name(hello), Some("Windows Hello"));
        assert_eq!(name(Uuid::nil()), None);
        assert_eq!(from_bytes(&[0; 16]), None);
        assert_eq!(from_bytes(hello.as_bytes()), Some(hello));
        assert_eq!(from_bytes(&[1, 2, 3]), None);
    }
}
