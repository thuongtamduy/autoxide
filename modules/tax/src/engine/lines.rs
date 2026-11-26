//! Chia số của một nhóm về từng dòng (D-20261126-tax-rounding-allocation).
//!
//! Chuỗi không có thuế theo đơn vị: mỗi cột (giá chưa thuế, từng thuế) chia theo số tiền dòng
//! (`Money::allocate`) — đúng tỉ lệ vì mọi phần của dòng tỉ lệ thuận với số tiền của nó. Chuỗi có thuế theo
//! đơn vị thì không còn tỉ lệ: mỗi cột chia theo giá trị CHÍNH XÁC của chính dòng đó (tách riêng dòng theo
//! số lượng của nó), nên thuế theo đơn vị đi theo số lượng — dòng số lượng 0 không mang nó. Mỗi ô là phần
//! nguyên dưới hay trên của phần đúng tỉ lệ, cột cộng lại đúng tổng nhóm.

use autoxide_kernel::decimal::{Decimal, Rounding};
use autoxide_kernel::error::Error;
use autoxide_kernel::money::Money;

use super::{Basis, ResolvedRate, hundred, reverse_split, too_large};
use crate::api::PriceMode;

/// Dữ liệu của một nhóm để chia về dòng; `slot` là vị trí dòng trong nhóm.
pub(super) struct Split<'a> {
    chain: &'a [ResolvedRate],
    amounts: &'a [Money],
    /// Tỉ lệ theo số tiền dòng (`ratios`).
    shares: &'a [u64],
    /// Số lượng của từng dòng (0 khi không có).
    quantities: &'a [Decimal],
    /// Chuỗi có thuế theo đơn vị: trọng số chính xác theo cột (0 = giá chưa thuế, 1.. = thuế), rồi theo dòng.
    weights: Option<Vec<Vec<Decimal>>>,
}

/// Không chia được mà mọi ô ≥ 0.
pub(super) enum Stuck {
    /// Dòng ở vị trí này có giá thấp hơn thuế theo đơn vị nằm trong nó.
    Line(usize),
    Error(Error),
}

impl From<Error> for Stuck {
    fn from(error: Error) -> Self {
        Stuck::Error(error)
    }
}

impl<'a> Split<'a> {
    pub(super) fn new(
        mode: PriceMode,
        chain: &'a [ResolvedRate],
        amounts: &'a [Money],
        shares: &'a [u64],
        quantities: &'a [Decimal],
    ) -> Result<Self, Error> {
        let weights = if chain.iter().any(|rate| rate.basis == Basis::PerUnit) {
            let mut columns = vec![Vec::with_capacity(amounts.len()); chain.len() + 1];
            for (amount, quantity) in amounts.iter().zip(quantities) {
                let mut base = match mode {
                    PriceMode::Exclusive => amount.to_decimal(),
                    PriceMode::Inclusive => reverse_split(chain, amount.to_decimal(), *quantity)?,
                };
                columns[0].push(base.max(Decimal::ZERO));
                for (position, rate) in chain.iter().enumerate() {
                    let tax = match rate.basis {
                        Basis::Exempt => Some(Decimal::ZERO),
                        Basis::Percent => base.checked_mul(rate.rate_percent / hundred()),
                        Basis::PerUnit => rate.per_unit.map_or(Some(Decimal::ZERO), |unit| {
                            unit.to_decimal().checked_mul(*quantity)
                        }),
                    }
                    .ok_or_else(too_large)?;
                    columns[position + 1].push(tax.max(Decimal::ZERO));
                    base = base.checked_add(tax).ok_or_else(too_large)?;
                }
            }
            Some(columns)
        } else {
            None
        };
        Ok(Split { chain, amounts, shares, quantities, weights })
    }

    fn per_unit(&self, position: usize) -> bool {
        self.chain[position].basis == Basis::PerUnit
    }

    /// Chia tổng của một cột (0 = giá chưa thuế, 1.. = thuế) về dòng.
    fn allocate(&self, column: usize, total: Money) -> Result<Vec<Money>, Error> {
        let Some(weights) = &self.weights else {
            return total.allocate(self.shares);
        };
        let weights = &weights[column];
        // Thuế theo đơn vị luôn theo trọng số (I2: dòng số lượng 0 không mang nó); cột khác có trọng số
        // cộng lại 0 (vd giá chưa thuế chính xác ~0 mà làm tròn ra 1) thì lùi về số tiền dòng.
        if column > 0 && self.per_unit(column - 1)
            || weights.iter().any(|weight| *weight > Decimal::ZERO)
        {
            by_weights(total, weights)
        } else {
            total.allocate(self.shares)
        }
    }

    /// Chia mỗi thuế của nhóm về dòng (giá chưa thuế đã biết từng dòng — chế độ chưa gồm thuế).
    pub(super) fn columns(&self, taxes: &[Money]) -> Result<Vec<Vec<Money>>, Error> {
        taxes.iter().enumerate().map(|(position, tax)| self.allocate(position + 1, *tax)).collect()
    }

    /// Giá đã gồm thuế: trả (giá chưa thuế từng dòng, thuế từng tầng từng dòng) với mỗi dòng cộng đúng số
    /// tiền của nó, mỗi cột cộng đúng tổng nhóm, mọi ô ≥ 0.
    ///
    /// Cột "phần còn lại" `L` là thuế cuối có tổng khác 0 không theo đơn vị (như trước), không có thì giá
    /// chưa thuế; ô của nó = số tiền dòng − các ô khác. Ô âm sửa bằng hoán đổi 2×2: dòng `s` có ô `L` âm
    /// lấy một đơn vị từ cột `j` mà nó có ô > 0, dòng `s'` có ô `L` ≥ 1 (và được mang cột `j`: thuế theo đơn
    /// vị chỉ ở dòng có số lượng > 0) đổi ngược lại. Tổng dòng, tổng cột giữ nguyên; không ô nào xuống dưới
    /// 0; tổng phần âm giảm 1 mỗi lượt nên vòng dừng.
    ///
    /// Không có thuế theo đơn vị thì luôn hoán đổi được: ô `L` âm nghĩa là các ô khác của dòng cộng lại >
    /// số tiền dòng ≥ 0, tức có ô > 0; cột `L` cộng lại ≥ 0 mà có ô âm nên có dòng ô `L` ≥ 1, và dòng nào
    /// cũng được mang cột giá chưa thuế hay thuế theo phần trăm. Có thuế theo đơn vị mà không tìm được dòng
    /// nhận thì số tiền các dòng có số lượng không đủ chứa thuế theo đơn vị — báo lỗi như giá chưa thuế âm.
    pub(super) fn inclusive(
        &self,
        net: Money,
        taxes: &[Money],
    ) -> Result<(Vec<Money>, Vec<Vec<Money>>), Stuck> {
        // Cột 0 = giá chưa thuế, cột 1.. = thuế theo thứ tự tầng.
        let mut columns = Vec::with_capacity(taxes.len() + 1);
        columns.push(self.allocate(0, net)?);
        columns.extend(self.columns(taxes)?);
        let rest = taxes
            .iter()
            .enumerate()
            .rposition(|(position, tax)| !tax.is_zero() && !self.per_unit(position))
            .map_or(0, |position| position + 1);
        for (slot, amount) in self.amounts.iter().enumerate() {
            let mut left = *amount;
            for (column, cells) in columns.iter().enumerate() {
                if column != rest {
                    left = left.checked_sub(cells[slot])?;
                }
            }
            columns[rest][slot] = left;
        }
        self.repair(&mut columns, rest)?;
        let nets = columns.remove(0);
        Ok((nets, columns))
    }

    /// Dòng `slot` được mang ô khác 0 ở `column` không.
    fn may_carry(&self, column: usize, slot: usize) -> bool {
        column == 0 || !self.per_unit(column - 1) || self.quantities[slot] > Decimal::ZERO
    }

    fn repair(&self, columns: &mut [Vec<Money>], rest: usize) -> Result<(), Stuck> {
        let rows = self.amounts.len();
        let Some(first) = self.amounts.first() else { return Ok(()) };
        let unit = Money::new(1, first.currency());
        // Con trỏ dòng nhận cho mỗi cột cho: một dòng đã không nhận được (ô `rest` < 1 hay không được mang
        // cột đó) thì không bao giờ nhận được nữa — ô `rest` của dòng nhận chỉ giảm, dòng âm chỉ tăng tới 0.
        // Nên mỗi con trỏ chỉ tiến, cả vòng O(số dòng × số cột + tổng phần âm × số cột).
        let mut cursors = vec![0usize; columns.len()];
        for slot in 0..rows {
            while columns[rest][slot].is_negative() {
                // Cột cho: thuế có ô lớn nhất ở dòng này (bằng nhau thì tầng sau), rồi tới giá chưa thuế.
                let mut donors: Vec<usize> = (1..columns.len())
                    .filter(|column| *column != rest && columns[*column][slot].minor() > 0)
                    .collect();
                donors.sort_by_key(|column| {
                    std::cmp::Reverse((columns[*column][slot].minor(), *column))
                });
                if rest != 0 && columns[0][slot].minor() > 0 {
                    donors.push(0);
                }
                let mut moved = false;
                for donor in donors {
                    let cursor = &mut cursors[donor];
                    while *cursor < rows
                        && !(columns[rest][*cursor].minor() >= 1 && self.may_carry(donor, *cursor))
                    {
                        *cursor += 1;
                    }
                    let Some(receiver) = (*cursor < rows).then_some(*cursor) else {
                        continue;
                    };
                    columns[donor][slot] = columns[donor][slot].checked_sub(unit)?;
                    columns[rest][slot] = columns[rest][slot].checked_add(unit)?;
                    columns[rest][receiver] = columns[rest][receiver].checked_sub(unit)?;
                    columns[donor][receiver] = columns[donor][receiver].checked_add(unit)?;
                    moved = true;
                    break;
                }
                if !moved {
                    return Err(Stuck::Line(slot));
                }
            }
        }
        Ok(())
    }
}

/// Chia `total` theo trọng số thập phân: phần nguyên dưới của phần đúng tỉ lệ, rồi phần lẻ lớn
/// nhất nhận thêm 1 (bằng nhau thì dòng trước) — như `Money::allocate`. Trọng số 0 luôn nhận 0.
pub(super) fn by_weights(total: Money, weights: &[Decimal]) -> Result<Vec<Money>, Error> {
    let currency = total.currency();
    let sum = weights.iter().try_fold(Decimal::ZERO, |sum, weight| sum.checked_add(*weight));
    let sum = sum.ok_or_else(too_large)?;
    if total.is_zero() {
        return Ok(vec![Money::zero(currency); weights.len()]);
    }
    if sum <= Decimal::ZERO || total.is_negative() {
        return Err(Error::internal("tax rounding: a per-unit tax has no quantity to split by"));
    }
    let amount = total.to_decimal();
    let mut shares = Vec::with_capacity(weights.len());
    let mut remainders = Vec::with_capacity(weights.len());
    for (slot, weight) in weights.iter().enumerate() {
        // Chia trước rồi nhân: tỉ lệ ≤ 1 nên tích không tràn.
        let exact = weight
            .checked_div(sum)
            .and_then(|ratio| ratio.checked_mul(amount))
            .ok_or_else(too_large)?;
        let floor = exact.floor();
        let fraction = exact - floor;
        shares.push(Money::from_decimal(floor, currency, Rounding::HalfUp)?);
        if *weight > Decimal::ZERO {
            remainders.push((fraction, slot));
        }
    }
    let mut left = total.checked_sub(super::sum(&shares, currency)?)?.minor();
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let unit = Money::new(1, currency);
    for (_, slot) in remainders {
        if left <= 0 {
            break;
        }
        shares[slot] = shares[slot].checked_add(unit)?;
        left -= 1;
    }
    if left != 0 {
        return Err(Error::internal("tax rounding: a per-unit tax does not split evenly"));
    }
    Ok(shares)
}
