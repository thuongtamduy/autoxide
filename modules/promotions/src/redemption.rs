//! Giữ → xác nhận → trả lượt dùng cho một đơn (D-222): như tồn kho. Mọi bộ đếm (lượt của chương
//! trình, của mã, ngân sách, lượt theo ngày, mỗi khoá người mua, suất flash sale, quà) đổi bằng
//! `rows::adjust` có trần trong MỘT transaction, theo thứ tự (`counter`, `counter_id`) cố định —
//! đủ hết hoặc không giữ gì; voucher đổi trạng thái bằng `UPDATE` có điều kiện. Mỗi bộ đếm đã cộng
//! ghi một dòng `promotions_redemption_holds`; trả lại đúng các dòng đó.
//!
//! Gọi `Catalog` (qua `evaluate`) TRƯỚC khi mở transaction; transaction chạy lại được
//! (`write_with_retry`) vì không gọi ra ngoài.

use autoxide_kernel::time::{Date, Timestamp};
use jiff::SignedDuration;
use std::collections::BTreeMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::ExprTrait;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::json;
use uuid::Uuid;

use crate::api::{Redemption, RedemptionStatus, ReserveRequest};
use crate::entity::{
    code, customer_usage, daily_usage, flash_item, gift_option, hold, promotion, redemption, stats,
    voucher,
};
use crate::evaluate::{Options, Outcome, canonical_cart};
use crate::events::{self, Change};
use crate::model::{Kind, Period};
use crate::service::{PromotionsService, is_unique_violation, now};

/// Một bộ đếm cần cộng khi giữ.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Counter {
    /// `budget`, `code`, `customer`, `daily`, `flash`, `gift`, `promotion`, `voucher`.
    counter: &'static str,
    /// Khoá của bộ đếm; đủ để trả lại: `<promotion>`, `<promotion>|<ngày>`,
    /// `<promotion>|<subject>|<kỳ>|<khoá người mua>`, `<promotion>|<biến thể>`, `<mã>|<promotion>`,
    /// `<voucher>|<promotion>`.
    counter_id: String,
    quantity: i64,
    promotion: Uuid,
    title: String,
}

fn redemption_not_found() -> Error {
    Error::not_found(
        "promotions.redemption_not_found",
        "No promotion usage is held for this order.",
    )
}

fn redemption_expired() -> Error {
    Error::conflict(
        "promotions.redemption_expired",
        "The promotion usage held for this order has expired or was released.",
    )
}

fn exhausted(counter: &Counter) -> Error {
    let error = match counter.counter {
        "budget" => Error::conflict(
            "promotions.budget_exhausted",
            "Promotion `{title}` has used up its budget.",
        ),
        "daily" => Error::conflict(
            "promotions.daily_limit_reached",
            "Promotion `{title}` has no uses left today.",
        ),
        "customer" => Error::conflict(
            "promotions.customer_limit_reached",
            "You have used all your uses of promotion `{title}`.",
        ),
        "flash" => Error::conflict(
            "promotions.flash_sold_out",
            "The flash-sale units of `{variant}` are sold out.",
        ),
        "gift" => {
            Error::conflict("promotions.gift_out_of_stock", "Gift `{variant}` is out of stock.")
        }
        "voucher" => Error::conflict("promotions.voucher_used", "The voucher has been used."),
        _ => Error::conflict("promotions.usage_exhausted", "Promotion `{title}` has no uses left."),
    };
    let variant = counter.counter_id.split('|').nth(1).unwrap_or_default().to_owned();
    error.with_param("title", &counter.title).with_param("variant", variant)
}

/// Khoá kỳ của hạn mức mỗi người theo giờ địa phương.
fn period_key(period: Period, local: Date) -> String {
    match period {
        Period::Lifetime => "lifetime".to_owned(),
        Period::Day => local.to_string(),
        Period::Week => {
            let week = local.to_jiff().iso_week_date();
            format!("{}-W{:02}", week.year(), week.week())
        }
        Period::Month => format!("{}-{:02}", local.year(), local.month()),
    }
}

/// Dạng API của một lượt giữ đã lưu.
pub(crate) fn present(row: &redemption::Model) -> Result<Redemption, Error> {
    let status = match row.status.as_str() {
        "confirmed" => RedemptionStatus::Confirmed,
        "released" => RedemptionStatus::Released,
        "expired" => RedemptionStatus::Expired,
        _ => RedemptionStatus::Held,
    };
    let evaluation = serde_json::from_str(&row.evaluation)
        .map_err(|_| Error::internal("stored promotion evaluation is malformed"))?;
    Ok(Redemption {
        id: row.id,
        order_id: row.order_id,
        status,
        evaluation,
        expires_at: if status == RedemptionStatus::Held { row.expires_at } else { None },
    })
}

impl PromotionsService {
    /// Bộ đếm cần giữ cho kết quả tính `outcome`, theo thứ tự (`counter`, `counter_id`).
    async fn counters(
        &self,
        tenant: &TenantContext,
        outcome: &Outcome,
    ) -> Result<Vec<Counter>, Error> {
        let cart = &outcome.cart;
        let zone = self.timezone(tenant, cart.org_id).await?;
        let local = cart.at.date_in(&zone);
        let mut keys: Vec<String> = cart.buyer_keys.clone();
        keys.sort();
        keys.dedup();
        let mut counters = Vec::new();
        for (program, applied) in &outcome.applied {
            let (title, _) = program.text(&cart.locale);
            let pid = program.id;
            let push =
                |counters: &mut Vec<Counter>, counter: &'static str, id: String, quantity: i64| {
                    counters.push(Counter {
                        counter,
                        counter_id: id,
                        quantity,
                        promotion: pid,
                        title: title.clone(),
                    });
                };
            push(&mut counters, "promotion", pid.to_string(), 1);
            if program.budget_minor.is_some() {
                push(&mut counters, "budget", pid.to_string(), applied.total());
            }
            if program.daily_usage_limit.is_some() {
                push(&mut counters, "daily", format!("{pid}|{local}"), 1);
            }
            if program.per_customer_limit.is_some() {
                let period = period_key(program.per_customer_period, local);
                for key in &keys {
                    push(&mut counters, "customer", format!("{pid}|-|{period}|{key}"), 1);
                }
            }
            for part in applied.lines.iter().filter(|part| part.units > 0) {
                let variant = cart.lines[part.line].variant_id;
                push(&mut counters, "flash", format!("{pid}|{variant}"), part.units);
                if program
                    .flash_item(variant)
                    .is_some_and(|item| item.per_customer_quantity_limit.is_some())
                {
                    for key in &keys {
                        push(
                            &mut counters,
                            "customer",
                            format!("{pid}|{variant}|lifetime|{key}"),
                            part.units,
                        );
                    }
                }
            }
            if program.kind == Kind::GiftChoice {
                for (variant, quantity) in &applied.gifts {
                    push(&mut counters, "gift", format!("{pid}|{variant}"), *quantity);
                }
            }
            if let Some(unlock) = &applied.unlock {
                if let Some(code_id) = unlock.code_id {
                    push(&mut counters, "code", format!("{code_id}|{pid}"), 1);
                }
                if let Some(voucher_id) = unlock.voucher_id {
                    push(&mut counters, "voucher", format!("{voucher_id}|{pid}"), 1);
                }
            }
        }
        counters.sort();
        Ok(counters)
    }

    /// Giữ mọi lượt của giỏ cho đơn `order_id`.
    pub(crate) async fn reserve(
        &self,
        tenant: &TenantContext,
        request: &ReserveRequest,
    ) -> Result<Submitted<Redemption>, Error> {
        let at = now();
        if request.expires_at <= at
            || request.expires_at > at + SignedDuration::from_hours(self.config.max_hold_hours)
        {
            return Err(Error::validation(vec![autoxide_kernel::error::FieldError::new(
                "expires_at",
                "range",
                "must be in the future and at most `AUTOXIDE_PROMOTIONS_MAX_HOLD_HOURS` from now",
            )]));
        }
        let request_fingerprint =
            autoxide_kernel::secret::sha256(&canonical_cart(&request.cart, None));
        if let Some(existing) = self.redemption_row(tenant, request.order_id).await? {
            return replay(&existing, &request_fingerprint, request.order_id);
        }
        let outcome = self.evaluate_cart(tenant, &request.cart, &Options::default()).await?;
        if request
            .expected_fingerprint
            .as_ref()
            .is_some_and(|expected| *expected != outcome.evaluation.fingerprint)
        {
            return Err(Error::conflict(
                "promotions.changed",
                "The promotions of the cart have changed; review them before placing the order.",
            ));
        }
        let counters = self.counters(tenant, &outcome).await?;
        let zone = self.timezone(tenant, request.cart.org_id).await?;
        let day = request.cart.at.date_in(&zone).to_string();
        let order_value: i64 = request
            .cart
            .lines
            .iter()
            .map(|line| line.unit_price.minor().saturating_mul(line.quantity))
            .sum();
        let evaluation_json = serde_json::to_string(&outcome.evaluation)
            .map_err(|_| Error::internal("cannot store the promotion evaluation"))?;
        let database = tenant.database();
        let (counters, request_fingerprint, evaluation_json, day) =
            (&counters, &request_fingerprint, &evaluation_json, &day);
        let outcome = &outcome;
        let created = database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let id = Uuid::now_v7();
                let inserted = redemption::ActiveModel {
                    id: Set(id),
                    tenant_id: Set(tenant.id().as_uuid()),
                    order_id: Set(request.order_id),
                    org_id: Set(request.cart.org_id.as_uuid()),
                    buyer_id: Set(request.cart.buyer_id),
                    status: Set("held".to_owned()),
                    fingerprint: Set(outcome.evaluation.fingerprint.clone()),
                    request_fingerprint: Set(request_fingerprint.clone()),
                    evaluation: Set(evaluation_json.clone()),
                    total_discount_minor: Set(outcome.evaluation.total_discount.minor()),
                    order_value_minor: Set(order_value),
                    currency: Set(request.cart.currency.code().to_owned()),
                    day: Set(day.clone()),
                    expires_at: Set(Some(crate::service::micros(request.expires_at))),
                    created_at: Set(now()),
                    updated_at: Set(now()),
                }
                .insert(&transaction)
                .await;
                match inserted {
                    Err(error) if is_unique_violation(&error) => return Ok(None),
                    Err(error) => return Err(error.into()),
                    Ok(_) => {}
                }
                let mut exhausted_budgets = Vec::new();
                for counter in counters {
                    if !take(&transaction, tenant, counter, request).await? {
                        return Err(exhausted(counter));
                    }
                    if counter.counter == "budget"
                        && budget_full(&transaction, tenant, counter.promotion).await?
                    {
                        exhausted_budgets.push(counter.promotion);
                    }
                    hold::ActiveModel {
                        id: Set(Uuid::now_v7()),
                        tenant_id: Set(tenant.id().as_uuid()),
                        redemption_id: Set(id),
                        counter: Set(counter.counter.to_owned()),
                        counter_id: Set(counter.counter_id.clone()),
                        quantity: Set(counter.quantity),
                    }
                    .insert(&transaction)
                    .await?;
                }
                for promotion_id in exhausted_budgets {
                    self.outbox
                        .record(
                            &transaction,
                            tenant,
                            Actor::system(),
                            events::promotion_subject(promotion_id),
                            &events::BudgetExhausted(Change::after(json!({"id": promotion_id}))),
                        )
                        .await?;
                }
                transaction.commit().await?;
                Ok(Some(id))
            })
            .await?;
        let Some(_) = created else {
            // Một request khác cùng `order_id` vừa giữ xong.
            let existing = self
                .redemption_row(tenant, request.order_id)
                .await?
                .ok_or_else(redemption_not_found)?;
            return replay(&existing, request_fingerprint, request.order_id);
        };
        let row = self
            .redemption_row(tenant, request.order_id)
            .await?
            .ok_or_else(redemption_not_found)?;
        Ok(Submitted::Created(present(&row)?))
    }

    pub(crate) async fn redemption_row(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
    ) -> Result<Option<redemption::Model>, Error> {
        Ok(redemption::Entity::find()
            .filter(redemption::Column::OrderId.eq(order_id))
            .scoped(tenant)
            .one(tenant.database().write())
            .await?)
    }

    /// Giữ → đã dùng.
    pub(crate) async fn confirm(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
    ) -> Result<Redemption, Error> {
        let database = tenant.database();
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let row = redemption::Entity::find()
                    .filter(redemption::Column::OrderId.eq(order_id))
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                    .ok_or_else(redemption_not_found)?;
                match row.status.as_str() {
                    "confirmed" => return present(&row),
                    "held" => {}
                    _ => return Err(redemption_expired()),
                }
                let at = now();
                let changed = redemption::Entity::update_many()
                    .col_expr(redemption::Column::Status, Expr::value("confirmed"))
                    .col_expr(redemption::Column::UpdatedAt, Expr::value(at))
                    .filter(redemption::Column::Id.eq(row.id))
                    .filter(redemption::Column::Status.eq("held"))
                    .filter(redemption::Column::ExpiresAt.gt(at))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?
                    .rows_affected;
                if changed == 0 {
                    return Err(redemption_expired());
                }
                let holds = holds_of(&transaction, tenant, row.id).await?;
                for found in holds.iter().filter(|found| found.counter == "voucher") {
                    let voucher_id = first_uuid(&found.counter_id);
                    voucher::Entity::update_many()
                        .col_expr(voucher::Column::Status, Expr::value("used"))
                        .col_expr(voucher::Column::UpdatedAt, Expr::value(at))
                        .filter(voucher::Column::Id.eq(voucher_id))
                        .filter(voucher::Column::Status.eq("held"))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                    self.outbox
                        .record(
                            &transaction,
                            tenant,
                            Actor::system(),
                            Some(format!("promotions.voucher:{voucher_id}")),
                            &events::VoucherUsed(Change::after(json!({"id": voucher_id, "order_id": order_id}))),
                        )
                        .await?;
                }
                let redemption = present(&row)?;
                record_stats(&transaction, tenant, &row, &redemption, 1).await?;
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        Actor::system(),
                        Some(format!("promotions.redemption:{}", row.id)),
                        &events::RedemptionConfirmed(Change::after(json!({
                            "order_id": order_id,
                            "total_discount": redemption.evaluation.total_discount,
                            "promotions": redemption.evaluation.applied.iter().map(|found| found.promotion_id).collect::<Vec<_>>(),
                        }))),
                    )
                    .await?;
                transaction.commit().await?;
                let mut confirmed = redemption;
                confirmed.status = RedemptionStatus::Confirmed;
                confirmed.expires_at = None;
                Ok(confirmed)
            })
            .await
    }

    /// Trả lượt của đơn: đang giữ → trả mọi bộ đếm; đã xác nhận → trả lượt của chương trình cho
    /// phép (`return_usage_on_cancel`) và đưa voucher còn hạn về ví. Gọi lại: trả như cũ.
    pub(crate) async fn release(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        expired: bool,
    ) -> Result<Option<Redemption>, Error> {
        let database = tenant.database();
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let Some(row) = redemption::Entity::find()
                    .filter(redemption::Column::OrderId.eq(order_id))
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                else {
                    return Ok(None);
                };
                let was_confirmed = match row.status.as_str() {
                    "held" => false,
                    "confirmed" if !expired => true,
                    _ => return Ok(Some(present(&row)?)),
                };
                let at = now();
                let next = if expired { "expired" } else { "released" };
                // Đổi có điều kiện: hai lần `release` đua nhau chỉ một lần trả bộ đếm.
                let changed = redemption::Entity::update_many()
                    .col_expr(redemption::Column::Status, Expr::value(next))
                    .col_expr(redemption::Column::UpdatedAt, Expr::value(at))
                    .filter(redemption::Column::Id.eq(row.id))
                    .filter(redemption::Column::Status.eq(row.status.clone()))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?
                    .rows_affected;
                if changed == 0 {
                    // Lần trả khác vừa xong trước: trả trạng thái mới, không cộng lại lần nữa.
                    let fresh = redemption::Entity::find_by_id(row.id)
                        .scoped(tenant)
                        .one(&transaction)
                        .await?;
                    return fresh.as_ref().map(present).transpose();
                }
                let redemption = present(&row)?;
                let keep_usage: Vec<Uuid> = if was_confirmed {
                    redemption
                        .evaluation
                        .applied
                        .iter()
                        .filter(|found| !found.return_usage_on_cancel)
                        .map(|found| found.promotion_id)
                        .collect()
                } else {
                    Vec::new()
                };
                let holds = holds_of(&transaction, tenant, row.id).await?;
                for found in &holds {
                    give_back(&transaction, tenant, found, &keep_usage, at).await?;
                }
                if was_confirmed {
                    record_stats(&transaction, tenant, &row, &redemption, -1).await?;
                }
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        Actor::system(),
                        Some(format!("promotions.redemption:{}", row.id)),
                        &events::RedemptionReleased(Change::after(json!({
                            "order_id": order_id, "status": next, "was_confirmed": was_confirmed,
                        }))),
                    )
                    .await?;
                transaction.commit().await?;
                let mut released = redemption;
                released.status =
                    if expired { RedemptionStatus::Expired } else { RedemptionStatus::Released };
                released.expires_at = None;
                Ok(Some(released))
            })
            .await
    }

    /// Job `promotions.expire_holds`: lượt giữ quá hạn → `expired`, trả bộ đếm.
    pub(crate) async fn expire_holds(&self, tenant: &TenantContext) -> Result<u64, Error> {
        let due = redemption::Entity::find()
            .filter(redemption::Column::Status.eq("held"))
            .filter(redemption::Column::ExpiresAt.lte(now()))
            .scoped(tenant)
            .order_by_asc(redemption::Column::ExpiresAt)
            .all(tenant.database().read())
            .await?;
        let mut count = 0;
        for row in due {
            if self.release(tenant, row.order_id, true).await?.is_some() {
                count += 1;
            }
        }
        Ok(count)
    }
}

fn replay(
    existing: &redemption::Model,
    fingerprint: &str,
    order_id: Uuid,
) -> Result<Submitted<Redemption>, Error> {
    if existing.request_fingerprint == fingerprint {
        Ok(Submitted::Replayed(present(existing)?))
    } else {
        Err(Error::conflict(
            "promotions.reference_reused",
            "Order `{order_id}` already holds promotion usage for a different cart.",
        )
        .with_param("order_id", order_id))
    }
}

fn first_uuid(text: &str) -> Uuid {
    text.split('|').next().and_then(|part| part.parse().ok()).unwrap_or_default()
}

fn part_uuid(text: &str, index: usize) -> Uuid {
    text.split('|').nth(index).and_then(|part| part.parse().ok()).unwrap_or_default()
}

async fn holds_of<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    redemption_id: Uuid,
) -> Result<Vec<hold::Model>, Error> {
    Ok(hold::Entity::find()
        .filter(hold::Column::RedemptionId.eq(redemption_id))
        .scoped(tenant)
        .order_by_asc(hold::Column::Counter)
        .order_by_asc(hold::Column::CounterId)
        .all(transaction)
        .await?)
}

/// Ngân sách của chương trình đã dùng hết chưa (để phát event).
async fn budget_full<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<bool, Error> {
    let row = promotion::Entity::find_by_id(id).scoped(tenant).one(transaction).await?;
    Ok(row
        .is_some_and(|row| row.budget_minor.is_some_and(|budget| row.budget_used_minor >= budget)))
}

/// Cộng một bộ đếm trong transaction; `false` khi vượt giới hạn (đọc trong cùng transaction).
async fn take<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    counter: &Counter,
    request: &ReserveRequest,
) -> Result<bool, Error> {
    let tenant_id = tenant.id().as_uuid();
    let tenant_filter = |column: sea_orm::sea_query::SimpleExpr| column;
    let pid = counter.promotion;
    let quantity = counter.quantity;
    let row = promotion::Entity::find_by_id(pid).scoped(tenant).one(transaction).await?;
    let Some(program) = row else { return Ok(false) };
    let changed = match counter.counter {
        "promotion" => {
            let ceiling = program.usage_limit.unwrap_or(i64::MAX);
            rows::adjust::<promotion::Entity, _>(
                transaction,
                promotion::Column::UsageCount,
                tenant_filter(
                    promotion::Column::Id.eq(pid).and(promotion::Column::TenantId.eq(tenant_id)),
                ),
                quantity,
                Bounds { floor: 0, ceiling },
            )
            .await?
        }
        "budget" => {
            let ceiling = program.budget_minor.unwrap_or(i64::MAX);
            rows::adjust::<promotion::Entity, _>(
                transaction,
                promotion::Column::BudgetUsedMinor,
                promotion::Column::Id.eq(pid).and(promotion::Column::TenantId.eq(tenant_id)),
                quantity,
                Bounds { floor: 0, ceiling },
            )
            .await?
        }
        "daily" => {
            let day = counter.counter_id.split('|').nth(1).unwrap_or_default().to_owned();
            daily_usage::Entity::insert(daily_usage::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant_id),
                promotion_id: Set(pid),
                day: Set(day.clone()),
                used: Set(0),
            })
            .on_conflict(
                OnConflict::columns([
                    daily_usage::Column::TenantId,
                    daily_usage::Column::PromotionId,
                    daily_usage::Column::Day,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(transaction)
            .await?;
            rows::adjust::<daily_usage::Entity, _>(
                transaction,
                daily_usage::Column::Used,
                daily_usage::Column::TenantId
                    .eq(tenant_id)
                    .and(daily_usage::Column::PromotionId.eq(pid))
                    .and(daily_usage::Column::Day.eq(day)),
                quantity,
                Bounds { floor: 0, ceiling: program.daily_usage_limit.unwrap_or(i64::MAX) },
            )
            .await?
        }
        "customer" => {
            let mut parts = counter.counter_id.splitn(4, '|');
            let (_, subject, period, key) =
                (parts.next(), parts.next(), parts.next(), parts.next());
            let (subject, period, key) = (
                subject.unwrap_or("-").to_owned(),
                period.unwrap_or_default().to_owned(),
                key.unwrap_or_default().to_owned(),
            );
            let ceiling = if subject == "-" {
                program.per_customer_limit.unwrap_or(i64::MAX)
            } else {
                let variant: Uuid = subject.parse().unwrap_or_default();
                flash_item::Entity::find()
                    .filter(flash_item::Column::PromotionId.eq(pid))
                    .filter(flash_item::Column::VariantId.eq(variant))
                    .scoped(tenant)
                    .one(transaction)
                    .await?
                    .and_then(|item| item.per_customer_quantity_limit)
                    .unwrap_or(i64::MAX)
            };
            customer_usage::Entity::insert(customer_usage::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant_id),
                promotion_id: Set(pid),
                subject: Set(subject.clone()),
                buyer_key: Set(key.clone()),
                period_key: Set(period.clone()),
                used: Set(0),
            })
            .on_conflict(
                OnConflict::columns([
                    customer_usage::Column::TenantId,
                    customer_usage::Column::PromotionId,
                    customer_usage::Column::Subject,
                    customer_usage::Column::BuyerKey,
                    customer_usage::Column::PeriodKey,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(transaction)
            .await?;
            rows::adjust::<customer_usage::Entity, _>(
                transaction,
                customer_usage::Column::Used,
                customer_usage::Column::TenantId
                    .eq(tenant_id)
                    .and(customer_usage::Column::PromotionId.eq(pid))
                    .and(customer_usage::Column::Subject.eq(subject))
                    .and(customer_usage::Column::BuyerKey.eq(key))
                    .and(customer_usage::Column::PeriodKey.eq(period)),
                quantity,
                Bounds { floor: 0, ceiling },
            )
            .await?
        }
        "flash" => {
            let variant = part_uuid(&counter.counter_id, 1);
            let item = flash_item::Entity::find()
                .filter(flash_item::Column::PromotionId.eq(pid))
                .filter(flash_item::Column::VariantId.eq(variant))
                .scoped(tenant)
                .one(transaction)
                .await?;
            let Some(item) = item else { return Ok(false) };
            rows::adjust::<flash_item::Entity, _>(
                transaction,
                flash_item::Column::QuantityUsed,
                flash_item::Column::Id.eq(item.id),
                quantity,
                Bounds { floor: 0, ceiling: item.quantity_limit.unwrap_or(i64::MAX) },
            )
            .await?
        }
        "gift" => {
            let variant = part_uuid(&counter.counter_id, 1);
            let gift = gift_option::Entity::find()
                .filter(gift_option::Column::PromotionId.eq(pid))
                .filter(gift_option::Column::VariantId.eq(variant))
                .scoped(tenant)
                .one(transaction)
                .await?;
            let Some(gift) = gift else { return Ok(false) };
            rows::adjust::<gift_option::Entity, _>(
                transaction,
                gift_option::Column::QuantityUsed,
                gift_option::Column::Id.eq(gift.id),
                quantity,
                Bounds { floor: 0, ceiling: gift.stock_limit.unwrap_or(i64::MAX) },
            )
            .await?
        }
        "code" => {
            let code_id = first_uuid(&counter.counter_id);
            let found = code::Entity::find_by_id(code_id).scoped(tenant).one(transaction).await?;
            let Some(found) = found else { return Ok(false) };
            rows::adjust::<code::Entity, _>(
                transaction,
                code::Column::UsageCount,
                code::Column::Id.eq(code_id),
                quantity,
                Bounds { floor: 0, ceiling: found.usage_limit.unwrap_or(i64::MAX) },
            )
            .await?
        }
        "voucher" => {
            let voucher_id = first_uuid(&counter.counter_id);
            let mut update = voucher::Entity::update_many()
                .col_expr(voucher::Column::Status, Expr::value("held"))
                .col_expr(voucher::Column::UsedOrderId, Expr::value(Some(request.order_id)))
                .col_expr(voucher::Column::UpdatedAt, Expr::value(now()))
                .filter(voucher::Column::Id.eq(voucher_id))
                .filter(voucher::Column::Status.eq("available"));
            update = match request.cart.buyer_id {
                Some(buyer) => update.filter(voucher::Column::HolderUserId.eq(buyer)),
                None => return Ok(false),
            };
            update.scoped(tenant).exec(transaction).await?.rows_affected
        }
        _ => 0,
    };
    Ok(changed > 0)
}

/// Trả một bộ đếm (giữ lượt của chương trình trong `keep_usage`; voucher còn hạn về ví).
async fn give_back<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    found: &hold::Model,
    keep_usage: &[Uuid],
    at: Timestamp,
) -> Result<(), Error> {
    let tenant_id = tenant.id().as_uuid();
    let quantity = -found.quantity;
    let pid = match found.counter.as_str() {
        "code" | "voucher" => part_uuid(&found.counter_id, 1),
        _ => first_uuid(&found.counter_id),
    };
    if found.counter == "voucher" {
        let voucher_id = first_uuid(&found.counter_id);
        // Voucher về ví nếu còn hạn, không thì hết hạn.
        for (status, still_valid) in [("available", true), ("expired", false)] {
            let expiry = if still_valid {
                voucher::Column::ExpiresAt.gt(at)
            } else {
                voucher::Column::ExpiresAt.lte(at)
            };
            voucher::Entity::update_many()
                .col_expr(voucher::Column::Status, Expr::value(status))
                .col_expr(voucher::Column::UsedOrderId, Expr::value(Option::<Uuid>::None))
                .col_expr(voucher::Column::UpdatedAt, Expr::value(at))
                .filter(voucher::Column::Id.eq(voucher_id))
                .filter(voucher::Column::Status.is_in(["held", "used"]))
                .filter(expiry)
                .scoped(tenant)
                .exec(transaction)
                .await?;
        }
        return Ok(());
    }
    if keep_usage.contains(&pid) {
        return Ok(());
    }
    match found.counter.as_str() {
        "promotion" | "budget" => {
            let column = if found.counter == "promotion" {
                promotion::Column::UsageCount
            } else {
                promotion::Column::BudgetUsedMinor
            };
            rows::adjust::<promotion::Entity, _>(
                transaction,
                column,
                promotion::Column::Id.eq(pid).and(promotion::Column::TenantId.eq(tenant_id)),
                quantity,
                Bounds::NON_NEGATIVE,
            )
            .await?;
        }
        "daily" => {
            let day = found.counter_id.split('|').nth(1).unwrap_or_default().to_owned();
            rows::adjust::<daily_usage::Entity, _>(
                transaction,
                daily_usage::Column::Used,
                daily_usage::Column::TenantId
                    .eq(tenant_id)
                    .and(daily_usage::Column::PromotionId.eq(pid))
                    .and(daily_usage::Column::Day.eq(day)),
                quantity,
                Bounds::NON_NEGATIVE,
            )
            .await?;
        }
        "customer" => {
            let mut parts = found.counter_id.splitn(4, '|');
            let (_, subject, period, key) =
                (parts.next(), parts.next(), parts.next(), parts.next());
            rows::adjust::<customer_usage::Entity, _>(
                transaction,
                customer_usage::Column::Used,
                customer_usage::Column::TenantId
                    .eq(tenant_id)
                    .and(customer_usage::Column::PromotionId.eq(pid))
                    .and(customer_usage::Column::Subject.eq(subject.unwrap_or("-")))
                    .and(customer_usage::Column::PeriodKey.eq(period.unwrap_or_default()))
                    .and(customer_usage::Column::BuyerKey.eq(key.unwrap_or_default())),
                quantity,
                Bounds::NON_NEGATIVE,
            )
            .await?;
        }
        "flash" | "gift" => {
            let variant = part_uuid(&found.counter_id, 1);
            if found.counter == "flash" {
                rows::adjust::<flash_item::Entity, _>(
                    transaction,
                    flash_item::Column::QuantityUsed,
                    flash_item::Column::TenantId
                        .eq(tenant_id)
                        .and(flash_item::Column::PromotionId.eq(pid))
                        .and(flash_item::Column::VariantId.eq(variant)),
                    quantity,
                    Bounds::NON_NEGATIVE,
                )
                .await?;
            } else {
                rows::adjust::<gift_option::Entity, _>(
                    transaction,
                    gift_option::Column::QuantityUsed,
                    gift_option::Column::TenantId
                        .eq(tenant_id)
                        .and(gift_option::Column::PromotionId.eq(pid))
                        .and(gift_option::Column::VariantId.eq(variant)),
                    quantity,
                    Bounds::NON_NEGATIVE,
                )
                .await?;
            }
        }
        "code" => {
            rows::adjust::<code::Entity, _>(
                transaction,
                code::Column::UsageCount,
                code::Column::Id
                    .eq(first_uuid(&found.counter_id))
                    .and(code::Column::TenantId.eq(tenant_id)),
                quantity,
                Bounds::NON_NEGATIVE,
            )
            .await?;
        }
        _ => {}
    }
    Ok(())
}

/// Cộng (`sign` = 1) hay trừ (−1) số liệu ngày của mỗi chương trình đã áp, cùng transaction.
async fn record_stats<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    row: &redemption::Model,
    redemption: &Redemption,
    sign: i64,
) -> Result<(), Error> {
    let mut by_program: BTreeMap<Uuid, (i64, i64, i64)> = BTreeMap::new();
    for line in &redemption.evaluation.lines {
        let entry = by_program.entry(line.promotion_id).or_default();
        entry.0 += line.amount.minor();
        for share in &line.funding {
            if share.payer_kind == crate::api::PayerKind::Platform {
                entry.1 += share.amount.minor();
            } else {
                entry.2 += share.amount.minor();
            }
        }
    }
    for found in &redemption.evaluation.shipping {
        let entry = by_program.entry(found.promotion_id).or_default();
        entry.0 += found.amount.minor();
        for share in &found.funding {
            if share.payer_kind == crate::api::PayerKind::Platform {
                entry.1 += share.amount.minor();
            } else {
                entry.2 += share.amount.minor();
            }
        }
    }
    for applied in &redemption.evaluation.applied {
        by_program.entry(applied.promotion_id).or_default();
    }
    let tenant_id = tenant.id().as_uuid();
    for (promotion_id, (discount, platform, seller)) in by_program {
        stats::Entity::insert(stats::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            promotion_id: Set(promotion_id),
            day: Set(row.day.clone()),
            redemptions: Set(0),
            discount_minor: Set(0),
            order_value_minor: Set(0),
            platform_funded_minor: Set(0),
            seller_funded_minor: Set(0),
        })
        .on_conflict(
            OnConflict::columns([
                stats::Column::TenantId,
                stats::Column::PromotionId,
                stats::Column::Day,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(transaction)
        .await?;
        stats::Entity::update_many()
            .col_expr(stats::Column::Redemptions, Expr::col(stats::Column::Redemptions).add(sign))
            .col_expr(
                stats::Column::DiscountMinor,
                Expr::col(stats::Column::DiscountMinor).add(sign * discount),
            )
            .col_expr(
                stats::Column::OrderValueMinor,
                Expr::col(stats::Column::OrderValueMinor).add(sign * row.order_value_minor),
            )
            .col_expr(
                stats::Column::PlatformFundedMinor,
                Expr::col(stats::Column::PlatformFundedMinor).add(sign * platform),
            )
            .col_expr(
                stats::Column::SellerFundedMinor,
                Expr::col(stats::Column::SellerFundedMinor).add(sign * seller),
            )
            .filter(stats::Column::TenantId.eq(tenant_id))
            .filter(stats::Column::PromotionId.eq(promotion_id))
            .filter(stats::Column::Day.eq(row.day.clone()))
            .exec(transaction)
            .await?;
    }
    Ok(())
}

impl PromotionsService {
    /// Một trang lượt giữ trong phạm vi `promotions.read` của người gọi.
    pub(crate) async fn redemptions(
        &self,
        tenant: &TenantContext,
        principal: &autoxide_kernel::access::Principal,
        order_id: Option<Uuid>,
        status: Option<String>,
        page: &autoxide_kernel::pagination::PageRequest,
    ) -> Result<autoxide_kernel::pagination::Page<redemption::Model>, Error> {
        let mut condition = sea_orm::Condition::all().add(principal.scope_condition(
            &crate::permissions::READ,
            redemption::Column::OrgId,
            None,
        )?);
        if let Some(order) = order_id {
            condition = condition.add(redemption::Column::OrderId.eq(order));
        }
        if let Some(status) = status {
            condition = condition.add(redemption::Column::Status.eq(status));
        }
        let select = redemption::Entity::find().filter(condition).scoped(tenant);
        let read = tenant.database().read();
        autoxide_kernel::pagination::fetch_page(
            select,
            redemption::Column::Id,
            |row| row.id,
            page,
            read,
        )
        .await
    }

    /// Gỡ kẹt bằng tay: trả lượt giữ `id` như khi huỷ đơn (`promotions.publish` ở cửa hàng).
    pub(crate) async fn release_by_id(
        &self,
        tenant: &TenantContext,
        principal: &autoxide_kernel::access::Principal,
        id: Uuid,
    ) -> Result<Redemption, Error> {
        let read = tenant.database().read();
        let row = redemption::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(redemption_not_found)?;
        crate::service::writable(
            principal,
            read,
            &crate::permissions::PUBLISH,
            row.org_id,
            redemption_not_found,
        )
        .await?;
        self.release(tenant, row.order_id, false).await?.ok_or_else(redemption_not_found)
    }
}

#[cfg(test)]
mod tests {
    use super::{Period, period_key};
    use autoxide_kernel::time::Date;

    #[test]
    fn khoa_ky_theo_ngay_tuan_thang() {
        let day = Date::new(2026, 11, 18).unwrap();
        assert_eq!(period_key(Period::Lifetime, day), "lifetime");
        assert_eq!(period_key(Period::Day, day), "2026-11-18");
        assert_eq!(period_key(Period::Week, day), "2026-W47");
        assert_eq!(period_key(Period::Month, day), "2026-11");
    }
}
