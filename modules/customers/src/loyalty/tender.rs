//! Điểm là một cách trả tiền của đơn (`Tender` loại `points`, D-222), cắm vào `orders`.
//!
//! - `hold(reference "orders:<id>")`: trừ số dư dùng được, cộng phần giữ, đánh dấu phần giữ trên
//!   các lô (khoá lô theo `id`), ghi dòng `customers_point_holds` có hạn. Gọi lại cùng `reference`
//!   trả kết quả cũ.
//! - `orders.order_confirmed` → xác nhận: lô mất điểm thật, ghi bút toán đổi điểm.
//! - `orders.order_cancelled` hay hết hạn (job) → trả: điểm về lô và số dư.
//!
//! Mọi bước đổi trạng thái chỗ giữ bằng cập nhật có điều kiện `status = 'held'`: xác nhận và trả
//! cùng lúc chỉ một bên thắng.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::money::Money;
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_orders::api::{Tender, TenderLimit, events as orders};
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use serde_json::{Value, json};
use uuid::Uuid;

use super::current_rule;
use super::ledger::{EXPENSE, LIABILITY, Line, PREPAYMENTS};
use super::points::{Taken, insufficient};
use crate::entity::{point_hold, points_balance, points_lot, profile};
use crate::events::{Change, PointsRedeemed};
use crate::profiles::subject;
use crate::service::{CustomersService, now};

pub(crate) const HELD: &str = "held";
pub(crate) const CONFIRMED: &str = "confirmed";
pub(crate) const RELEASED: &str = "released";

fn need_account() -> Error {
    Error::new(
        ErrorKind::Validation,
        "customers.points_need_account",
        "Sign in to pay with points.",
    )
}

fn below_minimum(min: i64) -> Error {
    Error::new(
        ErrorKind::Validation,
        "customers.redeem_below_minimum",
        "Redeem at least {min} points.",
    )
    .with_param("min", min)
}

fn above_limit(max_percent: i64) -> Error {
    Error::new(
        ErrorKind::Validation,
        "customers.redeem_above_limit",
        "Points can pay at most {max_percent}% of the order.",
    )
    .with_param("max_percent", max_percent)
}

fn not_available() -> Error {
    Error::new(
        ErrorKind::Validation,
        "customers.points_unavailable",
        "This shop has no loyalty program in the order's currency; pay another way.",
    )
}

/// `Tender` loại `points` cho `orders`.
pub(crate) struct PointsTender(pub Arc<CustomersService>);

#[async_trait::async_trait]
impl Tender for PointsTender {
    fn kind(&self) -> &'static str {
        "points"
    }

    /// `input`: `{"points": <số điểm>}`. Trả tiền được trừ = điểm × giá trị điểm hiện hành.
    async fn hold(
        &self,
        tenant: &TenantContext,
        reference: &str,
        buyer: Option<Uuid>,
        input: &Value,
        max: Money,
        expires_at: Timestamp,
    ) -> Result<Money, Error> {
        let buyer = buyer.ok_or_else(need_account)?;
        let points = input["points"].as_i64().filter(|points| *points > 0).ok_or_else(|| {
            Error::validation(vec![autoxide_kernel::error::FieldError::new(
                "points",
                "invalid",
                "must be a positive whole number of points",
            )])
        })?;
        self.0.hold_points(tenant, reference, buyer, points, max, expires_at).await
    }

    async fn release(&self, tenant: &TenantContext, reference: &str) -> Result<(), Error> {
        self.0.release_hold(tenant, reference).await
    }

    /// Số điểm tối đa dùng được cho giỏ (báo giá giỏ, website hiện trước khi đặt). Khách chưa đăng
    /// nhập hay không có chương trình đúng tiền tệ của giỏ: không báo.
    async fn limit(
        &self,
        tenant: &TenantContext,
        buyer: Option<Uuid>,
        max: Money,
    ) -> Result<Option<TenderLimit>, Error> {
        match buyer {
            Some(buyer) => self.0.points_limit(tenant, buyer, max).await,
            None => Ok(None),
        }
    }
}

impl CustomersService {
    /// Tối đa điểm `buyer` dùng được cho một đơn còn phải trả `max`, cùng luật `hold_points` kiểm:
    /// số dư dùng được, trần % đơn, giá trị điểm hiện hành; dưới mức đổi tối thiểu thì 0. Chỉ đọc.
    pub(crate) async fn points_limit(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        max: Money,
    ) -> Result<Option<TenderLimit>, Error> {
        let read = tenant.database().read();
        let Some(rule) = current_rule(read, tenant, now()).await? else {
            return Ok(None);
        };
        if rule.currency != max.currency().code() || rule.point_value_minor <= 0 {
            return Ok(None);
        }
        let profile = profile::Entity::find()
            .filter(profile::Column::UserId.eq(buyer))
            .scoped(tenant)
            .one(read)
            .await?;
        let balance = match profile {
            Some(profile) => self.balance_of(read, tenant, profile.id).await?.0,
            None => 0,
        };
        let cap = super::mul_div(max.minor(), i64::from(rule.max_redeem_ratio_bp), 10_000);
        let mut points = Ord::max(Ord::min(balance, cap / rule.point_value_minor), 0);
        if points < rule.min_redeem_points {
            points = 0;
        }
        let amount = Money::new(points.saturating_mul(rule.point_value_minor), max.currency());
        Ok(Some(TenderLimit::new(amount, Some(points))))
    }

    pub(crate) async fn hold_points(
        &self,
        tenant: &TenantContext,
        reference: &str,
        buyer: Uuid,
        points: i64,
        max: Money,
        expires_at: Timestamp,
    ) -> Result<Money, Error> {
        let read = tenant.database().read();
        if let Some(existing) =
            point_hold::Entity::find_by_id((tenant.id().as_uuid(), reference.to_owned()))
                .one(read)
                .await?
        {
            return Money::from_columns(existing.amount_minor, &existing.currency);
        }
        let rule = current_rule(read, tenant, now()).await?.ok_or_else(not_available)?;
        if rule.currency != max.currency().code() {
            return Err(not_available());
        }
        if points < rule.min_redeem_points {
            return Err(below_minimum(rule.min_redeem_points));
        }
        let amount = points.saturating_mul(rule.point_value_minor);
        let limit = super::mul_div(max.minor(), i64::from(rule.max_redeem_ratio_bp), 10_000);
        if amount > limit {
            return Err(above_limit(i64::from(rule.max_redeem_ratio_bp) / 100));
        }
        let profile = profile::Entity::find()
            .filter(profile::Column::UserId.eq(buyer))
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| insufficient(0, points))?;
        let database = tenant.database();
        let transaction = database.begin_write().await?;
        self.ensure_balance(&transaction, tenant, profile.id).await?;
        // Trừ số dư TRƯỚC (một câu có điều kiện): không đủ thì dừng trước khi đụng lô.
        if !self
            .adjust_balance(
                &transaction,
                tenant,
                profile.id,
                points_balance::Column::Balance,
                -points,
            )
            .await?
        {
            let (available, _) = self.balance_of(&transaction, tenant, profile.id).await?;
            return Err(insufficient(available, points));
        }
        self.adjust_balance(&transaction, tenant, profile.id, points_balance::Column::Held, points)
            .await?;
        let lots = self.lock_lots(&transaction, tenant, profile.id).await?;
        let taken = Self::allocate(&lots, points)?;
        for part in &taken {
            points_lot::Entity::update_many()
                .col_expr(
                    points_lot::Column::Held,
                    Expr::col(points_lot::Column::Held).add(part.points),
                )
                .filter(points_lot::Column::Id.eq(part.lot_id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        point_hold::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            reference: Set(reference.to_owned()),
            profile_id: Set(profile.id),
            points: Set(points),
            amount_minor: Set(amount),
            currency: Set(rule.currency.clone()),
            lots: Set(serde_json::to_string(&taken).map_err(Error::internal)?),
            status: Set(HELD.to_owned()),
            expires_at: Set(expires_at),
            created_at: Set(now()),
        }
        .insert(&transaction)
        .await?;
        self.record_entry(
            &transaction,
            tenant,
            profile.id,
            None,
            "hold",
            -points,
            -amount,
            &rule.currency,
            reference,
            None,
            Actor::user(buyer),
        )
        .await?;
        transaction.commit().await?;
        Money::from_columns(amount, &rule.currency)
    }

    /// Đổi trạng thái chỗ giữ `reference` từ `held` sang `to` (một câu có điều kiện). Trả chỗ giữ
    /// khi đổi được, `None` khi không còn ở `held`.
    async fn take_hold<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        reference: &str,
        to: &str,
    ) -> Result<Option<(point_hold::Model, Vec<Taken>)>, Error> {
        let Some(row) = rows::lock_one(
            connection,
            point_hold::Entity::find_by_id((tenant.id().as_uuid(), reference.to_owned())),
        )
        .await?
        else {
            return Ok(None);
        };
        if row.status != HELD {
            return Ok(None);
        }
        point_hold::Entity::update_many()
            .col_expr(point_hold::Column::Status, Expr::value(to))
            .filter(point_hold::Column::Reference.eq(reference))
            .filter(point_hold::Column::Status.eq(HELD))
            .scoped(tenant)
            .exec(connection)
            .await?;
        let taken: Vec<Taken> = serde_json::from_str(&row.lots).map_err(Error::internal)?;
        Ok(Some((row, taken)))
    }

    /// Trả chỗ giữ: điểm về lô và số dư (không có / đã xử lý: không làm gì).
    pub(crate) async fn release_hold(
        &self,
        tenant: &TenantContext,
        reference: &str,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let Some((row, taken)) = self.take_hold(&transaction, tenant, reference, RELEASED).await?
        else {
            return Ok(());
        };
        for part in &taken {
            points_lot::Entity::update_many()
                .col_expr(
                    points_lot::Column::Held,
                    Expr::col(points_lot::Column::Held).sub(part.points),
                )
                .filter(points_lot::Column::Id.eq(part.lot_id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        self.adjust_balance(
            &transaction,
            tenant,
            row.profile_id,
            points_balance::Column::Held,
            -row.points,
        )
        .await?;
        self.adjust_balance(
            &transaction,
            tenant,
            row.profile_id,
            points_balance::Column::Balance,
            row.points,
        )
        .await?;
        self.record_entry(
            &transaction,
            tenant,
            row.profile_id,
            None,
            "release",
            row.points,
            row.amount_minor,
            &row.currency,
            reference,
            None,
            Actor::system(),
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Xác nhận chỗ giữ khi đơn được xác nhận: lô mất điểm thật, ghi bút toán đổi điểm.
    pub(crate) async fn confirm_hold(
        &self,
        tenant: &TenantContext,
        reference: &str,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let Some((row, taken)) = self.take_hold(&transaction, tenant, reference, CONFIRMED).await?
        else {
            return Ok(());
        };
        let mut liability = 0_i64;
        for part in &taken {
            points_lot::Entity::update_many()
                .col_expr(
                    points_lot::Column::Held,
                    Expr::col(points_lot::Column::Held).sub(part.points),
                )
                .col_expr(
                    points_lot::Column::Remaining,
                    Expr::col(points_lot::Column::Remaining).sub(part.points),
                )
                .filter(points_lot::Column::Id.eq(part.lot_id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
            let value = part.points.saturating_mul(part.value);
            liability = liability.saturating_add(value);
            self.record_entry(
                &transaction,
                tenant,
                row.profile_id,
                Some(part.lot_id),
                "redeem",
                -part.points,
                -value,
                &row.currency,
                reference,
                None,
                Actor::system(),
            )
            .await?;
        }
        self.adjust_balance(
            &transaction,
            tenant,
            row.profile_id,
            points_balance::Column::Held,
            -row.points,
        )
        .await?;
        // Nghĩa vụ giải phóng theo giá trị lúc tích; người mua được trừ theo giá trị hiện hành;
        // chênh lệch là chi phí (hay giảm chi phí) của chương trình.
        let mut lines =
            vec![Line::debit(LIABILITY, liability), Line::credit(PREPAYMENTS, row.amount_minor)];
        let gap = row.amount_minor - liability;
        if gap > 0 {
            lines.push(Line::debit(EXPENSE, gap));
        } else if gap < 0 {
            lines.push(Line::credit(EXPENSE, -gap));
        }
        self.queue_posting(
            &transaction,
            tenant,
            format!("customers:points:redeem:{reference}"),
            "Loyalty points redeemed for an order",
            &row.currency,
            lines,
        )
        .await?;
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::system(),
                Some(subject(row.profile_id)),
                &PointsRedeemed(Change::after(json!({
                    "profile_id": row.profile_id, "points": row.points, "reference": reference,
                    "amount": Money::from_columns(row.amount_minor, &row.currency)?,
                }))),
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Trả mọi chỗ giữ quá hạn của khách.
    pub(crate) async fn release_expired_holds(&self, tenant: &TenantContext) -> Result<u64, Error> {
        let due = point_hold::Entity::find()
            .filter(point_hold::Column::Status.eq(HELD))
            .filter(point_hold::Column::ExpiresAt.lte(now()))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let count = due.len() as u64;
        for row in due {
            self.release_hold(tenant, &row.reference).await?;
        }
        Ok(count)
    }
}

/// Bên nghe `orders.order_confirmed` / `orders.order_cancelled`: xác nhận / trả điểm đã giữ.
pub(crate) struct OrderHolds(pub Arc<CustomersService>);

#[async_trait::async_trait]
impl EventHandler for OrderHolds {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(reference) = event.payload["after"]["hold_reference"].as_str() else {
            return Ok(());
        };
        match event.name.as_str() {
            orders::ORDER_CONFIRMED => {
                self.0.confirm_hold(&event.tenant, reference).await?;
                self.0.settle_value_holds(&event.tenant, reference, true).await
            }
            orders::ORDER_CANCELLED => {
                self.0.release_hold(&event.tenant, reference).await?;
                self.0.settle_value_holds(&event.tenant, reference, false).await
            }
            _ => Ok(()),
        }
    }
}
