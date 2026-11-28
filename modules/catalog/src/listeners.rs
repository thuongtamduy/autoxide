//! Event `catalog` nghe. Bên nhận chịu được nhận lại một event (D-054): mọi việc ở đây là ghi đè
//! hay đặt lại, không cộng dồn.
//!
//! - `privacy::USER_DELETED` → bỏ `created_by` của sản phẩm, bảng giá, dòng giá người đó tạo.
//! - `inventory.cost_changed` (module `inventory`, bước 42, có thể chưa cài — nhận bằng
//!   `Subscription::optional`) → giá vốn trong `catalog_variant_costs` cho sàn giá (D-222). Payload
//!   mong đợi: `{"after": {"org_id", "variant_id", "unit_cost": {"amount": "<số nguyên>",
//!   "currency": "VND"}}}` — xem `api::events::INVENTORY_COST_CHANGED`.
//! - `orders.order_completed`, `orders.review_published` / `review_moderated` (module `orders`,
//!   đăng ký sau `catalog`) → số bán, mua cùng, điểm đánh giá (`recommend.rs`).
//! - `orders.order_confirmed` (đã trả tiền) → phát quyền tải hàng số; `orders.order_cancelled`,
//!   `orders.refund_completed` không gắn trả hàng → thu hồi (`digital.rs`). Phát theo khoá
//!   (dòng đơn, file) và thu hồi chỉ đặt `revoked_at` khi chưa có: nhận lại không đổi gì thêm.
//! - `compliance.kyc_expired` / `kyc_revoked` của một nút, `organizations.unit_moved` → sàn dừng hàng của
//!   người bán không còn định danh (`seller_pause.rs`); chỉ đổi dòng còn `active`, nhận lại không đổi gì.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler, Subscription};
use autoxide_kernel::money::Money;
use autoxide_kernel::privacy::deleted_user;
use autoxide_kernel::tenancy::ScopeExt;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::api::events::{
    INVENTORY_COST_CHANGED, ORDERS_ORDER_CANCELLED, ORDERS_ORDER_COMPLETED, ORDERS_ORDER_CONFIRMED,
    ORDERS_REFUND_COMPLETED, ORDERS_REVIEW_MODERATED, ORDERS_REVIEW_PUBLISHED,
};
use crate::digital::PaidLine;
use crate::entity::{download_grant, price, price_list, product};
use crate::service::CatalogService;

/// Nút tổ chức đổi chỗ hay đổi trạng thái (module `organizations`): website nào thấy sản phẩm nào
/// đổi theo cây, nên dựng lại storefront của khách.
const UNIT_MOVED: &str = "organizations.unit_moved";
const UNIT_STATUS_CHANGED: &str = "organizations.unit_status_changed";

pub(crate) fn subscriptions(service: &Arc<CatalogService>) -> Vec<Subscription> {
    vec![
        Subscription::new(autoxide_kernel::privacy::USER_DELETED, service.clone()),
        Subscription::optional(INVENTORY_COST_CHANGED, service.clone()),
        Subscription::optional(UNIT_MOVED, service.clone()),
        Subscription::optional(
            crate::api::events::PROMOTIONS_AUTOMATIC_PRICES_CHANGED,
            service.clone(),
        ),
        Subscription::optional(UNIT_STATUS_CHANGED, service.clone()),
        Subscription::optional(ORDERS_ORDER_COMPLETED, service.clone()),
        Subscription::optional(ORDERS_REVIEW_PUBLISHED, service.clone()),
        Subscription::optional(ORDERS_REVIEW_MODERATED, service.clone()),
        Subscription::optional(ORDERS_ORDER_CONFIRMED, service.clone()),
        Subscription::optional(ORDERS_ORDER_CANCELLED, service.clone()),
        Subscription::optional(ORDERS_REFUND_COMPLETED, service.clone()),
        Subscription::optional(autoxide_compliance::api::KYC_EXPIRED, service.clone()),
        Subscription::optional(autoxide_compliance::api::KYC_REVOKED, service.clone()),
    ]
}

/// Bên nghe `privacy::USER_DELETED` khi module đang tắt: bỏ `created_by` như lúc bật.
pub(crate) struct ForgetUsers;

#[async_trait::async_trait]
impl EventHandler for ForgetUsers {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        match deleted_user(event) {
            Some(user_id) => CatalogService::forget_user(event, user_id).await,
            None => Ok(()),
        }
    }
}

#[async_trait::async_trait]
impl EventHandler for CatalogService {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        if let Some(user_id) = deleted_user(event) {
            return Self::forget_user(event, user_id).await;
        }
        if event.name == INVENTORY_COST_CHANGED {
            let after = &event.payload["after"];
            let parsed = (|| {
                let org: Uuid = after.get("org_id")?.as_str()?.parse().ok()?;
                let variant: Uuid = after.get("variant_id")?.as_str()?.parse().ok()?;
                let cost: Money = serde_json::from_value(after.get("unit_cost")?.clone()).ok()?;
                Some((org, variant, cost))
            })();
            let Some((org, variant, cost)) = parsed else {
                tracing::warn!(event_id = %event.id, "inventory.cost_changed without org_id, variant_id or unit_cost; ignored");
                return Ok(());
            };
            return self.record_cost(&event.tenant, org, variant, cost).await;
        }
        if event.name == crate::api::events::PROMOTIONS_AUTOMATIC_PRICES_CHANGED {
            let variants: Vec<Uuid> = event.payload["after"]["variant_ids"]
                .as_array()
                .map(|ids| ids.iter().filter_map(|id| id.as_str()?.parse().ok()).collect())
                .unwrap_or_default();
            if variants.is_empty() {
                tracing::warn!(event_id = %event.id, "promotions.automatic_prices_changed without variant_ids; ignored");
                return Ok(());
            }
            // Job tính lại từ đầu: nhận lại event chỉ dựng lại thêm một lần.
            let write = event.tenant.database().write();
            return self.queue_variants(write, &event.tenant, variants).await;
        }
        if event.name == ORDERS_ORDER_COMPLETED {
            let Some((order, lines)) = crate::recommend::completed_order(&event.payload) else {
                tracing::warn!(event_id = %event.id, "orders.order_completed without order_id or lines; ignored");
                return Ok(());
            };
            return self
                .record_completed_order(&event.tenant, order, &lines, event.occurred_at)
                .await;
        }
        if event.name == ORDERS_REVIEW_PUBLISHED || event.name == ORDERS_REVIEW_MODERATED {
            let Some((product, count, sum)) = crate::recommend::product_rating(&event.payload)
            else {
                tracing::warn!(event_id = %event.id, "review event without product_id or product_rating; ignored");
                return Ok(());
            };
            return self.record_rating(&event.tenant, product, count, sum).await;
        }
        if event.name == ORDERS_ORDER_CONFIRMED {
            let after = &event.payload["after"];
            if after["payment_status"].as_str() != Some("paid") {
                // Thu khi giao (COD) hay chưa trả: chưa phát quyền tải.
                return Ok(());
            }
            let Some((order, buyer, lines)) = paid_order(after) else {
                tracing::warn!(event_id = %event.id, "orders.order_confirmed without order_id or lines; ignored");
                return Ok(());
            };
            return self.issue_downloads(&event.tenant, order, buyer, &lines).await.map(|_| ());
        }
        if event.name == ORDERS_ORDER_CANCELLED || event.name == ORDERS_REFUND_COMPLETED {
            let after = &event.payload["after"];
            // Hoàn tiền của một lần trả hàng chỉ có hàng vật lý (hàng số không trả được).
            if event.name == ORDERS_REFUND_COMPLETED && !after["return_id"].is_null() {
                return Ok(());
            }
            let Some(order) = after["order_id"].as_str().and_then(|id| id.parse().ok()) else {
                tracing::warn!(event_id = %event.id, event = %event.name, "order event without order_id; ignored");
                return Ok(());
            };
            let reason =
                if event.name == ORDERS_ORDER_CANCELLED { "cancelled" } else { "refunded" };
            let actor = autoxide_kernel::event::Actor::system();
            return self
                .revoke_downloads(&event.tenant, actor, order, None, reason)
                .await
                .map(|_| ());
        }
        if event.name == autoxide_compliance::api::KYC_EXPIRED
            || event.name == autoxide_compliance::api::KYC_REVOKED
        {
            // Chỉ hồ sơ của nút (người bán); hồ sơ người dùng, email… không gắn hàng.
            if event.payload["subject_kind"] != "org" {
                return Ok(());
            }
            let Some(org) = event.payload["subject_id"].as_str().and_then(|id| id.parse().ok())
            else {
                tracing::warn!(event_id = %event.id, event = %event.name, "KYC event without subject_id; ignored");
                return Ok(());
            };
            let node = Some(autoxide_kernel::tenancy::OrgId::from_uuid(org));
            return self.pause_unverified_sellers(&event.tenant, node).await.map(|_| ());
        }
        if event.name == UNIT_MOVED || event.name == UNIT_STATUS_CHANGED {
            if event.name == UNIT_MOVED {
                // Cửa hàng con chuyển sang dưới người bán chưa định danh: hàng của nó không còn được bán (LG-3b).
                self.pause_unverified_sellers(&event.tenant, None).await?;
            }
            // Job tính lại từ đầu: nhận lại event chỉ dựng lại thêm một lần.
            let write = event.tenant.database().write();
            return self.queue_all(write, &event.tenant).await;
        }
        Ok(())
    }
}

impl CatalogService {
    // Không dùng `self`: module đang tắt (chế độ "nghỉ", bước 70 P9) gọi được qua [`ForgetUsers`].
    async fn forget_user(event: &Delivered, user_id: Uuid) -> Result<(), Error> {
        let tenant = &event.tenant;
        let write = tenant.database().write();
        product::Entity::update_many()
            .col_expr(product::Column::CreatedBy, Expr::value(None::<Uuid>))
            .filter(product::Column::CreatedBy.eq(user_id))
            .scoped(tenant)
            .exec(write)
            .await?;
        price_list::Entity::update_many()
            .col_expr(price_list::Column::CreatedBy, Expr::value(None::<Uuid>))
            .filter(price_list::Column::CreatedBy.eq(user_id))
            .scoped(tenant)
            .exec(write)
            .await?;
        price::Entity::update_many()
            .col_expr(price::Column::CreatedBy, Expr::value(None::<Uuid>))
            .filter(price::Column::CreatedBy.eq(user_id))
            .scoped(tenant)
            .exec(write)
            .await?;
        // Quyền tải vẫn dùng được bằng token đã gửi; chỉ bỏ liên kết với tài khoản đã xoá.
        download_grant::Entity::update_many()
            .col_expr(download_grant::Column::BuyerId, Expr::value(None::<Uuid>))
            .filter(download_grant::Column::BuyerId.eq(user_id))
            .scoped(tenant)
            .exec(write)
            .await?;
        tracing::info!(user_id = %user_id, "catalog records of a deleted user anonymised");
        Ok(())
    }
}

/// Đọc `order_id`, `buyer_user_id` và `lines[].{line_id, variant_id}` của đơn đã trả tiền.
fn paid_order(after: &serde_json::Value) -> Option<(Uuid, Option<Uuid>, Vec<PaidLine>)> {
    let order: Uuid = after.get("order_id")?.as_str()?.parse().ok()?;
    let buyer = after.get("buyer_user_id").and_then(|id| id.as_str()?.parse().ok());
    let lines = after
        .get("lines")?
        .as_array()?
        .iter()
        .filter_map(|line| {
            let line_id = line.get("line_id")?.as_str()?.parse().ok()?;
            let variant_id = line.get("variant_id")?.as_str()?.parse().ok()?;
            Some(PaidLine { line_id, variant_id })
        })
        .collect();
    Some((order, buyer, lines))
}
