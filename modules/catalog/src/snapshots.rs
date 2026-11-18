//! `impl Catalog` cho module khác: ảnh chụp biến thể, giá, sàn giá, thành phần combo. Mỗi hàm là
//! vài câu query cố định, không phụ thuộc số ID (≤ [`MAX_IDS`]).

use std::collections::HashMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::money::Currency;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::api::{
    BundleComponent, Catalog, MAX_IDS, PriceFloor, PriceQuery, PriceSnapshot, ProductKind,
    SaleMode, VariantSnapshot,
};
use crate::entity::{option, option_value, product, product_category, translation, variant};
use crate::errors;
use crate::products::{parse_kind, parse_status};
use crate::service::{CatalogService, pick_text, texts_of};
use crate::variants::{parse_sale_mode, sku_key};

impl CatalogService {
    /// Ảnh chụp các biến thể `rows`, tên theo `locale`.
    async fn snapshots(
        &self,
        tenant: &TenantContext,
        rows: Vec<variant::Model>,
        locale: Option<&str>,
    ) -> Result<Vec<VariantSnapshot>, Error> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let read = tenant.database().read();
        let product_ids: Vec<Uuid> = rows.iter().map(|row| row.product_id).collect();
        let products: HashMap<Uuid, product::Model> = product::Entity::find()
            .filter(product::Column::Id.is_in(product_ids.clone()))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .map(|row| (row.id, row))
            .collect();
        let texts = translation::Entity::find()
            .filter(translation::Column::ProductId.is_in(product_ids.clone()))
            .scoped(tenant)
            .all(read)
            .await?;
        let values: HashMap<Uuid, option_value::Model> = option_value::Entity::find()
            .filter(option_value::Column::ProductId.is_in(product_ids.clone()))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .map(|row| (row.id, row))
            .collect();
        let option_positions: HashMap<Uuid, i32> = option::Entity::find()
            .filter(option::Column::ProductId.is_in(product_ids.clone()))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .map(|row| (row.id, row.position))
            .collect();
        let links = product_category::Entity::find()
            .filter(product_category::Column::ProductId.is_in(product_ids))
            .scoped(tenant)
            .all(read)
            .await?;
        let own: Vec<Uuid> = links.iter().map(|link| link.category_id).collect();
        let paths = self.with_ancestors(read, tenant, &own).await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let Some(product) = products.get(&row.product_id) else { continue };
            let text = locale
                .and_then(|locale| {
                    texts.iter().find(|text| text.product_id == product.id && text.locale == locale)
                })
                .or_else(|| {
                    texts.iter().find(|text| {
                        text.product_id == product.id && text.locale == product.default_locale
                    })
                })
                .or_else(|| texts.iter().find(|text| text.product_id == product.id));
            let actual_locale =
                text.map_or_else(|| product.default_locale.clone(), |text| text.locale.clone());
            // Nhãn biến thể theo thứ tự tuỳ chọn (màu trước cỡ…), theo ngôn ngữ xin trước.
            let mut labels: Vec<(i32, String)> = row
                .option_key
                .split(',')
                .filter_map(|id| Uuid::parse_str(id).ok())
                .filter_map(|id| values.get(&id))
                .map(|value| {
                    (
                        option_positions.get(&value.option_id).copied().unwrap_or(i32::MAX),
                        pick_text(
                            &texts_of(&value.labels),
                            locale.or(Some(&actual_locale)),
                            &product.default_locale,
                        ),
                    )
                })
                .collect();
            labels.sort_by_key(|(position, _)| *position);
            let mut category_ids: Vec<Uuid> = Vec::new();
            for link in links.iter().filter(|link| link.product_id == product.id) {
                for (descendant, ancestor, _) in &paths {
                    if *descendant == link.category_id && !category_ids.contains(ancestor) {
                        category_ids.push(*ancestor);
                    }
                }
            }
            let kind = parse_kind(&product.kind);
            let sale_mode = parse_sale_mode(&row.sale_mode);
            out.push(VariantSnapshot {
                variant_id: row.id,
                product_id: product.id,
                org_id: OrgId::from_uuid(product.org_id),
                kind,
                status: parse_status(&product.status),
                variant_active: row.active,
                sku: row.sku,
                barcode: row.barcode,
                name: text.map(|text| text.name.clone()).unwrap_or_default(),
                variant_label: labels
                    .into_iter()
                    .map(|(_, label)| label)
                    .collect::<Vec<_>>()
                    .join(" / "),
                locale: actual_locale,
                tax_category: product.tax_category.clone(),
                category_ids,
                weight_grams: row.weight_grams,
                length_mm: row.length_mm,
                width_mm: row.width_mm,
                height_mm: row.height_mm,
                requires_shipping: matches!(kind, ProductKind::Physical | ProductKind::Bundle),
                track_inventory: product.track_inventory,
                sale_mode,
                preorder_available_on: row.preorder_available_on,
                preorder_limit: row.preorder_limit,
                preorder_ends_at: row.preorder_ends_at,
                allow_backorder: sale_mode == SaleMode::Backorder,
                lot_tracked: product.lot_tracked,
                serial_tracked: product.serial_tracked,
                warranty_months: product.warranty_months,
                image_media_id: row.image_media_id,
                updated_at: row.updated_at,
            });
        }
        Ok(out)
    }
}

#[async_trait::async_trait]
impl Catalog for CatalogService {
    async fn variants(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
        locale: Option<&str>,
    ) -> Result<Vec<VariantSnapshot>, Error> {
        if ids.len() > MAX_IDS {
            return Err(errors::too_many_ids(MAX_IDS));
        }
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = variant::Entity::find()
            .filter(variant::Column::Id.is_in(ids.iter().copied()))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let snapshots = self.snapshots(tenant, rows, locale).await?;
        // Theo thứ tự `ids`.
        Ok(ids
            .iter()
            .filter_map(|id| snapshots.iter().find(|snapshot| snapshot.variant_id == *id).cloned())
            .collect())
    }

    async fn variant_by_code(
        &self,
        tenant: &TenantContext,
        code: &str,
        locale: Option<&str>,
    ) -> Result<Option<VariantSnapshot>, Error> {
        let code = code.trim();
        if code.is_empty() {
            return Ok(None);
        }
        let read = tenant.database().read();
        let mut row = variant::Entity::find()
            .filter(variant::Column::SkuKey.eq(sku_key(code)))
            .scoped(tenant)
            .one(read)
            .await?;
        if row.is_none() {
            row = variant::Entity::find()
                .filter(variant::Column::Barcode.eq(code))
                .scoped(tenant)
                .one(read)
                .await?;
        }
        let Some(row) = row else { return Ok(None) };
        Ok(self.snapshots(tenant, vec![row], locale).await?.into_iter().next())
    }

    async fn prices(
        &self,
        tenant: &TenantContext,
        query: &PriceQuery,
    ) -> Result<Vec<PriceSnapshot>, Error> {
        self.resolve_prices(tenant, query, false).await
    }

    async fn reference_prices(
        &self,
        tenant: &TenantContext,
        query: &PriceQuery,
    ) -> Result<Vec<PriceSnapshot>, Error> {
        self.resolve_prices(tenant, query, true).await
    }

    async fn price_floors(
        &self,
        tenant: &TenantContext,
        org_id: OrgId,
        currency: Currency,
        variant_ids: &[Uuid],
    ) -> Result<Vec<PriceFloor>, Error> {
        self.floors(tenant, org_id, currency, variant_ids).await
    }

    async fn bundle_components(
        &self,
        tenant: &TenantContext,
        variant_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<BundleComponent>>, Error> {
        if variant_ids.len() > MAX_IDS {
            return Err(errors::too_many_ids(MAX_IDS));
        }
        self.components_of(tenant, variant_ids).await
    }
}
