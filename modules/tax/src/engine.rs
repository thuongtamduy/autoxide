//! Tính thuế của một chứng từ từ thuế suất đã tra (không chạm database — test riêng được).
//!
//! Quy tắc (đặc tả 36, bảng làm tròn):
//! - Dòng gom theo **chuỗi thuế suất** giống nhau (vd TTĐB 65% rồi GTGT 10%). Thuế của mỗi nhóm tính
//!   trên **tổng** nhóm, làm tròn `HalfUp` một lần, rồi chia lại về dòng bằng `Money::allocate` theo tỉ
//!   lệ số tiền của dòng — tổng các dòng luôn bằng tổng chứng từ, không làm tròn hai lần.
//! - Nhiều tầng: tầng sau tính trên giá + thuế các tầng trước (GTGT trên giá đã có TTĐB).
//! - Giá đã gồm thuế: tách ngược qua các tầng (`base / (1 + r)`, trừ thuế theo đơn vị), làm tròn
//!   giá chưa thuế một lần; thuế tầng cuối = tổng − giá chưa thuế − các thuế trước (phần dư dồn vào
//!   GTGT); ở dòng, thuế tầng cuối = tổng dòng − phần còn lại của dòng.

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
        // Giá đã gồm thuế: phần dư làm tròn dồn vào tầng CHỊU thuế cuối cùng. Tầng "không chịu thuế"
        // luôn có số thuế 0 — dồn vào nó thì hoá đơn ghi số thuế khác 0 cho dòng không chịu thuế.
        // Tầng thuế suất 0% cũng phải ra đúng 0 (nếu nhận phần dư âm 1 đồng do tầng trước làm tròn lên thì
        // ra thuế âm), nên ưu tiên tầng cuối có thuế suất khác 0; chỉ khi mọi tầng đều 0 mới lùi về
        // tầng chịu thuế cuối (khi đó phần dư luôn bằng 0).
        let last_taxable = chain
            .iter()
            .rposition(|rate| match rate.basis {
                Basis::Exempt => false,
                Basis::Percent => !rate.rate_percent.is_zero(),
                Basis::PerUnit => rate.per_unit.is_some_and(|unit| !unit.to_decimal().is_zero()),
            })
            .or_else(|| chain.iter().rposition(|rate| rate.basis != Basis::Exempt));
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
        // Giá chưa thuế của nhóm, và thuế từng tầng (tổng nhóm), cùng cơ sở của từng tầng.
        let (net, taxes, bases) = match mode {
            PriceMode::Exclusive => {
                let mut base = group_total;
                let mut taxes = Vec::with_capacity(chain.len());
                let mut bases = Vec::with_capacity(chain.len());
                for rate in chain {
                    let tax = stage_tax(rate, base, quantity)?;
                    bases.push(base);
                    taxes.push(tax);
                    base = base.checked_add(tax)?;
                }
                (group_total, taxes, bases)
            }
            PriceMode::Inclusive => {
                // Tách ngược: cơ sở trước mỗi tầng, từ tầng cuối về tầng đầu.
                let mut base = group_total.to_decimal();
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
                let net = Money::from_decimal(base, currency, Rounding::HalfUp)?;
                // Giá đã gồm thuế thấp hơn thuế theo đơn vị nằm trong nó: giá chưa thuế âm là dữ liệu
                // sai (hoá đơn không ghi được), báo cho người gọi thay vì tính tiếp.
                if net.is_negative() {
                    return Err(Error::validation(vec![FieldError::new(
                        format!("lines[{}].amount", indexes[0]),
                        "range",
                        "is lower than the per-unit taxes it includes",
                    )]));
                }
                let mut running = net;
                let mut taxes = Vec::with_capacity(chain.len());
                let mut bases = Vec::with_capacity(chain.len());
                for (position, rate) in chain.iter().enumerate() {
                    bases.push(running);
                    let tax = if Some(position) == last_taxable {
                        // Tầng cuối nhận phần dư: tổng luôn khớp giá đã gồm thuế.
                        group_total.checked_sub(running)?
                    } else {
                        stage_tax(rate, running, quantity)?
                    };
                    taxes.push(tax);
                    running = running.checked_add(tax)?;
                }
                if chain.is_empty() { (group_total, taxes, bases) } else { (net, taxes, bases) }
            }
        };
        for ((rate, tax), base) in chain.iter().zip(&taxes).zip(&bases) {
            let key =
                (rate.tax_kind.clone(), to_column(rate.rate_percent), rate.basis == Basis::Exempt);
            let entry = totals.entry(key).or_insert((Money::zero(currency), Money::zero(currency)));
            entry.0 = entry.0.checked_add(*base)?;
            entry.1 = entry.1.checked_add(*tax)?;
        }
        // Chia về dòng.
        let line_nets = match mode {
            PriceMode::Exclusive => amounts.clone(),
            PriceMode::Inclusive => net.allocate(&shares)?,
        };
        let mut line_taxes: Vec<Vec<Money>> = Vec::with_capacity(chain.len());
        for (position, tax) in taxes.iter().enumerate() {
            let last_inclusive = mode == PriceMode::Inclusive && Some(position) == last_taxable;
            if last_inclusive {
                // Thuế tầng cuối của dòng = tổng dòng − giá chưa thuế − các thuế trước của dòng.
                let mut column = Vec::with_capacity(indexes.len());
                for (slot, amount) in amounts.iter().enumerate() {
                    let mut rest = amount.checked_sub(line_nets[slot])?;
                    for earlier in &line_taxes {
                        rest = rest.checked_sub(earlier[slot])?;
                    }
                    column.push(rest);
                }
                line_taxes.push(column);
            } else {
                line_taxes.push(tax.allocate(&shares)?);
            }
        }
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
        assert_eq!(taxed.taxes[1].amount, vnd(0));
        assert!(!taxed.taxes[0].amount.is_negative());
        assert_eq!(taxed.gross, vnd(454_881_118_563));
        let sum: i64 = taxed.taxes.iter().map(|tax| tax.amount.minor()).sum();
        assert_eq!(taxed.net.minor() + sum, taxed.gross.minor());
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

/// Thử phá bộ tính thuế (D-20261123-property-fuzzing): tổng luôn khớp, không âm, không tràn thành panic.
#[cfg(test)]
mod properties {
    use super::*;
    use proptest::prelude::*;

    /// Một thuế suất; `per_unit = false` thì không sinh thuế theo đơn vị.
    fn rate(per_unit: bool) -> impl Strategy<Value = (Basis, i64, i64)> {
        prop_oneof![
            4 => (0i64..=10_000).prop_map(|bp| (Basis::Percent, bp, 0)),
            u32::from(per_unit) => (0i64..=5_000).prop_map(|unit| (Basis::PerUnit, 0, unit)),
            1 => Just((Basis::Exempt, 0, 0)),
        ]
    }

    /// Chuỗi thuế suất: mỗi tầng một ID riêng.
    fn chain() -> impl Strategy<Value = Vec<ResolvedRate>> {
        chain_of(true)
    }

    fn chain_of(per_unit: bool) -> impl Strategy<Value = Vec<ResolvedRate>> {
        proptest::collection::vec(rate(per_unit), 0..3).prop_map(|rates| {
            rates
                .into_iter()
                .enumerate()
                .map(|(stage, (basis, bp, unit))| ResolvedRate {
                    id: Uuid::from_u128(
                        u128::from(stage as u64) << 64
                            | u128::from(bp.unsigned_abs()) << 32
                            | u128::from(unit.unsigned_abs()),
                    ),
                    tax_kind: format!("kind{stage}"),
                    basis,
                    rate_percent: Decimal::new(bp, 2),
                    per_unit: (basis == Basis::PerUnit).then(|| Money::new(unit, Currency::VND)),
                    stage: i32::try_from(stage).unwrap(),
                })
                .collect()
        })
    }

    fn document() -> impl Strategy<Value = (PriceMode, Vec<(TaxableLine, Vec<ResolvedRate>)>)> {
        document_of(true)
    }

    fn document_of(
        per_unit: bool,
    ) -> impl Strategy<Value = (PriceMode, Vec<(TaxableLine, Vec<ResolvedRate>)>)> {
        let mode = prop_oneof![Just(PriceMode::Inclusive), Just(PriceMode::Exclusive)];
        let lines = (chain_of(per_unit), chain_of(per_unit)).prop_flat_map(|(first, second)| {
            proptest::collection::vec((0i64..=1_000_000_000_000, 0i64..=1_000, any::<bool>()), 1..8)
                .prop_map(move |lines| {
                    lines
                        .into_iter()
                        .enumerate()
                        .map(|(index, (amount, quantity, pick))| {
                            let line = TaxableLine::new(
                                format!("l{index}"),
                                "c",
                                Money::new(amount, Currency::VND),
                            )
                            .with_quantity(Decimal::from(quantity));
                            (line, if pick { first.clone() } else { second.clone() })
                        })
                        .collect::<Vec<_>>()
                })
        });
        (mode, lines)
    }

    proptest! {
        #[test]
        fn tong_dong_khop_va_tong_chung_tu_khop(document in document()) {
            let (mode, lines) = document;
            let computed = match compute(mode, Currency::VND, &lines) {
                Ok(computed) => computed,
                // Chỉ một lỗi được phép: giá đã gồm thuế thấp hơn thuế theo đơn vị.
                Err(error) => {
                    prop_assert!(mode == PriceMode::Inclusive && lines.iter().any(|(_, chain)| chain.iter().any(|rate| rate.basis == Basis::PerUnit)));
                    prop_assert_eq!(error.fields()[0].code.as_str(), "range");
                    return Ok(());
                }
            };
            prop_assert_eq!(computed.lines.len(), lines.len());
            for taxed in &computed.lines {
                prop_assert!(!taxed.net.is_negative(), "{:?}", taxed);
            }
            let mut by_kind: BTreeMap<(String, bool), i128> = BTreeMap::new();
            for (taxed, (line, chain)) in computed.lines.iter().zip(&lines) {
                prop_assert_eq!(&taxed.line_id, &line.line_id);
                let taxes: i128 = taxed.taxes.iter().map(|tax| i128::from(tax.amount.minor())).sum();
                prop_assert_eq!(i128::from(taxed.gross.minor()), i128::from(taxed.net.minor()) + taxes);
                match mode {
                    PriceMode::Exclusive => prop_assert_eq!(taxed.net, line.amount),
                    PriceMode::Inclusive => prop_assert_eq!(taxed.gross, line.amount),
                }
                prop_assert_eq!(taxed.taxes.len(), chain.len());
                for tax in &taxed.taxes {
                    if tax.exempt {
                        prop_assert!(tax.amount.is_zero(), "thuế của dòng không chịu thuế: {:?}", taxed);
                    }
                    *by_kind.entry((tax.tax_kind.clone(), tax.exempt)).or_default() += i128::from(tax.amount.minor());
                }
            }
            let mut totals: BTreeMap<(String, bool), i128> = BTreeMap::new();
            for total in &computed.totals {
                *totals.entry((total.tax_kind.clone(), total.exempt)).or_default() += i128::from(total.amount.minor());
            }
            prop_assert_eq!(by_kind, totals);
        }

        #[test]
        fn gia_chua_thue_phan_tram_khong_am_va_thue_khong_am(
            // Chỉ thuế theo phần trăm: thuế theo đơn vị lớn hơn giá thì giá chưa thuế âm là đúng số học.
            document in document_of(false),
        ) {
            let (mode, lines) = document;
            let computed = compute(mode, Currency::VND, &lines).unwrap();
            for taxed in &computed.lines {
                prop_assert!(!taxed.net.is_negative(), "{:?}", taxed);
                for tax in &taxed.taxes {
                    prop_assert!(!tax.amount.is_negative(), "{:?}", taxed);
                }
            }
        }

        #[test]
        fn so_tien_bien_khong_panic(
            amount in any::<i64>(),
            chain in chain(),
            mode in prop_oneof![Just(PriceMode::Inclusive), Just(PriceMode::Exclusive)],
            quantity in any::<i64>(),
        ) {
            let line = TaxableLine::new("l", "c", Money::new(amount, Currency::VND))
                .with_quantity(Decimal::from(quantity));
            let _ = compute(mode, Currency::VND, &[(line, chain)]);
        }

        #[test]
        fn so_luong_cuc_lon_khong_panic(
            lo in any::<u32>(),
            mid in any::<u32>(),
            hi in any::<u32>(),
            negative in any::<bool>(),
            scale in 0u32..=28,
            lines in 1usize..4,
            chain in chain(),
            mode in prop_oneof![Just(PriceMode::Inclusive), Just(PriceMode::Exclusive)],
        ) {
            // Số lượng do người gọi gửi (`DecimalString`, tới 28 chữ số): cộng hay nhân có thể vượt
            // giới hạn của `Decimal` — phải là lỗi, không phải panic.
            let quantity = Decimal::from_parts(lo, mid, hi, negative, scale);
            let document: Vec<_> = (0..lines)
                .map(|index| {
                    let line = TaxableLine::new(format!("l{index}"), "c", Money::new(1_000, Currency::VND))
                        .with_quantity(quantity);
                    (line, chain.clone())
                })
                .collect();
            let _ = compute(mode, Currency::VND, &document);
        }
    }
    fn rate_of(stage: i32, basis: Basis, percent: i64) -> ResolvedRate {
        ResolvedRate {
            id: Uuid::from_u128(u128::try_from(stage).unwrap() + 1),
            tax_kind: format!("kind{stage}"),
            basis,
            rate_percent: Decimal::new(percent, 2),
            per_unit: (basis == Basis::PerUnit).then(|| Money::new(49, Currency::VND)),
            stage,
        }
    }

    #[test]
    fn tang_khong_chiu_thue_cuoi_chuoi_khong_nhan_phan_du_lam_tron() {
        // Ca tái hiện: giá đã gồm thuế, chuỗi [10.31%, không chịu thuế] — trước khi sửa, phần dư làm
        // tròn (-1 đồng) rơi vào tầng không chịu thuế.
        let chain = vec![rate_of(0, Basis::Percent, 1031), rate_of(1, Basis::Exempt, 0)];
        let lines: Vec<_> = [261_698_875_581, 340_453_309_228, 478_001_272_827]
            .into_iter()
            .enumerate()
            .map(|(index, amount)| {
                (
                    TaxableLine::new(format!("l{index}"), "c", Money::new(amount, Currency::VND)),
                    chain.clone(),
                )
            })
            .collect();
        let computed = compute(PriceMode::Inclusive, Currency::VND, &lines).unwrap();
        for (taxed, (line, _)) in computed.lines.iter().zip(&lines) {
            assert_eq!(taxed.gross, line.amount);
            assert!(taxed.taxes[1].exempt && taxed.taxes[1].amount.is_zero(), "{taxed:?}");
        }
    }

    #[test]
    fn so_luong_qua_lon_la_loi_kiem_dau_vao_khong_panic() {
        // Ca tái hiện: số lượng 28 chữ số làm phép nhân `Decimal` tràn và panic trước khi sửa.
        let quantity = Decimal::from_parts(0, 0, 88_446_609, false, 0);
        let line =
            TaxableLine::new("l0", "c", Money::new(1_000, Currency::VND)).with_quantity(quantity);
        let error = compute(
            PriceMode::Inclusive,
            Currency::VND,
            &[(line, vec![rate_of(0, Basis::PerUnit, 0)])],
        )
        .unwrap_err();
        assert_eq!(error.code(), "validation_failed");
    }
}
