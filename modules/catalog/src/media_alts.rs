//! `alt` của ảnh sản phẩm theo ngôn ngữ (D-202). `catalog_product_media.alt` là bản ngôn ngữ mặc
//! định; bản ghi đè theo ngôn ngữ ở `catalog_product_media_alts`, ngôn ngữ kiểm theo ngôn ngữ cấu
//! hình của nội dung sản phẩm. Bỏ ảnh khỏi sản phẩm thì bỏ luôn bản ghi đè (cùng transaction, ở
//! `put_media`).
//!
//! Đọc công khai theo `locale` ([`CatalogService::public_alt`]): bản ghi đè của sản phẩm →
//! `alt` của sản phẩm khi `locale` là ngôn ngữ mặc định → `alt` của `media` theo ngôn ngữ
//! (`Media::find_many_in`: bản dịch hay mặc định của media) → `alt` của sản phẩm.

use autoxide_kernel::time::Timestamp;
use std::collections::HashMap;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

use crate::entity::{media_alt, product_media};
use crate::permissions::PRODUCTS_WRITE;
use crate::products::ProductDetail;
use crate::service::{CatalogService, check_length, fail_if_any, now};

fn product_media_not_found(media_id: Uuid) -> Error {
    Error::not_found(
        "catalog.product_media_not_found",
        "The image `{media_id}` is not one of this product's images.",
    )
    .with_param("media_id", media_id)
}

impl CatalogService {
    /// Đặt `alt` của ảnh `media_id` của sản phẩm `id` ở ngôn ngữ `locale`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn put_media_alt(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        media_id: Uuid,
        locale: &str,
        alt: &str,
        expected: Option<Timestamp>,
    ) -> Result<ProductDetail, Error> {
        let locale = self.locale(locale)?;
        let alt = alt.trim();
        let mut problems = Vec::new();
        check_length(&mut problems, "alt", alt, 1, 500);
        fail_if_any(problems)?;
        self.change_media_alt(tenant, principal, id, media_id, &locale, Some(alt), expected).await
    }

    /// Bỏ `alt` theo ngôn ngữ `locale` (đọc công khai lại theo thứ tự lùi).
    pub(crate) async fn delete_media_alt(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        media_id: Uuid,
        locale: &str,
        expected: Option<Timestamp>,
    ) -> Result<ProductDetail, Error> {
        let locale = self.locale(locale)?;
        self.change_media_alt(tenant, principal, id, media_id, &locale, None, expected).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn change_media_alt(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        media_id: Uuid,
        locale: &str,
        alt: Option<&str>,
        expected: Option<Timestamp>,
    ) -> Result<ProductDetail, Error> {
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let before =
            self.product_for_write(&transaction, tenant, principal, id, &PRODUCTS_WRITE).await?;
        let attached = product_media::Entity::find()
            .filter(product_media::Column::ProductId.eq(id))
            .filter(product_media::Column::MediaId.eq(media_id))
            .scoped(tenant)
            .one(&transaction)
            .await?;
        if attached.is_none() {
            return Err(product_media_not_found(media_id));
        }
        media_alt::Entity::delete_many()
            .filter(media_alt::Column::ProductId.eq(id))
            .filter(media_alt::Column::MediaId.eq(media_id))
            .filter(media_alt::Column::Locale.eq(locale))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if let Some(alt) = alt {
            media_alt::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                product_id: Set(id),
                media_id: Set(media_id),
                locale: Set(locale.to_owned()),
                alt: Set(alt.to_owned()),
                updated_at: Set(now()),
            }
            .insert(&transaction)
            .await?;
        }
        // Phiên bản mới của sản phẩm (ETag, `If-Match`) và event `catalog.product_updated`.
        let after = self.touch(&transaction, tenant, actor, &before, expected).await?;
        transaction.commit().await?;
        self.detail(tenant, after).await
    }
}

/// `alt` công khai của ảnh `row` ở `locale` (thứ tự ở đầu file). `media_alt`: `alt` của media theo
/// ngôn ngữ đó (bản dịch hay mặc định của media).
pub(crate) fn public_alt(
    row: &product_media::Model,
    overrides: &HashMap<Uuid, String>,
    is_default_locale: bool,
    media_alt: &str,
) -> String {
    if let Some(alt) = overrides.get(&row.media_id) {
        return alt.clone();
    }
    if is_default_locale && !row.alt.is_empty() {
        return row.alt.clone();
    }
    if !media_alt.is_empty() {
        return media_alt.to_owned();
    }
    row.alt.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alt_theo_thu_tu_lui() {
        let media = Uuid::now_v7();
        let row = product_media::Model {
            id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
            product_id: Uuid::now_v7(),
            media_id: media,
            variant_id: None,
            position: 0,
            alt: "Áo đỏ".to_owned(),
        };
        let mut overrides = HashMap::new();
        assert_eq!(public_alt(&row, &overrides, true, "Red shirt"), "Áo đỏ");
        assert_eq!(public_alt(&row, &overrides, false, "Red shirt"), "Red shirt");
        assert_eq!(public_alt(&row, &overrides, false, ""), "Áo đỏ");
        overrides.insert(media, "A red T-shirt".to_owned());
        assert_eq!(public_alt(&row, &overrides, false, "Red shirt"), "A red T-shirt");
    }
}
