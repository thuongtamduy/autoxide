//! Test của `breached.rs`, ở file riêng (D-20261127-codeql-skip-tests: CodeQL bỏ qua `tests.rs`, nơi mật khẩu mẫu
//! của test không phải lỗ hổng).

use super::*;

#[test]
fn danh_sach_da_sap_va_khong_trung() {
    // Tìm nhị phân chỉ đúng khi danh sách sắp đúng thứ tự byte: kiểm ở đây để ai cập nhật file
    // mà quên sắp thì test đỏ ngay.
    assert!(ENTRIES.len() > 50_000, "{}", ENTRIES.len());
    assert!(ENTRIES.windows(2).all(|pair| pair[0].as_bytes() < pair[1].as_bytes()));
    assert!(ENTRIES.iter().all(|entry| *entry == entry.to_lowercase()));
    assert!(ENTRIES.iter().all(|entry| *entry == entry.trim()));
    // Mục ngắn hơn mức tối thiểu không bao giờ được tra (luật độ dài chặn trước): không giữ.
    let lengths = crate::password::MIN_LENGTH..=crate::password::MAX_LENGTH;
    assert!(ENTRIES.iter().all(|entry| lengths.contains(&entry.chars().count())));
}

#[test]
fn chan_mat_khau_trong_danh_sach_khong_phan_biet_hoa_thuong() {
    assert!(is_common("123456789012345"));
    assert!(is_common("qwertyuiopasdfghjkl"));
    assert!(is_common("PassWord123456789"));
    assert!(is_common(" password123456789 "));
    assert!(is_common("aaaaaaaaaaaaaa"));
    assert!(is_common("ééééééééééééé"));
}

#[test]
fn mat_khau_kho_doan_thi_qua() {
    assert!(!is_common("correct horse battery staple"));
    assert!(!is_common("mậtkhẩuđủdài"));
    assert!(!is_common("a brand new long password"));
    assert!(!is_common(""));
}
