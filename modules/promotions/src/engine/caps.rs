//! Trần pháp luật và sàn giá (D-222): mỗi dòng, cộng MỌI khoản giảm (mọi lớp, mọi bên trả tiền)
//! rồi so với trần; vượt thì cắt, khoản áp sau bị cắt trước. Hàm thuần — service tính trần của
//! từng dòng từ giá tham chiếu (`Catalog::reference_prices`), dữ liệu luật và sàn giá
//! (`Catalog::price_floors`).

use uuid::Uuid;

use super::{Draft, ReasonDraft, Rejection};

/// Tiền giảm tối đa của cả một dòng; `None` = không giới hạn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LineCap {
    /// Trần pháp luật: `max_bp` × giá tham chiếu × số lượng; `Some(0)` khi thiếu giá tham chiếu.
    pub law: Option<i64>,
    /// Sàn giá `block`: (giá − sàn) × số lượng.
    pub floor: Option<i64>,
}

/// Khoản giảm bị giữ lại hẳn trên một dòng.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Withheld {
    pub program: Uuid,
    pub line: usize,
    pub reason: ReasonDraft,
}

/// Cắt `draft` theo `caps` (một phần tử mỗi dòng của giỏ). `missing_reference[line]`: dòng chưa
/// có giá tham chiếu (lý do riêng).
pub(crate) fn apply(
    draft: &mut Draft,
    caps: &[LineCap],
    missing_reference: &[bool],
) -> Vec<Withheld> {
    let mut withheld = Vec::new();
    for (line, cap) in caps.iter().enumerate() {
        for (limit, by_law) in [(cap.law, true), (cap.floor, false)] {
            let Some(limit) = limit else { continue };
            let total: i64 = draft
                .applied
                .iter()
                .flat_map(|applied| applied.lines.iter())
                .filter(|part| part.line == line)
                .map(|part| part.amount)
                .fold(0, i64::saturating_add);
            let mut excess = total - limit.max(0);
            // Khoản áp sau (lớp sau, ưu tiên thấp hơn) bị cắt trước.
            for applied in draft.applied.iter_mut().rev() {
                if excess <= 0 {
                    break;
                }
                for part in applied.lines.iter_mut().filter(|part| part.line == line) {
                    let cut = excess.min(part.amount);
                    if cut <= 0 {
                        continue;
                    }
                    part.amount -= cut;
                    excess -= cut;
                    if by_law {
                        part.capped_by_law = true;
                    } else {
                        part.capped_by_floor = true;
                    }
                    if part.amount == 0 {
                        let code = if !by_law {
                            "promotions.reason.capped_by_floor"
                        } else if missing_reference.get(line).copied().unwrap_or(false) {
                            "promotions.reason.no_reference_price"
                        } else {
                            "promotions.reason.capped_by_law"
                        };
                        withheld.push(Withheld {
                            program: applied.program,
                            line,
                            reason: ReasonDraft::new(code),
                        });
                    }
                }
            }
        }
    }
    // Chương trình không còn giảm gì: bỏ; mã / voucher của nó thành bị từ chối kèm lý do.
    let mut kept = Vec::with_capacity(draft.applied.len());
    for mut applied in std::mem::take(&mut draft.applied) {
        applied.lines.retain(|part| part.amount > 0);
        if applied.lines.is_empty() && applied.shipping == 0 && applied.gifts.is_empty() {
            if let Some(unlock) = &applied.unlock {
                let reason =
                    withheld.iter().find(|found| found.program == applied.program).map_or_else(
                        || ReasonDraft::new("promotions.reason.capped_by_law"),
                        |found| found.reason.clone(),
                    );
                draft.rejected.push(Rejection {
                    program: Some(applied.program),
                    code: unlock.code.clone(),
                    voucher_id: unlock.voucher_id,
                    reason,
                });
            }
            continue;
        }
        kept.push(applied);
    }
    draft.applied = kept;
    withheld
}
