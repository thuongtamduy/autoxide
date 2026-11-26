//! Bất biến của sổ cái (D-20261126-independent-review, mục 71-G9): bút toán chỉ qua cửa kiểm khi cân nợ / có,
//! cùng một loại tiền, mọi dòng dương — và không bao giờ panic dù số tiền cực lớn. Phần thuần (không database).

use super::*;
use proptest::prelude::*;

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

/// Số tiền dương đủ nhỏ để tổng không tràn.
fn amounts() -> impl Strategy<Value = Vec<i64>> {
    proptest::collection::vec(1i64..=1_000_000_000_000, 1..6)
}

/// Bút toán cân: các dòng nợ, và các dòng có là cùng các số đó theo thứ tự ngược.
fn balanced(amounts: &[i64], currency: Currency) -> Vec<NewLine> {
    let debits = amounts
        .iter()
        .enumerate()
        .map(|(index, minor)| NewLine::debit(account(index), Money::new(*minor, currency)));
    let credits =
        amounts.iter().rev().enumerate().map(|(index, minor)| {
            NewLine::credit(account(index + 50), Money::new(*minor, currency))
        });
    debits.chain(credits).collect()
}

proptest! {
    #[test]
    fn but_toan_can_thi_qua_cua_kiem(amounts in amounts(), currency in currency()) {
        match checked(&entry(balanced(&amounts, currency)), None) {
            Ok(checked) => prop_assert_eq!(checked.currency, currency),
            Err(error) => prop_assert!(false, "bút toán cân bị chặn: {}", error.code()),
        }
    }

    #[test]
    fn lech_mot_don_vi_la_khong_can(
        amounts in amounts(),
        currency in currency(),
        which in any::<proptest::sample::Index>(),
        delta in 1i64..=1_000_000,
    ) {
        let mut lines = balanced(&amounts, currency);
        // Cộng `delta` vào đúng một dòng: nợ ≠ có.
        let index = which.index(lines.len());
        lines[index].amount = Money::new(lines[index].amount.minor() + delta, currency);
        let error = checked(&entry(lines), None).err().unwrap();
        prop_assert_eq!(error.code(), "ledger.unbalanced");
    }

    #[test]
    fn mot_dong_khac_loai_tien_bi_chan(
        amounts in amounts(),
        which in any::<proptest::sample::Index>(),
    ) {
        let mut lines = balanced(&amounts, Currency::VND);
        let index = which.index(lines.len());
        lines[index].amount = Money::new(lines[index].amount.minor(), Currency::USD);
        let error = checked(&entry(lines), None).err().unwrap();
        prop_assert_eq!(error.code(), "ledger.currency_mixed");
    }

    #[test]
    fn dong_khong_hoac_am_bi_chan(
        amounts in amounts(),
        currency in currency(),
        which in any::<proptest::sample::Index>(),
        bad in i64::MIN..=0,
    ) {
        let mut lines = balanced(&amounts, currency);
        let index = which.index(lines.len());
        lines[index].amount = Money::new(bad, currency);
        prop_assert!(checked(&entry(lines), None).is_err());
    }

    /// Số tiền cực lớn: hoặc cân và qua, hoặc lỗi có mã — không panic, không tràn âm thầm.
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
        match checked(&entry(lines.clone()), None) {
            Ok(_) => prop_assert!(expected_balanced, "bút toán lệch mà qua cửa: {lines:?}"),
            Err(error) => {
                let code = error.code();
                prop_assert!(
                    matches!(code, "ledger.unbalanced" | "money.overflow" | "validation"),
                    "{code}"
                );
                // Cân thật thì chỉ được lỗi khi tràn số học.
                if expected_balanced && code != "money.overflow" {
                    prop_assert!(false, "bút toán cân bị chặn: {code}");
                }
            }
        }
    }
}
