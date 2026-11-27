//! Xuất kho theo event giao hàng của `orders` (UT-1): hàng đã tới tay người mua, sổ kho phải theo.
//!
//! Khác `fulfil` gọi tay, vì hàng ĐÃ rời đi rồi (không còn quyền từ chối):
//! - event nêu TỔNG đơn vị của dòng đã giao (`Want::UpTo`): kho xuất tới đúng tổng đó, kẹp vào phần còn giữ —
//!   event tới lại, tới sai thứ tự, hay sau lần `fulfil` tay lúc gửi kiện đều không xuất thừa;
//! - kho tự chia số lượng và số serial theo từng dòng giữ (kho): serial nào xuất ở kho đang giữ serial đó;
//!   phần thiếu số serial (chưa ghi, hay số không ở kho nào đang giữ dòng) giữ nguyên chỗ giữ (cảnh báo trong
//!   log) tới khi nhân viên ghi serial (`orders.line_serials_recorded`) — các dòng khác vẫn xuất;
//! - chỗ giữ đã hết hạn / đã trả (`lapsed`): phần đã trả lại `available` vẫn xuất, trừ thẳng tồn;
//! - lô đã quá hạn được lấy khi lô còn hạn không đủ (cảnh báo trong log, xem `lots::lots_out`).

use std::collections::{HashMap, VecDeque};

use autoxide_catalog::api::VariantSnapshot;
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde::Serialize;
use uuid::Uuid;

use crate::entity::{reservation_line, serial};
use crate::fulfil_plan::Take;
use crate::reservations::remaining;

/// Bao nhiêu đơn vị (của dòng đơn; combo: số combo) cần xuất cho một dòng.
#[derive(Debug, Clone, Serialize)]
pub(crate) enum Want {
    /// Thêm đúng chừng này (event cũ, trước khi event mang `delivered_quantity`).
    More(i64),
    /// Mọi phần còn giữ (event cũ không nêu dòng). Với `line_key` rỗng: của mọi dòng ([`DeliveredLine::all`]).
    Remainder,
    /// Tới khi tổng đã xuất của dòng bằng chừng này (tổng đã tới tay người mua).
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

/// Phần còn xuất được của dòng giữ: phần đang giữ; chỗ giữ đã hết hạn / đã trả (`lapsed`): phần đã trả lại.
fn capacity(line: &reservation_line::Model, lapsed: bool) -> i64 {
    if lapsed { line.released_quantity } else { remaining(line) }
}

/// Phần xuất cho event giao hàng, tính trong transaction đang khoá chỗ giữ `lines`. Dòng không có trong chỗ
/// giữ (hàng số, dịch vụ) bị bỏ qua.
pub(crate) async fn plan<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    order: Uuid,
    lines: &[reservation_line::Model],
    variants: &HashMap<Uuid, VariantSnapshot>,
    wanted: &[DeliveredLine],
    lapsed: bool,
) -> Result<Vec<Take>, Error> {
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
        let own: Vec<usize> =
            (0..lines.len()).filter(|index| lines[*index].line_key == want.line_key).collect();
        if own.is_empty() {
            continue;
        }
        let rows: Vec<&reservation_line::Model> = own.iter().map(|index| &lines[*index]).collect();
        let held = units(&rows, |line| capacity(line, lapsed));
        let issued = units(&rows, |line| line.fulfilled_quantity);
        let asked = match want.want {
            Want::More(quantity) => quantity,
            Want::Remainder => held,
            Want::UpTo(total) => total - issued,
        };
        if asked > held {
            tracing::warn!(
                order_id = %order,
                line_key = want.line_key.as_str(),
                asked,
                held,
                "the delivery names more units than are still held; issuing what is held"
            );
        }
        let mut quantity = asked.min(held);
        if quantity <= 0 {
            continue;
        }
        // Mỗi biến thể thật (thành phần combo) một lần, kèm số cần cho mỗi đơn vị dòng đơn.
        let mut parts: Vec<(Uuid, i64)> =
            rows.iter().map(|line| (line.variant_id, line.per_unit.max(1))).collect();
        parts.sort_unstable();
        parts.dedup();
        // Số serial còn trong kho theo (biến thể, kho), giữ thứ tự nhân viên ghi.
        let mut buckets: HashMap<(Uuid, Uuid), VecDeque<String>> = HashMap::new();
        for (variant, _) in &parts {
            if !variants.get(variant).is_some_and(|v| v.serial_tracked) {
                continue;
            }
            let orgs: Vec<Uuid> = rows.iter().map(|line| line.org_id).collect();
            let found = serial::Entity::find()
                .filter(serial::Column::VariantId.eq(*variant))
                .filter(serial::Column::OrgId.is_in(orgs))
                .filter(serial::Column::Status.eq("in_stock"))
                .filter(serial::Column::Serial.is_in(want.serials.iter().map(String::as_str)))
                .scoped(tenant)
                .all(transaction)
                .await?;
            for code in &want.serials {
                if let Some(row) = found.iter().find(|row| &row.serial == code)
                    && let Some(org) = row.org_id
                {
                    buckets.entry((*variant, org)).or_default().push_back(code.clone());
                }
            }
        }
        let tracked = |variant: Uuid| variants.get(&variant).is_some_and(|v| v.serial_tracked);
        // Số đơn vị dòng đơn xuất được: theo phần còn giữ, và với hàng theo serial theo số serial ở mỗi kho.
        let mut possible = quantity;
        for (variant, per_unit) in &parts {
            let mut by_org: HashMap<Uuid, i64> = HashMap::new();
            for line in rows.iter().filter(|line| line.variant_id == *variant) {
                *by_org.entry(line.org_id).or_default() += capacity(line, lapsed);
            }
            let total: i64 = by_org
                .iter()
                .map(|(org, room)| {
                    if tracked(*variant) {
                        let codes = buckets.get(&(*variant, *org)).map_or(0, VecDeque::len);
                        (*room).min(i64::try_from(codes).unwrap_or(i64::MAX))
                    } else {
                        *room
                    }
                })
                .sum();
            possible = possible.min(total / per_unit);
        }
        if possible < quantity {
            tracing::warn!(
                order_id = %order,
                line_key = want.line_key.as_str(),
                missing = quantity - possible,
                "serial-tracked units stay reserved until serial numbers in the holding warehouse are recorded on the order line"
            );
            quantity = possible;
        }
        if quantity <= 0 {
            continue;
        }
        for (variant, per_unit) in &parts {
            let mut need = quantity * per_unit;
            for index in own.iter().copied().filter(|index| lines[*index].variant_id == *variant) {
                let line = &lines[index];
                let mut take = need.min(capacity(line, lapsed));
                let serials = if tracked(*variant) {
                    let bucket = buckets.entry((*variant, line.org_id)).or_default();
                    take = take.min(i64::try_from(bucket.len()).unwrap_or(i64::MAX));
                    let count = usize::try_from(take).unwrap_or(0);
                    Some(bucket.drain(..count).collect())
                } else {
                    None
                };
                if take > 0 {
                    out.push(Take { index, quantity: take, serials });
                    need -= take;
                }
            }
            if need > 0 {
                return Err(Error::internal("delivery plan left units without a holding line"));
            }
        }
    }
    Ok(out)
}
