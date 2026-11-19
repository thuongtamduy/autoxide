//! Số đếm theo từng giá trị lọc (facet, D-201) cho website: "Màu: Đỏ (12), Xanh (5)", "Nơi bán:
//! Cửa hàng A (8)". Số đếm của một bộ lọc tính trên tập đã lọc bởi mọi bộ lọc khác (không lọc theo
//! chính nó), để chọn thêm một giá trị không làm các giá trị còn lại về 0. Mỗi câu đếm là một
//! `GROUP BY` trên index bắt đầu bằng `tenant_id`, chạy như nhau trên SQLite và Postgres.

use std::collections::HashMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt, Site, TenantContext};
use sea_orm::sea_query::{Alias, Expr, Query};
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use uuid::Uuid;

use crate::entity::{attribute, attribute_option, product_attribute, storefront_entry};
use crate::public::{PublicFilter, pick};
use crate::public_filter::Skip;
use crate::service::CatalogService;

/// Số giá trị tối đa trả cho mỗi bộ lọc (nhiều nhất trước).
pub(crate) const MAX_VALUES: u64 = 50;
/// Số thuộc tính tối đa có số đếm (thứ tự mã thuộc tính).
const MAX_ATTRIBUTES: u64 = 20;

/// Một giá trị lọc và số sản phẩm khớp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FacetValue {
    /// Giá trị gửi lại trong bộ lọc: mã lựa chọn, `true` / `false`, ID nút người bán.
    pub value: String,
    pub label: String,
    pub count: u64,
}

/// Số đếm của một thuộc tính lọc được.
#[derive(Debug, Clone)]
pub(crate) struct AttributeFacet {
    pub code: String,
    pub name: String,
    pub values: Vec<FacetValue>,
}

/// Mọi số đếm của một tập lọc.
#[derive(Debug, Clone)]
pub(crate) struct Facets {
    /// Số sản phẩm khớp mọi bộ lọc.
    pub total: u64,
    pub attributes: Vec<AttributeFacet>,
    pub sellers: Vec<FacetValue>,
}

impl CatalogService {
    /// Số đếm theo giá trị lọc cho `filter` ở website `site`. Thuộc tính có số đếm: thuộc tính
    /// lọc được kiểu lựa chọn (`option`, `options`) và đúng / sai (`boolean`).
    pub(crate) async fn public_facets(
        &self,
        tenant: &TenantContext,
        site: &Site,
        filter: &PublicFilter,
    ) -> Result<Facets, Error> {
        let read = tenant.database().read();
        // Kiểm mọi bộ lọc một lần (lỗi 422 gom một lần) và đếm tổng.
        let all = self.filter_condition(tenant, site, filter, Skip::default()).await?;
        let total = storefront_entry::Entity::find().filter(all).scoped(tenant).count(read).await?;
        let attributes = attribute::Entity::find()
            .filter(attribute::Column::Filterable.eq(true))
            .filter(attribute::Column::DataType.is_in(["option", "options", "boolean"]))
            .scoped(tenant)
            .order_by_asc(attribute::Column::Code)
            .limit(MAX_ATTRIBUTES)
            .all(read)
            .await?;
        let mut out = Vec::new();
        for found in attributes {
            let skip = Skip { attribute: Some(found.code.as_str()), seller: false };
            let condition = self.filter_condition(tenant, site, filter, skip).await?;
            let products = in_storefront(tenant, condition);
            let counted = |column: product_attribute::Column| {
                product_attribute::Entity::find()
                    .select_only()
                    .column(column)
                    .column_as(product_attribute::Column::Id.count(), "n")
                    .filter(product_attribute::Column::AttributeId.eq(found.id))
                    .filter(product_attribute::Column::ProductId.in_subquery(products.clone()))
                    .scoped(tenant)
                    .group_by(column)
                    .order_by(Expr::col(Alias::new("n")), Order::Desc)
                    .limit(MAX_VALUES)
            };
            let values = if found.data_type == "boolean" {
                let rows: Vec<(Option<bool>, i64)> =
                    counted(product_attribute::Column::ValueBoolean).into_tuple().all(read).await?;
                rows.into_iter()
                    .filter_map(|(value, count)| {
                        let value = value?.to_string();
                        Some(FacetValue { label: value.clone(), value, count: count_of(count) })
                    })
                    .collect()
            } else {
                let rows: Vec<(Option<Uuid>, i64)> =
                    counted(product_attribute::Column::OptionId).into_tuple().all(read).await?;
                let options: HashMap<Uuid, attribute_option::Model> =
                    attribute_option::Entity::find()
                        .filter(attribute_option::Column::AttributeId.eq(found.id))
                        .scoped(tenant)
                        .all(read)
                        .await?
                        .into_iter()
                        .map(|option| (option.id, option))
                        .collect();
                rows.into_iter()
                    .filter_map(|(id, count)| {
                        let option = options.get(&id?)?;
                        Some(FacetValue {
                            value: option.code.clone(),
                            label: pick(&option.labels, &filter.locale),
                            count: count_of(count),
                        })
                    })
                    .collect()
            };
            out.push(AttributeFacet {
                name: pick(&found.names, &filter.locale),
                code: found.code,
                values,
            });
        }
        let skip = Skip { attribute: None, seller: true };
        let condition = self.filter_condition(tenant, site, filter, skip).await?;
        let rows: Vec<(Uuid, i64)> = storefront_entry::Entity::find()
            .select_only()
            .column(storefront_entry::Column::OwnerOrgId)
            .column_as(storefront_entry::Column::Id.count(), "n")
            .filter(condition)
            .scoped(tenant)
            .group_by(storefront_entry::Column::OwnerOrgId)
            .order_by(Expr::col(Alias::new("n")), Order::Desc)
            .limit(MAX_VALUES)
            .into_tuple()
            .all(read)
            .await?;
        let owners: Vec<Uuid> = rows.iter().map(|(owner, _)| *owner).collect();
        let names = self.seller_names(tenant, &owners).await?;
        let sellers = rows
            .into_iter()
            .map(|(owner, count)| FacetValue {
                value: owner.to_string(),
                label: names.get(&owner).cloned().unwrap_or_default(),
                count: count_of(count),
            })
            .collect();
        Ok(Facets { total, attributes: out, sellers })
    }
}

/// Subquery "ID sản phẩm của các dòng storefront khớp `condition`".
fn in_storefront(
    tenant: &TenantContext,
    condition: Condition,
) -> sea_orm::sea_query::SelectStatement {
    Query::select()
        .column(storefront_entry::Column::ProductId)
        .from(storefront_entry::Entity)
        .and_where(storefront_entry::Column::TenantId.eq(tenant.id().as_uuid()))
        .cond_where(condition)
        .to_owned()
}

fn count_of(count: i64) -> u64 {
    u64::try_from(count).unwrap_or(0)
}
