//! Test thuần của `invoices`. Test chạy cùng `tax` và `ledger` (phát hành, ghi sổ, hoá đơn điện tử)
//! nằm ở `crates/app/src/e2e/invoices.rs`: luật ranh giới cấm module dựng module khác.

use chrono::NaiveDate;

use crate::service::{retention_until, valid_tax_code};

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
    let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
    assert_eq!(retention_until(day(2026, 3, 5)), day(2036, 12, 31));
    assert_eq!(retention_until(day(2026, 12, 31)), day(2036, 12, 31));
}
