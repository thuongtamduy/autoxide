//! Loại thông báo của báo khi có hàng lại, và bên nghe `inventory.level_changed` gửi chúng.
//!
//! Tin **giao dịch** (người nhận tự xin, đã xác nhận hai bước, đồng ý riêng cho mục đích này) — không
//! phải tin quảng cáo: chỉ nói món đã có lại, không kèm khuyến mãi (D-20261119-step42-restock-alerts;
//! căn cứ NĐ 91/2020/NĐ-CP — cần đối chiếu văn bản gốc / người có chuyên môn pháp lý).

use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_catalog::api::VariantSnapshot;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler, Subscription};
use autoxide_kernel::secret::{random_token, sha256};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, root_of};
use autoxide_notifications::api::{
    Category, Channel, Notification, NotificationKind, NotificationTemplate, Notifier,
};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{CONFIRMED, recipient, token_link};
use crate::api::events::LEVEL_CHANGED;
use crate::entity::restock_alert;
use crate::public::StockLevel;
use crate::service::{InventoryService, now};

pub(crate) const RESTOCK_CONFIRM: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::Email, Channel::Sms],
    NotificationTemplate::new(
        "inventory.restock_confirm",
        "Confirm your back-in-stock alert",
        "Open the link to confirm that you want a message when {product} is back in stock. If you \
         did not ask for this, ignore this message.",
        "Confirm the alert for {product}",
    ),
);

pub(crate) const BACK_IN_STOCK: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::Email, Channel::Sms, Channel::InApp],
    NotificationTemplate::new(
        "inventory.back_in_stock",
        "{product} is back in stock",
        "{product}, which you asked about, is available again. To stop these alerts, open the \
         link.",
        "{product} is back in stock",
    ),
);

/// Khai hai loại thông báo; gọi trong `Module::start`.
pub(crate) fn declare(notifier: &dyn Notifier) -> Result<(), Error> {
    notifier.declare(RESTOCK_CONFIRM)?;
    notifier.declare(BACK_IN_STOCK)
}

/// Tên hiển thị của biến thể trong tin.
pub(crate) fn product_name(variant: &VariantSnapshot) -> String {
    if variant.variant_label.is_empty() {
        variant.name.clone()
    } else {
        format!("{} ({})", variant.name, variant.variant_label)
    }
}

pub(crate) fn subscriptions(service: &Arc<InventoryService>) -> Vec<Subscription> {
    if service.notifier.is_none() {
        return Vec::new();
    }
    vec![Subscription::new(LEVEL_CHANGED, Arc::new(Levels(service.clone())))]
}

struct Levels(Arc<InventoryService>);

fn available(side: &Value) -> Option<i64> {
    side.get("available").and_then(Value::as_i64)
}

#[async_trait::async_trait]
impl EventHandler for Levels {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let (Some(before), Some(after)) =
            (available(&event.payload["before"]), available(&event.payload["after"]))
        else {
            return Ok(());
        };
        let Some(variant) =
            event.payload["after"]["variant_id"].as_str().and_then(|id| id.parse::<Uuid>().ok())
        else {
            return Ok(());
        };
        if before <= 0 && after > 0 {
            self.0.announce_restock(&event.tenant, variant).await
        } else if before > 0 && after <= 0 {
            self.0.rearm_restock(&event.tenant, variant).await
        } else {
            Ok(())
        }
    }
}

impl InventoryService {
    /// Cửa hàng nào (trong `stores`; `nil` = kho bán online, tức nút gốc) bán được `variant` lúc này.
    async fn selling(
        &self,
        tenant: &TenantContext,
        variant: Uuid,
        stores: &[Uuid],
    ) -> Result<BTreeMap<Uuid, bool>, Error> {
        let root = root_of(tenant.id()).as_uuid();
        let mut answer = BTreeMap::new();
        for store in stores {
            let selling = if store.is_nil() { root } else { *store };
            let rows = self.levels_at(tenant, &[variant], &[selling]).await?;
            let open =
                rows.iter().any(|row| matches!(row.level, StockLevel::InStock | StockLevel::Low));
            answer.insert(*store, open);
        }
        Ok(answer)
    }

    async fn alerts_of(
        &self,
        tenant: &TenantContext,
        variant: Uuid,
        armed: bool,
    ) -> Result<Vec<restock_alert::Model>, Error> {
        Ok(restock_alert::Entity::find()
            .filter(restock_alert::Column::VariantId.eq(variant))
            .filter(restock_alert::Column::Status.eq(CONFIRMED))
            .filter(restock_alert::Column::Armed.eq(armed))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?)
    }

    /// Một đợt có hàng của `variant`: báo mỗi đăng ký đang chờ ở cửa hàng nay bán được, một lần.
    pub(crate) async fn announce_restock(
        &self,
        tenant: &TenantContext,
        variant: Uuid,
    ) -> Result<(), Error> {
        let Some(notifier) = &self.notifier else { return Ok(()) };
        let waiting = self.alerts_of(tenant, variant, true).await?;
        if waiting.is_empty() {
            return Ok(());
        }
        let mut stores: Vec<Uuid> = waiting.iter().map(|row| row.store_key).collect();
        stores.sort_unstable();
        stores.dedup();
        let open = self.selling(tenant, variant, &stores).await?;
        let Some(snapshot) =
            self.catalog.variants(tenant, &[variant], None).await?.into_iter().next()
        else {
            return Ok(());
        };
        let product = product_name(&snapshot);
        let page = self.restock_page(tenant).await?;
        for row in waiting {
            if !open.get(&row.store_key).copied().unwrap_or(false) {
                continue;
            }
            let Some(to) = recipient(&row) else { continue };
            // Giành lượt gửi trước: chỉ một lần giao event (hay một máy) thắng câu `UPDATE` này.
            let count = row.sent_count + 1;
            let raw = random_token(24)?;
            let won = restock_alert::Entity::update_many()
                .col_expr(restock_alert::Column::Armed, Expr::value(false))
                .col_expr(restock_alert::Column::SentCount, Expr::value(count))
                .col_expr(restock_alert::Column::TokenHash, Expr::value(Some(sha256(&raw))))
                .col_expr(restock_alert::Column::UpdatedAt, Expr::value(now()))
                .filter(restock_alert::Column::Id.eq(row.id))
                .filter(restock_alert::Column::Armed.eq(true))
                .filter(restock_alert::Column::SentCount.eq(row.sent_count))
                .scoped(tenant)
                .exec(tenant.database().write())
                .await?
                .rows_affected;
            if won == 0 {
                continue;
            }
            let mut notification = Notification::new(
                BACK_IN_STOCK.key(),
                to,
                json!({"product": product}),
                format!("inventory.back_in_stock:{}:{count}", row.id),
            );
            if let Some(url) = page.as_deref() {
                notification = notification.link(token_link(url, tenant, &raw));
            }
            if let Some(locale) = &row.locale {
                notification = notification.locale(locale.clone());
            }
            if let Err(error) = notifier.notify(tenant.id(), notification).await {
                // Trả lượt lại để lần giao event sau gửi được (cùng khoá chống trùng).
                restock_alert::Entity::update_many()
                    .col_expr(restock_alert::Column::Armed, Expr::value(true))
                    .col_expr(restock_alert::Column::SentCount, Expr::value(row.sent_count))
                    .filter(restock_alert::Column::Id.eq(row.id))
                    .filter(restock_alert::Column::SentCount.eq(count))
                    .scoped(tenant)
                    .exec(tenant.database().write())
                    .await?;
                return Err(error);
            }
        }
        Ok(())
    }

    /// Cửa hàng hết hàng lại: bật chờ đợt sau cho đăng ký đã được báo ở cửa hàng đó.
    pub(crate) async fn rearm_restock(
        &self,
        tenant: &TenantContext,
        variant: Uuid,
    ) -> Result<(), Error> {
        let done = self.alerts_of(tenant, variant, false).await?;
        if done.is_empty() {
            return Ok(());
        }
        let mut stores: Vec<Uuid> = done.iter().map(|row| row.store_key).collect();
        stores.sort_unstable();
        stores.dedup();
        let open = self.selling(tenant, variant, &stores).await?;
        let out: Vec<Uuid> = done
            .iter()
            .filter(|row| !open.get(&row.store_key).copied().unwrap_or(true))
            .map(|row| row.id)
            .collect();
        if out.is_empty() {
            return Ok(());
        }
        restock_alert::Entity::update_many()
            .col_expr(restock_alert::Column::Armed, Expr::value(true))
            .col_expr(restock_alert::Column::UpdatedAt, Expr::value(now()))
            .filter(restock_alert::Column::Id.is_in(out))
            .filter(restock_alert::Column::Status.eq(CONFIRMED))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        Ok(())
    }
}
