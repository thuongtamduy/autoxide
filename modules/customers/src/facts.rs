//! Lịch sử mua dựng từ event của `orders` (bước 47 mục 1): bảng `customers_order_facts` chép tóm tắt
//! đơn (không đọc bảng của `orders`), tổng chi và số đơn trên hồ sơ.
//!
//! - Tổng chi = Σ `goods_paid` của đơn `completed` − hoàn tiền (D-226).
//! - Số đơn = đơn đã đặt chưa bị huỷ.
//!
//! Event nhận lại không cộng hai lần: dòng của đơn chèn kiểu "trùng thì bỏ qua", trạng thái đổi bằng
//! câu cập nhật có điều kiện (đếm số dòng bị đổi), khoản hoàn khoá theo `refund_id`.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::money::Money;
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_orders::api::events as orders;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use serde_json::Value;
use uuid::Uuid;

use crate::entity::{order_fact, profile, refund_fact};
use crate::service::{CustomersService, now};

const COMPLETED: &str = "completed";
const CANCELLED: &str = "cancelled";

/// Bên nghe `orders.order_placed`, `orders.order_status_changed`, `orders.refund_completed`.
pub(crate) struct OrderEvents(pub Arc<CustomersService>);

#[async_trait::async_trait]
impl EventHandler for OrderEvents {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let after = &event.payload["after"];
        match event.name.as_str() {
            orders::ORDER_PLACED => {
                self.0.order_seen(&event.tenant, after).await?;
            }
            orders::ORDER_STATUS_CHANGED => {
                self.0.order_status(&event.tenant, after).await?;
            }
            orders::REFUND_COMPLETED => {
                self.0.order_refunded(&event.tenant, after).await?;
            }
            _ => {}
        }
        Ok(())
    }
}

/// Tóm tắt đơn đọc từ thân event.
struct OrderInfo {
    order_id: Uuid,
    org_id: Uuid,
    number: String,
    channel: String,
    status: String,
    buyer_user_id: Option<Uuid>,
    buyer_key: Option<String>,
    goods_paid: Money,
    placed_at: Timestamp,
}

fn uuid_of(value: &Value) -> Option<Uuid> {
    value.as_str().and_then(|text| text.parse().ok())
}

fn read(after: &Value) -> Result<Option<OrderInfo>, Error> {
    let (Some(order_id), Some(org_id)) = (uuid_of(&after["order_id"]), uuid_of(&after["org_id"]))
    else {
        return Ok(None);
    };
    let Some(goods_paid) = after.get("goods_paid").filter(|value| !value.is_null()) else {
        // Event của bản `orders` cũ (chưa có `goods_paid`): không đủ để tính tổng chi.
        return Ok(None);
    };
    let goods_paid: Money = serde_json::from_value(goods_paid.clone()).map_err(Error::internal)?;
    let placed_at = after["placed_at"]
        .as_str()
        .and_then(|text| text.parse::<Timestamp>().ok())
        .unwrap_or_else(now);
    Ok(Some(OrderInfo {
        order_id,
        org_id,
        number: after["number"].as_str().unwrap_or_default().to_owned(),
        channel: after["channel"].as_str().unwrap_or("web").to_owned(),
        status: after["status"].as_str().unwrap_or("pending_payment").to_owned(),
        buyer_user_id: uuid_of(&after["buyer_user_id"]),
        buyer_key: after["buyer_key"].as_str().map(str::to_owned),
        goods_paid,
        placed_at,
    }))
}

impl CustomersService {
    /// Hồ sơ của người mua của đơn (tạo khi chưa có); đơn không có tài khoản lẫn số → `None`.
    async fn buyer_profile(
        &self,
        tenant: &TenantContext,
        order: &OrderInfo,
    ) -> Result<Option<profile::Model>, Error> {
        let org = OrgId::from_uuid(order.org_id);
        if let Some(user_id) = order.buyer_user_id {
            return Ok(Some(self.profile_for_user(tenant, user_id, org, Actor::system()).await?));
        }
        if let Some(key) = &order.buyer_key {
            return Ok(Some(self.profile_for_phone_key(tenant, key, org).await?));
        }
        Ok(None)
    }

    /// Ghi dòng lịch sử của đơn (lần đầu thấy đơn: cộng số đơn). Trả dòng hiện có.
    async fn ensure_fact<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        profile_id: Uuid,
        order: &OrderInfo,
    ) -> Result<order_fact::Model, Error> {
        let at = now();
        let inserted = order_fact::Entity::insert(order_fact::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            order_id: Set(order.order_id),
            profile_id: Set(profile_id),
            org_id: Set(order.org_id),
            number: Set(order.number.clone()),
            channel: Set(order.channel.clone()),
            status: Set(order.status.clone()),
            currency: Set(order.goods_paid.currency().code().to_owned()),
            net_minor: Set(order.goods_paid.minor()),
            refunded_minor: Set(0),
            placed_at: Set(order.placed_at),
            completed_at: Set(None),
            updated_at: Set(at),
        })
        .on_conflict(
            OnConflict::columns([order_fact::Column::TenantId, order_fact::Column::OrderId])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(connection)
        .await?;
        if inserted == 1 && order.status != CANCELLED {
            rows::adjust::<profile::Entity, _>(
                connection,
                profile::Column::OrdersCount,
                profile::Column::Id.eq(profile_id),
                1,
                Bounds::NON_NEGATIVE,
            )
            .await?;
            // `if_null` (COALESCE) là hàm của trait `ExprTrait`; nhập trong khối này để `max`/`min`
            // của số nguyên ở chỗ khác không bị nhầm.
            use sea_orm::sea_query::ExprTrait as _;
            profile::Entity::update_many()
                .col_expr(
                    profile::Column::FirstOrderAt,
                    Expr::col(profile::Column::FirstOrderAt).if_null(order.placed_at),
                )
                .col_expr(profile::Column::LastOrderAt, Expr::value(order.placed_at))
                .col_expr(profile::Column::UpdatedAt, Expr::value(at))
                .filter(profile::Column::Id.eq(profile_id))
                .filter(
                    sea_orm::Condition::any()
                        .add(profile::Column::LastOrderAt.is_null())
                        .add(profile::Column::LastOrderAt.lt(order.placed_at)),
                )
                .scoped(tenant)
                .exec(connection)
                .await?;
            self.schedule_evaluation(connection, tenant, profile_id).await?;
        }
        order_fact::Entity::find_by_id((tenant.id().as_uuid(), order.order_id))
            .one(connection)
            .await?
            .ok_or_else(|| Error::internal("customer order fact vanished"))
    }

    async fn order_seen(&self, tenant: &TenantContext, after: &Value) -> Result<(), Error> {
        let Some(order) = read(after)? else { return Ok(()) };
        let Some(profile) = self.buyer_profile(tenant, &order).await? else { return Ok(()) };
        let transaction = tenant.database().begin_write().await?;
        self.ensure_fact(&transaction, tenant, profile.id, &order).await?;
        transaction.commit().await?;
        // Người mua đã đặt đơn: thôi nhắc giỏ bỏ dở.
        if let Some(user) = order.buyer_user_id {
            self.cancel_reminders(tenant, user).await?;
        }
        Ok(())
    }

    async fn order_status(&self, tenant: &TenantContext, after: &Value) -> Result<(), Error> {
        let Some(order) = read(after)? else { return Ok(()) };
        let Some(profile) = self.buyer_profile(tenant, &order).await? else { return Ok(()) };
        let transaction = tenant.database().begin_write().await?;
        let fact = self.ensure_fact(&transaction, tenant, profile.id, &order).await?;
        let at = now();
        // Đơn đã huỷ / hoàn tất không đổi nữa (máy trạng thái của `orders`); điều kiện này cũng làm
        // event nhận lại thành không làm gì.
        let changed = order_fact::Entity::update_many()
            .col_expr(order_fact::Column::Status, Expr::value(order.status.as_str()))
            .col_expr(order_fact::Column::UpdatedAt, Expr::value(at))
            .col_expr(
                order_fact::Column::CompletedAt,
                Expr::value((order.status == COMPLETED).then_some(at)),
            )
            .filter(order_fact::Column::OrderId.eq(order.order_id))
            .filter(order_fact::Column::Status.is_not_in([COMPLETED, CANCELLED]))
            .filter(order_fact::Column::Status.ne(order.status.as_str()))
            .scoped(tenant)
            .exec(&transaction)
            .await?
            .rows_affected;
        let completed = changed == 1 && order.status == COMPLETED;
        let referral = if completed {
            self.qualify_referral(&transaction, tenant, &fact).await?
        } else {
            None
        };
        if changed == 1 {
            match order.status.as_str() {
                COMPLETED => self.add_spent(&transaction, tenant, &fact).await?,
                CANCELLED => {
                    rows::adjust::<profile::Entity, _>(
                        &transaction,
                        profile::Column::OrdersCount,
                        profile::Column::Id.eq(fact.profile_id),
                        -1,
                        Bounds::NON_NEGATIVE,
                    )
                    .await?;
                    self.schedule_evaluation(&transaction, tenant, fact.profile_id).await?;
                }
                _ => {}
            }
        }
        transaction.commit().await?;
        if completed {
            // Voucher sau khi mua: gọi `promotions` sau khi commit (không gọi trong transaction ghi);
            // event giao lại thì `source_ref` chống cấp thêm.
            self.purchase_vouchers(tenant, &fact).await?;
        }
        if let Some((row, rule)) = referral {
            self.referral_vouchers(tenant, &row, &rule).await?;
        }
        Ok(())
    }

    /// Đơn vừa hoàn tất: cộng tiền hàng (trừ phần đã hoàn) vào tổng chi của hồ sơ.
    async fn add_spent<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        fact: &order_fact::Model,
    ) -> Result<(), Error> {
        let amount = (fact.net_minor - fact.refunded_minor).max(0);
        self.change_spent(connection, tenant, fact.profile_id, &fact.currency, amount).await?;
        // Hạng trước, điểm sau: đơn vừa hoàn tất được tích theo hạng mà chính nó giúp đạt.
        self.evaluate_tier(connection, tenant, fact.profile_id).await?;
        self.earn_for_order(connection, tenant, fact).await?;
        self.schedule_evaluation(connection, tenant, fact.profile_id).await
    }

    /// Cộng `delta` vào tổng chi (tiền tệ đầu tiên của hồ sơ; đơn tiền tệ khác không cộng — tổng
    /// chi một số, không cộng lẫn hai đồng tiền).
    async fn change_spent<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        profile_id: Uuid,
        currency: &str,
        delta: i64,
    ) -> Result<(), Error> {
        profile::Entity::update_many()
            .col_expr(profile::Column::Currency, Expr::value(currency))
            .filter(profile::Column::Id.eq(profile_id))
            .filter(profile::Column::Currency.is_null())
            .scoped(tenant)
            .exec(connection)
            .await?;
        let same_currency = sea_orm::Condition::all()
            .add(profile::Column::Id.eq(profile_id))
            .add(profile::Column::Currency.eq(currency))
            .add(profile::Column::TenantId.eq(tenant.id().as_uuid()));
        let changed = rows::adjust::<profile::Entity, _>(
            connection,
            profile::Column::TotalSpentMinor,
            same_currency.clone(),
            delta,
            Bounds::NON_NEGATIVE,
        )
        .await?;
        if changed == 0 && delta < 0 {
            // Hoàn nhiều hơn phần còn tính: tổng chi về 0, không âm.
            profile::Entity::update_many()
                .col_expr(profile::Column::TotalSpentMinor, Expr::value(0_i64))
                .filter(same_currency)
                .scoped(tenant)
                .exec(connection)
                .await?;
        } else if changed == 0 {
            tracing::warn!(
                tenant_id = %tenant.id().as_uuid(),
                profile_id = %profile_id,
                "order in another currency not added to the customer's total spent"
            );
        }
        Ok(())
    }

    async fn order_refunded(&self, tenant: &TenantContext, after: &Value) -> Result<(), Error> {
        let (Some(order_id), Some(refund_id)) =
            (uuid_of(&after["order_id"]), uuid_of(&after["refund_id"]))
        else {
            return Ok(());
        };
        let amount: Money =
            serde_json::from_value(after["amount"].clone()).map_err(Error::internal)?;
        let transaction = tenant.database().begin_write().await?;
        let Some(fact) = order_fact::Entity::find_by_id((tenant.id().as_uuid(), order_id))
            .one(&transaction)
            .await?
        else {
            // Đơn đặt trước khi cài `customers`: không có lịch sử để trừ.
            return Ok(());
        };
        let inserted = refund_fact::Entity::insert(refund_fact::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            refund_id: Set(refund_id),
            order_id: Set(order_id),
            amount_minor: Set(amount.minor()),
            created_at: Set(now()),
        })
        .on_conflict(
            OnConflict::columns([refund_fact::Column::TenantId, refund_fact::Column::RefundId])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        if inserted == 1 {
            rows::adjust::<order_fact::Entity, _>(
                &transaction,
                order_fact::Column::RefundedMinor,
                sea_orm::Condition::all()
                    .add(order_fact::Column::OrderId.eq(order_id))
                    .add(order_fact::Column::TenantId.eq(tenant.id().as_uuid())),
                amount.minor(),
                Bounds::NON_NEGATIVE,
            )
            .await?;
            if fact.status == COMPLETED {
                // Phần đã cộng vào tổng chi: trừ lại (không trừ quá tiền hàng của đơn).
                let counted = (fact.net_minor - fact.refunded_minor).max(0);
                let net_after = (counted - amount.minor()).max(0);
                self.revoke_referral(&transaction, tenant, &fact, net_after).await?;
                let delta = amount.minor().min(counted);
                if delta > 0 {
                    self.change_spent(
                        &transaction,
                        tenant,
                        fact.profile_id,
                        &fact.currency,
                        -delta,
                    )
                    .await?;
                    self.reverse_for_refund(&transaction, tenant, &fact, refund_id, delta).await?;
                    self.evaluate_tier(&transaction, tenant, fact.profile_id).await?;
                    self.schedule_evaluation(&transaction, tenant, fact.profile_id).await?;
                }
            }
        }
        transaction.commit().await?;
        Ok(())
    }
}
