//! Loại đối tượng `promotions.promotion` trong sổ đối tượng của kernel (D-123, D-238): menu của
//! website (và module khác) trỏ tới một chương trình mà không phụ thuộc crate `promotions`. Chỉ
//! công khai khi chương trình đánh dấu `public` đang chạy; khi đó tiêu đề, điều kiện và đường dẫn
//! chuẩn `/promotions/<id>` (website hiện trang bằng `GET /api/v1/promotions/public/promotions`).

use std::collections::HashMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::resource::{Resource, ResourceResolver};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use uuid::Uuid;

use crate::entity::{promotion, translation};
use crate::model::Status;

/// Tên loại đối tượng của chương trình.
pub(crate) const PROMOTION: &str = "promotions.promotion";

pub(crate) struct PromotionResolver;

#[async_trait::async_trait]
impl ResourceResolver for PromotionResolver {
    async fn resolve(&self, tenant: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        Ok(self.resolve_many(tenant, &[id.to_owned()], None).await?.remove(id))
    }

    // Một câu query cho chương trình, một cho bản dịch: menu tra nhiều mục một lần.
    async fn resolve_many(
        &self,
        tenant: &TenantContext,
        ids: &[String],
        locale: Option<&str>,
    ) -> Result<HashMap<String, Resource>, Error> {
        let wanted: Vec<Uuid> = ids.iter().filter_map(|id| id.parse().ok()).collect();
        if wanted.is_empty() {
            return Ok(HashMap::new());
        }
        let read = tenant.database().read();
        let rows = promotion::Entity::find()
            .filter(promotion::Column::Id.is_in(wanted.clone()))
            .scoped(tenant)
            .all(read)
            .await?;
        let texts = translation::Entity::find()
            .filter(translation::Column::PromotionId.is_in(wanted))
            .scoped(tenant)
            .order_by_asc(translation::Column::Id)
            .all(read)
            .await?;
        let now = crate::service::now();
        Ok(rows
            .into_iter()
            .map(|row| {
                let live = row.public
                    && row.status == Status::Active.as_str()
                    && row.starts_at <= now
                    && row.ends_at > now;
                // Chưa công khai: không đưa tiêu đề, điều kiện ra ngoài module.
                let resource = if live {
                    let text = texts
                        .iter()
                        .find(|text| {
                            text.promotion_id == row.id && Some(text.locale.as_str()) == locale
                        })
                        .or_else(|| texts.iter().find(|text| text.promotion_id == row.id));
                    Resource::new(text.map(|text| text.title.clone()).unwrap_or_default())
                        .public(true)
                        .summary(text.map(|text| text.terms.clone()).unwrap_or_default())
                        .url(Some(format!("/promotions/{}", row.id)))
                        .locale(text.map(|text| text.locale.clone()))
                } else {
                    Resource::new("")
                };
                (row.id.to_string(), resource)
            })
            .collect())
    }
}
