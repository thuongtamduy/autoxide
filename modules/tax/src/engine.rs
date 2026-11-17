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
        let amounts: Vec<Money> = indexes.iter().map(|index| lines[*index].0.amount).collect();
        let quantity: Decimal = indexes
            .iter()
            .filter_map(|index| lines[*index].0.quantity.map(|quantity| quantity.0))
            .sum();
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
                            base - unit * quantity
                        }
                    };
                }
                let net = Money::from_decimal(base, currency, Rounding::HalfUp)?;
                let mut running = net;
                let mut taxes = Vec::with_capacity(chain.len());
                let mut bases = Vec::with_capacity(chain.len());
                for (position, rate) in chain.iter().enumerate() {
                    bases.push(running);
                    let tax = if position + 1 == chain.len() {
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
            let last_inclusive = mode == PriceMode::Inclusive && position + 1 == chain.len();
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
