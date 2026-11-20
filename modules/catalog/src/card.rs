//! Phần thêm của thẻ sản phẩm công khai (D-201): tóm tắt biến thể ("3 cỡ, 2 màu"), thương hiệu,
//! giá ngay trước giá giảm tạm thời (giá gạch của JSON-LD). Đọc theo lô cho cả trang, số câu query
//! cố định.

use std::collections::HashMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use uuid::Uuid;

use crate::api::{PriceQuery, PriceSnapshot};
use crate::entity::{attribute, attribute_option, option, option_value, product_attribute};
use crate::public::pick;
use crate::service::CatalogService;
use crate::variants::OptionDetail;

/// Mã thuộc tính mang thương hiệu (thuộc tính `text` hay `option` khách tự khai).
pub(crate) const BRAND: &str = "brand";

/// Một tuỳ chọn của sản phẩm và số giá trị của nó: "Cỡ" × 3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OptionCount {
    pub name: String,
    pub count: usize,
}

/// Số giá trị của từng tuỳ chọn, từ tuỳ chọn đã đọc (trang sản phẩm đã có, không đọc lại).
pub(crate) fn option_counts_of(options: &[OptionDetail], locale: &str) -> Vec<OptionCount> {
    options
        .iter()
        .map(|detail| OptionCount {
            name: pick(&detail.option.names, locale),
            count: detail.values.len(),
        })
        .collect()
}

impl CatalogService {
    /// Tóm tắt tuỳ chọn của các sản phẩm `ids`, tên theo `locale`, theo thứ tự tuỳ chọn.
    pub(crate) async fn option_counts(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
        locale: &str,
    ) -> Result<HashMap<Uuid, Vec<OptionCount>>, Error> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let read = tenant.database().read();
        let options = option::Entity::find()
            .filter(option::Column::ProductId.is_in(ids.iter().copied()))
            .scoped(tenant)
            .order_by_asc(option::Column::Position)
            .all(read)
            .await?;
        let values = option_value::Entity::find()
            .filter(option_value::Column::ProductId.is_in(ids.iter().copied()))
            .scoped(tenant)
            .all(read)
            .await?;
        let mut out: HashMap<Uuid, Vec<OptionCount>> = HashMap::new();
        for row in options {
            let count = values.iter().filter(|value| value.option_id == row.id).count();
            out.entry(row.product_id)
                .or_default()
                .push(OptionCount { name: pick(&row.names, locale), count });
        }
        Ok(out)
    }

    /// Thương hiệu của các sản phẩm `ids` (thuộc tính mã [`BRAND`]), nhãn theo `locale`.
    pub(crate) async fn brands(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
        locale: &str,
    ) -> Result<HashMap<Uuid, String>, Error> {
        let read = tenant.database().read();
        let Some(brand) = attribute::Entity::find()
            .filter(attribute::Column::Code.eq(BRAND))
            .scoped(tenant)
            .one(read)
            .await?
        else {
            return Ok(HashMap::new());
        };
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = product_attribute::Entity::find()
            .filter(product_attribute::Column::AttributeId.eq(brand.id))
            .filter(product_attribute::Column::ProductId.is_in(ids.iter().copied()))
            .scoped(tenant)
            .all(read)
            .await?;
        let options: HashMap<Uuid, attribute_option::Model> = attribute_option::Entity::find()
            .filter(attribute_option::Column::AttributeId.eq(brand.id))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .map(|row| (row.id, row))
            .collect();
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let label = match row.option_id {
                    Some(id) => pick(&options.get(&id)?.labels, locale),
                    None => row.value_text?,
                };
                Some((row.product_id, label))
            })
            .collect())
    }

    /// Giá ngay trước giá giảm tạm thời (dòng giá có `valid_to`) của từng biến thể, cho giá gạch:
    /// giá thắng ngay trước `valid_from` của dòng đó. Biến thể không đang giảm tạm thời, hay giá
    /// trước đó không cao hơn, thì vắng.
    pub(crate) async fn regular_prices(
        &self,
        tenant: &TenantContext,
        site: autoxide_kernel::tenancy::OrgId,
        current: &HashMap<Uuid, PriceSnapshot>,
    ) -> Result<HashMap<Uuid, PriceSnapshot>, Error> {
        let mut out = HashMap::new();
        for price in current.values().filter(|price| price.valid_to.is_some()) {
            let query = PriceQuery::new(
                site,
                price.amount.currency(),
                price.valid_from,
                vec![price.variant_id],
            );
            if let Some(before) = self.resolve_prices(tenant, &query, true).await?.pop()
                && before.amount.minor() > price.amount.minor()
            {
                out.insert(price.variant_id, before);
            }
        }
        Ok(out)
    }
}
