//! Xuất kho theo event giao hàng của `orders` (UT-1): hàng đã tới tay người mua, sổ kho phải theo.
//!
//! Khác `fulfil` gọi tay ở ba chỗ, vì hàng ĐÃ rời đi rồi (không còn quyền từ chối):
//! - số lượng mỗi dòng bị kẹp vào phần còn giữ — event tới lại hay tới sau lần xuất khác không xuất thừa;
//! - hàng theo serial chỉ xuất phần có số serial đã ghi ở đơn và còn trong kho; phần thiếu số giữ nguyên
//!   chỗ giữ (cảnh báo trong log) tới khi nhân viên ghi serial (`orders.line_serials_recorded`);
//! - lô đã quá hạn được lấy khi lô còn hạn không đủ (cảnh báo trong log, xem `lots::lots_out`).
//!
//! Giới hạn: số serial phải nằm ở kho của dòng giữ; một dòng giữ ở nhiều kho thì serial chia theo thứ tự
//! kho, sai kho thì lần xuất lỗi (log) — xem docs/INVENTORY.md.

use std::collections::HashMap;

use autoxide_catalog::api::VariantSnapshot;
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde::Serialize;
use uuid::Uuid;

use crate::api::FulfilLine;
use crate::entity::{reservation_line, serial};
use crate::reservations::remaining;

/// Bao nhiêu đơn vị (của dòng đơn; combo: số combo) cần xuất cho một dòng.
#[derive(Debug, Clone, Serialize)]
pub(crate) enum Want {
    /// Thêm đúng chừng này (một lần giao).
    More(i64),
    /// Mọi phần còn giữ (event cũ không nêu dòng). Với `line_key` rỗng: của mọi dòng ([`DeliveredLine::all`]).
    Remainder,
    /// Tới khi tổng đã xuất của dòng bằng chừng này (đã giao `n` đơn vị, vd khi ghi serial sau).
    UpTo(i64),
}

/// Một dòng của event giao hàng.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct DeliveredLine {
    pub line_key: String,
    pub want: Want,
    /// Số serial đã ghi ở đơn cho dòng (mọi số, kể cả số đã xuất trước).
    pub serials: Vec<String>,
}

impl DeliveredLine {
    pub(crate) fn new(line_key: String, want: Want, serials: Vec<String>) -> Self {
        DeliveredLine { line_key, want, serials }
    }

    /// Phần còn giữ của mọi dòng (event cũ không nêu dòng nào).
    pub(crate) fn all() -> Self {
        DeliveredLine { line_key: String::new(), want: Want::Remainder, serials: Vec::new() }
    }
}

/// Số đơn vị của dòng đơn theo `count` của từng dòng giữ: nhỏ nhất qua các biến thể (combo).
fn units(own: &[&reservation_line::Model], count: impl Fn(&reservation_line::Model) -> i64) -> i64 {
    let mut by_variant: HashMap<Uuid, (i64, i64)> = HashMap::new();
    for line in own {
        let entry = by_variant.entry(line.variant_id).or_insert((0, line.per_unit.max(1)));
        entry.0 += count(line);
    }
    by_variant.values().map(|(total, per_unit)| total / per_unit).min().unwrap_or(0)
}

/// Dòng xuất cho event giao hàng, tính trong transaction đang khoá chỗ giữ `lines`. Dòng không có trong chỗ
/// giữ (hàng số, dịch vụ) bị bỏ qua.
pub(crate) async fn plan<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    order: Uuid,
    lines: &[reservation_line::Model],
    variants: &HashMap<Uuid, VariantSnapshot>,
    wanted: &[DeliveredLine],
) -> Result<Vec<FulfilLine>, Error> {
    let mut keys: Vec<&str> = lines.iter().map(|line| line.line_key.as_str()).collect();
    keys.sort_unstable();
    keys.dedup();
    let every: Vec<DeliveredLine>;
    let wanted = if wanted.iter().any(|want| want.line_key.is_empty()) {
        every = keys
            .iter()
            .map(|key| DeliveredLine::new((*key).to_owned(), Want::Remainder, Vec::new()))
            .collect();
        every.as_slice()
    } else {
        wanted
    };
    let mut out = Vec::new();
    for want in wanted {
        let own: Vec<&reservation_line::Model> =
            lines.iter().filter(|line| line.line_key == want.line_key).collect();
        if own.is_empty() {
            continue;
        }
        let held = units(&own, remaining);
        let issued = units(&own, |line| line.fulfilled_quantity);
        let mut quantity = match want.want {
            Want::More(quantity) => quantity,
            Want::Remainder => held,
            Want::UpTo(total) => total - issued,
        }
        .min(held);
        if quantity <= 0 {
            continue;
        }
        let tracked = own
            .iter()
            .find(|line| variants.get(&line.variant_id).is_some_and(|v| v.serial_tracked));
        let mut serials = Vec::new();
        if let Some(line) = tracked {
            let orgs: Vec<Uuid> = own.iter().map(|line| line.org_id).collect();
            let in_stock: Vec<String> = serial::Entity::find()
                .filter(serial::Column::VariantId.eq(line.variant_id))
                .filter(serial::Column::OrgId.is_in(orgs))
                .filter(serial::Column::Status.eq("in_stock"))
                .filter(serial::Column::Serial.is_in(want.serials.iter().map(String::as_str)))
                .scoped(tenant)
                .all(transaction)
                .await?
                .into_iter()
                .map(|row| row.serial)
                .collect();
            // Giữ thứ tự nhân viên ghi; mỗi đơn vị của dòng đơn cần `per_unit` số.
            let per_unit = line.per_unit.max(1);
            let usable: Vec<String> =
                want.serials.iter().filter(|code| in_stock.contains(code)).cloned().collect();
            let covered = i64::try_from(usable.len()).unwrap_or(i64::MAX) / per_unit;
            if covered < quantity {
                tracing::warn!(
                    order_id = %order,
                    line_key = want.line_key.as_str(),
                    missing = quantity - covered,
                    "serial-tracked units stay reserved until their serial numbers are recorded on the order line"
                );
                quantity = covered;
            }
            let take = usize::try_from(quantity * per_unit).unwrap_or(0);
            serials = usable.into_iter().take(take).collect();
        }
        if quantity > 0 {
            out.push(FulfilLine::new(want.line_key.clone(), quantity).serials(serials));
        }
    }
    Ok(out)
}
