//! Job hằng giờ gom hàng đợi báo yêu thích thành tin: mỗi hồ sơ một tin (tối đa [`MAX_ITEMS`] sản
//! phẩm), chỉ trong khung giờ quảng cáo, có trần tin mỗi ngày. Trước khi gửi kiểm lại: còn bật, còn
//! đồng ý, còn hàng / còn giảm đủ, còn bán. Transaction ngắn đánh dấu "đã báo" có điều kiện (worker
//! khác đã báo thì bỏ dòng đó), rồi sau commit mới gọi `Notifier` với `dedupe_key` theo lô.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::HashMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, root_of};
use autoxide_notifications::api::{Notification, Recipient};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{BACK_IN_STOCK, COOLDOWN_DAYS, MAX_ITEMS, PRICE_DROP, WISHLIST_ALERT, dropped};
use crate::entity::{
    wishlist_alert_batch, wishlist_alert_queue, wishlist_alert_setting, wishlist_item,
};
use crate::events::{Change, WishlistAlertSent};
use crate::profiles::subject;
use crate::service::{CustomersService, now};

pub(crate) const SEND_WISHLIST_ALERTS: &str = "customers.send_wishlist_alerts";

const PENDING: &str = "pending";
const SENT: &str = "sent";
/// Giờ Việt Nam khi quy tắc không nêu độ lệch.
const DEFAULT_OFFSET_MINUTES: i64 = 7 * 60;

/// `HH:MM` → số phút trong ngày.
fn minutes(raw: &str) -> Option<i64> {
    let (hours, mins) = raw.split_once(':')?;
    let (hours, mins) = (hours.parse::<i64>().ok()?, mins.parse::<i64>().ok()?);
    ((0..24).contains(&hours) && (0..60).contains(&mins)).then_some(hours * 60 + mins)
}

/// `at` có nằm trong khung `{"from": "07:00", "to": "22:00"}` (giờ địa phương lệch `offset` phút)
/// không. Khung hỏng thì coi như không mở.
fn inside(window: &Value, offset: i64, at: Timestamp) -> bool {
    let (Some(from), Some(to)) = (
        window.get("from").and_then(Value::as_str).and_then(minutes),
        window.get("to").and_then(Value::as_str).and_then(minutes),
    ) else {
        return false;
    };
    let local = at + SignedDuration::from_mins(offset);
    let now_minutes = local.as_unix_seconds().rem_euclid(86_400) / 60;
    if from <= to {
        (from..to).contains(&now_minutes)
    } else {
        now_minutes >= from || now_minutes < to
    }
}

/// Khung giờ gửi được lúc `at` theo thân quy tắc `notifications.send_windows`: phải nằm trong MỌI
/// khung của các kênh có khung (kênh `null` không giới hạn); không kênh nào có khung → 07:00–22:00.
/// Trả (trong khung không, độ lệch giờ phút).
pub(crate) fn window_open(body: Option<&Value>, at: Timestamp) -> (bool, i64) {
    let offset = body
        .and_then(|body| body.get("utc_offset_minutes"))
        .and_then(Value::as_i64)
        .unwrap_or(DEFAULT_OFFSET_MINUTES);
    let windows: Vec<&Value> = body
        .and_then(|body| body.get("marketing"))
        .and_then(Value::as_object)
        .map(|channels| channels.values().filter(|window| !window.is_null()).collect())
        .unwrap_or_default();
    let open = if windows.is_empty() {
        inside(&json!({"from": "07:00", "to": "22:00"}), offset, at)
    } else {
        windows.iter().all(|window| inside(window, offset, at))
    };
    (open, offset)
}

/// Một sản phẩm trong tin.
struct Item {
    queue: wishlist_alert_queue::Model,
    row: wishlist_item::Model,
}

impl CustomersService {
    /// Một lượt của job cho khách `tenant`: gửi lại lô còn `pending`, rồi gom hàng đợi. Trả số tin.
    pub(crate) async fn send_wishlist_alerts(&self, tenant: &TenantContext) -> Result<u64, Error> {
        if self.notifier.is_none() {
            return Ok(0);
        }
        let at = now();
        let body = match &self.rules {
            Some(rules) => rules
                .effective(tenant.id(), "notifications.send_windows", at)
                .await?
                .map(|rule| rule.body),
            None => None,
        };
        let (open, offset) = window_open(body.as_ref(), at);
        let mut sent = 0;
        // Lô đã ghi mà chưa gửi xong (lỗi giữa chừng): gửi lại cùng `dedupe_key`, không gửi hai lần.
        let pending = wishlist_alert_batch::Entity::find()
            .filter(wishlist_alert_batch::Column::Status.eq(PENDING))
            .scoped(tenant)
            .limit(100)
            .all(tenant.database().read())
            .await?;
        for batch in pending {
            self.deliver_batch(tenant, &batch).await?;
            sent += 1;
        }
        if !open {
            return Ok(sent);
        }
        let profiles: Vec<Uuid> = wishlist_alert_queue::Entity::find()
            .select_only()
            .column(wishlist_alert_queue::Column::ProfileId)
            .distinct()
            .scoped(tenant)
            .limit(1000)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        let day = (at + SignedDuration::from_mins(offset)).date_utc().to_string();
        for profile in profiles {
            if self.alert_profile(tenant, profile, &day, at).await? {
                sent += 1;
            }
        }
        Ok(sent)
    }

    /// Bỏ các dòng hàng đợi `drop` của hồ sơ.
    async fn drop_queued(
        &self,
        tenant: &TenantContext,
        drop: &[&wishlist_alert_queue::Model],
    ) -> Result<(), Error> {
        for queued in drop {
            wishlist_alert_queue::Entity::delete_many()
                .filter(wishlist_alert_queue::Column::ProfileId.eq(queued.profile_id))
                .filter(wishlist_alert_queue::Column::ProductId.eq(queued.product_id))
                .filter(wishlist_alert_queue::Column::VariantKey.eq(queued.variant_key.as_str()))
                .filter(wishlist_alert_queue::Column::Kind.eq(queued.kind.as_str()))
                .scoped(tenant)
                .exec(tenant.database().write())
                .await?;
        }
        Ok(())
    }

    /// Gom và gửi một tin cho hồ sơ `profile` nếu còn gì đáng báo. Trả có gửi không.
    async fn alert_profile(
        &self,
        tenant: &TenantContext,
        profile: Uuid,
        day: &str,
        at: Timestamp,
    ) -> Result<bool, Error> {
        let read = tenant.database().read();
        let Some(setting) =
            wishlist_alert_setting::Entity::find_by_id((tenant.id().as_uuid(), profile))
                .one(read)
                .await?
        else {
            self.stop_wishlist_alerts(tenant, profile).await?;
            return Ok(false);
        };
        let today = wishlist_alert_batch::Entity::find()
            .filter(wishlist_alert_batch::Column::ProfileId.eq(profile))
            .filter(wishlist_alert_batch::Column::LocalDay.eq(day))
            .scoped(tenant)
            .count(read)
            .await?;
        if today >= self.config.wishlist_alerts_per_day {
            return Ok(false);
        }
        if !self.alert_consented(tenant, setting.user_id).await? {
            self.stop_wishlist_alerts(tenant, profile).await?;
            return Ok(false);
        }
        let queued = wishlist_alert_queue::Entity::find()
            .filter(wishlist_alert_queue::Column::ProfileId.eq(profile))
            .scoped(tenant)
            .order_by_asc(wishlist_alert_queue::Column::QueuedAt)
            .limit(50)
            .all(read)
            .await?;
        let items = self.still_true(tenant, &setting, &queued).await?;
        if items.is_empty() {
            return Ok(false);
        }
        // Transaction ngắn: đánh dấu "đã báo" có điều kiện, đặt mốc mới, xoá hàng đợi, ghi lô.
        let transaction = tenant.database().begin_write().await?;
        let mut lines = Vec::new();
        let cutoff = at - SignedDuration::from_hours(24 * COOLDOWN_DAYS);
        for item in &items {
            let (column, price_drop) = if item.queue.kind == PRICE_DROP {
                (wishlist_item::Column::PriceDropAlertedAt, true)
            } else {
                (wishlist_item::Column::BackInStockAlertedAt, false)
            };
            let mut update = wishlist_item::Entity::update_many()
                .col_expr(column, Expr::value(at))
                .filter(wishlist_item::Column::ProfileId.eq(profile))
                .filter(wishlist_item::Column::ProductId.eq(item.row.product_id))
                .filter(wishlist_item::Column::VariantKey.eq(item.row.variant_key.as_str()))
                .filter(Condition::any().add(column.is_null()).add(column.lt(cutoff)))
                .scoped(tenant);
            if price_drop {
                // Mốc mới = giá đã báo: lần sau phải giảm tiếp so với giá này.
                update = update
                    .col_expr(
                        wishlist_item::Column::BaselinePriceMinor,
                        Expr::value(item.queue.price_minor),
                    )
                    .col_expr(
                        wishlist_item::Column::BaselineCurrency,
                        Expr::value(item.queue.currency.clone()),
                    )
                    .col_expr(wishlist_item::Column::BaselineAt, Expr::value(at));
            }
            let changed = update.exec(&transaction).await?.rows_affected;
            wishlist_alert_queue::Entity::delete_many()
                .filter(wishlist_alert_queue::Column::ProfileId.eq(profile))
                .filter(wishlist_alert_queue::Column::ProductId.eq(item.queue.product_id))
                .filter(
                    wishlist_alert_queue::Column::VariantKey.eq(item.queue.variant_key.as_str()),
                )
                .filter(wishlist_alert_queue::Column::Kind.eq(item.queue.kind.as_str()))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
            if changed == 1 {
                lines.push(json!({
                    "kind": item.queue.kind, "product_id": item.row.product_id,
                    "variant_id": item.queue.variant_id.or(item.row.variant_id),
                    "price": item.queue.price_minor.map(|minor| json!({
                        "amount": minor.to_string(), "currency": item.queue.currency,
                    })),
                }));
            }
        }
        if lines.is_empty() {
            transaction.commit().await?;
            return Ok(false);
        }
        let batch = wishlist_alert_batch::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            profile_id: Set(profile),
            user_id: Set(setting.user_id),
            local_day: Set(day.to_owned()),
            items: Set(Value::Array(lines).to_string()),
            status: Set(PENDING.to_owned()),
            created_at: Set(at),
            sent_at: Set(None),
        };
        let batch = sea_orm::ActiveModelTrait::insert(batch, &transaction).await?;
        transaction.commit().await?;
        self.deliver_batch(tenant, &batch).await?;
        Ok(true)
    }

    /// Kiểm lại từng dòng hàng đợi; bỏ dòng không còn đúng. Trả tối đa [`MAX_ITEMS`] dòng còn đúng.
    async fn still_true(
        &self,
        tenant: &TenantContext,
        setting: &wishlist_alert_setting::Model,
        queued: &[wishlist_alert_queue::Model],
    ) -> Result<Vec<Item>, Error> {
        let read = tenant.database().read();
        let rows: HashMap<(Uuid, String), wishlist_item::Model> = wishlist_item::Entity::find()
            .filter(wishlist_item::Column::ProfileId.eq(setting.profile_id))
            .filter(wishlist_item::Column::ProductId.is_in(queued.iter().map(|row| row.product_id)))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .map(|row| ((row.product_id, row.variant_key.clone()), row))
            .collect();
        let products: Vec<Uuid> = queued.iter().map(|row| row.product_id).collect();
        let variants: Vec<Uuid> = queued.iter().filter_map(|row| row.variant_id).collect();
        let prices = match &self.catalog {
            Some(catalog) => catalog.display_prices(tenant, &products, &variants).await?,
            None => autoxide_catalog::api::DisplayPrices::default(),
        };
        let stocked: Vec<Uuid> = match &self.inventory {
            Some(inventory) if !variants.is_empty() => {
                let mut totals: HashMap<Uuid, i64> = HashMap::new();
                for level in inventory.availability(tenant, root_of(tenant.id()), &variants).await?
                {
                    *totals.entry(level.variant_id).or_default() += level.available;
                }
                totals.into_iter().filter(|(_, total)| *total > 0).map(|(id, _)| id).collect()
            }
            _ => Vec::new(),
        };
        let mut items = Vec::new();
        let mut drop = Vec::new();
        for queue in queued {
            let row = rows.get(&(queue.product_id, queue.variant_key.clone()));
            let ok = match (row, queue.kind.as_str()) {
                (None, _) => false,
                (Some(_), BACK_IN_STOCK) => {
                    setting.back_in_stock
                        && queue.variant_id.is_some_and(|variant| {
                            stocked.contains(&variant) && prices.variants.contains_key(&variant)
                        })
                }
                (Some(row), PRICE_DROP) => {
                    let current = match row.variant_id {
                        Some(variant) => prices.variants.get(&variant),
                        None => prices.products.get(&row.product_id),
                    };
                    setting.price_drop
                        && match (
                            current,
                            row.baseline_price_minor,
                            row.baseline_currency.as_deref(),
                        ) {
                            (Some(current), Some(baseline), Some(currency)) => {
                                currency == current.currency().code()
                                    && dropped(
                                        current.minor(),
                                        baseline,
                                        self.config.price_drop_min_bp,
                                    )
                            }
                            _ => false,
                        }
                }
                _ => false,
            };
            match (ok, row) {
                (true, Some(row)) if items.len() < MAX_ITEMS => {
                    let mut queue = queue.clone();
                    if queue.kind == PRICE_DROP {
                        // Giá báo là giá hiện tại, không phải giá lúc xếp hàng.
                        let current = match row.variant_id {
                            Some(variant) => prices.variants.get(&variant),
                            None => prices.products.get(&row.product_id),
                        };
                        if let Some(current) = current {
                            queue.price_minor = Some(current.minor());
                            queue.currency = Some(current.currency().code().to_owned());
                        }
                    }
                    items.push(Item { queue, row: row.clone() });
                }
                (true, _) => {}
                (false, _) => drop.push(queue),
            }
        }
        self.drop_queued(tenant, &drop).await?;
        Ok(items)
    }

    /// Gửi một lô (gọi lại an toàn: `dedupe_key` theo lô), rồi đặt `sent` và phát event.
    async fn deliver_batch(
        &self,
        tenant: &TenantContext,
        batch: &wishlist_alert_batch::Model,
    ) -> Result<(), Error> {
        let Some(notifier) = &self.notifier else { return Ok(()) };
        let items: Value = serde_json::from_str(&batch.items).map_err(Error::internal)?;
        let count = items.as_array().map_or(0, Vec::len);
        let notification = Notification::new(
            WISHLIST_ALERT.template.key,
            Recipient::User(batch.user_id),
            json!({"count": count, "items": items}),
            format!("customers.wishlist_alert:{}", batch.id),
        );
        notifier.notify(tenant.id(), notification).await?;
        let transaction = tenant.database().begin_write().await?;
        let changed = wishlist_alert_batch::Entity::update_many()
            .col_expr(wishlist_alert_batch::Column::Status, Expr::value(SENT))
            .col_expr(wishlist_alert_batch::Column::SentAt, Expr::value(now()))
            .filter(wishlist_alert_batch::Column::Id.eq(batch.id))
            .filter(wishlist_alert_batch::Column::Status.eq(PENDING))
            .scoped(tenant)
            .exec(&transaction)
            .await?
            .rows_affected;
        if changed == 1 {
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::system(),
                    Some(subject(batch.profile_id)),
                    &WishlistAlertSent(Change::after(json!({
                        "profile_id": batch.profile_id, "batch_id": batch.id, "items": items,
                    }))),
                )
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}
