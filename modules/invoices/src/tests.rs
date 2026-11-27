//! Test thuần của `invoices`. Test chạy cùng `tax` và `ledger` (phát hành, ghi sổ, hoá đơn điện tử)
//! nằm ở `crates/app/src/e2e/invoices.rs`: luật ranh giới cấm module dựng module khác.

use crate::service::{retention_until, valid_tax_code};
use autoxide_kernel::time::Date;

#[test]
fn ma_so_thue_nguoi_mua() {
    for good in ["0101234567", "0101234567001", "0101234567-001", "001099012345"] {
        assert!(valid_tax_code(good), "{good}");
    }
    for bad in ["", "123", "01012345678", "0101234567-01", "abcdefghij"] {
        assert!(!valid_tax_code(bad), "{bad}");
    }
}

#[test]
fn han_luu_la_het_nam_cong_10() {
    let day = |y, m, d| Date::new(y, m, d).unwrap();
    assert_eq!(retention_until(day(2026, 3, 5)), day(2036, 12, 31));
    assert_eq!(retention_until(day(2026, 12, 31)), day(2036, 12, 31));
}

/// Nhà cung cấp không có lệnh huỷ (SePay eInvoice): lỗi rõ hướng điều chỉnh / thay thế (kiểm độc lập #575, N2).
#[test]
fn nha_cung_cap_khong_huy_duoc_thi_bao_dieu_chinh() {
    use crate::issue::provider_cancel_error;
    let mapped = provider_cancel_error(autoxide_kernel::einvoice::not_supported("sepay"), "sepay");
    assert_eq!(mapped.code(), "invoices.einvoice_cancel_unsupported");
    assert!(mapped.message().contains("adjust"), "{}", mapped.message());
    let other = autoxide_kernel::error::Error::conflict("einvoice.provider_unavailable", "Down.");
    assert_eq!(provider_cancel_error(other, "sepay").code(), "einvoice.provider_unavailable");
}
