//! Bất biến của sổ cái (D-20261126-independent-review, mục 71-G9): bút toán chỉ qua cửa kiểm khi cân nợ / có,
//! cùng một loại tiền, mọi dòng dương — và không bao giờ panic dù số tiền cực lớn. Phần thuần (không database).

use super::*;
use proptest::prelude::*;

use std::collections::BTreeSet;

fn book() -> OrgId {
    OrgId::from_uuid(Uuid::from_u128(1))
}

fn entry(lines: Vec<NewLine>) -> NewEntry {
    NewEntry::new(book(), "orders", "order.paid", "ref-1", "Order paid", lines)
}

fn account(index: usize) -> LineAccount {
    LineAccount::Id(Uuid::from_u128(index as u128 + 100))
}

fn currency() -> impl Strategy<Value = Currency> {
    prop_oneof![Just(Currency::VND), Just(Currency::USD), Just(Currency::parse("KWD").unwrap())]
}

/// Chia `total` thành các phần dương bằng các điểm cắt (phân biệt, trong 1..total).
fn parts(total: i64, cuts: &BTreeSet<i64>) -> Vec<i64> {
    let mut edges = vec![0];
    edges.extend(cuts.iter().copied().filter(|cut| *cut < total));
    edges.push(total);
    edges.windows(2).map(|pair| pair[1] - pair[0]).collect()
}

/// Bút toán cân: một tổng chung, bên nợ và bên có mỗi bên chia ĐỘC LẬP thành số phần khác nhau (vd một dòng
/// nợ 100 ứng với hai dòng có 60 + 40), rồi trộn thứ tự để hai bên xen kẽ.
fn balanced(currency: Currency) -> impl Strategy<Value = Vec<NewLine>> {
    let cuts = || proptest::collection::btree_set(1i64..1_000_000_000_000, 0..5);
    (1_000_000_000_000i64..=2_000_000_000_000, cuts(), cuts()).prop_flat_map(
        move |(total, debit_cuts, credit_cuts)| {
            let debits = parts(total, &debit_cuts).into_iter().map(|minor| (Side::Debit, minor));
            let credits = parts(total, &credit_cuts).into_iter().map(|minor| (Side::Credit, minor));
            let lines: Vec<NewLine> = debits
                .chain(credits)
                .enumerate()
                .map(|(index, (side, minor))| NewLine {
                    account: account(index),
                    side,
                    amount: Money::new(minor, currency),
                })
                .collect();
            Just(lines).prop_shuffle()
        },
    )
}

fn code_of(lines: Vec<NewLine>) -> Option<String> {
    checked(&entry(lines), None).err().map(|error| error.code().to_owned())
}

proptest! {
    #[test]
    fn but_toan_can_thi_qua_cua_kiem(lines in currency().prop_flat_map(balanced)) {
        let expected = lines[0].amount.currency();
        match checked(&entry(lines), None) {
            Ok(checked) => prop_assert_eq!(checked.currency, expected),
            Err(error) => prop_assert!(false, "bút toán cân bị chặn: {}", error.code()),
        }
    }

    #[test]
    fn lech_mot_don_vi_la_khong_can(
        lines in currency().prop_flat_map(balanced),
        which in any::<proptest::sample::Index>(),
        delta in 1i64..=1_000_000,
    ) {
        let mut lines = lines;
        let currency = lines[0].amount.currency();
        // Cộng `delta` vào đúng một dòng: nợ ≠ có.
        let index = which.index(lines.len());
        lines[index].amount = Money::new(lines[index].amount.minor() + delta, currency);
        let code = code_of(lines);
        prop_assert_eq!(code.as_deref(), Some("ledger.unbalanced"));
    }

    #[test]
    fn mot_dong_khac_loai_tien_bi_chan(
        lines in balanced(Currency::VND),
        which in any::<proptest::sample::Index>(),
    ) {
        let mut lines = lines;
        let index = which.index(lines.len());
        lines[index].amount = Money::new(lines[index].amount.minor(), Currency::USD);
        let code = code_of(lines);
        prop_assert_eq!(code.as_deref(), Some("ledger.currency_mixed"));
    }

    // Dòng 0 hay âm BỊ CHẶN dù bút toán vẫn cân: thêm cặp dòng bù nhau (nợ `bad`, có `bad`) vào bút toán cân thì
    // tổng nợ vẫn bằng tổng có, nên chỉ luật "số tiền > 0" mới chặn được. Thiếu luật đó test này đỏ.
    #[test]
    fn dong_khong_hoac_am_bi_chan_du_but_toan_van_can(
        lines in balanced(Currency::VND),
        bad in i64::MIN / 4..=0,
        zero_only in any::<bool>(),
    ) {
        let mut lines = lines;
        let at = lines.len();
        if zero_only {
            // Một dòng 0 ở bên nợ: tổng không đổi.
            lines.push(NewLine::debit(account(at), Money::zero(Currency::VND)));
        } else {
            lines.push(NewLine::debit(account(at), Money::new(bad, Currency::VND)));
            lines.push(NewLine::credit(account(at + 1), Money::new(bad, Currency::VND)));
        }
        let code = code_of(lines);
        prop_assert_eq!(code.as_deref(), Some("validation_failed"));
    }

    // Số cực lớn trộn lẫn: hoặc cân và qua, hoặc lỗi có mã `ledger.unbalanced` / `money.overflow` — không panic,
    // không tràn âm thầm, và bút toán cân thật chỉ bị chặn khi tổng tràn số học.
    #[test]
    fn so_cuc_lon_khong_panic_khong_tran_am_tham(
        minors in proptest::collection::vec(prop_oneof![1i64..=10, (i64::MAX - 10)..=i64::MAX], 2..6),
    ) {
        let lines: Vec<NewLine> = minors
            .iter()
            .enumerate()
            .map(|(index, minor)| {
                let side = if index % 2 == 0 { Side::Debit } else { Side::Credit };
                NewLine { account: account(index), side, amount: Money::new(*minor, Currency::VND) }
            })
            .collect();
        let sum = |side: Side| -> i128 {
            lines.iter().filter(|line| line.side == side).map(|line| i128::from(line.amount.minor())).sum()
        };
        let expected_balanced = sum(Side::Debit) == sum(Side::Credit);
        let overflows = sum(Side::Debit) > i128::from(i64::MAX) || sum(Side::Credit) > i128::from(i64::MAX);
        let code = code_of(lines.clone());
        match code.as_deref() {
            None => prop_assert!(expected_balanced && !overflows, "bút toán lệch hay tràn mà qua cửa: {lines:?}"),
            Some("money.overflow") => prop_assert!(overflows, "báo tràn mà không tràn: {lines:?}"),
            Some("ledger.unbalanced") => prop_assert!(!expected_balanced && !overflows, "{lines:?}"),
            Some(other) => prop_assert!(false, "mã lỗi lạ: {other}"),
        }
    }

    // `n` dòng MAX mỗi bên: cân nhưng tổng tràn i64 → lỗi tràn, không phải "cân" cũng không phải panic.
    #[test]
    fn nhieu_dong_max_moi_ben_bao_tran(n in 2usize..6) {
        let mut lines = Vec::new();
        for index in 0..n {
            lines.push(NewLine::debit(account(index), Money::new(i64::MAX, Currency::VND)));
            lines.push(NewLine::credit(account(index + 50), Money::new(i64::MAX, Currency::VND)));
        }
        let code = code_of(lines);
        prop_assert_eq!(code.as_deref(), Some("money.overflow"));
    }
}
