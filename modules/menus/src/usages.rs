//! "Đang dùng ở đâu" (bước 51, D-267): menu có mục trỏ tới một bản ghi (đích của mục, hay ô gợi ý).

use std::collections::BTreeMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::usages::{KeyedReferrer, ReferenceSource, Referrer};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Select};

use crate::entity::{item, menu};

pub(crate) struct Menus;

/// Mục menu trỏ tới `kind` / `id`: đích của mục hay ô gợi ý. Mỗi nhánh của `OR` có index (tenant_id, cột
/// loại, cột id, id) — m0009.
fn pointing(tenant: &TenantContext, kind: &str, id: &str) -> Select<item::Entity> {
    item::Entity::find()
        .filter(
            Condition::any()
                .add(
                    Condition::all()
                        .add(item::Column::RefKind.eq(kind))
                        .add(item::Column::RefId.eq(id)),
                )
                .add(
                    Condition::all()
                        .add(item::Column::FeatureKind.eq(kind))
                        .add(item::Column::FeatureId.eq(id)),
                ),
        )
        .scoped(tenant)
}

/// Mỗi mục thành một dòng "đang dùng" (menu của nó, nhãn của mục), giữ thứ tự của `items`.
async fn referrers_of(
    tenant: &TenantContext,
    id: &str,
    items: Vec<item::Model>,
) -> Result<Vec<KeyedReferrer>, Error> {
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<uuid::Uuid> = items.iter().map(|item| item.menu_id).collect();
    let menus: BTreeMap<uuid::Uuid, menu::Model> = menu::Entity::find()
        .filter(menu::Column::Id.is_in(ids))
        .scoped(tenant)
        .all(tenant.database().read())
        .await?
        .into_iter()
        .map(|menu| (menu.id, menu))
        .collect();
    Ok(items
        .into_iter()
        .filter_map(|item| {
            let menu = menus.get(&item.menu_id)?;
            let field =
                if item.ref_id.as_deref() == Some(id) { "menu_item" } else { "menu_feature" };
            Some(KeyedReferrer::new(
                item.id,
                Referrer::new(
                    "menus.menu",
                    menu.id.to_string(),
                    format!("{} ({}) · {}", menu.key, menu.locale, item.label),
                    field,
                ),
            ))
        })
        .collect())
}

#[async_trait::async_trait]
impl ReferenceSource for Menus {
    async fn referrers(
        &self,
        tenant: &TenantContext,
        kind: &str,
        id: &str,
        limit: usize,
    ) -> Result<Vec<Referrer>, Error> {
        let items = pointing(tenant, kind, id)
            .order_by_desc(item::Column::Id)
            .limit(u64::try_from(limit).unwrap_or(u64::MAX))
            .all(tenant.database().read())
            .await?;
        Ok(referrers_of(tenant, id, items).await?.into_iter().map(|row| row.referrer).collect())
    }

    // Keyset theo id của mục menu (UUIDv7) trong database, cùng con trỏ với các nguồn khác.
    async fn referrers_page(
        &self,
        tenant: &TenantContext,
        kind: &str,
        id: &str,
        page: &PageRequest,
    ) -> Result<Page<KeyedReferrer>, Error> {
        let read = tenant.database().read();
        let items =
            fetch_page(pointing(tenant, kind, id), item::Column::Id, |row| row.id, page, read)
                .await?;
        let referrers = referrers_of(tenant, id, items.data).await?;
        // Mục không còn menu thì bị bỏ — không xảy ra trong thực tế: xoá menu xoá mục của nó trong cùng
        // transaction (service.rs), nên trang vẫn là `limit` mục đầu sau con trỏ như bản gộp cần.
        Ok(Page { data: referrers, pagination: items.pagination })
    }
}
