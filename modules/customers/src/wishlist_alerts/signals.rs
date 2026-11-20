//! Tín hiệu từ `catalog` (giá) và `inventory` (tồn) → hàng đợi báo yêu thích. Event chỉ là tín hiệu:
//! đọc lại giá / tồn hiện tại theo lô, không tin số trong event đến trễ hay đến lại. Không gửi ở đây.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::{OnConflict, Query};
use sea_orm::{ActiveValue::Set, ColumnTrait, Condition, EntityTrait, QueryFilter};
use serde_json::Value;
use uuid::Uuid;

use super::{BACK_IN_STOCK, PRICE_DROP, cooling, dropped};
use crate::entity::{wishlist_alert_queue, wishlist_alert_setting, wishlist_item};
use crate::service::{CustomersService, now};

/// Hồ sơ đang bật loại `kind` (câu con, để lọc dòng yêu thích trong database).
fn enabled(tenant: &TenantContext, kind: &str) -> sea_orm::sea_query::SelectStatement {
    let column = if kind == BACK_IN_STOCK {
        wishlist_alert_setting::Column::BackInStock
    } else {
        wishlist_alert_setting::Column::PriceDrop
    };
    Query::select()
        .column(wishlist_alert_setting::Column::ProfileId)
        .from(wishlist_alert_setting::Entity)
        .and_where(wishlist_alert_setting::Column::TenantId.eq(tenant.id().as_uuid()))
        .and_where(column.eq(true))
        .to_owned()
}

impl CustomersService {
    /// Ghi (hay cập nhật) một dòng hàng đợi.
    async fn enqueue_alert(
        &self,
        tenant: &TenantContext,
        row: &wishlist_item::Model,
        kind: &str,
        variant: Option<Uuid>,
        price: Option<Money>,
    ) -> Result<(), Error> {
        wishlist_alert_queue::Entity::insert(wishlist_alert_queue::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            profile_id: Set(row.profile_id),
            product_id: Set(row.product_id),
            variant_key: Set(row.variant_key.clone()),
            kind: Set(kind.to_owned()),
            variant_id: Set(variant),
            price_minor: Set(price.map(|money| money.minor())),
            currency: Set(price.map(|money| money.currency().code().to_owned())),
            queued_at: Set(now()),
        })
        .on_conflict(
            OnConflict::columns([
                wishlist_alert_queue::Column::TenantId,
                wishlist_alert_queue::Column::ProfileId,
                wishlist_alert_queue::Column::ProductId,
                wishlist_alert_queue::Column::VariantKey,
                wishlist_alert_queue::Column::Kind,
            ])
            .update_columns([
                wishlist_alert_queue::Column::VariantId,
                wishlist_alert_queue::Column::PriceMinor,
                wishlist_alert_queue::Column::Currency,
            ])
            .to_owned(),
        )
        .exec_without_returning(tenant.database().write())
        .await?;
        Ok(())
    }

    /// Giá của các sản phẩm `products` có thể đã đổi: dòng yêu thích của người bật `price_drop` giảm
    /// đủ ngưỡng so với mốc → xếp hàng. Chưa có mốc / khác tiền tệ → đặt mốc, không báo.
    pub(crate) async fn price_signal(
        &self,
        tenant: &TenantContext,
        products: &[Uuid],
    ) -> Result<u64, Error> {
        let Some(catalog) = &self.catalog else { return Ok(0) };
        let at = now();
        let mut queued = 0;
        for chunk in products.chunks(autoxide_catalog::api::MAX_IDS) {
            let rows = wishlist_item::Entity::find()
                .filter(wishlist_item::Column::ProductId.is_in(chunk.iter().copied()))
                .filter(wishlist_item::Column::ProfileId.in_subquery(enabled(tenant, PRICE_DROP)))
                .scoped(tenant)
                .all(tenant.database().read())
                .await?;
            let variants: Vec<Uuid> = rows
                .iter()
                .filter_map(|row| row.variant_id)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let mut prices = autoxide_catalog::api::DisplayPrices::default();
            for part in variants.chunks(autoxide_catalog::api::MAX_IDS) {
                let found = catalog.display_prices(tenant, &[], part).await?;
                prices.variants.extend(found.variants);
            }
            prices.products = catalog.display_prices(tenant, chunk, &[]).await?.products;
            for row in &rows {
                let current = match row.variant_id {
                    Some(variant) => prices.variants.get(&variant),
                    None => prices.products.get(&row.product_id),
                };
                let Some(current) = current.copied() else { continue };
                let baseline = match (row.baseline_price_minor, row.baseline_currency.as_deref()) {
                    (Some(minor), Some(currency)) if currency == current.currency().code() => minor,
                    _ => {
                        self.set_baseline(tenant, row, current, false).await?;
                        continue;
                    }
                };
                if dropped(current.minor(), baseline, self.config.price_drop_min_bp)
                    && !cooling(row.price_drop_alerted_at, at)
                {
                    self.enqueue_alert(tenant, row, PRICE_DROP, row.variant_id, Some(current))
                        .await?;
                    queued += 1;
                }
            }
        }
        Ok(queued)
    }

    /// Biến thể `variant` vừa có hàng lại: dòng theo biến thể đó và dòng theo sản phẩm của nó, của
    /// người bật `back_in_stock`, trừ người đã tự đăng ký báo có hàng ở `inventory` (một lần có hàng
    /// một tin, D-237).
    pub(crate) async fn stock_signal(
        &self,
        tenant: &TenantContext,
        variant: Uuid,
    ) -> Result<u64, Error> {
        let Some(inventory) = &self.inventory else { return Ok(0) };
        let product = match &self.catalog {
            Some(catalog) => catalog
                .variants(tenant, &[variant], None)
                .await?
                .first()
                .map(|snapshot| snapshot.product_id),
            None => None,
        };
        let mut matches = Condition::any().add(wishlist_item::Column::VariantId.eq(variant));
        if let Some(product) = product {
            matches = matches.add(
                Condition::all()
                    .add(wishlist_item::Column::ProductId.eq(product))
                    .add(wishlist_item::Column::VariantKey.eq("")),
            );
        }
        let rows = wishlist_item::Entity::find()
            .filter(matches)
            .filter(wishlist_item::Column::ProfileId.in_subquery(enabled(tenant, BACK_IN_STOCK)))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        if rows.is_empty() {
            return Ok(0);
        }
        let owners: HashMap<Uuid, Uuid> = wishlist_alert_setting::Entity::find()
            .filter(
                wishlist_alert_setting::Column::ProfileId
                    .is_in(rows.iter().map(|row| row.profile_id)),
            )
            .scoped(tenant)
            .all(tenant.database().read())
            .await?
            .into_iter()
            .map(|setting| (setting.profile_id, setting.user_id))
            .collect();
        let at = now();
        let mut queued = 0;
        for row in &rows {
            let Some(user) = owners.get(&row.profile_id) else { continue };
            if cooling(row.back_in_stock_alerted_at, at) {
                continue;
            }
            if inventory.restock_alerts_of(tenant, *user, &[variant]).await?.contains(&variant) {
                continue;
            }
            self.enqueue_alert(tenant, row, BACK_IN_STOCK, Some(variant), None).await?;
            queued += 1;
        }
        Ok(queued)
    }
}

/// ID trong một mảng JSON (`["…", …]`), bỏ phần không phải UUID.
fn ids(value: &Value) -> Vec<Uuid> {
    value
        .as_array()
        .map(|items| {
            items.iter().filter_map(|item| item.as_str()?.parse().ok()).collect::<Vec<Uuid>>()
        })
        .unwrap_or_default()
}

/// Bên nghe event giá của `catalog` và tồn của `inventory`.
pub(crate) struct AlertSignals(pub Arc<CustomersService>);

impl AlertSignals {
    /// Biến thể → sản phẩm (qua `catalog`).
    async fn products_of(
        &self,
        tenant: &TenantContext,
        variants: Vec<Uuid>,
    ) -> Result<Vec<Uuid>, Error> {
        let Some(catalog) = &self.0.catalog else { return Ok(Vec::new()) };
        let mut products = BTreeSet::new();
        for chunk in variants.chunks(autoxide_catalog::api::MAX_IDS) {
            for snapshot in catalog.variants(tenant, chunk, None).await? {
                products.insert(snapshot.product_id);
            }
        }
        Ok(products.into_iter().collect())
    }
}

#[async_trait::async_trait]
impl EventHandler for AlertSignals {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        use autoxide_catalog::api::events as catalog;
        let tenant = &event.tenant;
        let after = &event.payload["after"];
        let products = match event.name.as_str() {
            catalog::DISPLAY_PRICE_CHANGED => {
                after["product_id"].as_str().and_then(|id| id.parse().ok()).into_iter().collect()
            }
            catalog::PRICE_CHANGED => {
                let variants = after["prices"]
                    .as_array()
                    .map(|prices| {
                        prices
                            .iter()
                            .filter_map(|price| price["variant_id"].as_str()?.parse().ok())
                            .collect()
                    })
                    .unwrap_or_default();
                self.products_of(tenant, variants).await?
            }
            catalog::PRICE_EFFECTIVE => {
                self.products_of(tenant, ids(&after["variant_ids"])).await?
            }
            autoxide_inventory::api::events::LEVEL_CHANGED => {
                let before = event.payload["before"]["available"].as_i64().unwrap_or(0);
                let now_available = after["available"].as_i64().unwrap_or(0);
                if before <= 0
                    && now_available > 0
                    && let Some(variant) =
                        after["variant_id"].as_str().and_then(|id| id.parse().ok())
                {
                    self.0.stock_signal(tenant, variant).await?;
                }
                return Ok(());
            }
            _ => return Ok(()),
        };
        self.0.price_signal(tenant, &products).await?;
        Ok(())
    }
}
