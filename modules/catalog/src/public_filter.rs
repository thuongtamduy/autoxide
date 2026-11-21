//! Điều kiện lọc của đọc công khai (`GET /public/products` và số đếm facet), dùng chung để danh
//! sách và số đếm luôn khớp nhau. Mọi bộ lọc thành subquery có index trên
//! `catalog_storefront_entries`, không đổ danh sách ID ra bộ nhớ.

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::search;
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, Site, TenantContext};
use autoxide_kernel::time::{Date, Timestamp};
use sea_orm::sea_query::{IntoCondition, Query};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter};

use crate::attributes::scale_decimal;
use crate::entity::{
    attribute, attribute_option, category_path, product_attribute, product_category,
    storefront_entry, translation,
};
use crate::public::{PublicFilter, SoldBy};
use crate::service::CatalogService;

/// Bộ lọc bỏ qua khi đếm facet (đếm theo giá trị của một bộ lọc thì không lọc theo chính nó).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Skip<'a> {
    pub attribute: Option<&'a str>,
    pub seller: bool,
}

impl CatalogService {
    /// Kiểm bộ lọc thuộc tính: chỉ thuộc tính `filterable`; giá trị theo kiểu của thuộc tính.
    /// Trả điều kiện trên `storefront_entry::Column::ProductId`.
    pub(crate) async fn attribute_conditions(
        &self,
        tenant: &TenantContext,
        filters: &[(String, Vec<String>)],
        problems: &mut Vec<FieldError>,
    ) -> Result<Vec<Condition>, Error> {
        let read = tenant.database().read();
        let mut out = Vec::new();
        for (code, values) in filters {
            let field = format!("attr.{code}");
            let Some(found) = attribute::Entity::find()
                .filter(attribute::Column::Code.eq(code.as_str()))
                .filter(attribute::Column::Filterable.eq(true))
                .scoped(tenant)
                .one(read)
                .await?
            else {
                problems.push(FieldError::new(
                    field,
                    "unknown",
                    "is not a filterable attribute of this catalog",
                ));
                continue;
            };
            let options = attribute_option::Entity::find()
                .filter(attribute_option::Column::AttributeId.eq(found.id))
                .scoped(tenant)
                .all(read)
                .await?;
            let mut any = Condition::any();
            for raw in values {
                let raw = raw.trim();
                let value = match found.data_type.as_str() {
                    "text" => Some(product_attribute::Column::ValueText.eq(raw).into_condition()),
                    "integer" => raw.parse::<i64>().ok().map(|number| {
                        product_attribute::Column::ValueInteger.eq(number).into_condition()
                    }),
                    "decimal" => scale_decimal(raw, found.scale).map(|number| {
                        product_attribute::Column::ValueInteger.eq(number).into_condition()
                    }),
                    "boolean" => match raw {
                        "true" => {
                            Some(product_attribute::Column::ValueBoolean.eq(true).into_condition())
                        }
                        "false" => {
                            Some(product_attribute::Column::ValueBoolean.eq(false).into_condition())
                        }
                        _ => None,
                    },
                    "date" => Date::parse_from_str(raw, "%Y-%m-%d")
                        .ok()
                        .map(|date| product_attribute::Column::ValueDate.eq(date).into_condition()),
                    _ => options.iter().find(|option| option.code == raw).map(|option| {
                        product_attribute::Column::OptionId.eq(option.id).into_condition()
                    }),
                };
                match value {
                    Some(condition) => any = any.add(condition),
                    None => problems.push(FieldError::new(
                        field.clone(),
                        "type",
                        "is not a valid value of the attribute",
                    )),
                }
            }
            let matching = Query::select()
                .column(product_attribute::Column::ProductId)
                .from(product_attribute::Entity)
                .and_where(product_attribute::Column::TenantId.eq(tenant.id().as_uuid()))
                .and_where(product_attribute::Column::AttributeId.eq(found.id))
                .cond_where(any)
                .to_owned();
            out.push(
                Condition::all().add(storefront_entry::Column::ProductId.in_subquery(matching)),
            );
        }
        Ok(out)
    }

    /// Điều kiện trên `catalog_storefront_entries` cho bộ lọc `filter` ở website `site`; lỗi bộ
    /// lọc gom một lần (422).
    pub(crate) async fn filter_condition(
        &self,
        tenant: &TenantContext,
        site: &Site,
        filter: &PublicFilter,
        skip: Skip<'_>,
    ) -> Result<Condition, Error> {
        let tenant_id = tenant.id().as_uuid();
        let mut problems = Vec::new();
        let mut condition = self.visible_on(tenant, site).await?;
        // Chỉ sản phẩm có bản dịch bằng ngôn ngữ hỏi (không lấy ngôn ngữ khác thế vào, như
        // `content`); `q` tìm trong bản dịch đó.
        let mut texts = Query::select()
            .column(translation::Column::ProductId)
            .from(translation::Entity)
            .and_where(translation::Column::TenantId.eq(tenant_id))
            .and_where(translation::Column::Locale.eq(filter.locale.as_str()))
            .to_owned();
        if let Some(q) = filter.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
            // Câu tìm và các cách nói khác theo từ đồng nghĩa của khách (D-201).
            let mut any = Condition::any();
            for variant in self.expand_query(tenant, q).await? {
                any = any.add(search::contains(translation::Column::SearchKey, &variant));
            }
            texts.cond_where(any);
        }
        condition = condition.add(storefront_entry::Column::ProductId.in_subquery(texts));
        // Facet của một thuộc tính đếm trên tập lọc bởi mọi bộ lọc TRỪ chính thuộc tính đó.
        let attributes: Vec<(String, Vec<String>)> = filter
            .attributes
            .iter()
            .filter(|(code, _)| skip.attribute != Some(code.as_str()))
            .cloned()
            .collect();
        for attribute in self.attribute_conditions(tenant, &attributes, &mut problems).await? {
            condition = condition.add(attribute);
        }
        if let Some(slug) = &filter.category {
            match self.public_category(tenant, &filter.locale, slug).await {
                Ok(found) => {
                    // Danh mục và mọi danh mục con của nó.
                    let below = Query::select()
                        .column(category_path::Column::DescendantId)
                        .from(category_path::Entity)
                        .and_where(category_path::Column::TenantId.eq(tenant_id))
                        .and_where(category_path::Column::AncestorId.eq(found.category.id))
                        .to_owned();
                    let products = Query::select()
                        .column(product_category::Column::ProductId)
                        .from(product_category::Entity)
                        .and_where(product_category::Column::TenantId.eq(tenant_id))
                        .and_where(product_category::Column::CategoryId.in_subquery(below))
                        .to_owned();
                    condition =
                        condition.add(storefront_entry::Column::ProductId.in_subquery(products));
                }
                Err(_) => problems.push(FieldError::new(
                    "category",
                    "unknown",
                    "is not the slug of a visible category in this language",
                )),
            }
        }
        if !filter.category_ids.is_empty() {
            let below = Query::select()
                .column(category_path::Column::DescendantId)
                .from(category_path::Entity)
                .and_where(category_path::Column::TenantId.eq(tenant_id))
                .and_where(category_path::Column::AncestorId.is_in(filter.category_ids.clone()))
                .to_owned();
            let products = Query::select()
                .column(product_category::Column::ProductId)
                .from(product_category::Entity)
                .and_where(product_category::Column::TenantId.eq(tenant_id))
                .and_where(product_category::Column::CategoryId.in_subquery(below))
                .to_owned();
            condition = condition.add(storefront_entry::Column::ProductId.in_subquery(products));
        }
        if let Some(after) = filter.published_after {
            condition = condition.add(storefront_entry::Column::PublishedAt.gte(after));
        }
        if let Some(collection) = filter.collection {
            let chosen = Query::select()
                .column(crate::entity::collection_item::Column::ProductId)
                .from(crate::entity::collection_item::Entity)
                .and_where(crate::entity::collection_item::Column::TenantId.eq(tenant_id))
                .and_where(crate::entity::collection_item::Column::CollectionId.eq(collection))
                .to_owned();
            condition = condition.add(storefront_entry::Column::ProductId.in_subquery(chosen));
        }
        if let Some(min) = filter.price_min {
            condition = condition.add(storefront_entry::Column::MinPriceMinor.gte(min));
        }
        if let Some(max) = filter.price_max {
            condition = condition.add(storefront_entry::Column::MinPriceMinor.lte(max));
        }
        if filter.on_sale {
            condition = condition.add(storefront_entry::Column::SalePriceMinor.is_not_null()).add(
                Condition::any()
                    .add(storefront_entry::Column::SaleEndsAt.is_null())
                    .add(storefront_entry::Column::SaleEndsAt.gt(Timestamp::now())),
            );
        }
        if let Some(seller) = filter.seller.filter(|_| !skip.seller) {
            condition = condition.add(OrgTree::subtree_condition(
                tenant.id(),
                &[OrgId::from_uuid(seller)],
                storefront_entry::Column::OwnerOrgId,
            ));
        }
        match filter.sold_by.filter(|_| !skip.seller) {
            Some(SoldBy::Own) => {
                condition =
                    condition.add(storefront_entry::Column::OwnerOrgId.is_in(site.lineage_ids()));
            }
            Some(SoldBy::Marketplace) => {
                condition = condition
                    .add(storefront_entry::Column::OwnerOrgId.is_not_in(site.lineage_ids()));
            }
            None => {}
        }
        if let Some(rating) = filter.min_rating {
            condition = condition.add(storefront_entry::Column::RatingX100.gte(rating));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        Ok(condition)
    }
}
