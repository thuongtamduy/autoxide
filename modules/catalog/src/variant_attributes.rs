//! Thuộc tính theo biến thể cho trang sản phẩm (website bán hàng, 16/11/2026): thông tin khác nhau
//! giữa các biến thể (số người dùng, trong hộp có gì, cách dùng / chăm sóc, dùng được máy rửa bát…).
//! Dùng chung bộ thuộc tính của khách và cách kiểm giá trị của sản phẩm (`attributes.rs`) — thuộc
//! tính là dữ liệu khai theo loại sản phẩm, không viết cứng ngành nào. Thuộc tính kiểu chữ có thêm
//! bản dịch theo ngôn ngữ (`texts`).

use std::collections::{BTreeMap, HashMap};

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::Value;
use uuid::Uuid;

use crate::attributes::{attribute_not_found, parse_value};
use crate::entity::{product_attribute, variant_attribute};
use crate::public::PublicAttribute;
use crate::service::{CatalogService, fail_if_any};

/// Giá trị một thuộc tính của biến thể.
#[derive(Debug, Clone)]
pub(crate) struct VariantAttributeInput {
    pub attribute_id: Uuid,
    pub value: Value,
    /// Bản dịch của giá trị chữ, theo ngôn ngữ (chỉ cho thuộc tính kiểu `text`).
    pub texts: BTreeMap<String, String>,
}

impl CatalogService {
    /// Giá trị thuộc tính của biến thể `id` (đọc, kiểm quyền đọc sản phẩm).
    pub(crate) async fn get_variant_attributes(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<variant_attribute::Model>, Error> {
        let read = tenant.database().read();
        let not_found = || crate::errors::variant_not_found(id);
        let row = crate::entity::variant::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(not_found)?;
        let product = crate::entity::product::Entity::find_by_id(row.product_id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(not_found)?;
        crate::service::readable(
            principal,
            read,
            &crate::permissions::PRODUCTS_READ,
            product.org_id,
            not_found,
        )
        .await?;
        // Theo thứ tự của lần thay gần nhất (`PUT` ghi lại cả tập), keyset trong database (index
        // `catalog_variant_attributes_page_idx`).
        let select = variant_attribute::Entity::find()
            .filter(variant_attribute::Column::VariantId.eq(id))
            .scoped(tenant);
        fetch_page(select, variant_attribute::Column::Id, |row| row.id, page, read).await
    }

    /// Thay cả tập giá trị thuộc tính của biến thể `id`.
    pub(crate) async fn put_variant_attributes(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        values: Vec<VariantAttributeInput>,
    ) -> Result<Vec<variant_attribute::Model>, Error> {
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let (row, product) = self.variant_for_write(&transaction, tenant, principal, id).await?;
        let ids: Vec<Uuid> = values.iter().map(|value| value.attribute_id).collect();
        let details = self.attribute_details(&transaction, tenant, &ids).await?;
        let mut problems = Vec::new();
        let mut rows = Vec::new();
        let tenant_locales = self.locales(tenant).await?;
        for (index, input) in values.iter().enumerate() {
            let Some(detail) = details.get(&input.attribute_id) else {
                return Err(attribute_not_found(input.attribute_id));
            };
            if rows.iter().any(|(attribute, _, _): &(Uuid, _, _)| *attribute == input.attribute_id)
            {
                problems.push(FieldError::new(
                    format!("attributes[{index}].attribute_id"),
                    "duplicate",
                    "list each attribute once",
                ));
                continue;
            }
            let mut texts = BTreeMap::new();
            if !input.texts.is_empty() && detail.attribute.data_type != "text" {
                problems.push(FieldError::new(
                    format!("attributes[{index}].texts"),
                    "type",
                    "only text attributes have translations",
                ));
            }
            for (locale, text) in &input.texts {
                let field = format!("attributes[{index}].texts.{locale}");
                let Some(locale) = crate::config::canonical_locale(locale)
                    .filter(|locale| tenant_locales.contains(locale))
                else {
                    problems.push(FieldError::new(field, "locale", "is not a configured language"));
                    continue;
                };
                let text = text.trim();
                if text.is_empty() || text.chars().count() > 1_000 {
                    problems.push(FieldError::new(field, "length", "must be 1 to 1000 characters"));
                    continue;
                }
                texts.insert(locale, text.to_owned());
            }
            match parse_value(detail, &input.value, &format!("attributes[{index}].value")) {
                Ok(stored) => {
                    for item in stored {
                        rows.push((input.attribute_id, item, texts.clone()));
                    }
                }
                Err(problem) => problems.push(problem),
            }
        }
        fail_if_any(problems)?;
        variant_attribute::Entity::delete_many()
            .filter(variant_attribute::Column::VariantId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        for (attribute_id, stored, texts) in rows {
            variant_attribute::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                product_id: Set(row.product_id),
                variant_id: Set(id),
                attribute_id: Set(attribute_id),
                value_text: Set(stored.text),
                value_integer: Set(stored.integer),
                value_boolean: Set(stored.boolean),
                value_date: Set(stored.date),
                option_id: Set(stored.option_id),
                texts: Set((!texts.is_empty())
                    .then(|| serde_json::to_string(&texts))
                    .transpose()
                    .map_err(Error::internal)?),
            }
            .insert(&transaction)
            .await?;
        }
        // Trang sản phẩm đổi: phiên bản mới (ETag công khai) và event `catalog.product_updated`.
        self.touch(&transaction, tenant, actor, &product, None).await?;
        let saved = rows_of(&transaction, tenant, &[id]).await?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Thuộc tính hiển thị của từng biến thể trong `variants`, ở ngôn ngữ `locale`.
    pub(crate) async fn public_variant_attributes(
        &self,
        tenant: &TenantContext,
        variants: &[Uuid],
        locale: &str,
    ) -> Result<HashMap<Uuid, Vec<PublicAttribute>>, Error> {
        let read = tenant.database().read();
        let rows = rows_of(read, tenant, variants).await?;
        if rows.is_empty() {
            return Ok(HashMap::new());
        }
        let ids: Vec<Uuid> = rows.iter().map(|row| row.attribute_id).collect();
        let details = self.attribute_details(read, tenant, &ids).await?;
        let mut by_variant: HashMap<Uuid, Vec<product_attribute::Model>> = HashMap::new();
        for row in rows {
            // Chữ theo ngôn ngữ hỏi, không có thì bản mặc định.
            let text = row.value_text.clone().map(|default| {
                let texts: BTreeMap<String, String> = row
                    .texts
                    .as_deref()
                    .and_then(|raw| serde_json::from_str(raw).ok())
                    .unwrap_or_default();
                texts.get(locale).cloned().unwrap_or(default)
            });
            // Cùng hình dạng dòng thuộc tính của sản phẩm để trình bày bằng một cách.
            by_variant.entry(row.variant_id).or_default().push(product_attribute::Model {
                id: row.id,
                tenant_id: row.tenant_id,
                product_id: row.product_id,
                attribute_id: row.attribute_id,
                value_text: text,
                value_integer: row.value_integer,
                value_boolean: row.value_boolean,
                value_date: row.value_date,
                option_id: row.option_id,
            });
        }
        Ok(by_variant
            .into_iter()
            .map(|(variant, rows)| {
                (variant, crate::public::present_attributes(rows, &details, locale))
            })
            .collect())
    }
}

async fn rows_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    variants: &[Uuid],
) -> Result<Vec<variant_attribute::Model>, Error> {
    Ok(variant_attribute::Entity::find()
        .filter(variant_attribute::Column::VariantId.is_in(variants.iter().copied()))
        .scoped(tenant)
        .order_by_asc(variant_attribute::Column::Id)
        .all(connection)
        .await?)
}
