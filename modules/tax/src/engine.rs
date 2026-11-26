//! Tính thuế của một chứng từ từ thuế suất đã tra (không chạm database — test riêng được).
//!
//! Quy tắc (đặc tả 36, bảng làm tròn):
//! - Dòng gom theo **chuỗi thuế suất** giống nhau (vd TTĐB 65% rồi GTGT 10%). Thuế của mỗi nhóm tính
//!   trên **tổng** nhóm, làm tròn `HalfUp` một lần, rồi chia lại về dòng bằng `Money::allocate` theo tỉ
//!   lệ số tiền của dòng — tổng các dòng luôn bằng tổng chứng từ, không làm tròn hai lần.
//! - Nhiều tầng: tầng sau tính trên giá + thuế các tầng trước (GTGT trên giá đã có TTĐB).
//! - Giá đã gồm thuế: tách ngược qua các tầng (`base / (1 + r)`, trừ thuế theo đơn vị), làm tròn
//!   giá chưa thuế một lần; thuế từng tầng tính xuôi trên cơ sở đã làm tròn; phần lệch do làm tròn dồn
//!   vào tầng chịu thuế cuối khi được, không thì sang tầng khác sao cho mọi thuế ≥ 0 và tầng thuế danh
//!   nghĩa 0 ra đúng 0 (`settle`); ở dòng, tầng cuối có thuế lấy phần còn lại của dòng, ô âm sửa bằng
//!   hoán đổi giữ nguyên tổng dòng và tổng cột (`lines`). Thuế theo đơn vị chia về dòng theo số lượng. D-20261126-tax-rounding-allocation.

use std::collections::BTreeMap;

use autoxide_kernel::decimal::{Decimal, Rounding, to_column};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::money::{Currency, Money};
use uuid::Uuid;

use crate::api::{PriceMode, TaxComponent, TaxTotal, TaxableLine, TaxedLine};

/// Cách tính của một thuế suất.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Basis {
    Percent,
    PerUnit,
    /// Không chịu thuế: không có số thuế, ghi "không chịu thuế" trên hoá đơn.
    Exempt,
}

impl Basis {
    pub(crate) fn parse(raw: &str) -> Option<Basis> {
        match raw {
            "percent" => Some(Basis::Percent),
            "per_unit" => Some(Basis::PerUnit),
            "exempt" => Some(Basis::Exempt),
            _ => None,
        }
    }
}

/// Một thuế suất đã tra cho một dòng.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedRate {
    pub id: Uuid,
    pub tax_kind: String,
    pub basis: Basis,
    pub rate_percent: Decimal,
    /// Với `PerUnit`: số tiền mỗi đơn vị.
    pub per_unit: Option<Money>,
    pub stage: i32,
}

impl ResolvedRate {
    fn component(&self, amount: Money) -> TaxComponent {
        TaxComponent {
            tax_kind: self.tax_kind.clone(),
            rate_percent: to_column(self.rate_percent),
            exempt: self.basis == Basis::Exempt,
            amount,
        }
    }
}

/// Kết quả tính.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Computed {
    pub lines: Vec<TaxedLine>,
    pub totals: Vec<TaxTotal>,
}

fn hundred() -> Decimal {
    Decimal::new(100, 0)
}

/// Tỉ lệ chia (số đơn vị nhỏ nhất của dòng); mọi dòng bằng 0 thì chia đều.
fn ratios(amounts: &[Money]) -> Vec<u64> {
    let values: Vec<u64> = amounts.iter().map(|amount| amount.minor().unsigned_abs()).collect();
    if values.iter().all(|value| *value == 0) { vec![1; values.len()] } else { values }
}

fn sum(amounts: &[Money], currency: Currency) -> Result<Money, Error> {
    amounts.iter().try_fold(Money::zero(currency), |total, amount| total.checked_add(*amount))
}

/// Thuế một tầng trên `base` (tổng nhóm); `quantity` cho thuế theo đơn vị.
fn stage_tax(rate: &ResolvedRate, base: Money, quantity: Decimal) -> Result<Money, Error> {
    match rate.basis {
        Basis::Exempt => Ok(Money::zero(base.currency())),
        Basis::Percent => base.apply_rate(rate.rate_percent / hundred(), Rounding::HalfUp),
        Basis::PerUnit => {
            let unit = rate
                .per_unit
                .ok_or_else(|| Error::internal("a per-unit tax rate has no amount per unit"))?;
            unit.convert(quantity, base.currency(), Rounding::HalfUp)
        }
    }
}

/// Tính thuế của mọi dòng; mỗi dòng đi kèm chuỗi thuế suất của nó (theo `stage` tăng dần, đã bỏ
/// những thuế không áp cho người bán).
pub(crate) fn compute(
    mode: PriceMode,
    currency: Currency,
    lines: &[(TaxableLine, Vec<ResolvedRate>)],
) -> Result<Computed, Error> {
    let mut problems = Vec::new();
    for (index, (line, chain)) in lines.iter().enumerate() {
        if line.amount.currency() != currency {
            problems.push(FieldError::new(
                format!("lines[{index}].amount"),
                "currency",
                "must be in the document currency",
            ));
        }
        if line.amount.is_negative() {
            problems.push(FieldError::new(
                format!("lines[{index}].amount"),
                "range",
                "must not be negative",
            ));
        }
        if line
            .quantity
            .is_some_and(|quantity| quantity.0.is_sign_negative() && !quantity.0.is_zero())
        {
            problems.push(FieldError::new(
                format!("lines[{index}].quantity"),
                "range",
                "must not be negative",
            ));
        }
        if chain.iter().any(|rate| rate.basis == Basis::PerUnit) && line.quantity.is_none() {
            problems.push(FieldError::new(
                format!("lines[{index}].quantity"),
                "required",
                "is required: this category has a tax per unit",
            ));
        }
        // Giá đã gồm thuế: thuế theo đơn vị của dòng đi theo số lượng của chính dòng, nên số tiền dòng phải
        // chứa được nó (cùng luật với cả nhóm ở `inclusive_group`).
        if mode == PriceMode::Inclusive
            && chain.iter().any(|rate| rate.basis == Basis::PerUnit)
            && !line.amount.is_negative()
        {
            let quantity = line.quantity.map_or(Decimal::ZERO, |quantity| quantity.0);
            let below = reverse_split(chain, line.amount.to_decimal(), quantity)
                .and_then(|net| Money::from_decimal(net, line.amount.currency(), Rounding::HalfUp))
                .is_ok_and(|net| net.is_negative());
            if below {
                problems.push(FieldError::new(
                    format!("lines[{index}].amount"),
                    "range",
                    "is lower than the per-unit taxes it includes",
                ));
            }
        }
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    // Nhóm theo chuỗi thuế suất (ID theo thứ tự), giữ thứ tự xuất hiện của dòng.
    let mut groups: BTreeMap<Vec<Uuid>, Vec<usize>> = BTreeMap::new();
    for (index, (_, chain)) in lines.iter().enumerate() {
        groups.entry(chain.iter().map(|rate| rate.id).collect()).or_default().push(index);
    }
    let mut taxed: Vec<Option<TaxedLine>> = vec![None; lines.len()];
    // Tổng theo (loại thuế, thuế suất, không chịu thuế) → (cơ sở, số thuế).
    let mut totals: BTreeMap<(String, String, bool), (Money, Money)> = BTreeMap::new();
    for indexes in groups.values() {
        let chain = &lines[indexes[0]].1;
        let amounts: Vec<Money> = indexes.iter().map(|index| lines[*index].0.amount).collect();
        // Số lượng do người gọi gửi (tới 28 chữ số): cộng / nhân `Decimal` bằng phép có kiểm tràn —
        // phép `+` / `*` thường của `rust_decimal` panic khi tràn (D-20261123-property-fuzzing).
        let quantity = indexes
            .iter()
            .filter_map(|index| lines[*index].0.quantity.map(|quantity| quantity.0))
            .try_fold(Decimal::ZERO, Decimal::checked_add)
            .ok_or_else(too_large)?;
        let group_total = sum(&amounts, currency)?;
        let shares = ratios(&amounts);
        // Giá chưa thuế của nhóm và thuế từng tầng (tổng nhóm).
        let (net, taxes) = match mode {
            PriceMode::Exclusive => {
                let mut base = group_total;
                let mut taxes = Vec::with_capacity(chain.len());
                for rate in chain {
                    let tax = stage_tax(rate, base, quantity)?;
                    taxes.push(tax);
                    base = base.checked_add(tax)?;
                }
                (group_total, taxes)
            }
            PriceMode::Inclusive => inclusive_group(chain, group_total, quantity, indexes[0])?,
        };
        // Cơ sở của từng tầng: giá chưa thuế cộng thuế các tầng trước.
        let mut running = net;
        for (rate, tax) in chain.iter().zip(&taxes) {
            let key =
                (rate.tax_kind.clone(), to_column(rate.rate_percent), rate.basis == Basis::Exempt);
            let entry = totals.entry(key).or_insert((Money::zero(currency), Money::zero(currency)));
            entry.0 = entry.0.checked_add(running)?;
            entry.1 = entry.1.checked_add(*tax)?;
            running = running.checked_add(*tax)?;
        }
        // Chia về dòng: theo số tiền dòng, riêng thuế theo đơn vị theo số lượng của dòng.
        let quantities: Vec<Decimal> = indexes
            .iter()
            .map(|index| lines[*index].0.quantity.map_or(Decimal::ZERO, |quantity| quantity.0))
            .collect();
        let split = lines::Split::new(mode, chain, &amounts, &shares, &quantities)?;
        let (line_nets, line_taxes) = match mode {
            PriceMode::Exclusive => (amounts.clone(), split.columns(&taxes)?),
            PriceMode::Inclusive => split.inclusive(net, &taxes).map_err(|error| match error {
                lines::Stuck::Line(slot) => per_unit_does_not_fit(indexes[slot]),
                lines::Stuck::Error(error) => error,
            })?,
        };
        for (slot, index) in indexes.iter().enumerate() {
            let (line, _) = &lines[*index];
            let components: Vec<TaxComponent> = chain
                .iter()
                .zip(&line_taxes)
                .map(|(rate, column)| rate.component(column[slot]))
                .collect();
            let mut gross = line_nets[slot];
            for component in &components {
                gross = gross.checked_add(component.amount)?;
            }
            taxed[*index] = Some(TaxedLine {
                line_id: line.line_id.clone(),
                category: line.category.clone(),
                net: line_nets[slot],
                taxes: components,
                gross,
            });
        }
    }
    let lines = taxed.into_iter().flatten().collect();
    let totals = totals
        .into_iter()
        .map(|((tax_kind, rate_percent, exempt), (base, amount))| TaxTotal {
            tax_kind,
            rate_percent,
            exempt,
            base,
            amount,
        })
        .collect();
    Ok(Computed { lines, totals })
}

/// Tách ngược chính xác giá đã gồm thuế qua các tầng (từ cuối về đầu): `x / (1 + r)` hay
/// `x − đơn giá × số lượng`.
fn reverse_split(
    chain: &[ResolvedRate],
    gross: Decimal,
    quantity: Decimal,
) -> Result<Decimal, Error> {
    let mut base = gross;
    for rate in chain.iter().rev() {
        base = match rate.basis {
            Basis::Exempt => base,
            Basis::Percent => base / (Decimal::ONE + rate.rate_percent / hundred()),
            Basis::PerUnit => {
                let unit = rate.per_unit.map_or(Decimal::ZERO, Money::to_decimal);
                unit.checked_mul(quantity)
                    .and_then(|tax| base.checked_sub(tax))
                    .ok_or_else(too_large)?
            }
        };
    }
    Ok(base)
}

/// Giá đã gồm thuế, cấp nhóm: trả (giá chưa thuế, thuế từng tầng) với giá chưa thuế + Σ thuế = `gross`.
///
/// 1. Tách ngược chính xác (`Decimal`): `x = gross`, qua từng tầng từ cuối về đầu `x / (1 + r)` hay
///    `x − đơn giá × số lượng`. Giá chưa thuế `n = HalfUp(x)` (như trước).
/// 2. Tính xuôi như hoá đơn ghi: thuế mỗi tầng = `HalfUp(thuế suất × cơ sở đã làm tròn)` — người đọc
///    hoá đơn tự kiểm được từng dòng thuế.
/// 3. Lệch `D = (gross − n) − Σ thuế` (do làm tròn) dồn vào tầng chịu thuế cuối nếu được (hành vi cũ),
///    không thì sang tầng khác — xem `settle`.
fn inclusive_group(
    chain: &[ResolvedRate],
    gross: Money,
    quantity: Decimal,
    first_line: usize,
) -> Result<(Money, Vec<Money>), Error> {
    let currency = gross.currency();
    let base = reverse_split(chain, gross.to_decimal(), quantity)?;
    let net = Money::from_decimal(base, currency, Rounding::HalfUp)?;
    // Giá đã gồm thuế thấp hơn thuế theo đơn vị nằm trong nó: giá chưa thuế âm là dữ liệu sai (hoá
    // đơn không ghi được), báo cho người gọi thay vì tính tiếp.
    if net.is_negative() {
        return Err(below_per_unit(first_line));
    }
    if chain.is_empty() {
        return Ok((gross, Vec::new()));
    }
    // Thuế CHÍNH XÁC của từng tầng (tính xuôi từ giá chưa thuế chính xác) > 0 hay không: chỉ tầng có
    // thuế thực dương mới được nhận thêm đơn vị (I2: thuế suất 0%, không chịu thuế, đơn giá × số
    // lượng = 0 luôn ra đúng 0).
    let mut exact = base;
    let mut positive = Vec::with_capacity(chain.len());
    for rate in chain {
        let tax = match rate.basis {
            Basis::Exempt => Decimal::ZERO,
            Basis::Percent => {
                exact.checked_mul(rate.rate_percent / hundred()).ok_or_else(too_large)?
            }
            Basis::PerUnit => rate
                .per_unit
                .map_or(Some(Decimal::ZERO), |unit| unit.to_decimal().checked_mul(quantity))
                .ok_or_else(too_large)?,
        };
        positive.push(tax > Decimal::ZERO);
        exact = exact.checked_add(tax).ok_or_else(too_large)?;
    }
    let mut running = net;
    let mut taxes = Vec::with_capacity(chain.len());
    for rate in chain {
        let tax = stage_tax(rate, running, quantity)?;
        taxes.push(tax);
        running = running.checked_add(tax)?;
    }
    // Thứ tự ưu tiên nhận phần lệch: tầng theo phần trăm từ cuối về đầu, rồi mới tới tầng theo đơn vị
    // (số cố định theo đơn vị — chỉ nhận khi không còn tầng nào khác). Chuỗi không có thuế theo đơn vị:
    // đúng thứ tự cũ (tầng chịu thuế cuối trước).
    let order: Vec<usize> = (0..chain.len())
        .rev()
        .filter(|position| chain[*position].basis == Basis::Percent)
        .chain((0..chain.len()).rev().filter(|position| chain[*position].basis == Basis::PerUnit))
        .collect();
    settle(&mut taxes, gross.checked_sub(net)?, &positive, &order)?;
    Ok((net, taxes))
}

/// Sửa thuế các tầng cho tổng đúng bằng `target` (= tổng − giá chưa thuế, I3), không ra số âm (I1),
/// không làm tầng có thuế danh nghĩa 0 khác 0 (I2).
///
/// Thiếu (`D > 0`): cộng cả `D` vào tầng đầu tiên theo `order` có thuế chính xác dương. Luôn có tầng như
/// vậy: mọi thuế chính xác bằng 0 thì giá chưa thuế chính xác = tổng (số nguyên), `n` = tổng, `target` = 0
/// = Σ thuế.
///
/// Thừa (`D < 0`): trừ theo `order`, mỗi tầng tới khi về 0. Luôn đủ chỗ trừ: `target ≥ 0` (mọi thuế chính
/// xác ≥ 0 nên giá chưa thuế chính xác ≤ tổng, làm tròn vẫn ≤ tổng vì tổng là số nguyên), nên khi Σ thuế >
/// `target` còn ít nhất một tầng ≥ 1 (tầng không chịu thuế luôn 0, không cần có trong `order`); trừ không
/// xuống dưới 0. Tầng thuế danh nghĩa 0 có số làm tròn 0 nên không bao giờ bị chọn.
///
/// Chuỗi không có thuế theo đơn vị: khi cách cũ (dồn hết vào tầng cuối) đã cho số không âm ở tầng có thuế
/// thực, kết quả trùng cách cũ.
fn settle(
    taxes: &mut [Money],
    target: Money,
    positive: &[bool],
    order: &[usize],
) -> Result<(), Error> {
    let currency = target.currency();
    let mut gap = target.checked_sub(sum(taxes, currency)?)?.minor();
    if gap > 0 {
        let pick = order
            .iter()
            .copied()
            .find(|position| positive[*position])
            .ok_or_else(|| Error::internal("tax rounding: no stage can take the remainder"))?;
        taxes[pick] = taxes[pick].checked_add(Money::new(gap, currency))?;
        return Ok(());
    }
    for position in order.iter().copied() {
        if gap == 0 {
            break;
        }
        let take = taxes[position].minor().min(-gap);
        if take > 0 {
            taxes[position] = taxes[position].checked_sub(Money::new(take, currency))?;
            gap += take;
        }
    }
    if gap != 0 {
        return Err(Error::internal("tax rounding: the taxes cannot reach the expected total"));
    }
    Ok(())
}

/// Giá đã gồm thuế của dòng thấp hơn thuế theo đơn vị nằm trong nó: giá chưa thuế âm là dữ liệu sai
/// (hoá đơn không ghi được), báo cho người gọi thay vì tính tiếp.
fn below_per_unit(line: usize) -> Error {
    Error::validation(vec![FieldError::new(
        format!("lines[{line}].amount"),
        "range",
        "is lower than the per-unit taxes it includes",
    )])
}

/// Không có cách chia nào về dòng mà mọi số ≥ 0: thuế theo đơn vị của nhóm lớn hơn số tiền các dòng có
/// số lượng chứa được (`lines::Split::inclusive`).
fn per_unit_does_not_fit(line: usize) -> Error {
    Error::validation(vec![FieldError::new(
        format!("lines[{line}].amount"),
        "range",
        "together with the other lines that have a quantity, is too low to hold the per-unit taxes of its group",
    )])
}

/// Số lượng hay số tiền vượt giới hạn tính được (28 chữ số của `Decimal`).
fn too_large() -> Error {
    Error::validation(vec![FieldError::new(
        "lines",
        "range",
        "the quantities or amounts are too large to compute",
    )])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vnd(minor: i64) -> Money {
        Money::new(minor, Currency::VND)
    }

    fn percent(stage: i32, kind: &str, rate: i64) -> ResolvedRate {
        ResolvedRate {
            id: Uuid::from_u128(
                u128::try_from(stage * 1000 + i32::try_from(rate).unwrap()).unwrap(),
            ),
            tax_kind: kind.to_owned(),
            basis: Basis::Percent,
            rate_percent: Decimal::new(rate, 0),
            per_unit: None,
            stage,
        }
    }

    fn line(id: &str, amount: i64) -> TaxableLine {
        TaxableLine::new(id, "vat.standard", vnd(amount))
    }

    #[test]
    fn thue_tinh_tren_tong_nhom_roi_chia_ve_dong() {
        // Ba dòng 333 đ, GTGT 10%: theo từng dòng sẽ là 33 × 3 = 99; theo nhóm là round(99.9) = 100.
        let vat = percent(2, "vat", 10);
        let lines = vec![
            (line("a", 333), vec![vat.clone()]),
            (line("b", 333), vec![vat.clone()]),
            (line("c", 333), vec![vat]),
        ];
        let computed = compute(PriceMode::Exclusive, Currency::VND, &lines).unwrap();
        let taxes: Vec<i64> =
            computed.lines.iter().map(|line| line.taxes[0].amount.minor()).collect();
        assert_eq!(taxes.iter().sum::<i64>(), 100);
        assert_eq!(computed.totals[0].amount, vnd(100));
        assert_eq!(computed.totals[0].base, vnd(999));
    }

    #[test]
    fn gia_da_gom_thue_tang_thue_suat_khong_khong_nhan_phan_du_am() {
        // Hồi quy (issue #531): tầng 0,01% làm tròn lên, phần dư −1 đ từng rơi vào tầng 0% cuối chuỗi.
        let rate = |stage: i32, kind: &str, percent: Decimal| ResolvedRate {
            id: Uuid::from_u128(u128::try_from(stage).unwrap()),
            tax_kind: kind.to_owned(),
            basis: Basis::Percent,
            rate_percent: percent,
            per_unit: None,
            stage,
        };
        let chain = vec![rate(0, "kind0", Decimal::new(1, 2)), rate(1, "kind1", Decimal::ZERO)];
        let taxable = line("l0", 454_881_118_563).with_quantity(Decimal::ZERO);
        let computed = compute(PriceMode::Inclusive, Currency::VND, &[(taxable, chain)]).unwrap();
        let taxed = &computed.lines[0];
        assert_eq!(taxed.net, vnd(454_835_635_000));
        assert_eq!(taxed.taxes[0].amount, vnd(45_483_563));
        assert_eq!(taxed.taxes[1].amount, vnd(0));
        assert_eq!(taxed.gross, vnd(454_881_118_563));
        let sum: i64 = taxed.taxes.iter().map(|tax| tax.amount.minor()).sum();
        assert_eq!(taxed.net.minor() + sum, taxed.gross.minor());
    }

    #[test]
    fn gia_da_gom_thue_theo_don_vi_so_luong_khong_khong_nhan_phan_du_am() {
        let chain = vec![
            ResolvedRate {
                id: Uuid::from_u128(1),
                tax_kind: "kind0".to_owned(),
                basis: Basis::Percent,
                rate_percent: Decimal::new(1, 2),
                per_unit: None,
                stage: 0,
            },
            ResolvedRate {
                id: Uuid::from_u128(2),
                tax_kind: "kind1".to_owned(),
                basis: Basis::PerUnit,
                rate_percent: Decimal::ZERO,
                per_unit: Some(vnd(49)),
                stage: 1,
            },
        ];
        let taxable = line("l0", 454_881_118_563).with_quantity(Decimal::ZERO);
        let computed = compute(PriceMode::Inclusive, Currency::VND, &[(taxable, chain)]).unwrap();
        let taxed = &computed.lines[0];
        assert_eq!(taxed.taxes[1].amount, vnd(0));
        assert!(!taxed.taxes[0].amount.is_negative());
        assert_eq!(taxed.net.minor() + taxed.taxes[0].amount.minor(), taxed.gross.minor());
    }

    #[test]
    fn tach_thue_tu_gia_da_gom_cong_dung_tong() {
        let vat = percent(2, "vat", 8);
        let lines = vec![(line("a", 100_000), vec![vat.clone()]), (line("b", 55_555), vec![vat])];
        let computed = compute(PriceMode::Inclusive, Currency::VND, &lines).unwrap();
        for line in &computed.lines {
            let tax: i64 = line.taxes.iter().map(|tax| tax.amount.minor()).sum();
            assert_eq!(line.net.minor() + tax, line.gross.minor());
        }
        assert_eq!(computed.lines[0].gross, vnd(100_000));
        assert_eq!(computed.lines[1].gross, vnd(55_555));
        // 155555 / 1.08 = 144032.4… → 144032; thuế 11523.
        assert_eq!(computed.totals[0].base, vnd(144_032));
        assert_eq!(computed.totals[0].amount, vnd(11_523));
    }

    #[test]
    fn ttdb_roi_gtgt_cong_don() {
        let excise = percent(1, "excise", 65);
        let vat = percent(2, "vat", 10);
        let lines = vec![(line("beer", 100_000), vec![excise, vat])];
        let computed = compute(PriceMode::Exclusive, Currency::VND, &lines).unwrap();
        let taxes: Vec<i64> =
            computed.lines[0].taxes.iter().map(|tax| tax.amount.minor()).collect();
        // TTĐB 65.000; GTGT 10% của 165.000 = 16.500.
        assert_eq!(taxes, [65_000, 16_500]);
        assert_eq!(computed.lines[0].gross, vnd(181_500));
        // Giá đã gồm: tách ngược ra đúng giá chưa thuế.
        let back = compute(
            PriceMode::Inclusive,
            Currency::VND,
            &[(line("beer", 181_500), vec![percent(1, "excise", 65), percent(2, "vat", 10)])],
        )
        .unwrap();
        assert_eq!(back.lines[0].net, vnd(100_000));
        let taxes: Vec<i64> = back.lines[0].taxes.iter().map(|tax| tax.amount.minor()).collect();
        assert_eq!(taxes, [65_000, 16_500]);
    }

    #[test]
    fn khong_chiu_thue_va_dong_am_bi_tu_choi() {
        let exempt = ResolvedRate { basis: Basis::Exempt, ..percent(2, "vat", 0) };
        let computed =
            compute(PriceMode::Exclusive, Currency::VND, &[(line("a", 5_000), vec![exempt])])
                .unwrap();
        assert!(computed.lines[0].taxes[0].exempt);
        assert_eq!(computed.lines[0].gross, vnd(5_000));
        let error = compute(
            PriceMode::Exclusive,
            Currency::VND,
            &[(line("a", -1), vec![percent(2, "vat", 10)])],
        )
        .unwrap_err();
        assert_eq!(error.code(), "validation_failed");
    }
}

/// Thử phá bộ tính thuế (D-20261123-property-fuzzing) và các bất biến làm tròn (D-20261126-tax-rounding-allocation).
mod lines;
#[cfg(test)]
mod properties;
