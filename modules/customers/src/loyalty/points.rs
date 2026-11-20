//! Lô điểm, sổ điểm và số dư (bước 47 mục 3).
//!
//! - Tích: một lô mới (`source_ref` unique — event nhận lại không tích hai lần), cộng số dư.
//! - Tiêu ngay (điều chỉnh âm, đổi lấy voucher, thu hồi khi hoàn): trừ số dư bằng `rows::adjust`
//!   TRƯỚC (thất bại → `customers.insufficient_points` trước khi đụng lô), rồi khoá các lô theo `id`
//!   (cùng thứ tự ở mọi transaction — không deadlock) và tiêu lô hết hạn sớm nhất trước.
//! - Hết hạn: job hằng ngày, phần chưa giữ của lô quá hạn.

use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::Actor;
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, ExprTrait as _, OnConflict};
use sea_orm::{ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde_json::json;
use uuid::Uuid;

use super::ledger::{BREAKAGE, EXPENSE, LIABILITY, Line};
use super::{current_rule, current_tiers, expiry_after, mul_div};
use crate::entity::{order_fact, points_balance, points_entry, points_lot, profile};
use crate::events::{Change, PointsAdjusted, PointsEarned, PointsExpired};
use crate::profiles::subject;
use crate::service::{CustomersService, now};

/// Một phần của lô bị tiêu: lô, số điểm, giá trị một điểm lúc tích.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct Taken {
    pub lot_id: Uuid,
    pub points: i64,
    pub value: i64,
}

/// Lô mới.
pub(crate) struct NewLot<'a> {
    pub profile_id: Uuid,
    pub points: i64,
    pub value: i64,
    pub currency: &'a str,
    pub valid_months: i32,
    /// Khoá chống tích hai lần, vd `orders:<id>:earn`.
    pub source_ref: String,
    /// `earn`, `adjust` hay `restore`.
    pub kind: &'static str,
    pub actor: Actor,
    pub reason: Option<String>,
}

pub(crate) fn insufficient(available: i64, requested: i64) -> Error {
    Error::new(
        ErrorKind::Conflict,
        "customers.insufficient_points",
        "Not enough points: {available} available, {requested} requested.",
    )
    .with_param("available", available)
    .with_param("requested", requested)
}

impl CustomersService {
    pub(crate) async fn ensure_balance<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        profile_id: Uuid,
    ) -> Result<(), Error> {
        points_balance::Entity::insert(points_balance::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            profile_id: Set(profile_id),
            balance: Set(0),
            held: Set(0),
            updated_at: Set(now()),
        })
        .on_conflict(
            OnConflict::columns([
                points_balance::Column::TenantId,
                points_balance::Column::ProfileId,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(connection)
        .await?;
        Ok(())
    }

    /// Cộng / trừ cột `column` của số dư (không âm). Trả `false` khi không đủ.
    pub(crate) async fn adjust_balance<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        profile_id: Uuid,
        column: points_balance::Column,
        delta: i64,
    ) -> Result<bool, Error> {
        let filter = Condition::all()
            .add(points_balance::Column::TenantId.eq(tenant.id().as_uuid()))
            .add(points_balance::Column::ProfileId.eq(profile_id));
        let changed = rows::adjust::<points_balance::Entity, _>(
            connection,
            column,
            filter,
            delta,
            Bounds::NON_NEGATIVE,
        )
        .await?;
        Ok(changed == 1)
    }

    /// Số dư hiện tại (dùng được, đang giữ).
    pub(crate) async fn balance_of<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        profile_id: Uuid,
    ) -> Result<(i64, i64), Error> {
        let row = points_balance::Entity::find_by_id((tenant.id().as_uuid(), profile_id))
            .one(connection)
            .await?;
        Ok(row.map_or((0, 0), |row| (row.balance, row.held)))
    }

    /// Ghi một dòng sổ điểm.
    #[expect(clippy::too_many_arguments, reason = "một dòng sổ điểm có chừng đó trường")]
    pub(crate) async fn record_entry<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        profile_id: Uuid,
        lot_id: Option<Uuid>,
        kind: &str,
        points: i64,
        value_minor: i64,
        currency: &str,
        reference: &str,
        reason: Option<String>,
        actor: Actor,
    ) -> Result<Uuid, Error> {
        let id = Uuid::now_v7();
        points_entry::Entity::insert(points_entry::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            profile_id: Set(profile_id),
            lot_id: Set(lot_id),
            lot_key: Set(lot_id.map(|id| id.to_string()).unwrap_or_default()),
            kind: Set(kind.to_owned()),
            points: Set(points),
            value_minor: Set(value_minor),
            currency: Set(currency.to_owned()),
            reference: Set(reference.to_owned()),
            reason: Set(reason),
            actor_id: Set(actor.user_id),
            created_at: Set(now()),
        })
        .exec_without_returning(connection)
        .await?;
        Ok(id)
    }

    /// Tạo lô mới; `None` khi `source_ref` đã có lô (gọi lại không tích thêm).
    pub(crate) async fn add_lot<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        lot: NewLot<'_>,
    ) -> Result<Option<Uuid>, Error> {
        if lot.points <= 0 {
            return Ok(None);
        }
        let id = Uuid::now_v7();
        let at = now();
        let inserted = points_lot::Entity::insert(points_lot::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            profile_id: Set(lot.profile_id),
            earned: Set(lot.points),
            remaining: Set(lot.points),
            held: Set(0),
            value_minor_per_point: Set(lot.value),
            currency: Set(lot.currency.to_owned()),
            expires_at: Set(expiry_after(at, lot.valid_months)),
            source_ref: Set(lot.source_ref.clone()),
            created_at: Set(at),
        })
        .on_conflict(
            OnConflict::columns([points_lot::Column::TenantId, points_lot::Column::SourceRef])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(connection)
        .await?;
        if inserted == 0 {
            return Ok(None);
        }
        self.ensure_balance(connection, tenant, lot.profile_id).await?;
        self.adjust_balance(
            connection,
            tenant,
            lot.profile_id,
            points_balance::Column::Balance,
            lot.points,
        )
        .await?;
        let value = lot.points.saturating_mul(lot.value);
        let entry = self
            .record_entry(
                connection,
                tenant,
                lot.profile_id,
                Some(id),
                lot.kind,
                lot.points,
                value,
                lot.currency,
                &lot.source_ref,
                lot.reason.clone(),
                lot.actor,
            )
            .await?;
        self.queue_posting(
            connection,
            tenant,
            format!("customers:points:{entry}"),
            "Loyalty points earned",
            lot.currency,
            vec![Line::debit(EXPENSE, value), Line::credit(LIABILITY, value)],
        )
        .await?;
        let payload =
            json!({"profile_id": lot.profile_id, "points": lot.points, "source": lot.kind});
        if lot.kind == "adjust" {
            self.outbox
                .record(
                    connection,
                    tenant,
                    lot.actor,
                    Some(subject(lot.profile_id)),
                    &PointsAdjusted(Change::after(payload)),
                )
                .await?;
        } else {
            self.outbox
                .record(
                    connection,
                    tenant,
                    lot.actor,
                    Some(subject(lot.profile_id)),
                    &PointsEarned(Change::after(payload)),
                )
                .await?;
        }
        Ok(Some(id))
    }

    /// Khoá các lô còn điểm chưa giữ của hồ sơ (theo `id`), trả theo thứ tự tiêu: hết hạn sớm
    /// nhất trước.
    pub(crate) async fn lock_lots<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        profile_id: Uuid,
    ) -> Result<Vec<points_lot::Model>, Error> {
        let select = points_lot::Entity::find()
            .filter(points_lot::Column::ProfileId.eq(profile_id))
            .filter(
                Expr::col(points_lot::Column::Remaining).gt(Expr::col(points_lot::Column::Held)),
            )
            .scoped(tenant);
        let mut lots = rows::lock(connection, select, points_lot::Column::Id).await?;
        lots.sort_by(|a, b| a.expires_at.cmp(&b.expires_at).then(a.id.cmp(&b.id)));
        Ok(lots)
    }

    /// Chia `points` cho các lô theo thứ tự tiêu (đã khoá).
    pub(crate) fn allocate(lots: &[points_lot::Model], points: i64) -> Result<Vec<Taken>, Error> {
        let mut left = points;
        let mut taken = Vec::new();
        for lot in lots {
            if left == 0 {
                break;
            }
            let free = lot.remaining - lot.held;
            let take = Ord::min(free, left);
            if take > 0 {
                taken.push(Taken {
                    lot_id: lot.id,
                    points: take,
                    value: lot.value_minor_per_point,
                });
                left -= take;
            }
        }
        if left > 0 {
            return Err(Error::internal("points balance and lots disagree"));
        }
        Ok(taken)
    }

    /// Tiêu ngay `points` điểm (điều chỉnh âm, đổi voucher, thu hồi): trừ số dư rồi lô. Ghi một
    /// dòng sổ mỗi lô với `kind`, `reference`. Trả các phần đã tiêu.
    #[expect(clippy::too_many_arguments, reason = "tiêu điểm cần chừng đó ngữ cảnh")]
    pub(crate) async fn spend_now<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        profile_id: Uuid,
        points: i64,
        kind: &str,
        reference: &str,
        reason: Option<String>,
        actor: Actor,
    ) -> Result<Vec<Taken>, Error> {
        self.ensure_balance(connection, tenant, profile_id).await?;
        if !self
            .adjust_balance(
                connection,
                tenant,
                profile_id,
                points_balance::Column::Balance,
                -points,
            )
            .await?
        {
            let (available, _) = self.balance_of(connection, tenant, profile_id).await?;
            return Err(insufficient(available, points));
        }
        let lots = self.lock_lots(connection, tenant, profile_id).await?;
        let taken = Self::allocate(&lots, points)?;
        for part in &taken {
            points_lot::Entity::update_many()
                .col_expr(
                    points_lot::Column::Remaining,
                    Expr::col(points_lot::Column::Remaining).sub(part.points),
                )
                .filter(points_lot::Column::Id.eq(part.lot_id))
                .scoped(tenant)
                .exec(connection)
                .await?;
            let currency = lots
                .iter()
                .find(|lot| lot.id == part.lot_id)
                .map_or("VND", |lot| lot.currency.as_str());
            self.record_entry(
                connection,
                tenant,
                profile_id,
                Some(part.lot_id),
                kind,
                -part.points,
                -part.points.saturating_mul(part.value),
                currency,
                reference,
                reason.clone(),
                actor,
            )
            .await?;
        }
        Ok(taken)
    }

    /// Tích điểm cho đơn vừa hoàn tất (gọi trong transaction của `facts`): theo quy tắc hiện hành
    /// và hệ số hạng của hồ sơ, trên tiền hàng đã trả trừ phần hoàn (D-226).
    pub(crate) async fn earn_for_order<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        fact: &order_fact::Model,
    ) -> Result<(), Error> {
        let at = now();
        let Some(rule) = current_rule(connection, tenant, at).await? else { return Ok(()) };
        if rule.currency != fact.currency || rule.earn_per_minor <= 0 {
            return Ok(());
        }
        let base = Ord::max(fact.net_minor - fact.refunded_minor, 0);
        let mut points = (base / rule.earn_per_minor).saturating_mul(rule.points_per_unit);
        let tier_code = profile::Entity::find_by_id(fact.profile_id)
            .one(connection)
            .await?
            .and_then(|row| row.tier_code);
        if let Some(code) = tier_code
            && let Some(tier) = current_tiers(connection, tenant, at)
                .await?
                .into_iter()
                .find(|tier| tier.code == code)
        {
            points = mul_div(points, i64::from(tier.earn_multiplier_bp), 10_000);
        }
        let lot = NewLot {
            profile_id: fact.profile_id,
            points,
            value: rule.point_value_minor,
            currency: &rule.currency,
            valid_months: rule.points_valid_months,
            source_ref: format!("orders:{}:earn", fact.order_id),
            kind: "earn",
            actor: Actor::system(),
            reason: None,
        };
        self.add_lot(connection, tenant, lot).await?;
        Ok(())
    }

    /// Hoàn tiền đơn đã hoàn tất: thu hồi phần điểm tương ứng của lô đã tích cho đơn — tối đa phần
    /// còn chưa dùng (không làm số dư âm; phần đã tiêu không thu hồi được, đã nằm ở chi phí).
    pub(crate) async fn reverse_for_refund<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        fact: &order_fact::Model,
        refund_id: Uuid,
        amount: i64,
    ) -> Result<(), Error> {
        let source = format!("orders:{}:earn", fact.order_id);
        let Some(lot) = rows::lock_one(
            connection,
            points_lot::Entity::find()
                .filter(points_lot::Column::SourceRef.eq(source))
                .scoped(tenant),
        )
        .await?
        else {
            return Ok(());
        };
        let wanted = mul_div(lot.earned, amount, Ord::max(fact.net_minor, 1));
        let free = Ord::max(lot.remaining - lot.held, 0);
        let take = Ord::min(wanted, free);
        if take <= 0 {
            return Ok(());
        }
        if !self
            .adjust_balance(
                connection,
                tenant,
                lot.profile_id,
                points_balance::Column::Balance,
                -take,
            )
            .await?
        {
            return Ok(());
        }
        points_lot::Entity::update_many()
            .col_expr(
                points_lot::Column::Remaining,
                Expr::col(points_lot::Column::Remaining).sub(take),
            )
            .filter(points_lot::Column::Id.eq(lot.id))
            .scoped(tenant)
            .exec(connection)
            .await?;
        let value = take.saturating_mul(lot.value_minor_per_point);
        let entry = self
            .record_entry(
                connection,
                tenant,
                lot.profile_id,
                Some(lot.id),
                "reverse",
                -take,
                -value,
                &lot.currency,
                &format!("orders:refund:{refund_id}"),
                None,
                Actor::system(),
            )
            .await?;
        self.queue_posting(
            connection,
            tenant,
            format!("customers:points:{entry}"),
            "Loyalty points taken back after a refund",
            &lot.currency,
            vec![Line::debit(LIABILITY, value), Line::credit(EXPENSE, value)],
        )
        .await?;
        self.outbox
            .record(
                connection,
                tenant,
                Actor::system(),
                Some(subject(lot.profile_id)),
                &PointsAdjusted(Change::after(json!({
                    "profile_id": lot.profile_id, "points": -take, "source": "refund",
                }))),
            )
            .await
    }

    /// Hết hạn mọi lô quá hạn của khách (phần chưa giữ). Trả số lô đã xử lý.
    pub(crate) async fn expire_lots(
        &self,
        tenant: &TenantContext,
        at: DateTime<Utc>,
    ) -> Result<u64, Error> {
        let due: Vec<Uuid> = points_lot::Entity::find()
            .filter(points_lot::Column::ExpiresAt.lte(at))
            .filter(
                Expr::col(points_lot::Column::Remaining).gt(Expr::col(points_lot::Column::Held)),
            )
            .scoped(tenant)
            .order_by_asc(points_lot::Column::Id)
            .all(tenant.database().read())
            .await?
            .into_iter()
            .map(|lot| lot.id)
            .collect();
        let mut done = 0;
        for id in due {
            let transaction = tenant.database().begin_write().await?;
            let Some(lot) =
                rows::lock_one(&transaction, points_lot::Entity::find_by_id(id).scoped(tenant))
                    .await?
            else {
                continue;
            };
            let expired = lot.remaining - lot.held;
            if expired <= 0 || lot.expires_at > at {
                continue;
            }
            points_lot::Entity::update_many()
                .col_expr(points_lot::Column::Remaining, Expr::value(lot.held))
                .filter(points_lot::Column::Id.eq(lot.id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
            // Số dư luôn gồm phần chưa giữ của lô; trừ không đủ nghĩa là sổ lệch — báo lỗi để
            // transaction rollback, không nuốt.
            if !self
                .adjust_balance(
                    &transaction,
                    tenant,
                    lot.profile_id,
                    points_balance::Column::Balance,
                    -expired,
                )
                .await?
            {
                return Err(Error::internal("points balance lower than an expiring lot"));
            }
            let value = expired.saturating_mul(lot.value_minor_per_point);
            let entry = self
                .record_entry(
                    &transaction,
                    tenant,
                    lot.profile_id,
                    Some(lot.id),
                    "expire",
                    -expired,
                    -value,
                    &lot.currency,
                    &format!("expire:{}", lot.id),
                    None,
                    Actor::system(),
                )
                .await?;
            self.queue_posting(
                &transaction,
                tenant,
                format!("customers:points:{entry}"),
                "Loyalty points expired",
                &lot.currency,
                vec![Line::debit(LIABILITY, value), Line::credit(BREAKAGE, value)],
            )
            .await?;
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::system(),
                    Some(subject(lot.profile_id)),
                    &PointsExpired(Change::after(json!({
                        "profile_id": lot.profile_id, "points": expired, "lot_id": lot.id,
                    }))),
                )
                .await?;
            transaction.commit().await?;
            done += 1;
        }
        Ok(done)
    }
}
