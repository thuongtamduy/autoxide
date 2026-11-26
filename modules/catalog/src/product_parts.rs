//! Phần của sản phẩm sửa theo từng tập: nội dung theo ngôn ngữ, danh mục, ảnh (media công khai).

use std::collections::BTreeMap;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use uuid::Uuid;

use crate::entity::{product_category, product_media, translation, variant};
use crate::errors;
use crate::permissions::PRODUCTS_WRITE;
use crate::products::{
    ProductDetail, TextInput, check_text, free_slug, slug_conflict, translation_row,
};
use crate::service::{CatalogService, check_length, fail_if_any};

impl CatalogService {
    /// Đặt (thêm hay thay) nội dung bằng ngôn ngữ `locale`.
    pub(crate) async fn put_translation(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        locale: &str,
        text: TextInput,
    ) -> Result<ProductDetail, Error> {
        let locale = self.locale(tenant, locale).await?;
        let mut problems = Vec::new();
        let text = check_text(&mut problems, &text, id);
        fail_if_any(problems)?;
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let before =
            self.product_for_write(&transaction, tenant, principal, id, &PRODUCTS_WRITE).await?;
        let skus: Vec<String> = variant::Entity::find()
            .filter(variant::Column::ProductId.eq(id))
            .scoped(tenant)
            .all(&transaction)
            .await?
            .into_iter()
            .map(|row| row.sku)
            .collect();
        translation::Entity::delete_many()
            .filter(translation::Column::ProductId.eq(id))
            .filter(translation::Column::Locale.eq(locale.as_str()))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let mut text = text;
        if text.slug_generated {
            let base = text.slug.clone().unwrap_or_default();
            text.slug = Some(free_slug(&transaction, tenant, &locale, &base, id).await?);
        }
        let slug = text.slug.clone().unwrap_or_default();
        translation_row(tenant, id, &locale, &text, &skus)
            .insert(&transaction)
            .await
            .map_err(|error| slug_conflict(error, &slug))?;
        let after = self.touch(&transaction, tenant, actor, &before, None).await?;
        transaction.commit().await?;
        self.detail(tenant, after).await
    }

    /// Bỏ nội dung bằng ngôn ngữ `locale`; không bỏ được ngôn ngữ mặc định của sản phẩm.
    pub(crate) async fn delete_translation(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        locale: &str,
    ) -> Result<ProductDetail, Error> {
        let locale = self.locale(tenant, locale).await?;
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let before =
            self.product_for_write(&transaction, tenant, principal, id, &PRODUCTS_WRITE).await?;
        if before.default_locale == locale {
            return Err(Error::validation(vec![FieldError::new(
                "locale",
                "default_locale",
                "is the default language of the product; change `default_locale` first",
            )]));
        }
        translation::Entity::delete_many()
            .filter(translation::Column::ProductId.eq(id))
            .filter(translation::Column::Locale.eq(locale.as_str()))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let after = self.touch(&transaction, tenant, actor, &before, None).await?;
        transaction.commit().await?;
        self.detail(tenant, after).await
    }

    /// Thay cả tập danh mục của sản phẩm (danh mục đầu là danh mục chính).
    pub(crate) async fn put_categories(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        category_ids: Vec<Uuid>,
    ) -> Result<ProductDetail, Error> {
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let before =
            self.product_for_write(&transaction, tenant, principal, id, &PRODUCTS_WRITE).await?;
        let categories = self.existing_categories(&transaction, tenant, &category_ids).await?;
        product_category::Entity::delete_many()
            .filter(product_category::Column::ProductId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        for category in &categories {
            product_category::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                product_id: Set(id),
                category_id: Set(*category),
            }
            .insert(&transaction)
            .await?;
        }
        let mut row = before.clone().into_active_model();
        row.primary_category_id = Set(categories.first().copied());
        self.save_product(&transaction, tenant, actor, &before, row, None).await?;
        self.refresh_readiness(&transaction, tenant, &[id]).await?;
        transaction.commit().await?;
        self.get_row(tenant, id).await
    }

    /// Thay cả tập ảnh của sản phẩm. Ảnh phải là media công khai của khách
    /// (`catalog.media_not_public`); biến thể nêu kèm phải thuộc sản phẩm.
    pub(crate) async fn put_media(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        items: Vec<MediaInput>,
    ) -> Result<ProductDetail, Error> {
        let mut problems = Vec::new();
        if items.len() > 50 {
            problems.push(FieldError::new("media", "length", "must have at most 50 images"));
        }
        for (index, item) in items.iter().enumerate() {
            check_length(&mut problems, &format!("media[{index}].alt"), &item.alt, 0, 500);
        }
        fail_if_any(problems)?;
        // Kiểm media TRƯỚC transaction ghi: đó là lời gọi sang module khác.
        let ids: Vec<Uuid> = items.iter().map(|item| item.media_id).collect();
        self.check_public_media(tenant, &ids).await?;
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let before =
            self.product_for_write(&transaction, tenant, principal, id, &PRODUCTS_WRITE).await?;
        let variants: Vec<Uuid> = variant::Entity::find()
            .filter(variant::Column::ProductId.eq(id))
            .scoped(tenant)
            .all(&transaction)
            .await?
            .into_iter()
            .map(|row| row.id)
            .collect();
        if let Some(bad) = items
            .iter()
            .filter_map(|item| item.variant_id)
            .find(|variant| !variants.contains(variant))
        {
            return Err(errors::variant_not_found(bad));
        }
        product_media::Entity::delete_many()
            .filter(product_media::Column::ProductId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        // Ảnh bị bỏ khỏi sản phẩm: bỏ luôn `alt` theo ngôn ngữ của nó (D-202).
        crate::entity::media_alt::Entity::delete_many()
            .filter(crate::entity::media_alt::Column::ProductId.eq(id))
            .filter(crate::entity::media_alt::Column::MediaId.is_not_in(ids.clone()))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        for (position, item) in items.iter().enumerate() {
            product_media::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                product_id: Set(id),
                media_id: Set(item.media_id),
                variant_id: Set(item.variant_id),
                position: Set(i32::try_from(position).unwrap_or(i32::MAX)),
                alt: Set(item.alt.trim().to_owned()),
            }
            .insert(&transaction)
            .await?;
        }
        let after = self.touch(&transaction, tenant, actor, &before, None).await?;
        transaction.commit().await?;
        self.detail(tenant, after).await
    }

    /// Mọi ID trong `ids` là media công khai của khách. Chưa cài `media` thì không gắn ảnh được.
    pub(crate) async fn check_public_media(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<(), Error> {
        if ids.is_empty() {
            return Ok(());
        }
        let Some(media) = &self.media else {
            return Err(errors::media_not_public(ids[0]));
        };
        let found = media.find_many(tenant, ids).await?;
        let public: BTreeMap<Uuid, bool> = found
            .iter()
            .map(|item| (item.id, item.visibility == autoxide_media::api::MediaVisibility::Public))
            .collect();
        match ids.iter().find(|id| !public.get(id).copied().unwrap_or(false)) {
            Some(bad) => Err(errors::media_not_public(*bad)),
            None => Ok(()),
        }
    }
}

/// Một ảnh của sản phẩm.
#[derive(Debug, Clone)]
pub(crate) struct MediaInput {
    pub media_id: Uuid,
    pub variant_id: Option<Uuid>,
    /// `alt` bằng ngôn ngữ mặc định của sản phẩm; trống thì dùng `alt` của media.
    pub alt: String,
}
