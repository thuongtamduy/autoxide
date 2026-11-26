//! Chia số của một nhóm về từng dòng (D-20261126-tax-rounding-allocation).
//!
//! Chuỗi không có thuế theo đơn vị: mỗi cột (giá chưa thuế, từng thuế) chia theo số tiền dòng
//! (`Money::allocate`) — đúng tỉ lệ vì mọi phần của dòng tỉ lệ thuận với số tiền của nó. Chuỗi có thuế theo
//! đơn vị thì không còn tỉ lệ: mỗi cột chia theo giá trị CHÍNH XÁC của chính dòng đó (tách riêng dòng theo
//! số lượng của nó), nên thuế theo đơn vị đi theo số lượng — dòng số lượng 0 không mang nó. Mỗi ô là phần
//! nguyên dưới hay trên của phần đúng tỉ lệ, cột cộng lại đúng tổng nhóm.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

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
    /// chưa thuế; ô của nó = số tiền dòng − các ô khác, có thể âm vài đơn vị — sửa ở `repair`.
    ///
    /// Không có thuế theo đơn vị thì luôn sửa được (hoán đổi 2×2 đủ): ô `L` âm nghĩa là các ô khác của dòng
    /// cộng lại > số tiền dòng ≥ 0, tức có ô > 0; cột `L` cộng lại ≥ 0 mà có ô âm nên có dòng ô `L` ≥ 1, và
    /// dòng nào cũng được mang mọi cột. Có thuế theo đơn vị thì chỉ báo lỗi khi thật sự không có cách chia
    /// nào (thuế theo đơn vị của nhóm không vừa số tiền các dòng có số lượng).
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
        let mut cells: Vec<Vec<i64>> =
            columns.iter().map(|column| column.iter().map(|cell| cell.minor()).collect()).collect();
        self.repair(&mut cells, rest)?;
        let currency = net.currency();
        let mut columns: Vec<Vec<Money>> = cells
            .into_iter()
            .map(|column| column.into_iter().map(|minor| Money::new(minor, currency)).collect())
            .collect();
        let nets = columns.remove(0);
        Ok((nets, columns))
    }

    /// Dòng `slot` được mang ô khác 0 ở `column` không.
    fn may_carry(&self, column: usize, slot: usize) -> bool {
        column == 0 || !self.per_unit(column - 1) || self.quantities[slot] > Decimal::ZERO
    }

    /// Sửa các ô âm của cột `rest` mà giữ nguyên tổng dòng và tổng cột, mọi ô ≥ 0.
    ///
    /// Mỗi lượt tăng một ô âm `(s, rest)` thêm 1 bằng một chu trình đổi đơn vị:
    /// 1. Hoán đổi 2×2 (rẻ): cột cho `j` là thuế có ô lớn nhất ở dòng `s` (bằng nhau thì tầng sau), rồi giá
    ///    chưa thuế; dòng nhận là dòng có ô `rest` lớn nhất (bằng nhau thì dòng trước) được mang cột `j` —
    ///    đúng lựa chọn của D-20261126-tax-rounding-allocation, nên chuỗi không có thuế theo đơn vị cho số
    ///    như trước. Heap giữ dòng nhận: ô `rest` của dòng nhận chỉ giảm, dòng âm chỉ tăng tới 0, nên mỗi
    ///    lượt O(log số dòng).
    /// 2. Không hoán đổi 2×2 được (chỉ có thể khi có thuế theo đơn vị — cột đó chỉ ở dòng có số lượng):
    ///    tìm đường tăng bằng BFS trên đồ thị dòng × cột: từ dòng `s` sang cột `j` nếu ô `(s, j)` ≥ 1 (giảm
    ///    được), từ cột `j` sang dòng `r` nếu `r` được mang `j` (tăng được), tới khi gặp dòng `r ≠ s` có ô
    ///    `rest` ≥ 1. Đổi dọc đường: tổng dòng, tổng cột giữ nguyên, không ô nào xuống dưới 0.
    ///
    /// Đầy đủ: nếu có một cách chia hợp lệ `X` (mọi ô ≥ 0, cùng tổng dòng, tổng cột, cùng chỗ được mang) thì
    /// `X − M` có tổng dòng, tổng cột bằng 0 và dương ở `(s, rest)`; tách nó thành các chu trình xen kẽ, chu
    /// trình qua `(s, rest)` đi qua các ô `M` ≥ 1 (chỗ `X − M` âm) và các ô được mang (chỗ `X − M` dương) —
    /// chính là một đường BFS tìm được. Nên BFS không thấy đường nghĩa là không có cách chia nào: chỉ khi
    /// đó mới báo lỗi. Mỗi lượt giảm tổng phần âm 1 nên vòng dừng.
    fn repair(&self, cells: &mut [Vec<i64>], rest: usize) -> Result<(), Stuck> {
        let rows = self.amounts.len();
        let has_per_unit = self.weights.is_some();
        // Heap (ô `rest`, dòng): lớn nhất trước, bằng nhau thì dòng trước. Mục cũ (ô đã giảm) bỏ khi lấy ra.
        let mut all: BinaryHeap<(i64, Reverse<usize>)> = BinaryHeap::new();
        let mut carrying: BinaryHeap<(i64, Reverse<usize>)> = BinaryHeap::new();
        for row in 0..rows {
            if cells[rest][row] >= 1 {
                all.push((cells[rest][row], Reverse(row)));
                if has_per_unit && self.quantities[row] > Decimal::ZERO {
                    carrying.push((cells[rest][row], Reverse(row)));
                }
            }
        }
        for slot in 0..rows {
            while cells[rest][slot] < 0 {
                let mut donors: Vec<usize> = (1..cells.len())
                    .filter(|column| *column != rest && cells[*column][slot] >= 1)
                    .collect();
                donors.sort_by_key(|column| Reverse((cells[*column][slot], *column)));
                if rest != 0 && cells[0][slot] >= 1 {
                    donors.push(0);
                }
                let mut moved = false;
                for donor in donors {
                    let heap = if self.everyone_carries(donor) { &mut all } else { &mut carrying };
                    let Some(receiver) = top(heap, &cells[rest]) else {
                        continue;
                    };
                    cells[donor][slot] -= 1;
                    cells[rest][slot] += 1;
                    cells[rest][receiver] -= 1;
                    cells[donor][receiver] += 1;
                    self.requeue(&mut all, &mut carrying, &cells[rest], receiver);
                    moved = true;
                    break;
                }
                if moved {
                    continue;
                }
                let receiver = self.augment(cells, rest, slot).ok_or(Stuck::Line(slot))?;
                self.requeue(&mut all, &mut carrying, &cells[rest], receiver);
            }
        }
        Ok(())
    }

    /// Mọi dòng đều được mang cột này (giá chưa thuế, thuế theo phần trăm).
    fn everyone_carries(&self, column: usize) -> bool {
        column == 0 || !self.per_unit(column - 1)
    }

    /// Đưa dòng vừa giảm ô `rest` trở lại heap nếu còn nhận được.
    fn requeue(
        &self,
        all: &mut BinaryHeap<(i64, Reverse<usize>)>,
        carrying: &mut BinaryHeap<(i64, Reverse<usize>)>,
        rest: &[i64],
        row: usize,
    ) {
        if rest[row] >= 1 {
            all.push((rest[row], Reverse(row)));
            if self.weights.is_some() && self.quantities[row] > Decimal::ZERO {
                carrying.push((rest[row], Reverse(row)));
            }
        }
    }

    /// Đường tăng từ dòng `slot` (xem `repair`); trả dòng cuối đường (ô `rest` của nó vừa giảm 1).
    fn augment(&self, cells: &mut [Vec<i64>], rest: usize, slot: usize) -> Option<usize> {
        let rows = self.amounts.len();
        let mut row_from: Vec<Option<usize>> = vec![None; rows];
        let mut column_from: Vec<Option<usize>> = vec![None; cells.len()];
        let mut seen = vec![false; rows];
        seen[slot] = true;
        let mut queue = VecDeque::from([slot]);
        let mut end = None;
        'search: while let Some(row) = queue.pop_front() {
            for column in 0..cells.len() {
                if column == rest || column_from[column].is_some() || cells[column][row] < 1 {
                    continue;
                }
                column_from[column] = Some(row);
                for next in 0..rows {
                    if seen[next] || !self.may_carry(column, next) {
                        continue;
                    }
                    seen[next] = true;
                    row_from[next] = Some(column);
                    if cells[rest][next] >= 1 {
                        end = Some(next);
                        break 'search;
                    }
                    queue.push_back(next);
                }
            }
        }
        let end = end?;
        cells[rest][slot] += 1;
        cells[rest][end] -= 1;
        let mut row = end;
        while row != slot {
            let column = row_from[row]?;
            let previous = column_from[column]?;
            cells[column][row] += 1;
            cells[column][previous] -= 1;
            row = previous;
        }
        Some(end)
    }
}

/// Dòng ở đỉnh heap còn nhận được (ô `rest` ≥ 1 và đúng giá trị đã ghi); bỏ mục cũ.
fn top(heap: &mut BinaryHeap<(i64, Reverse<usize>)>, rest: &[i64]) -> Option<usize> {
    while let Some(&(value, Reverse(row))) = heap.peek() {
        if rest[row] == value && value >= 1 {
            return Some(row);
        }
        heap.pop();
    }
    None
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
