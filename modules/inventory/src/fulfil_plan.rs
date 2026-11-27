//! Chia một lần xuất kho theo chỗ giữ thành từng phần (dòng giữ, số lượng, số serial).
//!
//! `fulfil` gọi tay: [`manual_plan`] (người gọi nêu số lượng và serial theo dòng đơn). Event giao hàng:
//! `delivery::plan` (kho tự chia theo phần còn giữ và kho của serial). Cả hai trả [`Take`].

use std::collections::HashMap;

use autoxide_catalog::api::VariantSnapshot;
use autoxide_kernel::error::Error;
use uuid::Uuid;

use crate::api::FulfilRequest;
use crate::entity::reservation_line;
use crate::reservations::remaining;

/// Phần xuất ở một dòng giữ (`index` trong danh sách dòng giữ của chỗ giữ).
#[derive(Debug)]
pub(crate) struct Take {
    pub index: usize,
    pub quantity: i64,
    /// Số serial cho đúng `quantity` đơn vị (hàng theo serial).
    pub serials: Option<Vec<String>>,
}

fn over_fulfilment(line_key: &str) -> Error {
    Error::conflict(
        "inventory.over_fulfilment",
        "Line `{line_key}` ships more than what is reserved for it.",
    )
    .with_param("line_key", line_key.to_owned())
}

/// Phần xuất của `fulfil` gọi tay: số lượng theo [`takes`], số serial của mỗi dòng yêu cầu cắt lần lượt theo
/// thứ tự các phần (người gọi nêu đúng số cho từng phần).
pub(crate) fn manual_plan(
    lines: &[reservation_line::Model],
    request: &FulfilRequest,
    variants: &HashMap<Uuid, VariantSnapshot>,
) -> Result<Vec<Take>, Error> {
    let mut takes = takes(lines, request)?;
    takes.sort_by_key(|(index, _)| lines[*index].level_id);
    let mut serials_used: HashMap<&str, usize> = HashMap::new();
    let mut plan = Vec::with_capacity(takes.len());
    for (index, quantity) in takes {
        let line = &lines[index];
        let serials = if variants.get(&line.variant_id).is_some_and(|v| v.serial_tracked) {
            let wanted = request.lines.iter().find(|wanted| wanted.line_key == line.line_key);
            let all: &[String] = wanted.map_or(&[], |wanted| wanted.serials.as_slice());
            let used = serials_used.entry(line.line_key.as_str()).or_default();
            let count = usize::try_from(quantity).unwrap_or(usize::MAX);
            let Some(part) = all.get(*used..used.saturating_add(count)) else {
                return Err(crate::lots::serials_required(&line.line_key, quantity));
            };
            *used += count;
            Some(part.to_vec())
        } else {
            None
        };
        plan.push(Take { index, quantity, serials });
    }
    Ok(plan)
}

/// Phần lấy từ từng dòng chỗ giữ cho lần xuất `request` (chỉ số dòng, số lượng). Dòng đơn trống:
/// mọi phần còn giữ.
fn takes(
    lines: &[reservation_line::Model],
    request: &FulfilRequest,
) -> Result<Vec<(usize, i64)>, Error> {
    if request.lines.is_empty() {
        return Ok(lines
            .iter()
            .enumerate()
            .filter(|(_, line)| remaining(line) > 0)
            .map(|(index, line)| (index, remaining(line)))
            .collect());
    }
    let mut left: Vec<i64> = lines.iter().map(remaining).collect();
    let mut takes = Vec::new();
    for wanted in &request.lines {
        let own: Vec<usize> =
            (0..lines.len()).filter(|index| lines[*index].line_key == wanted.line_key).collect();
        if own.is_empty() {
            return Err(over_fulfilment(&wanted.line_key));
        }
        // Mỗi biến thể thật (thành phần combo) cần `số đơn vị × per_unit`, lấy lần lượt qua các kho.
        let mut variants: Vec<(Uuid, i64)> =
            own.iter().map(|index| (lines[*index].variant_id, lines[*index].per_unit)).collect();
        variants.sort_unstable();
        variants.dedup();
        for (variant, per_unit) in variants {
            let mut need = wanted
                .quantity
                .checked_mul(per_unit)
                .ok_or_else(|| over_fulfilment(&wanted.line_key))?;
            for index in own.iter().copied().filter(|index| lines[*index].variant_id == variant) {
                let take = need.min(left[index]);
                if take > 0 {
                    takes.push((index, take));
                    left[index] -= take;
                    need -= take;
                }
            }
            if need > 0 {
                return Err(over_fulfilment(&wanted.line_key));
            }
        }
    }
    Ok(takes)
}
