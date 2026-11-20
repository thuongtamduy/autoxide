//! Trả trước một phần cho hàng đặt trước (D-232, đặc tả 45 mục "Trả trước một phần cho hàng đặt
//! trước"). Gọi là **trả trước** (`advance`), không phải "đặt cọc" theo BLDS 2015 Điều 328 — không có
//! chế tài phạt cọc; khách muốn điều khoản cọc thật thì ghi trong điều kiện giao dịch của họ.
//!
//! - **Điều kiện** là cài đặt `orders.preorder_advance` của nút bán (kế thừa theo cây): không đặt =
//!   hàng đặt trước trả đủ lúc đặt (mặc định); đặt giá trị = trả trước một phần. Chụp vào
//!   `orders_advance_plans.terms` lúc đặt (kèm SHA-256) — đổi cài đặt sau không áp ngược.
//! - **Tính tiền** bằng [`split`] (hàm thuần, có test thuộc tính): `upfront + balance = total` từng
//!   đồng; `balance` tính bằng phép trừ.
//! - **Dòng tiền** (D-187): khoản `upfront` và khoản phần còn lại đều đi qua đối tác của `ledger`
//!   như mọi khoản thu. `ledger` ghi Có `receivable.customer` khi thu — số dư Có đó chính là nghĩa vụ
//!   với người mua tới khi giao (không cần tài khoản `orders.customer_advance` riêng).

use autoxide_kernel::error::Error;
use autoxide_kernel::money::Money;
use chrono::{DateTime, Duration, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Trạng thái của kế hoạch trả trước.
pub(crate) const AWAITING_ADVANCE: &str = "awaiting_advance";
pub(crate) const ADVANCE_PAID: &str = "advance_paid";
pub(crate) const BALANCE_REQUESTED: &str = "balance_requested";
pub(crate) const BALANCE_PAID: &str = "balance_paid";
pub(crate) const BALANCE_OVERDUE: &str = "balance_overdue";
pub(crate) const CANCELLED: &str = "cancelled";
/// Mục đích của khoản thu (`orders_payments.purpose`).
pub(crate) const PURPOSE_ADVANCE: &str = "advance";
pub(crate) const PURPOSE_BALANCE: &str = "balance";
/// Cách thu phần còn lại.
pub(crate) const PAYMENT_LINK: &str = "payment_link";

/// Điều kiện trả trước một phần người bán công bố (giá trị của `orders.preorder_advance`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Terms {
    /// Tỉ lệ trả trước, phần vạn (1–9999; 3000 = 30%). Đúng một trong `rate_bp` / `fixed`.
    #[serde(default)]
    pub rate_bp: Option<i64>,
    /// Số tiền cố định trả trước cho phần đặt trước của đơn.
    #[serde(default)]
    pub fixed: Option<Money>,
    #[serde(default)]
    pub min: Option<Money>,
    #[serde(default)]
    pub max: Option<Money>,
    /// Mốc chốt = ngày dự kiến có hàng trừ chừng này ngày (khi biến thể không có hạn nhận đặt).
    #[serde(default)]
    pub commit_days: Option<i64>,
    /// Người mua huỷ sau mốc chốt: giữ lại chừng này phần vạn của phần trả trước của dòng (mặc định 0
    /// — luôn hoàn đủ).
    #[serde(default)]
    pub retention_bp: i64,
    #[serde(default)]
    pub retention_max: Option<Money>,
    /// Hạn trả phần còn lại tính từ lúc hàng sẵn sàng (giờ).
    #[serde(default = "default_due_hours")]
    pub balance_due_hours: i64,
    /// Quá hạn trả phần còn lại: `cancel_per_terms` (huỷ, giữ lại như người mua huỷ sau mốc) hay
    /// `cancel_full_refund`.
    #[serde(default = "default_overdue")]
    pub overdue_action: String,
}

fn default_due_hours() -> i64 {
    72
}

fn default_overdue() -> String {
    "cancel_per_terms".to_owned()
}

impl Terms {
    /// Đọc và kiểm giá trị cài đặt; trả mọi chỗ sai trong một câu.
    pub(crate) fn parse(value: &Value) -> Result<Terms, String> {
        let terms: Terms = serde_json::from_value(value.clone()).map_err(|error| {
            format!("must be an object with `rate_bp` or `fixed` and optional limits ({error})")
        })?;
        let mut problems = Vec::new();
        match (terms.rate_bp, &terms.fixed) {
            (Some(rate), None) if (1..=9999).contains(&rate) => {}
            (None, Some(fixed)) if fixed.minor() > 0 => {}
            _ => problems.push("exactly one of `rate_bp` (1 to 9999) or `fixed` (above 0)"),
        }
        let amounts = [&terms.fixed, &terms.min, &terms.max, &terms.retention_max];
        let currencies: Vec<_> =
            amounts.iter().filter_map(|amount| amount.map(|a| a.currency())).collect();
        if currencies.windows(2).any(|pair| pair[0] != pair[1]) {
            problems.push("amounts in one currency");
        }
        if amounts.iter().any(|amount| amount.is_some_and(|a| a.minor() < 0)) {
            problems.push("amounts of 0 or more");
        }
        if let (Some(min), Some(max)) = (terms.min, terms.max)
            && min.minor() > max.minor()
        {
            problems.push("`min` at most `max`");
        }
        if !(0..=10_000).contains(&terms.retention_bp) {
            problems.push("`retention_bp` from 0 to 10000");
        }
        if !(1..=720).contains(&terms.balance_due_hours) {
            problems.push("`balance_due_hours` from 1 to 720");
        }
        if terms.commit_days.is_some_and(|days| !(0..=365).contains(&days)) {
            problems.push("`commit_days` from 0 to 365");
        }
        if !matches!(terms.overdue_action.as_str(), "cancel_per_terms" | "cancel_full_refund") {
            problems.push("`overdue_action` `cancel_per_terms` or `cancel_full_refund`");
        }
        if problems.is_empty() { Ok(terms) } else { Err(format!("needs {}", problems.join("; "))) }
    }

    /// Mốc chốt của một dòng: hạn nhận đặt của biến thể, không có thì ngày có hàng trừ
    /// `commit_days`; không có cả hai thì không có mốc (người mua huỷ luôn được hoàn đủ).
    pub(crate) fn commit_at(
        &self,
        ends_at: Option<DateTime<Utc>>,
        available_on: Option<chrono::NaiveDate>,
    ) -> Option<DateTime<Utc>> {
        ends_at.or_else(|| {
            let (day, days) = (available_on?, self.commit_days?);
            Some(day.and_time(NaiveTime::MIN).and_utc() - Duration::days(days))
        })
    }

    /// Phần giữ lại khi người mua huỷ dòng có phần trả trước `advance` sau mốc chốt.
    pub(crate) fn retention(&self, advance: i64) -> i64 {
        let share = i128::from(advance) * i128::from(self.retention_bp) / 10_000;
        let mut keep = i64::try_from(share).unwrap_or(advance);
        if let Some(max) = self.retention_max {
            keep = keep.min(max.minor());
        }
        keep.clamp(0, advance.max(0))
    }
}

/// Một dòng đưa vào phép chia: người mua trả `gross` (sau giảm, gồm thuế); `preorder`: hàng đặt trước.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SplitLine {
    pub gross: Money,
    pub preorder: bool,
}

/// Kết quả chia một đơn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Split {
    /// Phần của hàng đặt trước (tiền hàng + phần phí giao chia cho chúng).
    pub preorder_part: Money,
    pub advance: Money,
    /// Trả lúc đặt: phần hàng sẵn + `advance`.
    pub upfront: Money,
    /// Trả khi hàng về: `total − upfront`.
    pub balance: Money,
    /// Phần trả trước của từng dòng (0 với dòng hàng sẵn), cùng thứ tự với đầu vào.
    pub line_advance: Vec<Money>,
}

/// Chia đơn tổng `total` theo `terms`. `None`: không có hàng đặt trước, hay trả trước bằng cả phần
/// đặt trước (đơn coi như trả đủ).
pub(crate) fn split(
    lines: &[SplitLine],
    total: Money,
    terms: &Terms,
) -> Result<Option<Split>, Error> {
    let currency = total.currency();
    if !lines.iter().any(|line| line.preorder) {
        return Ok(None);
    }
    let goods = Money::sum(currency, lines.iter().map(|line| line.gross))?;
    // Phí giao (sau giảm, gồm thuế) chia theo tiền hàng của từng dòng.
    let shipping = total.checked_sub(goods)?;
    let weights: Vec<u64> =
        lines.iter().map(|line| line.gross.minor().max(0).unsigned_abs()).collect();
    let weights =
        if weights.iter().all(|weight| *weight == 0) { vec![1; lines.len()] } else { weights };
    let shares = shipping.allocate(&weights)?;
    let parts: Vec<Money> = lines
        .iter()
        .zip(&shares)
        .map(|(line, share)| line.gross.checked_add(*share))
        .collect::<Result<_, _>>()?;
    let preorder_part = Money::sum(
        currency,
        lines.iter().zip(&parts).filter(|(line, _)| line.preorder).map(|(_, part)| *part),
    )?;
    let base = preorder_part.minor().max(0);
    let mut advance = match (terms.rate_bp, terms.fixed) {
        (Some(rate), _) => {
            // Làm tròn xuống đơn vị nhỏ nhất.
            i64::try_from(i128::from(base) * i128::from(rate) / 10_000).map_err(Error::internal)?
        }
        (None, Some(fixed)) => fixed.minor(),
        (None, None) => return Ok(None),
    };
    if let Some(min) = terms.min {
        advance = advance.max(min.minor());
    }
    if let Some(max) = terms.max {
        advance = advance.min(max.minor());
    }
    let advance = advance.clamp(0, base);
    if advance >= base {
        return Ok(None);
    }
    let advance = Money::new(advance, currency);
    let preorder_weights: Vec<u64> = lines
        .iter()
        .zip(&parts)
        .map(|(line, part)| if line.preorder { part.minor().max(0).unsigned_abs() } else { 0 })
        .collect();
    let line_advance = if preorder_weights.iter().all(|weight| *weight == 0) {
        let marks: Vec<u64> = lines.iter().map(|line| u64::from(line.preorder)).collect();
        advance.allocate(&marks)?
    } else {
        advance.allocate(&preorder_weights)?
    };
    let upfront = total.checked_sub(preorder_part)?.checked_add(advance)?;
    let balance = total.checked_sub(upfront)?;
    Ok(Some(Split { preorder_part, advance, upfront, balance, line_advance }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::money::Currency;
    use serde_json::json;

    fn vnd(minor: i64) -> Money {
        Money::new(minor, Currency::VND)
    }

    fn rate(bp: i64) -> Terms {
        Terms::parse(&json!({"rate_bp": bp})).unwrap()
    }

    #[test]
    fn ba_muoi_phan_tram_phan_dat_truoc_gom_phi_giao() {
        // Hàng sẵn 100.000, hàng đặt trước 300.000, phí giao 40.000 chia theo tiền hàng (10.000 /
        // 30.000) → phần đặt trước 330.000, trả trước 30% = 99.000.
        let lines = [
            SplitLine { gross: vnd(100_000), preorder: false },
            SplitLine { gross: vnd(300_000), preorder: true },
        ];
        let split = split(&lines, vnd(440_000), &rate(3000)).unwrap().unwrap();
        assert_eq!(split.preorder_part, vnd(330_000));
        assert_eq!(split.advance, vnd(99_000));
        assert_eq!(split.upfront, vnd(209_000));
        assert_eq!(split.balance, vnd(231_000));
        assert_eq!(split.line_advance, vec![vnd(0), vnd(99_000)]);
    }

    #[test]
    fn san_tran_va_tra_du_thi_khong_chia() {
        let lines = [SplitLine { gross: vnd(1_000_000), preorder: true }];
        let terms =
            Terms::parse(&json!({"rate_bp": 1000, "min": {"amount": "200000", "currency": "VND"}}))
                .unwrap();
        assert_eq!(split(&lines, vnd(1_000_000), &terms).unwrap().unwrap().advance, vnd(200_000));
        let terms =
            Terms::parse(&json!({"fixed": {"amount": "2000000", "currency": "VND"}})).unwrap();
        assert_eq!(
            split(&lines, vnd(1_000_000), &terms).unwrap(),
            None,
            "trả trước bằng cả phần: trả đủ"
        );
        let only_stock = [SplitLine { gross: vnd(5), preorder: false }];
        assert_eq!(split(&only_stock, vnd(5), &rate(3000)).unwrap(), None);
    }

    #[test]
    fn dieu_kien_sai_bao_mot_lan() {
        let error =
            Terms::parse(&json!({"rate_bp": 0, "retention_bp": 20000, "overdue_action": "keep"}))
                .unwrap_err();
        assert!(
            error.contains("rate_bp")
                && error.contains("retention_bp")
                && error.contains("overdue_action"),
            "{error}"
        );
        assert!(
            Terms::parse(&json!({"rate_bp": 10, "fixed": {"amount": "1", "currency": "VND"}}))
                .is_err()
        );
        assert!(Terms::parse(&json!({"deposit": 10})).is_err());
    }

    #[test]
    fn giu_lai_theo_ti_le_va_tran() {
        let terms = Terms::parse(&json!({"rate_bp": 3000, "retention_bp": 5000,
            "retention_max": {"amount": "40000", "currency": "VND"}}))
        .unwrap();
        assert_eq!(terms.retention(60_000), 30_000);
        assert_eq!(terms.retention(100_000), 40_000);
        assert_eq!(rate(3000).retention(100_000), 0, "mặc định luôn hoàn đủ");
    }

    /// Thuộc tính: với mọi đơn, `upfront + balance = total`, `0 ≤ advance < phần đặt trước`, tổng
    /// phần trả trước của dòng bằng `advance`, dòng hàng sẵn không có phần trả trước.
    #[test]
    fn thuoc_tinh_tong_thu_luon_bang_gia_don() {
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = |bound: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % bound
        };
        for _ in 0..2000 {
            let count = 1 + next(5) as usize;
            let lines: Vec<SplitLine> = (0..count)
                .map(|_| SplitLine {
                    gross: vnd(i64::try_from(next(5_000_000)).unwrap()),
                    preorder: next(2) == 0,
                })
                .collect();
            let goods: i64 = lines.iter().map(|line| line.gross.minor()).sum();
            let total = vnd(goods + i64::try_from(next(100_000)).unwrap());
            let terms = rate(i64::try_from(1 + next(9999)).unwrap());
            let Some(split) = split(&lines, total, &terms).unwrap() else { continue };
            assert_eq!(split.upfront.minor() + split.balance.minor(), total.minor());
            assert!(
                split.advance.minor() >= 0 && split.advance.minor() < split.preorder_part.minor()
            );
            assert_eq!(
                split.line_advance.iter().map(|m| m.minor()).sum::<i64>(),
                split.advance.minor()
            );
            for (line, advance) in lines.iter().zip(&split.line_advance) {
                if !line.preorder {
                    assert_eq!(advance.minor(), 0);
                }
            }
        }
    }
}
