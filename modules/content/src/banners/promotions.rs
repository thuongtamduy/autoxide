//! Banner trỏ tới chương trình khuyến mại tự vào lưu trữ khi chương trình kết thúc hay bị huỷ:
//! quảng cáo không được hứa ưu đãi đã hết (Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15, Luật
//! Quảng cáo 16/2012/QH13 — cần đối chiếu văn bản gốc).
//!
//! Nghe `promotions.promotion_ended` / `promotions.promotion_cancelled` bằng `Subscription::optional`
//! (app có thể không cài `promotions`); chỉ dùng chủ thể của event, không đọc bảng của `promotions`.
//! Nhận lại event không đổi gì thêm: chỉ banner chưa lưu trữ bị đổi. Đọc công khai cũng tự bỏ
//! banner có đích không còn công khai, nên banner vẫn ẩn kể cả trước khi event tới.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Subscription};
use autoxide_kernel::tenancy::ScopeExt;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::json;

use crate::entity::banner;
use crate::events::{self, Change};
use crate::service::{ContentService, now};

/// Loại đối tượng chương trình khuyến mại trong sổ đối tượng.
pub(crate) const PROMOTION: &str = "promotions.promotion";
pub(crate) const PROMOTION_ENDED: &str = "promotions.promotion_ended";
pub(crate) const PROMOTION_CANCELLED: &str = "promotions.promotion_cancelled";

pub(crate) fn subscriptions(service: &Arc<ContentService>) -> Vec<Subscription> {
    let handler = Arc::new(Ended(service.clone()));
    vec![
        Subscription::optional(PROMOTION_ENDED, handler.clone()),
        Subscription::optional(PROMOTION_CANCELLED, handler),
    ]
}

pub(crate) struct Ended(pub Arc<ContentService>);

#[async_trait::async_trait]
impl EventHandler for Ended {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(id) = event
            .subject
            .as_deref()
            .and_then(|subject| subject.strip_prefix(PROMOTION))
            .and_then(|rest| rest.strip_prefix(':'))
            .filter(|id| !id.is_empty())
        else {
            return Ok(());
        };
        self.0.archive_linked(&event.tenant, PROMOTION, id).await
    }
}

impl ContentService {
    /// Lưu trữ mọi banner chưa lưu trữ có link tới đối tượng `kind:id`.
    pub(crate) async fn archive_linked(
        &self,
        tenant: &autoxide_kernel::tenancy::TenantContext,
        kind: &str,
        id: &str,
    ) -> Result<(), Error> {
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        let rows = banner::Entity::find()
            .scoped(tenant)
            .filter(banner::Column::LinkKind.eq(kind))
            .filter(banner::Column::LinkId.eq(id))
            .filter(banner::Column::Status.ne("archived"))
            .all(&transaction)
            .await?;
        if rows.is_empty() {
            return Ok(());
        }
        let ids: Vec<uuid::Uuid> = rows.iter().map(|row| row.id).collect();
        banner::Entity::update_many()
            .col_expr(banner::Column::Status, Expr::value("archived"))
            .col_expr(banner::Column::UpdatedAt, Expr::value(at))
            .scoped(tenant)
            .filter(banner::Column::Id.is_in(ids))
            .exec(&transaction)
            .await?;
        for row in &rows {
            let event = events::BannerChanged(Change {
                before: Some(json!({"status": row.status})),
                after: Some(json!({"status": "archived", "reason": "link_target_ended"})),
                published: None,
            });
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::system(),
                    events::subject("banner", row.id),
                    &event,
                )
                .await?;
        }
        transaction.commit().await?;
        tracing::info!(banners = rows.len(), kind, "banners archived: their link target ended");
        Ok(())
    }
}
