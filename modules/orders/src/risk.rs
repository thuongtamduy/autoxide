//! Chống "bom hàng" COD (D-194, đặc tả 45 "Chỗ dễ sai — Chống bom hàng"):
//!
//! - **Dữ liệu chỉ trong khách:** người mua nhận diện bằng khoá liên hệ theo khách
//!   (`SecretKey::phone_key` — HMAC của số E.164 bằng khoá riêng của khách), không lưu số; hai khách
//!   ra hai khoá khác nhau, nên không so được và không chia sẻ danh sách giữa các khách.
//! - **Kết quả giao** (`orders_delivery_outcomes`, một dòng mỗi đơn có khoá): `delivered`, `failed`
//!   (giao không thành), `refused` (huỷ sau khi đã đi giao / hoàn về). Ghi bằng `INSERT … ON
//!   CONFLICT DO UPDATE` theo đơn: event nhận lại không đếm hai lần.
//! - **Quy tắc** (`orders_cod_rules`, có ngày hiệu lực — dữ liệu, không viết cứng): đủ `min_orders`
//!   đơn đã có kết quả mà tỉ lệ không thành ≥ `max_fail_ratio_bp`, hay đơn lớn hơn
//!   `max_order_value` → đơn này phải trả trước (`orders.cod_not_allowed`). Mặc định không có quy tắc
//!   nào (khách tự đặt).
//! - **Danh sách chặn** (`orders_cod_blocks`): có lý do (danh sách `orders.cod_block`) và LUÔN có hạn
//!   (tối đa 365 ngày); gỡ được.
//!
//! Căn cứ xử lý dữ liệu cá nhân (Luật 91/2025/QH15, NĐ 356/2025): thực hiện hợp đồng / lợi ích hợp
//! pháp của người bán — **cần người có chuyên môn xác nhận** và khách ghi vào sổ căn cứ xử lý của
//! `compliance`.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Subscription};
use autoxide_kernel::money::Money;
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::json;
use uuid::Uuid;

use crate::api::{OrderStatus, events as names};
use crate::entity::{cod_block, cod_rule, delivery_outcome};
use crate::errors;
use crate::events::{Change, CodBlockChanged};
use crate::guests::DEFAULT_COUNTRY;
use crate::permissions::RISK_MANAGE;
use crate::reasons::COD_BLOCK;
use crate::service::{OrdersService, now};

pub(crate) const DELIVERED: &str = "delivered";
pub(crate) const FAILED: &str = "failed";
pub(crate) const REFUSED: &str = "refused";

/// Thống kê giao hàng của một người mua trong khách.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct BuyerStats {
    pub delivered: i64,
    pub failed: i64,
    pub refused: i64,
}

impl BuyerStats {
    pub(crate) fn total(self) -> i64 {
        self.delivered + self.failed + self.refused
    }

    /// Tỉ lệ không thành, phần vạn (0 khi chưa có đơn nào).
    pub(crate) fn fail_ratio_bp(self) -> i64 {
        let total = self.total();
        if total == 0 { 0 } else { (self.failed + self.refused) * 10_000 / total }
    }
}

/// Quy tắc mới.
#[derive(Debug, Clone)]
pub(crate) struct NewRule {
    pub org_id: OrgId,
    pub min_orders: i32,
    pub max_fail_ratio_bp: i32,
    pub max_order_value: Option<Money>,
    pub effective_from: Option<Timestamp>,
    pub effective_to: Option<Timestamp>,
}

pub(crate) fn subscriptions(service: &Arc<OrdersService>) -> Vec<Subscription> {
    vec![Subscription::new(names::ORDER_STATUS_CHANGED, Arc::new(Outcomes(service.clone())))]
}

struct Outcomes(Arc<OrdersService>);

#[async_trait::async_trait]
impl EventHandler for Outcomes {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let after = &event.payload["after"];
        let Some(id) = after["order_id"].as_str().and_then(|id| id.parse::<Uuid>().ok()) else {
            return Ok(());
        };
        let before = event.payload["before"]["status"].as_str().and_then(OrderStatus::parse);
        let status = after["status"].as_str().and_then(OrderStatus::parse);
        let outcome = match (before, status) {
            (_, Some(OrderStatus::Delivered)) => DELIVERED,
            (_, Some(OrderStatus::DeliveryFailed)) => FAILED,
            // Huỷ khi hàng đã đi giao: người nhận không nhận / hàng hoàn về.
            (
                Some(OrderStatus::Shipping | OrderStatus::DeliveryFailed),
                Some(OrderStatus::Cancelled),
            ) => REFUSED,
            _ => return Ok(()),
        };
        self.0.record_outcome(&event.tenant, id, outcome).await
    }
}

impl OrdersService {
    /// Ghi (hay đổi) kết quả giao của đơn `id`. Đơn không có khoá liên hệ (quầy) thì bỏ qua.
    async fn record_outcome(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        outcome: &str,
    ) -> Result<(), Error> {
        let row = self.order(tenant, id).await?;
        let Some(key) = row.buyer_key else { return Ok(()) };
        // Đơn đã giao thành công không bị kết quả cũ hơn ghi đè (event tới sai thứ tự).
        let existing = delivery_outcome::Entity::find()
            .filter(delivery_outcome::Column::OrderId.eq(id))
            .scoped(tenant)
            .one(tenant.database().write())
            .await?;
        if existing.is_some_and(|found| found.outcome == DELIVERED && outcome == FAILED) {
            return Ok(());
        }
        delivery_outcome::Entity::insert(delivery_outcome::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            order_id: Set(id),
            buyer_key: Set(key),
            outcome: Set(outcome.to_owned()),
            recorded_at: Set(now()),
        })
        .on_conflict(
            OnConflict::columns([
                delivery_outcome::Column::TenantId,
                delivery_outcome::Column::OrderId,
            ])
            .update_columns([
                delivery_outcome::Column::Outcome,
                delivery_outcome::Column::RecordedAt,
            ])
            .to_owned(),
        )
        .exec_without_returning(tenant.database().write())
        .await?;
        Ok(())
    }

    /// Thống kê giao hàng của khoá liên hệ `key` (một câu, có index).
    pub(crate) async fn buyer_stats(
        &self,
        tenant: &TenantContext,
        key: &str,
    ) -> Result<BuyerStats, Error> {
        use sea_orm::QuerySelect;
        let rows: Vec<(String, i64)> = delivery_outcome::Entity::find()
            .select_only()
            .column(delivery_outcome::Column::Outcome)
            .column_as(delivery_outcome::Column::OrderId.count(), "orders")
            .filter(delivery_outcome::Column::BuyerKey.eq(key))
            .scoped(tenant)
            .group_by(delivery_outcome::Column::Outcome)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        let mut stats = BuyerStats::default();
        for (outcome, count) in rows {
            match outcome.as_str() {
                DELIVERED => stats.delivered = count,
                FAILED => stats.failed = count,
                REFUSED => stats.refused = count,
                _ => {}
            }
        }
        Ok(stats)
    }

    async fn active_block(
        &self,
        tenant: &TenantContext,
        key: &str,
    ) -> Result<Option<cod_block::Model>, Error> {
        Ok(cod_block::Entity::find()
            .filter(cod_block::Column::BuyerKey.eq(key))
            .filter(cod_block::Column::ExpiresAt.gt(now()))
            .filter(cod_block::Column::LiftedAt.is_null())
            .scoped(tenant)
            .order_by_desc(cod_block::Column::ExpiresAt)
            .one(tenant.database().read())
            .await?)
    }

    /// Lúc đặt hàng COD: người mua bị chặn hay rơi vào quy tắc của nút bán (hay của cả khách) đang
    /// hiệu lực → `orders.cod_not_allowed` (phải trả trước).
    pub(crate) async fn check_cod_risk(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        phone: &str,
        total: Money,
    ) -> Result<(), Error> {
        let Some(key) = self.buyer_key(tenant, phone).await? else { return Ok(()) };
        if self.active_block(tenant, &key).await?.is_some() {
            return Err(errors::cod_not_allowed());
        }
        let at = now();
        let root = autoxide_kernel::tenancy::root_of(tenant.id());
        let rules = cod_rule::Entity::find()
            .filter(cod_rule::Column::OrgId.is_in([org.as_uuid(), root.as_uuid()]))
            .filter(cod_rule::Column::EffectiveFrom.lte(at))
            .filter(
                Condition::any()
                    .add(cod_rule::Column::EffectiveTo.is_null())
                    .add(cod_rule::Column::EffectiveTo.gt(at)),
            )
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        if rules.is_empty() {
            return Ok(());
        }
        let stats = self.buyer_stats(tenant, &key).await?;
        for rule in rules {
            let too_big = rule.max_order_value_minor.is_some_and(|max| {
                rule.currency.as_deref() == Some(total.currency().code()) && total.minor() > max
            });
            let risky = stats.total() >= i64::from(rule.min_orders)
                && stats.fail_ratio_bp() >= i64::from(rule.max_fail_ratio_bp);
            if too_big || risky {
                return Err(errors::cod_not_allowed());
            }
        }
        Ok(())
    }

    fn require_risk(principal: &Principal) -> Result<(), Error> {
        principal.require(&RISK_MANAGE)
    }

    /// Khoá liên hệ của `phone` (422 khi không phải số điện thoại).
    async fn key_of_phone(&self, tenant: &TenantContext, phone: &str) -> Result<String, Error> {
        match PhoneNumber::parse(phone, DEFAULT_COUNTRY) {
            Some(parsed) => self.secret.phone_key(tenant.id(), &parsed).await,
            None => Err(Error::validation(vec![FieldError::new(
                "phone",
                "invalid",
                "is not a phone number",
            )])),
        }
    }

    /// `GET /risk/buyers?phone=`: thống kê giao hàng và lần chặn còn hiệu lực của một số điện thoại.
    pub(crate) async fn risk_of_phone(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        phone: &str,
    ) -> Result<(BuyerStats, Option<cod_block::Model>), Error> {
        Self::require_risk(principal)?;
        let key = self.key_of_phone(tenant, phone).await?;
        let stats = self.buyer_stats(tenant, &key).await?;
        Ok((stats, self.active_block(tenant, &key).await?))
    }

    /// `GET /risk/cod-rules`: quy tắc của khách, mới trước.
    pub(crate) async fn cod_rules(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<Vec<cod_rule::Model>, Error> {
        Self::require_risk(principal)?;
        let scope = principal.scope_condition(&RISK_MANAGE, cod_rule::Column::OrgId, None)?;
        Ok(cod_rule::Entity::find()
            .filter(scope)
            .scoped(tenant)
            .order_by_desc(cod_rule::Column::Id)
            .all(tenant.database().read())
            .await?)
    }

    /// `POST /risk/cod-rules`: thêm quy tắc (không sửa quy tắc cũ — kết thúc nó rồi thêm mới).
    pub(crate) async fn add_cod_rule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        rule: NewRule,
    ) -> Result<cod_rule::Model, Error> {
        Self::require_risk(principal)?;
        let target = autoxide_kernel::access::OrgTarget::new(rule.org_id, None);
        principal.require_at(tenant.database().read(), &RISK_MANAGE, target).await?;
        let mut problems = Vec::new();
        if !(1..=1000).contains(&rule.min_orders) {
            problems.push(FieldError::new("min_orders", "range", "must be 1 to 1000"));
        }
        if !(1..=10_000).contains(&rule.max_fail_ratio_bp) {
            problems.push(FieldError::new("max_fail_ratio_bp", "range", "must be 1 to 10000"));
        }
        if rule.max_order_value.is_some_and(|value| !value.is_positive()) {
            problems.push(FieldError::new("max_order_value", "range", "must be more than 0"));
        }
        let from = rule.effective_from.unwrap_or_else(now);
        if rule.effective_to.is_some_and(|to| to <= from) {
            problems.push(FieldError::new(
                "effective_to",
                "range",
                "must be after `effective_from`",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let row = cod_rule::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(rule.org_id.as_uuid()),
            min_orders: Set(rule.min_orders),
            max_fail_ratio_bp: Set(rule.max_fail_ratio_bp),
            max_order_value_minor: Set(rule.max_order_value.map(Money::minor)),
            currency: Set(rule.max_order_value.map(|value| value.currency().code().to_owned())),
            action: Set("require_prepayment".to_owned()),
            effective_from: Set(from),
            effective_to: Set(rule.effective_to),
            created_by: Set(principal.user_id()),
            created_at: Set(now()),
        };
        Ok(row.insert(tenant.database().write()).await?)
    }

    /// `DELETE /risk/cod-rules/{id}`: kết thúc quy tắc từ bây giờ (giữ lịch sử, không xoá).
    pub(crate) async fn end_cod_rule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<cod_rule::Model, Error> {
        Self::require_risk(principal)?;
        let found = cod_rule::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(errors::cod_rule_not_found)?;
        let target = autoxide_kernel::access::OrgTarget::new(OrgId::from_uuid(found.org_id), None);
        principal.require_at(tenant.database().read(), &RISK_MANAGE, target).await?;
        let at = now();
        if found.effective_to.is_some_and(|to| to <= at) {
            return Ok(found);
        }
        let mut edit: cod_rule::ActiveModel = found.into();
        edit.effective_to = Set(Some(at));
        Ok(edit.update(tenant.database().write()).await?)
    }

    /// `GET /risk/cod-blocks`: lần chặn còn hiệu lực, hết hạn sớm trước.
    pub(crate) async fn cod_blocks(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<Vec<cod_block::Model>, Error> {
        Self::require_risk(principal)?;
        Ok(cod_block::Entity::find()
            .filter(cod_block::Column::ExpiresAt.gt(now()))
            .filter(cod_block::Column::LiftedAt.is_null())
            .scoped(tenant)
            .order_by_asc(cod_block::Column::ExpiresAt)
            .all(tenant.database().read())
            .await?)
    }

    /// `POST /risk/cod-blocks`: chặn COD của một số điện thoại, có lý do và hạn (tối đa 365 ngày).
    pub(crate) async fn block_cod(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        phone: &str,
        reason: &str,
        note: Option<String>,
        expires_at: Timestamp,
    ) -> Result<cod_block::Model, Error> {
        Self::require_risk(principal)?;
        let mut problems = Vec::new();
        let key = match PhoneNumber::parse(phone, DEFAULT_COUNTRY) {
            Some(parsed) => Some(self.secret.phone_key(tenant.id(), &parsed).await?),
            None => {
                problems.push(FieldError::new("phone", "invalid", "is not a phone number"));
                None
            }
        };
        if self.reasons.ensure(tenant, COD_BLOCK, reason).await.is_err() {
            problems.push(FieldError::new(
                "reason_code",
                "invalid",
                "is not a reason of the list `orders.cod_block`",
            ));
        }
        let at = now();
        if expires_at <= at || expires_at > at + SignedDuration::from_hours(24 * 365) {
            problems.push(FieldError::new(
                "expires_at",
                "range",
                "must be in the future, at most 365 days from now",
            ));
        }
        if note.as_ref().is_some_and(|note| note.chars().count() > 1000) {
            problems.push(
                FieldError::new("note", "length", "must be at most {max} characters")
                    .with_param("max", 1000),
            );
        }
        let Some(key) = key.filter(|_| problems.is_empty()) else {
            return Err(Error::validation(problems));
        };
        let actor = Actor::user(principal.user_id());
        let row = cod_block::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            buyer_key: Set(key),
            reason_code: Set(reason.to_owned()),
            note: Set(note),
            expires_at: Set(expires_at),
            created_by: Set(principal.user_id()),
            created_at: Set(at),
            lifted_by: Set(None),
            lifted_at: Set(None),
        };
        let transaction = tenant.database().begin_write().await?;
        let saved = row.insert(&transaction).await?;
        let event = CodBlockChanged(Change::after(json!({
            "block_id": saved.id, "tenant_id": saved.tenant_id, "action": "blocked",
            "reason_code": saved.reason_code, "expires_at": saved.expires_at,
        })));
        self.outbox
            .record(
                &transaction,
                tenant,
                actor,
                Some(format!("orders.cod_block:{}", saved.id)),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// `DELETE /risk/cod-blocks/{id}`: gỡ chặn sớm.
    pub(crate) async fn lift_cod_block(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<cod_block::Model, Error> {
        Self::require_risk(principal)?;
        let found = cod_block::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(errors::cod_block_not_found)?;
        if found.lifted_at.is_some() {
            return Ok(found);
        }
        let transaction = tenant.database().begin_write().await?;
        let at = now();
        let lifted = cod_block::Entity::update_many()
            .col_expr(cod_block::Column::LiftedAt, Expr::value(at))
            .col_expr(cod_block::Column::LiftedBy, Expr::value(principal.user_id()))
            .filter(cod_block::Column::Id.eq(id))
            .filter(cod_block::Column::LiftedAt.is_null())
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if lifted.rows_affected > 0 {
            let event = CodBlockChanged(Change::after(json!({
                "block_id": id, "tenant_id": tenant.id(), "action": "lifted",
            })));
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::user(principal.user_id()),
                    Some(format!("orders.cod_block:{id}")),
                    &event,
                )
                .await?;
        }
        let saved = cod_block::Entity::find_by_id(id).scoped(tenant).one(&transaction).await?;
        transaction.commit().await?;
        saved.ok_or_else(errors::cod_block_not_found)
    }
}
