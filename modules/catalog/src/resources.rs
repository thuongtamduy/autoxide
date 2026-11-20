//! Loại `catalog.product` và `catalog.category` trong sổ đối tượng của kernel (D-123, D-202,
//! D-238), cùng nhãn `seller`:
//!
//! - `catalog.product`: hỏi đáp của `comments` kiểm đích, menu trỏ tới sản phẩm. Công khai khi
//!   đang bán (`active`) và có nội dung ở ngôn ngữ hỏi; đường dẫn `/<tiền tố>/<slug>`, ảnh đầu tiên.
//!   Sản phẩm chưa công khai không đưa tên ra ngoài module.
//! - `catalog.category`: menu của website (`menus`, bước 68). `resolve_many` theo ngôn ngữ, cây
//!   (`children` theo thứ tự hiển thị), chỉ danh mục đang hiện (cả nhánh cha cũng hiện).
//! - Nhãn `seller` trên sản phẩm: tác giả có `catalog.products.write` ở nút sở hữu sản phẩm (phạm vi
//!   theo bước 34) — người quản lý trả lời hỏi đáp được nhận ra.
//! - Event vòng đời ([`resource_events`]): lưu trữ hay xoá sản phẩm thì `comments` đóng và ẩn hỏi đáp.

use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::resource::{
    BadgeResolver, Resource, ResourceChange, ResourceEvent, ResourceRef, ResourceResolver,
};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use uuid::Uuid;

use crate::api::events::{PRODUCT_ARCHIVED, PRODUCT_DELETED};
use crate::entity::{product, product_media, translation};
use crate::permissions::PRODUCTS_WRITE;
use crate::service::{CatalogService, at};

pub(crate) const PRODUCT: &str = "catalog.product";
pub(crate) const CATEGORY: &str = "catalog.category";
/// Đầu đường dẫn công khai của danh mục (website mở `/categories/<slug>`).
pub(crate) const CATEGORY_PATH_PREFIX: &str = "categories";
/// Nhãn của người quản lý sản phẩm.
pub(crate) const SELLER: &str = "seller";

/// Event vòng đời của sản phẩm cho module gắn dữ liệu vào nó (D-202).
pub(crate) fn resource_events() -> Vec<ResourceEvent> {
    vec![
        ResourceEvent::new(PRODUCT_ARCHIVED, PRODUCT, ResourceChange::Removed),
        ResourceEvent::new(PRODUCT_DELETED, PRODUCT, ResourceChange::Removed),
    ]
}

fn ids_of(ids: &[String]) -> Vec<Uuid> {
    ids.iter().filter_map(|id| id.parse().ok()).collect()
}

/// Bộ tra `catalog.product`.
pub(crate) struct ProductResolver(pub(crate) Arc<CatalogService>);

#[async_trait::async_trait]
impl ResourceResolver for ProductResolver {
    async fn resolve(&self, tenant: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        let mut found = self.resolve_many(tenant, &[id.to_owned()], None).await?;
        Ok(found.remove(id))
    }

    async fn resolve_many(
        &self,
        tenant: &TenantContext,
        ids: &[String],
        locale: Option<&str>,
    ) -> Result<HashMap<String, Resource>, Error> {
        let wanted = ids_of(ids);
        if wanted.is_empty() {
            return Ok(HashMap::new());
        }
        let read = tenant.database().read();
        let rows = product::Entity::find()
            .filter(product::Column::Id.is_in(wanted.clone()))
            .scoped(tenant)
            .all(read)
            .await?;
        let texts = translation::Entity::find()
            .filter(translation::Column::ProductId.is_in(wanted.clone()))
            .scoped(tenant)
            .all(read)
            .await?;
        // Ảnh đầu tiên: thứ tự giảm, ảnh vị trí nhỏ nhất ghi sau cùng.
        let mut images: HashMap<Uuid, Uuid> = HashMap::new();
        for row in product_media::Entity::find()
            .filter(product_media::Column::ProductId.is_in(wanted))
            .scoped(tenant)
            .order_by_desc(product_media::Column::Position)
            .all(read)
            .await?
        {
            images.insert(row.product_id, row.media_id);
        }
        let prefix = &self.0.config.product_path_prefix;
        Ok(rows
            .into_iter()
            .map(|row| {
                let own: Vec<&translation::Model> =
                    texts.iter().filter(|text| text.product_id == row.id).collect();
                // Ngôn ngữ hỏi; không hỏi thì ngôn ngữ mặc định của sản phẩm.
                let wanted = locale.unwrap_or(&row.default_locale);
                let text = own.iter().find(|text| text.locale == wanted).copied();
                let resource = match text.filter(|_| row.status == "active") {
                    Some(text) => Resource::new(text.name.clone())
                        .public(true)
                        .summary(text.short_description.clone())
                        .url(Some(format!("/{prefix}/{}", text.slug)))
                        .locale(Some(text.locale.clone()))
                        .image(images.get(&row.id).copied()),
                    None => Resource::new(String::new()),
                };
                (row.id.to_string(), resource)
            })
            .collect())
    }
}

/// Bộ tra `catalog.category`.
pub(crate) struct CategoryResolver(pub(crate) Arc<CatalogService>);

impl CategoryResolver {
    fn locale_or_default(&self, locale: Option<&str>) -> String {
        locale
            .and_then(|locale| self.0.config.locale(locale))
            .or_else(|| self.0.config.locales.first().cloned())
            .unwrap_or_else(|| "en".to_owned())
    }
}

#[async_trait::async_trait]
impl ResourceResolver for CategoryResolver {
    async fn resolve(&self, tenant: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        let mut found = self.resolve_many(tenant, &[id.to_owned()], None).await?;
        Ok(found.remove(id))
    }

    async fn resolve_many(
        &self,
        tenant: &TenantContext,
        ids: &[String],
        locale: Option<&str>,
    ) -> Result<HashMap<String, Resource>, Error> {
        let wanted = ids_of(ids);
        if wanted.is_empty() {
            return Ok(HashMap::new());
        }
        let locale = self.locale_or_default(locale);
        // Một lần đọc cả cây hiện được (danh mục là cấu hình nhỏ của khách).
        let shown = self.0.public_categories(tenant, &locale).await?;
        let mut found = HashMap::new();
        for id in wanted {
            let resource = match shown.iter().find(|row| row.category.id == id) {
                Some(row) => Resource::new(row.text.name.clone())
                    .public(true)
                    .url(Some(format!("/{CATEGORY_PATH_PREFIX}/{}", row.text.slug)))
                    .locale(Some(row.text.locale.clone())),
                None => {
                    // Có mà không hiện được thì vẫn trả (không công khai), không có thì vắng.
                    let exists = crate::entity::category::Entity::find_by_id(id)
                        .scoped(tenant)
                        .one(tenant.database().read())
                        .await?
                        .is_some();
                    if !exists {
                        continue;
                    }
                    Resource::new(String::new())
                }
            };
            found.insert(id.to_string(), resource);
        }
        Ok(found)
    }

    fn has_children(&self) -> bool {
        true
    }

    async fn children(
        &self,
        tenant: &TenantContext,
        id: &str,
        locale: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ResourceRef>, Error> {
        let Ok(parent) = id.parse::<Uuid>() else { return Ok(Vec::new()) };
        let locale = self.locale_or_default(locale);
        // `public_categories` đã sắp theo độ sâu rồi thứ tự hiển thị, và bỏ nhánh bị ẩn.
        let shown = self.0.public_categories(tenant, &locale).await?;
        if !shown.iter().any(|row| row.category.id == parent) {
            return Ok(Vec::new());
        }
        Ok(shown
            .iter()
            .filter(|row| row.category.parent_id == Some(parent))
            .take(limit)
            .map(|row| ResourceRef::new(CATEGORY, row.category.id.to_string()))
            .collect())
    }
}

/// Nhãn `seller`: tác giả quản lý được sản phẩm (có `catalog.products.write` ở nút sở hữu).
pub(crate) struct SellerBadge;

#[async_trait::async_trait]
impl BadgeResolver for SellerBadge {
    async fn badges(
        &self,
        tenant: &TenantContext,
        id: &str,
        author: &Principal,
    ) -> Result<Vec<&'static str>, Error> {
        let Ok(id) = id.parse::<Uuid>() else { return Ok(Vec::new()) };
        let read = tenant.database().read();
        let Some(row) = product::Entity::find_by_id(id).scoped(tenant).one(read).await? else {
            return Ok(Vec::new());
        };
        let manages = author.require_at(read, &PRODUCTS_WRITE, at(row.org_id)).await.is_ok();
        Ok(if manages { vec![SELLER] } else { Vec::new() })
    }
}
