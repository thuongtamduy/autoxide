//! Báo khi sản phẩm yêu thích có hàng lại hay giảm giá (bước 47, D-233,
//! D-20261120-step47-wishlist-alerts).
//!
//! - Người mua tự bật (mặc định tắt), hai loại riêng `back_in_stock` / `price_drop`, kèm đồng ý theo mục
//!   đích riêng `customers.wishlist_alerts` ghi ở sổ đồng ý của `compliance` (bước 37).
//! - Event của `catalog` / `inventory` chỉ là tín hiệu: [`signals`] đọc lại giá / tồn hiện tại rồi xếp
//!   hàng; không gửi trong handler.
//! - Job hằng giờ [`batches`] gom mỗi người một tin (tối đa 10 sản phẩm), trong khung giờ quảng cáo,
//!   có trần tin mỗi ngày, mỗi sản phẩm mỗi loại tối đa một lần mỗi 7 ngày.

pub(crate) mod batches;
pub(crate) mod signals;

use autoxide_compliance::api::{ConsentEvidence, ConsentKey, ConsentSubject};
use autoxide_kernel::entitlement::Feature;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use autoxide_notifications::api::{Category, Channel, NotificationKind, NotificationTemplate};
use jiff::SignedDuration;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::entity::{wishlist_alert_queue, wishlist_alert_setting, wishlist_item};
use crate::service::{CustomersService, now};

/// Mục đích trong sổ đồng ý.
pub(crate) const PURPOSE: &str = "customers.wishlist_alerts";
pub(crate) const BACK_IN_STOCK: &str = "back_in_stock";
pub(crate) const PRICE_DROP: &str = "price_drop";
/// Mỗi người mỗi sản phẩm mỗi loại tối đa một lần trong chừng này ngày (cố định, D-233).
pub(crate) const COOLDOWN_DAYS: i64 = 7;
/// Tối đa sản phẩm trong một tin.
pub(crate) const MAX_ITEMS: usize = 10;

pub(crate) const WISHLIST_ALERTS: Feature = Feature::new(
    "customers.wishlist_alerts",
    "Buyers can ask to hear when wishlist products are back in stock or cheaper.",
);

/// Tin báo yêu thích: quảng cáo, theo mục đích riêng.
pub(crate) const WISHLIST_ALERT: NotificationKind = NotificationKind::new(
    Category::Marketing,
    &[Channel::Email, Channel::Zns, Channel::WebPush, Channel::MobilePush, Channel::InApp],
    NotificationTemplate::new(
        "customers.wishlist_alert",
        "Good news about your wishlist",
        "{count} products on your wishlist are back in stock or cheaper now.",
        "Wishlist products back in stock or cheaper",
    ),
)
.purpose(PURPOSE);

fn consent_required() -> Error {
    Error::new(
        ErrorKind::Validation,
        "customers.wishlist_alert_consent_required",
        "Turning on wishlist alerts needs the buyer's consent: send `consent_text` and \
         `consent_version` as shown to them.",
    )
}

fn unavailable() -> Error {
    Error::new(
        ErrorKind::Validation,
        "customers.wishlist_alert_unavailable",
        "This shop cannot send this kind of wishlist alert.",
    )
}

pub(crate) fn key() -> ConsentKey {
    ConsentKey::new(PURPOSE, "", "")
}

/// Còn trong thời gian chờ của loại đó không (đã báo trong 7 ngày qua).
pub(crate) fn cooling(alerted_at: Option<Timestamp>, at: Timestamp) -> bool {
    alerted_at.is_some_and(|when| when > at - SignedDuration::from_hours(24 * COOLDOWN_DAYS))
}

/// Giảm đủ ngưỡng: `mới × 10000 ≤ mốc × (10000 − bp)` (số nguyên `i128`, không làm tròn).
pub(crate) fn dropped(current: i64, baseline: i64, min_bp: i64) -> bool {
    i128::from(current) * 10_000 <= i128::from(baseline) * i128::from(10_000 - min_bp)
}

/// Lựa chọn của người mua.
#[derive(Debug, Clone, Default)]
pub(crate) struct AlertChoice {
    pub back_in_stock: bool,
    pub price_drop: bool,
    pub consent_text: Option<String>,
    pub consent_version: Option<String>,
}

/// Trạng thái hiện tại.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AlertState {
    pub back_in_stock: bool,
    pub price_drop: bool,
    /// Khách gửi được báo có hàng lại (có `inventory`).
    pub back_in_stock_available: bool,
}

impl CustomersService {
    pub(crate) async fn wishlist_alerts(
        &self,
        tenant: &TenantContext,
        user: Uuid,
    ) -> Result<AlertState, Error> {
        let profile = self.me(tenant, user).await?;
        let row = wishlist_alert_setting::Entity::find_by_id((tenant.id().as_uuid(), profile.id))
            .one(tenant.database().read())
            .await?;
        Ok(AlertState {
            back_in_stock: row.as_ref().is_some_and(|row| row.back_in_stock),
            price_drop: row.as_ref().is_some_and(|row| row.price_drop),
            back_in_stock_available: self.inventory.is_some(),
        })
    }

    /// Bật / tắt báo. Bật ghi đồng ý vào sổ (gọi `compliance` TRƯỚC transaction của mình); tắt cả hai
    /// thì rút đồng ý. Tắt một loại xoá hàng đợi của loại đó ngay.
    pub(crate) async fn set_wishlist_alerts(
        &self,
        tenant: &TenantContext,
        user: Uuid,
        choice: AlertChoice,
    ) -> Result<AlertState, Error> {
        let any = choice.back_in_stock || choice.price_drop;
        if any {
            self.entitlements.require_feature(tenant.id(), &WISHLIST_ALERTS).await?;
        }
        let mut problems = Vec::new();
        let text = choice.consent_text.as_deref().map(str::trim).unwrap_or_default();
        let version = choice.consent_version.as_deref().map(str::trim).unwrap_or_default();
        if any {
            if text.is_empty() || version.is_empty() {
                return Err(consent_required());
            }
            if text.chars().count() > 2000 {
                problems.push(FieldError::new(
                    "consent_text",
                    "length",
                    "must be at most 2000 characters",
                ));
            }
            if version.chars().count() > 50 {
                problems.push(FieldError::new(
                    "consent_version",
                    "length",
                    "must be at most 50 characters",
                ));
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        if (choice.back_in_stock && self.inventory.is_none())
            || (any && (self.consents.is_none() || self.notifier.is_none()))
        {
            return Err(unavailable());
        }
        let profile = self.me(tenant, user).await?;
        let before = self.wishlist_alerts(tenant, user).await?;
        if let Some(consents) = &self.consents {
            let subject = ConsentSubject::User(user);
            let source_ref = format!("{PURPOSE}:{}:{}", profile.id, Uuid::now_v7());
            if any {
                let evidence = ConsentEvidence {
                    text: text.to_owned(),
                    text_version: version.to_owned(),
                    source: "wishlist_alerts".to_owned(),
                    ip_fingerprint: None,
                };
                consents.grant(tenant.id(), subject, key(), evidence, &source_ref).await?;
            } else if before.back_in_stock || before.price_drop {
                consents
                    .withdraw(tenant.id(), subject, key(), "wishlist_alerts", &source_ref)
                    .await?;
            }
        }
        let transaction = tenant.database().begin_write().await?;
        wishlist_alert_setting::Entity::insert(wishlist_alert_setting::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            profile_id: Set(profile.id),
            user_id: Set(user),
            back_in_stock: Set(choice.back_in_stock),
            price_drop: Set(choice.price_drop),
            updated_at: Set(now()),
        })
        .on_conflict(
            OnConflict::columns([
                wishlist_alert_setting::Column::TenantId,
                wishlist_alert_setting::Column::ProfileId,
            ])
            .update_columns([
                wishlist_alert_setting::Column::BackInStock,
                wishlist_alert_setting::Column::PriceDrop,
                wishlist_alert_setting::Column::UpdatedAt,
            ])
            .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        let off: Vec<&str> = [
            (!choice.back_in_stock).then_some(BACK_IN_STOCK),
            (!choice.price_drop).then_some(PRICE_DROP),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !off.is_empty() {
            wishlist_alert_queue::Entity::delete_many()
                .filter(wishlist_alert_queue::Column::ProfileId.eq(profile.id))
                .filter(wishlist_alert_queue::Column::Kind.is_in(off))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        transaction.commit().await?;
        self.wishlist_alerts(tenant, user).await
    }

    /// Tắt cả hai loại và xoá hàng đợi của hồ sơ (rút đồng ý, xoá tài khoản).
    pub(crate) async fn stop_wishlist_alerts(
        &self,
        tenant: &TenantContext,
        profile: Uuid,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        wishlist_alert_setting::Entity::update_many()
            .col_expr(wishlist_alert_setting::Column::BackInStock, Expr::value(false))
            .col_expr(wishlist_alert_setting::Column::PriceDrop, Expr::value(false))
            .col_expr(wishlist_alert_setting::Column::UpdatedAt, Expr::value(now()))
            .filter(wishlist_alert_setting::Column::ProfileId.eq(profile))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        wishlist_alert_queue::Entity::delete_many()
            .filter(wishlist_alert_queue::Column::ProfileId.eq(profile))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Đồng ý báo yêu thích còn hiệu lực không (không có sổ đồng ý: không).
    pub(crate) async fn alert_consented(
        &self,
        tenant: &TenantContext,
        user: Uuid,
    ) -> Result<bool, Error> {
        match &self.consents {
            Some(consents) => {
                consents.is_granted(tenant.id(), &ConsentSubject::User(user), &key()).await
            }
            None => Ok(false),
        }
    }

    /// Rút đồng ý mục đích này: event không nói tài khoản nào, nên kiểm lại mọi người đang bật.
    pub(crate) async fn alert_consent_withdrawn(
        &self,
        tenant: &TenantContext,
        payload: &serde_json::Value,
    ) -> Result<(), Error> {
        if payload["purpose"].as_str() != Some(PURPOSE) {
            return Ok(());
        }
        let on = wishlist_alert_setting::Entity::find()
            .filter(
                sea_orm::Condition::any()
                    .add(wishlist_alert_setting::Column::BackInStock.eq(true))
                    .add(wishlist_alert_setting::Column::PriceDrop.eq(true)),
            )
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        for row in on {
            if !self.alert_consented(tenant, row.user_id).await? {
                self.stop_wishlist_alerts(tenant, row.profile_id).await?;
            }
        }
        Ok(())
    }

    /// Đặt giá mốc cho các dòng yêu thích chưa có (giá công khai lúc thêm). Gọi sau commit, không
    /// trong transaction (đọc `catalog`). Không có `catalog`: bỏ qua, mốc đặt ở lần đầu thấy giá.
    pub(crate) async fn fill_baselines(
        &self,
        tenant: &TenantContext,
        profile: Uuid,
    ) -> Result<(), Error> {
        let Some(catalog) = &self.catalog else { return Ok(()) };
        let rows = wishlist_item::Entity::find()
            .filter(wishlist_item::Column::ProfileId.eq(profile))
            .filter(wishlist_item::Column::BaselinePriceMinor.is_null())
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        for chunk in rows.chunks(autoxide_catalog::api::MAX_IDS) {
            let products: Vec<Uuid> = chunk.iter().map(|row| row.product_id).collect();
            let variants: Vec<Uuid> = chunk.iter().filter_map(|row| row.variant_id).collect();
            let prices = catalog.display_prices(tenant, &products, &variants).await?;
            for row in chunk {
                let price = match row.variant_id {
                    Some(variant) => prices.variants.get(&variant),
                    None => prices.products.get(&row.product_id),
                };
                if let Some(price) = price {
                    self.set_baseline(tenant, row, *price, true).await?;
                }
            }
        }
        Ok(())
    }

    /// Ghi giá mốc của một dòng. `only_empty`: chỉ khi dòng chưa có mốc (không đè mốc đã có).
    pub(crate) async fn set_baseline(
        &self,
        tenant: &TenantContext,
        row: &wishlist_item::Model,
        price: autoxide_kernel::money::Money,
        only_empty: bool,
    ) -> Result<(), Error> {
        let mut update = wishlist_item::Entity::update_many()
            .col_expr(wishlist_item::Column::BaselinePriceMinor, Expr::value(price.minor()))
            .col_expr(wishlist_item::Column::BaselineCurrency, Expr::value(price.currency().code()))
            .col_expr(wishlist_item::Column::BaselineAt, Expr::value(now()))
            .filter(wishlist_item::Column::ProfileId.eq(row.profile_id))
            .filter(wishlist_item::Column::ProductId.eq(row.product_id))
            .filter(wishlist_item::Column::VariantKey.eq(row.variant_key.as_str()))
            .scoped(tenant);
        if only_empty {
            update = update.filter(wishlist_item::Column::BaselinePriceMinor.is_null());
        }
        update.exec(tenant.database().write()).await?;
        Ok(())
    }
}
