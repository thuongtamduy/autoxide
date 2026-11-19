//! Lõi ghi tồn: MỌI thay đổi `on_hand` / `available` đi qua file này, và chỉ bằng `rows::adjust`
//! (một câu `UPDATE … SET c = c + Δ WHERE …` có điều kiện, không đọc trước — D-181). Sổ biến động
//! `inventory_movements` chỉ được thêm dòng.
//!
//! Một lệnh nghiệp vụ (điều chỉnh, giữ chỗ, xuất kho…) mở MỘT transaction ghi, gọi [`record`] /
//! [`move_available`] cho từng dòng, rồi [`publish`] để ghi event (gộp theo dòng tồn) trước khi
//! commit.

use std::collections::BTreeMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::money::Money;
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter,
};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{level, movement};
use crate::errors;
use crate::events::{self, Change};
use crate::service::now;

/// Loại dòng của sổ biến động.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MovementKind {
    Receipt,
    Sale,
    Return,
    Adjustment,
    Count,
    TransferOut,
    TransferIn,
    Expiry,
    SupplierReturn,
}

impl MovementKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            MovementKind::Receipt => "receipt",
            MovementKind::Sale => "sale",
            MovementKind::Return => "return",
            MovementKind::Adjustment => "adjustment",
            MovementKind::Count => "count",
            MovementKind::TransferOut => "transfer_out",
            MovementKind::TransferIn => "transfer_in",
            MovementKind::Expiry => "expiry",
            MovementKind::SupplierReturn => "supplier_return",
        }
    }
}

/// Một dòng sổ biến động sắp ghi.
#[derive(Debug, Clone)]
pub(crate) struct NewMovement {
    pub org: Uuid,
    pub variant: Uuid,
    pub kind: MovementKind,
    /// Có dấu: dương vào kho, âm ra kho; khác 0.
    pub quantity: i64,
    /// `false` khi `available` đã đổi từ trước (xuất kho theo chỗ đã giữ): chỉ đổi `on_hand`.
    pub touches_available: bool,
    /// Giá vốn đơn vị của hàng vào (nhập, chuyển đến): tính lại giá vốn bình quân của dòng tồn.
    pub unit_cost: Option<Money>,
    pub lot_id: Option<Uuid>,
    pub reason_code: Option<String>,
    pub note: String,
    pub source_type: &'static str,
    pub source_id: Uuid,
    pub actor: Option<Uuid>,
}

/// Thay đổi của một dòng tồn trong transaction đang chạy.
#[derive(Debug, Clone, Copy, Default)]
struct Delta {
    on_hand: i64,
    available: i64,
}

/// Mọi thay đổi của một transaction, để [`publish`] ghi event gộp theo dòng tồn.
#[derive(Debug, Default)]
pub(crate) struct Touched {
    levels: BTreeMap<Uuid, Delta>,
    costs: BTreeMap<Uuid, Money>,
    movements: Vec<movement::Model>,
}

impl Touched {
    fn add(&mut self, level: Uuid, on_hand: i64, available: i64) {
        let delta = self.levels.entry(level).or_default();
        delta.on_hand += on_hand;
        delta.available += available;
    }
}

fn by_id(tenant: &TenantContext, id: Uuid) -> Condition {
    Condition::all()
        .add(level::Column::Id.eq(id))
        .add(level::Column::TenantId.eq(tenant.id().as_uuid()))
}

/// ID dòng tồn (biến thể × kho), tạo dòng `0 / 0` nếu chưa có. Trong transaction ghi.
pub(crate) async fn level_id<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    org: Uuid,
    variant: Uuid,
) -> Result<Uuid, Error> {
    let row = level::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        org_id: Set(org),
        variant_id: Set(variant),
        on_hand: Set(0),
        available: Set(0),
        incoming: Set(0),
        reorder_point: Set(None),
        allow_negative: Set(false),
        average_cost_minor: Set(0),
        average_cost_currency: Set(None),
        updated_at: Set(now()),
    };
    level::Entity::insert(row)
        .on_conflict(
            OnConflict::columns([
                level::Column::TenantId,
                level::Column::OrgId,
                level::Column::VariantId,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(transaction)
        .await?;
    let found = level::Entity::find()
        .filter(level::Column::OrgId.eq(org))
        .filter(level::Column::VariantId.eq(variant))
        .scoped(tenant)
        .one(transaction)
        .await?
        .ok_or_else(|| Error::internal("stock level vanished right after it was created"))?;
    Ok(found.id)
}

/// Ghi một dòng sổ biến động và đổi tồn theo nó. Hàng ra mà `on_hand` không đủ →
/// `inventory.insufficient_stock` (transaction của bên gọi phải bỏ).
pub(crate) async fn record<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    touched: &mut Touched,
    new: NewMovement,
) -> Result<movement::Model, Error> {
    let level = level_id(transaction, tenant, new.org, new.variant).await?;
    if new.quantity > 0
        && let Some(cost) = new.unit_cost
    {
        // Giá vốn bình quân cần đọc rồi ghi: khoá dòng trước (Postgres `FOR UPDATE`; SQLite đã
        // giữ quyền ghi cả database).
        let row = rows::lock_one(transaction, level::Entity::find_by_id(level).scoped(tenant))
            .await?
            .ok_or_else(|| Error::internal("stock level vanished while locked"))?;
        let average = average_cost(
            row.on_hand,
            row.average_cost_minor,
            row.average_cost_currency.as_deref(),
            new.quantity,
            cost,
        )?;
        if average != row.average_cost_minor
            || row.average_cost_currency.as_deref() != Some(cost.currency().code())
        {
            level::Entity::update_many()
                .col_expr(level::Column::AverageCostMinor, Expr::value(average))
                .col_expr(
                    level::Column::AverageCostCurrency,
                    Expr::value(cost.currency().code().to_owned()),
                )
                .filter(by_id(tenant, level))
                .exec(transaction)
                .await?;
            touched.costs.insert(level, Money::new(average, cost.currency()));
        }
    }
    let changed = rows::adjust::<level::Entity, _>(
        transaction,
        level::Column::OnHand,
        by_id(tenant, level),
        new.quantity,
        Bounds::NON_NEGATIVE,
    )
    .await?;
    if changed == 0 {
        let on_hand = level::Entity::find_by_id(level)
            .scoped(tenant)
            .one(transaction)
            .await?
            .map_or(0, |row| row.on_hand);
        return Err(errors::insufficient_stock(&format!(
            "variant `{}` (on hand {on_hand})",
            new.variant
        )));
    }
    let available = if new.touches_available {
        // `available` được âm (vd hàng hỏng khi đã giữ hết chỗ): chỉ chặn tràn.
        let changed = rows::adjust::<level::Entity, _>(
            transaction,
            level::Column::Available,
            by_id(tenant, level),
            new.quantity,
            Bounds::ANY,
        )
        .await?;
        if changed == 0 {
            return Err(Error::internal("available stock would overflow"));
        }
        new.quantity
    } else {
        0
    };
    touched.add(level, new.quantity, available);
    let row = movement::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        org_id: Set(new.org),
        variant_id: Set(new.variant),
        level_id: Set(level),
        kind: Set(new.kind.as_str().to_owned()),
        quantity: Set(new.quantity),
        unit_cost_minor: Set(new.unit_cost.map(Money::minor)),
        unit_cost_currency: Set(new.unit_cost.map(|cost| cost.currency().code().to_owned())),
        lot_id: Set(new.lot_id),
        reason_code: Set(new.reason_code),
        note: Set(new.note),
        source_type: Set(new.source_type.to_owned()),
        source_id: Set(new.source_id),
        actor_id: Set(new.actor),
        created_at: Set(now()),
    };
    let inserted = movement::Entity::insert(row).exec_with_returning(transaction).await?;
    touched.movements.push(inserted.clone());
    Ok(inserted)
}

/// Đổi `available` của dòng tồn `level` thêm `delta` trong `bounds` (giữ chỗ / trả chỗ). `false`:
/// không đủ (không đổi gì).
pub(crate) async fn move_available<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    touched: &mut Touched,
    level: Uuid,
    delta: i64,
    bounds: Bounds,
) -> Result<bool, Error> {
    let changed = rows::adjust::<level::Entity, _>(
        transaction,
        level::Column::Available,
        by_id(tenant, level),
        delta,
        bounds,
    )
    .await?;
    if changed > 0 {
        touched.add(level, 0, delta);
    }
    Ok(changed > 0)
}

/// Đổi `incoming` (hàng đang về: chuyển kho đang đi đường, đơn đặt hàng) của dòng tồn `level`
/// thêm `delta`, không xuống dưới 0.
pub(crate) async fn move_incoming<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    level: Uuid,
    delta: i64,
) -> Result<(), Error> {
    let changed = rows::adjust::<level::Entity, _>(
        transaction,
        level::Column::Incoming,
        by_id(tenant, level),
        delta,
        Bounds::NON_NEGATIVE,
    )
    .await?;
    if changed == 0 {
        return Err(Error::internal("incoming stock would go below zero"));
    }
    Ok(())
}

/// Đổi tồn của lô `lot` thêm `delta`, không xuống dưới 0. Lỗi `inventory.lot_unavailable` khi
/// lô không đủ.
pub(crate) async fn move_lot<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    lot: Uuid,
    delta: i64,
) -> Result<(), Error> {
    let filter = Condition::all()
        .add(crate::entity::lot::Column::Id.eq(lot))
        .add(crate::entity::lot::Column::TenantId.eq(tenant.id().as_uuid()));
    let changed = rows::adjust::<crate::entity::lot::Entity, _>(
        transaction,
        crate::entity::lot::Column::OnHand,
        filter,
        delta,
        Bounds::NON_NEGATIVE,
    )
    .await?;
    if changed == 0 {
        return Err(crate::lots::lot_unavailable(&lot.to_string()));
    }
    Ok(())
}

/// Giá vốn bình quân gia quyền sau khi nhập `quantity` đơn vị giá `cost`: `(tồn × giá cũ + nhập ×
/// giá nhập) / (tồn + nhập)`, làm tròn nửa lên xa số 0 về đơn vị nhỏ nhất. Tồn ≤ 0 (hay chưa có giá)
/// thì giá vốn = giá nhập. Tính bằng `i128`: không tràn với mọi số `i64`.
pub(crate) fn average_cost(
    on_hand: i64,
    average: i64,
    currency: Option<&str>,
    quantity: i64,
    cost: Money,
) -> Result<i64, Error> {
    let Some(currency) = currency.filter(|_| on_hand > 0) else {
        return Ok(cost.minor());
    };
    if currency != cost.currency().code() {
        return Err(errors::currency_mismatch(currency));
    }
    let total =
        i128::from(on_hand) * i128::from(average) + i128::from(quantity) * i128::from(cost.minor());
    let count = i128::from(on_hand) + i128::from(quantity);
    let rounded = round_half_away(total, count);
    i64::try_from(rounded).map_err(|_| Error::internal("average cost does not fit in 64 bits"))
}

/// `numerator / denominator` (mẫu > 0) làm tròn nửa lên xa số 0.
fn round_half_away(numerator: i128, denominator: i128) -> i128 {
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    if 2 * remainder.abs() >= denominator { quotient + numerator.signum() } else { quotient }
}

/// Ghi event của mọi thay đổi trong `touched` vào outbox (cùng transaction): một
/// `inventory.level_changed` mỗi dòng tồn (gộp), `low_stock` / `out_of_stock` khi vượt ngưỡng,
/// `movement_recorded` mỗi dòng sổ, `cost_changed` mỗi giá vốn mới. Cập nhật `updated_at`.
pub(crate) async fn publish<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    outbox: &Outbox,
    actor: &Actor,
    touched: Touched,
) -> Result<(), Error> {
    let stamp = now();
    for (id, delta) in &touched.levels {
        level::Entity::update_many()
            .col_expr(level::Column::UpdatedAt, Expr::value(stamp))
            .filter(by_id(tenant, *id))
            .exec(transaction)
            .await?;
        let row = level::Entity::find_by_id(*id)
            .scoped(tenant)
            .one(transaction)
            .await?
            .ok_or_else(|| Error::internal("stock level vanished in its own transaction"))?;
        let before_available = row.available - delta.available;
        let before_on_hand = row.on_hand - delta.on_hand;
        let after = json!({
            "level_id": row.id,
            "org_id": row.org_id,
            "variant_id": row.variant_id,
            "on_hand": row.on_hand,
            "available": row.available,
            "reserved": row.on_hand - row.available,
        });
        let subject = events::level_subject(row.id);
        if delta.on_hand != 0 || delta.available != 0 {
            let change = Change {
                before: Some(json!({"on_hand": before_on_hand, "available": before_available})),
                after: Some(after.clone()),
            };
            outbox
                .record(transaction, tenant, *actor, subject.clone(), &events::LevelChanged(change))
                .await?;
        }
        if let Some(point) = row.reorder_point
            && before_available > point
            && row.available <= point
        {
            let mut after = after.clone();
            after["reorder_point"] = json!(point);
            outbox
                .record(
                    transaction,
                    tenant,
                    *actor,
                    subject.clone(),
                    &events::LowStock(Change::after(after)),
                )
                .await?;
        }
        if before_available > 0 && row.available <= 0 {
            outbox
                .record(
                    transaction,
                    tenant,
                    *actor,
                    subject.clone(),
                    &events::OutOfStock(Change::after(after)),
                )
                .await?;
        }
    }
    for row in &touched.movements {
        let after = json!({
            "id": row.id,
            "level_id": row.level_id,
            "org_id": row.org_id,
            "variant_id": row.variant_id,
            "kind": row.kind,
            "quantity": row.quantity,
            "source_type": row.source_type,
            "source_id": row.source_id,
        });
        outbox
            .record(
                transaction,
                tenant,
                *actor,
                events::level_subject(row.level_id),
                &events::MovementRecorded(Change::after(after)),
            )
            .await?;
    }
    for (id, cost) in &touched.costs {
        let row = level::Entity::find_by_id(*id)
            .scoped(tenant)
            .one(transaction)
            .await?
            .ok_or_else(|| Error::internal("stock level vanished in its own transaction"))?;
        let after = json!({
            "org_id": row.org_id,
            "variant_id": row.variant_id,
            "unit_cost": cost,
        });
        outbox
            .record(
                transaction,
                tenant,
                *actor,
                events::level_subject(*id),
                &events::CostChanged(Change::after(after)),
            )
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::money::{Currency, Money};

    use super::{average_cost, round_half_away};

    #[test]
    fn lam_tron_nua_len_xa_so_0() {
        assert_eq!(round_half_away(5, 2), 3);
        assert_eq!(round_half_away(-5, 2), -3);
        assert_eq!(round_half_away(4, 3), 1);
        assert_eq!(round_half_away(10, 4), 3);
        assert_eq!(round_half_away(9, 4), 2);
    }

    #[test]
    fn gia_von_binh_quan_tinh_tay() {
        let vnd = |minor| Money::new(minor, Currency::parse("VND").unwrap());
        // Chưa có tồn: giá vốn = giá nhập.
        assert_eq!(average_cost(0, 0, None, 10, vnd(1000)).unwrap(), 1000);
        // 10 × 1000 + 5 × 1300 = 16 500 / 15 = 1100.
        assert_eq!(average_cost(10, 1000, Some("VND"), 5, vnd(1300)).unwrap(), 1100);
        // 3 × 1000 + 4 × 1001 = 7004 / 7 = 1000,57… → 1001.
        assert_eq!(average_cost(3, 1000, Some("VND"), 4, vnd(1001)).unwrap(), 1001);
        // 1 × 1 + 1 × 2 = 3 / 2 = 1,5 → 2 (nửa lên).
        assert_eq!(average_cost(1, 1, Some("VND"), 1, vnd(2)).unwrap(), 2);
        // Khác tiền tệ khi đang có tồn: lỗi.
        let usd = Money::new(100, Currency::parse("USD").unwrap());
        let error = average_cost(1, 1, Some("VND"), 1, usd).unwrap_err();
        assert_eq!(error.code(), "inventory.currency_mismatch");
        // Số lớn không tràn.
        assert_eq!(
            average_cost(i64::MAX / 2, 1_000_000, Some("VND"), i64::MAX / 2, vnd(1_000_000))
                .unwrap(),
            1_000_000
        );
    }
}
