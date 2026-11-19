//! Tình trạng hàng công khai theo cửa hàng (bước 42 mục 8, D-200).
//!
//! Mỗi biến thể × cửa hàng một mức: `in_stock`, `low` (kèm `quantity` khi tồn bán được dưới
//! `AUTOXIDE_INVENTORY_PUBLIC_LOW_THRESHOLD`), `out`, `preorder` (D-204). Không bao giờ trả số tồn từ ngưỡng
//! trở lên: người lạ không đọc được quy mô kho của người bán.
//!
//! "Tồn bán được ở cửa hàng" tính như khi giữ chỗ: các kho phục vụ cửa hàng (`inventory_sources`, hay
//! chính cửa hàng); cửa hàng cho chia kho thì cộng các kho, không thì lấy kho nhiều nhất (một đơn chỉ
//! lấy từ một kho). Combo: số bộ ráp được từ thành phần. Biến thể không theo dõi tồn luôn `in_stock`.
//! Biến thể không có, chưa bán hay đã ẩn bị bỏ qua (không báo lỗi để khỏi dò được ID).

use std::collections::{BTreeMap, HashMap};

use autoxide_catalog::api::{ProductKind, ProductStatus, SaleMode, VariantSnapshot};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::MAX_IDS;
use crate::entity::{level, settings};
use crate::service::InventoryService;

/// Số biến thể tối đa mỗi lần hỏi.
pub(crate) const MAX_PUBLIC_VARIANTS: usize = 100;
/// Số cửa hàng tối đa khi bỏ `store_id` (mọi cửa hàng nhận hàng tại chỗ).
const MAX_STORES: u64 = 200;

/// How much of a variant a store can sell, without revealing large stock numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = InventoryPublicStockLevel)]
pub(crate) enum StockLevel {
    /// Enough to sell; the number is not shown.
    InStock,
    /// Only a few left; `quantity` says how many.
    Low,
    /// None to sell now.
    Out,
    /// Not in stock yet but can be pre-ordered.
    Preorder,
}

/// Availability of one variant at one store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = InventoryPublicAvailability)]
pub(crate) struct PublicAvailability {
    pub variant_id: Uuid,
    /// The store (organization unit) selling it.
    pub store_id: Uuid,
    pub level: StockLevel,
    /// Units left, only when `level` is `low`.
    pub quantity: Option<i64>,
}

pub(crate) fn visible(variant: &VariantSnapshot) -> bool {
    variant.status == ProductStatus::Active && variant.variant_active
}

fn grade(
    variant: &VariantSnapshot,
    sellable: Option<i64>,
    threshold: i64,
) -> (StockLevel, Option<i64>) {
    let Some(sellable) = sellable else {
        return (StockLevel::InStock, None);
    };
    if sellable <= 0 {
        if variant.sale_mode == SaleMode::Preorder {
            return (StockLevel::Preorder, None);
        }
        return (StockLevel::Out, None);
    }
    if sellable < threshold {
        return (StockLevel::Low, Some(sellable));
    }
    (StockLevel::InStock, None)
}

impl InventoryService {
    /// Mức tồn công khai của `variant_ids` ở `store` (hay mọi cửa hàng nhận hàng tại chỗ). Chỉ đọc.
    pub(crate) async fn public_availability(
        &self,
        tenant: &TenantContext,
        variant_ids: &[Uuid],
        store: Option<Uuid>,
    ) -> Result<Vec<PublicAvailability>, Error> {
        if variant_ids.is_empty() || variant_ids.len() > MAX_PUBLIC_VARIANTS {
            return Err(Error::validation(vec![
                FieldError::new("variant_ids", "length", "give between 1 and {max} variant ids")
                    .with_param("max", MAX_PUBLIC_VARIANTS),
            ]));
        }
        let read = tenant.database().read();
        let stores: Vec<Uuid> = match store {
            Some(store) => vec![store],
            None => {
                settings::Entity::find()
                    .select_only()
                    .column(settings::Column::OrgId)
                    .filter(settings::Column::Pickup.eq(true))
                    .scoped(tenant)
                    .limit(MAX_STORES)
                    .into_tuple()
                    .all(read)
                    .await?
            }
        };
        self.levels_at(tenant, variant_ids, &stores).await
    }

    /// Mức tồn của `variant_ids` (tối đa `MAX_IDS`) ở từng cửa hàng `stores`; không kiểm số lượng.
    pub(crate) async fn levels_at(
        &self,
        tenant: &TenantContext,
        variant_ids: &[Uuid],
        stores: &[Uuid],
    ) -> Result<Vec<PublicAvailability>, Error> {
        let read = tenant.database().read();
        let mut ids = variant_ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        let variants: HashMap<Uuid, VariantSnapshot> = self
            .catalog
            .variants(tenant, &ids, None)
            .await?
            .into_iter()
            .filter(visible)
            .map(|variant| (variant.variant_id, variant))
            .collect();
        let bundles: Vec<Uuid> = variants
            .values()
            .filter(|variant| variant.kind == ProductKind::Bundle)
            .map(|variant| variant.variant_id)
            .collect();
        let components = if bundles.is_empty() {
            HashMap::new()
        } else {
            self.catalog.bundle_components(tenant, &bundles).await?
        };
        let part_ids: Vec<Uuid> =
            components.values().flatten().map(|part| part.variant_id).collect();
        let mut parts: HashMap<Uuid, VariantSnapshot> = HashMap::new();
        for chunk in part_ids.chunks(MAX_IDS) {
            for part in self.catalog.variants(tenant, chunk, None).await? {
                parts.insert(part.variant_id, part);
            }
        }
        // Biến thể cần đọc tồn: biến thể thường theo dõi tồn và thành phần theo dõi tồn của combo.
        let mut tracked: Vec<Uuid> = variants
            .values()
            .filter(|variant| variant.kind != ProductKind::Bundle && variant.track_inventory)
            .map(|variant| variant.variant_id)
            .chain(parts.values().filter(|part| part.track_inventory).map(|part| part.variant_id))
            .collect();
        tracked.sort_unstable();
        tracked.dedup();

        let mut answer = Vec::new();
        for &store in stores {
            // Cửa hàng không bán được (không phải kho, không có nguồn) → mọi biến thể theo dõi tồn là `out`.
            let (warehouses, split) =
                self.candidates(tenant, store, None, None).await.unwrap_or_default();
            let mut per_warehouse: BTreeMap<(Uuid, Uuid), i64> = BTreeMap::new();
            if !warehouses.is_empty() && !tracked.is_empty() {
                let rows: Vec<(Uuid, Uuid, i64)> = level::Entity::find()
                    .select_only()
                    .column(level::Column::OrgId)
                    .column(level::Column::VariantId)
                    .column(level::Column::Available)
                    .filter(level::Column::OrgId.is_in(warehouses.clone()))
                    .filter(level::Column::VariantId.is_in(tracked.clone()))
                    .scoped(tenant)
                    .into_tuple()
                    .all(read)
                    .await?;
                for (org, variant, available) in rows {
                    per_warehouse.insert((org, variant), available.max(0));
                }
            }
            // Số bán được của biến thể theo dõi tồn (thường hay combo) ở một kho.
            let at = |warehouse: Uuid, variant: &VariantSnapshot| -> i64 {
                if variant.kind != ProductKind::Bundle {
                    return per_warehouse
                        .get(&(warehouse, variant.variant_id))
                        .copied()
                        .unwrap_or(0);
                }
                let mut sets: Option<i64> = None;
                for part in components.get(&variant.variant_id).into_iter().flatten() {
                    let tracked_part =
                        parts.get(&part.variant_id).is_some_and(|found| found.track_inventory);
                    if !tracked_part || part.quantity <= 0 {
                        continue;
                    }
                    let have =
                        per_warehouse.get(&(warehouse, part.variant_id)).copied().unwrap_or(0);
                    let can = have / part.quantity;
                    sets = Some(sets.map_or(can, |sets| sets.min(can)));
                }
                sets.unwrap_or(i64::MAX)
            };
            for id in variant_ids {
                let Some(variant) = variants.get(id) else { continue };
                if answer
                    .iter()
                    .any(|row: &PublicAvailability| row.variant_id == *id && row.store_id == store)
                {
                    continue;
                }
                let tracks = if variant.kind == ProductKind::Bundle {
                    components.get(id).into_iter().flatten().any(|part| {
                        parts.get(&part.variant_id).is_some_and(|found| found.track_inventory)
                    })
                } else {
                    variant.track_inventory
                };
                let sellable = tracks.then(|| {
                    let each = warehouses.iter().map(|warehouse| at(*warehouse, variant));
                    if split {
                        each.fold(0i64, i64::saturating_add)
                    } else {
                        each.max().unwrap_or(0)
                    }
                });
                let (level, quantity) = grade(variant, sellable, self.config.public_low_threshold);
                answer.push(PublicAvailability {
                    variant_id: *id,
                    store_id: store,
                    level,
                    quantity,
                });
            }
        }
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::{StockLevel, grade};
    use crate::tests::physical;
    use autoxide_catalog::api::SaleMode;
    use uuid::Uuid;

    #[test]
    fn muc_cong_khai_khong_lo_so_lon() {
        let mut variant = physical(Uuid::now_v7());
        assert_eq!(grade(&variant, Some(500), 5), (StockLevel::InStock, None));
        assert_eq!(grade(&variant, Some(5), 5), (StockLevel::InStock, None));
        assert_eq!(grade(&variant, Some(4), 5), (StockLevel::Low, Some(4)));
        assert_eq!(grade(&variant, Some(0), 5), (StockLevel::Out, None));
        assert_eq!(grade(&variant, None, 5), (StockLevel::InStock, None));
        variant.sale_mode = SaleMode::Preorder;
        assert_eq!(grade(&variant, Some(-3), 5), (StockLevel::Preorder, None));
        assert_eq!(grade(&variant, Some(4), 5), (StockLevel::Low, Some(4)));
    }
}
