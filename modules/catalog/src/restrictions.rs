//! Thuộc tính của danh mục và của sản phẩm, hàng cấm / có điều kiện và giấy tờ (D-223…D-227):
//! mọi thay đổi ảnh hưởng tới `readiness` tính lại cột đó cho đúng các sản phẩm bị ảnh hưởng, trong
//! cùng transaction; job hằng ngày tính lại sản phẩm có giấy tờ (giấy tờ hết hạn theo ngày).
//!
//! Quy tắc hàng cấm / có điều kiện là dữ liệu dùng chung mọi khách (`catalog_restriction_rules`,
//! mặt điều khiển, D-198), nạp bằng `autoxide task catalog.load_restrictions` từ gói
//! `data/vn/restrictions.json` — căn cứ ghi trong từng quy tắc, cần đối chiếu văn bản gốc (D-186).

use autoxide_kernel::time::{Date, Timestamp};
use std::collections::BTreeMap;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page, page_of};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
};
use serde_json::Value;
use uuid::Uuid;

use crate::attributes::{
    AttributeDetail, CategoryAttributeInput, OptionInput, attribute_not_found, parse_value,
};
use crate::entity::{
    attribute, attribute_option, category, category_attribute, category_path, category_restriction,
    product, product_attribute, product_category, product_document, restriction_rule,
};
use crate::errors;
use crate::permissions::{CATEGORIES_MANAGE, DOCUMENTS_MANAGE, PRODUCTS_READ, PRODUCTS_WRITE};
use crate::readiness::{self, Readiness};
use crate::service::{
    CatalogService, check_length, fail_if_any, now, readable, texts_json, writable,
};

/// Một giá trị thuộc tính gửi lên cho sản phẩm.
#[derive(Debug, Clone)]
pub(crate) struct AttributeValueInput {
    pub attribute_id: Uuid,
    /// Theo kiểu của thuộc tính: chữ, số nguyên, số thập phân viết thành chữ, `true` / `false`,
    /// ngày `YYYY-MM-DD`, mã giá trị chọn, hay danh sách mã.
    pub value: Value,
}

/// Thay đổi cho thuộc tính; trường `None` giữ nguyên, `Some(None)` xoá giá trị.
#[derive(Debug, Clone, Default)]
pub(crate) struct AttributeChanges {
    pub names: Option<BTreeMap<String, String>>,
    pub unit: Option<Option<String>>,
    pub filterable: Option<bool>,
    pub legal_basis: Option<Option<String>>,
}

/// Giấy tờ mới của hàng có điều kiện.
#[derive(Debug, Clone)]
pub(crate) struct NewDocument {
    pub document_type: String,
    pub number: String,
    pub issued_by: String,
    pub issued_on: Date,
    pub expires_on: Option<Date>,
    pub file_key: Option<String>,
}

fn today() -> Date {
    Timestamp::now().date_utc()
}

fn document_not_found(id: Uuid) -> Error {
    Error::not_found("catalog.document_not_found", "There is no document `{id}` on this product.")
        .with_param("id", id)
}

fn rule_unknown(code: &str) -> Error {
    Error::not_found(
        "catalog.restriction_rule_not_found",
        "There is no restricted-goods rule `{code}`. Load the rules with `autoxide task \
         catalog.load_restrictions` and see `GET /api/v1/catalog/restriction-rules`.",
    )
    .with_param("code", code)
}

impl CatalogService {
    /// Tính lại và ghi `readiness` của các sản phẩm `ids` (bỏ ID không có).
    pub(crate) async fn refresh_readiness<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<(), Error> {
        if ids.is_empty() {
            return Ok(());
        }
        let control = self.databases.control().read();
        let rows = product::Entity::find()
            .filter(product::Column::Id.is_in(ids.iter().copied()))
            .scoped(tenant)
            .all(connection)
            .await?;
        for row in rows {
            let computed = readiness::compute(connection, control, tenant, &row, today()).await?;
            let value = computed.value();
            if row.readiness != value {
                product::Entity::update_many()
                    .col_expr(product::Column::Readiness, Expr::value(value))
                    .filter(product::Column::Id.eq(row.id))
                    .scoped(tenant)
                    .exec(connection)
                    .await?;
            }
        }
        Ok(())
    }

    /// `readiness` mới nhất của sản phẩm (để mở bán báo đúng thứ còn thiếu).
    pub(crate) async fn readiness_of<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        product: &product::Model,
    ) -> Result<Readiness, Error> {
        let control = self.databases.control().read();
        readiness::compute(connection, control, tenant, product, today()).await
    }

    /// Sản phẩm thuộc danh mục `category` hay một danh mục con của nó.
    async fn products_below<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        category: Uuid,
    ) -> Result<Vec<Uuid>, Error> {
        let below: Vec<Uuid> = category_path::Entity::find()
            .filter(category_path::Column::AncestorId.eq(category))
            .scoped(tenant)
            .all(connection)
            .await?
            .into_iter()
            .map(|path| path.descendant_id)
            .collect();
        let mut ids: Vec<Uuid> = product_category::Entity::find()
            .filter(product_category::Column::CategoryId.is_in(below))
            .scoped(tenant)
            .all(connection)
            .await?
            .into_iter()
            .map(|link| link.product_id)
            .collect();
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// Thay cả tập giá trị chọn của thuộc tính `option` / `options`. Bỏ giá trị đang có sản phẩm
    /// dùng thì `catalog.attribute_option_in_use`.
    pub(crate) async fn put_attribute_options(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        options: Vec<OptionInput>,
    ) -> Result<AttributeDetail, Error> {
        principal.require(&CATEGORIES_MANAGE)?;
        let mut problems = Vec::new();
        if options.len() > 500 {
            problems.push(FieldError::new("options", "length", "must have at most 500 options"));
        }
        let mut seen = std::collections::BTreeSet::new();
        for option in &options {
            if !seen.insert(option.code.clone())
                || option.code.is_empty()
                || option.code.len() > 64
                || !option
                    .code
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
            {
                problems.push(FieldError::new(
                    "options",
                    "code",
                    "each option needs its own code of 1 to 64 lower-case letters, digits, `_` or `-`",
                ));
                break;
            }
            for (locale, label) in &option.labels {
                self.locale(tenant, locale).await?;
                check_length(&mut problems, "options.labels", label, 1, 100);
            }
        }
        fail_if_any(problems)?;
        let transaction = tenant.database().begin_write().await?;
        let row = attribute::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| attribute_not_found(id))?;
        let existing = attribute_option::Entity::find()
            .filter(attribute_option::Column::AttributeId.eq(id))
            .scoped(tenant)
            .all(&transaction)
            .await?;
        let removed: Vec<Uuid> = existing
            .iter()
            .filter(|old| !options.iter().any(|option| option.code == old.code))
            .map(|old| old.id)
            .collect();
        if !removed.is_empty() {
            let used = product_attribute::Entity::find()
                .filter(product_attribute::Column::OptionId.is_in(removed.clone()))
                .scoped(tenant)
                .count(&transaction)
                .await?;
            if used > 0 {
                return Err(Error::conflict(
                    "catalog.attribute_option_in_use",
                    "Products still use an option you removed. Change those products first.",
                ));
            }
            attribute_option::Entity::delete_many()
                .filter(attribute_option::Column::Id.is_in(removed))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        for (position, option) in options.iter().enumerate() {
            let position = i32::try_from(position).unwrap_or(i32::MAX);
            match existing.iter().find(|old| old.code == option.code) {
                Some(old) => {
                    let mut changed: attribute_option::ActiveModel = old.clone().into();
                    changed.labels = Set(texts_json(&option.labels));
                    changed.position = Set(position);
                    changed.update(&transaction).await?;
                }
                None => {
                    attribute_option::ActiveModel {
                        id: Set(Uuid::now_v7()),
                        tenant_id: Set(tenant.id().as_uuid()),
                        attribute_id: Set(id),
                        code: Set(option.code.clone()),
                        labels: Set(texts_json(&option.labels)),
                        position: Set(position),
                    }
                    .insert(&transaction)
                    .await?;
                }
            }
        }
        transaction.commit().await?;
        let mut details =
            self.attribute_details(tenant.database().read(), tenant, &[row.id]).await?;
        details.remove(&row.id).ok_or_else(|| attribute_not_found(id))
    }

    /// Đổi tên, đơn vị, cờ lọc hay căn cứ pháp lý của thuộc tính (mã và kiểu không đổi được).
    pub(crate) async fn update_attribute(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        changes: AttributeChanges,
    ) -> Result<AttributeDetail, Error> {
        principal.require(&CATEGORIES_MANAGE)?;
        let AttributeChanges { names, unit, filterable, legal_basis } = changes;
        let mut problems = Vec::new();
        if let Some(names) = &names {
            for (locale, name) in names {
                self.locale(tenant, locale).await?;
                check_length(&mut problems, "names", name, 1, 100);
            }
        }
        fail_if_any(problems)?;
        let write = tenant.database().write();
        let row = attribute::Entity::find_by_id(id)
            .scoped(tenant)
            .one(write)
            .await?
            .ok_or_else(|| attribute_not_found(id))?;
        let mut changed: attribute::ActiveModel = row.into();
        if let Some(names) = names {
            changed.names = Set(texts_json(&names));
        }
        if let Some(unit) = unit {
            changed.unit = Set(unit);
        }
        if let Some(filterable) = filterable {
            changed.filterable = Set(filterable);
        }
        if let Some(legal_basis) = legal_basis {
            changed.legal_basis = Set(legal_basis);
        }
        changed.updated_at = Set(now());
        changed.update(write).await?;
        let mut details = self.attribute_details(tenant.database().read(), tenant, &[id]).await?;
        details.remove(&id).ok_or_else(|| attribute_not_found(id))
    }

    /// Thuộc tính của danh mục `id` (cả bắt buộc lẫn không), theo ngày hiệu lực.
    pub(crate) async fn category_attributes(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Vec<category_attribute::Model>, Error> {
        Ok(category_attribute::Entity::find()
            .filter(category_attribute::Column::CategoryId.eq(id))
            .scoped(tenant)
            .order_by_asc(category_attribute::Column::EffectiveFrom)
            .all(tenant.database().read())
            .await?)
    }

    /// Một trang thuộc tính của danh mục `id`, theo thứ tự của lần thay gần nhất (`PUT` ghi lại cả tập, id
    /// UUIDv7 theo thứ tự thân yêu cầu); keyset trong database (index `catalog_category_attributes_page_idx`).
    pub(crate) async fn category_attributes_page(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<category_attribute::Model>, Error> {
        let select = category_attribute::Entity::find()
            .filter(category_attribute::Column::CategoryId.eq(id))
            .scoped(tenant);
        fetch_page(
            select,
            category_attribute::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await
    }

    /// Thay cả tập thuộc tính của danh mục `id`; tính lại `readiness` của mọi sản phẩm trong nhánh.
    pub(crate) async fn put_category_attributes(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        items: Vec<CategoryAttributeInput>,
    ) -> Result<Vec<category_attribute::Model>, Error> {
        principal.require(&CATEGORIES_MANAGE)?;
        let mut problems = Vec::new();
        for (index, item) in items.iter().enumerate() {
            if item.effective_to.is_some_and(|to| to <= item.effective_from) {
                problems.push(FieldError::new(
                    format!("attributes[{index}].effective_to"),
                    "range",
                    "must be after `effective_from`",
                ));
            }
        }
        fail_if_any(problems)?;
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        category::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| errors::category_not_found(id))?;
        let ids: Vec<Uuid> = items.iter().map(|item| item.attribute_id).collect();
        let found = attribute::Entity::find()
            .filter(attribute::Column::Id.is_in(ids.clone()))
            .scoped(tenant)
            .count(&transaction)
            .await?;
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        if usize::try_from(found).unwrap_or(0) != unique.len() {
            let missing = ids.first().copied().unwrap_or_default();
            return Err(attribute_not_found(missing));
        }
        category_attribute::Entity::delete_many()
            .filter(category_attribute::Column::CategoryId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        for item in &items {
            category_attribute::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                category_id: Set(id),
                attribute_id: Set(item.attribute_id),
                required: Set(item.required),
                applies_to_descendants: Set(item.applies_to_descendants),
                effective_from: Set(item.effective_from),
                effective_to: Set(item.effective_to),
                legal_basis: Set(item.legal_basis.clone()),
            }
            .insert(&transaction)
            .await?;
        }
        let affected = self.products_below(&transaction, tenant, id).await?;
        self.refresh_readiness(&transaction, tenant, &affected).await?;
        self.record_category_change(&transaction, tenant, actor, id).await?;
        transaction.commit().await?;
        self.category_attributes(tenant, id).await
    }

    /// Mã quy tắc hàng cấm / có điều kiện gắn với danh mục `id`.
    pub(crate) async fn category_restrictions(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Vec<String>, Error> {
        Ok(category_restriction::Entity::find()
            .filter(category_restriction::Column::CategoryId.eq(id))
            .scoped(tenant)
            .order_by_asc(category_restriction::Column::RuleCode)
            .all(tenant.database().read())
            .await?
            .into_iter()
            .map(|link| link.rule_code)
            .collect())
    }

    /// Thay cả tập quy tắc hàng cấm / có điều kiện của danh mục `id`; mã phải có trong dữ liệu
    /// luật đã nạp. Tính lại `readiness` của mọi sản phẩm trong nhánh.
    pub(crate) async fn put_category_restrictions(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        codes: Vec<String>,
    ) -> Result<Vec<String>, Error> {
        principal.require(&CATEGORIES_MANAGE)?;
        let mut codes = codes;
        codes.sort();
        codes.dedup();
        let known: Vec<String> = restriction_rule::Entity::find()
            .filter(restriction_rule::Column::Code.is_in(codes.clone()))
            .all(self.databases.control().read())
            .await?
            .into_iter()
            .map(|rule| rule.code)
            .collect();
        if let Some(missing) = codes.iter().find(|code| !known.contains(code)) {
            return Err(rule_unknown(missing));
        }
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        category::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| errors::category_not_found(id))?;
        category_restriction::Entity::delete_many()
            .filter(category_restriction::Column::CategoryId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        for code in &codes {
            category_restriction::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                category_id: Set(id),
                rule_code: Set(code.clone()),
            }
            .insert(&transaction)
            .await?;
        }
        let affected = self.products_below(&transaction, tenant, id).await?;
        self.refresh_readiness(&transaction, tenant, &affected).await?;
        self.record_category_change(&transaction, tenant, actor, id).await?;
        transaction.commit().await?;
        Ok(codes)
    }

    /// Quy tắc hàng cấm / có điều kiện (dữ liệu dùng chung, chỉ đọc).
    /// Một trang quy tắc hàng cấm (`page_of`): dữ liệu luật dùng chung mọi khách ở mặt điều khiển, nạp bằng task
    /// của người vận hành — không lớn dần theo dữ liệu khách (như bảng thuế, D-20261126-list-pages-cluster-3).
    /// Theo vùng, mã, ngày hiệu lực (`YYYY-MM-DD` so theo byte đúng thứ tự ngày; bộ ba duy nhất).
    pub(crate) async fn restriction_rules(
        &self,
        page: &PageRequest,
    ) -> Result<Page<restriction_rule::Model>, Error> {
        let rows = restriction_rule::Entity::find().all(self.databases.control().read()).await?;
        page_of(
            rows,
            |row| format!("{}\u{1f}{}\u{1f}{}", row.jurisdiction, row.code, row.effective_from),
            page,
        )
    }

    /// Giá trị thuộc tính của sản phẩm (đọc, không kiểm quyền — dùng sau khi đã kiểm).
    pub(crate) async fn product_attributes<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        product_id: Uuid,
    ) -> Result<Vec<product_attribute::Model>, Error> {
        Ok(product_attribute::Entity::find()
            .filter(product_attribute::Column::ProductId.eq(product_id))
            .scoped(tenant)
            .order_by_asc(product_attribute::Column::Id)
            .all(connection)
            .await?)
    }

    /// Giá trị thuộc tính của sản phẩm `id` trong phạm vi đọc.
    pub(crate) async fn get_product_attributes(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<product_attribute::Model>, Error> {
        self.get_product(tenant, principal, id).await?;
        // Theo thứ tự của lần thay gần nhất (`PUT` ghi lại cả tập), keyset trong database (index
        // `catalog_product_attr_product_page_idx`).
        let select = product_attribute::Entity::find()
            .filter(product_attribute::Column::ProductId.eq(id))
            .scoped(tenant);
        fetch_page(
            select,
            product_attribute::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await
    }

    /// Thay cả tập giá trị thuộc tính của sản phẩm; tính lại `readiness`.
    pub(crate) async fn put_product_attributes(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        values: Vec<AttributeValueInput>,
    ) -> Result<Vec<product_attribute::Model>, Error> {
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let before =
            self.product_for_write(&transaction, tenant, principal, id, &PRODUCTS_WRITE).await?;
        let ids: Vec<Uuid> = values.iter().map(|value| value.attribute_id).collect();
        let details = self.attribute_details(&transaction, tenant, &ids).await?;
        let mut problems = Vec::new();
        let mut rows = Vec::new();
        for (index, input) in values.iter().enumerate() {
            let field = format!("attributes[{index}].value");
            let Some(detail) = details.get(&input.attribute_id) else {
                return Err(attribute_not_found(input.attribute_id));
            };
            if rows.iter().any(|(attribute, _): &(Uuid, _)| *attribute == input.attribute_id) {
                problems.push(FieldError::new(
                    format!("attributes[{index}].attribute_id"),
                    "duplicate",
                    "list each attribute once",
                ));
                continue;
            }
            match parse_value(detail, &input.value, &field) {
                Ok(stored) => {
                    for item in stored {
                        rows.push((input.attribute_id, item));
                    }
                }
                Err(problem) => problems.push(problem),
            }
        }
        fail_if_any(problems)?;
        product_attribute::Entity::delete_many()
            .filter(product_attribute::Column::ProductId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        for (attribute_id, stored) in rows {
            product_attribute::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                product_id: Set(id),
                attribute_id: Set(attribute_id),
                value_text: Set(stored.text),
                value_integer: Set(stored.integer),
                value_boolean: Set(stored.boolean),
                value_date: Set(stored.date),
                option_id: Set(stored.option_id),
            }
            .insert(&transaction)
            .await?;
        }
        self.refresh_readiness(&transaction, tenant, &[id]).await?;
        self.touch(&transaction, tenant, actor, &before, None).await?;
        let saved = self.product_attributes(&transaction, tenant, id).await?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Giấy tờ của sản phẩm `id` (cần `catalog.documents.manage` phủ nút của sản phẩm).
    pub(crate) async fn list_documents(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<product_document::Model>, Error> {
        let read = tenant.database().read();
        let row = product::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| errors::product_not_found(id))?;
        readable(principal, read, &PRODUCTS_READ, row.org_id, || errors::product_not_found(id))
            .await?;
        principal.require_at(read, &DOCUMENTS_MANAGE, crate::service::at(row.org_id)).await?;
        // Theo thứ tự tạo, keyset trong database (index `catalog_product_documents_page_idx`).
        let select = product_document::Entity::find()
            .filter(product_document::Column::ProductId.eq(id))
            .scoped(tenant);
        fetch_page(select, product_document::Column::Id, |row| row.id, page, read).await
    }

    /// Thêm giấy tờ cho sản phẩm `id`; tính lại `readiness`.
    pub(crate) async fn add_document(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        input: NewDocument,
    ) -> Result<product_document::Model, Error> {
        let mut problems = Vec::new();
        let code_ok = !input.document_type.is_empty()
            && input.document_type.len() <= 64
            && input
                .document_type
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !code_ok {
            problems.push(FieldError::new(
                "document_type",
                "format",
                "must be a document type code such as `alcohol_retail_licence`",
            ));
        }
        check_length(&mut problems, "number", input.number.trim(), 1, 100);
        check_length(&mut problems, "issued_by", input.issued_by.trim(), 1, 200);
        if input.expires_on.is_some_and(|day| day <= input.issued_on) {
            problems.push(FieldError::new("expires_on", "range", "must be after `issued_on`"));
        }
        fail_if_any(problems)?;
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        // Khoá sản phẩm như `product_for_write`: mở bán (kiểm giấy tờ) chờ lệnh này xong.
        let before = autoxide_kernel::rows::lock_one(
            &transaction,
            product::Entity::find_by_id(id).scoped(tenant),
        )
        .await?
        .ok_or_else(|| errors::product_not_found(id))?;
        writable(principal, &transaction, &PRODUCTS_READ, &DOCUMENTS_MANAGE, before.org_id, || {
            errors::product_not_found(id)
        })
        .await?;
        let row = product_document::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            product_id: Set(id),
            org_id: Set(before.org_id),
            document_type: Set(input.document_type),
            number: Set(input.number.trim().to_owned()),
            issued_by: Set(input.issued_by.trim().to_owned()),
            issued_on: Set(input.issued_on),
            expires_on: Set(input.expires_on),
            file_key: Set(input.file_key),
            created_at: Set(now()),
        }
        .insert(&transaction)
        .await?;
        self.refresh_readiness(&transaction, tenant, &[id]).await?;
        self.touch(&transaction, tenant, actor, &before, None).await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Bỏ giấy tờ `document` của sản phẩm `id`; tính lại `readiness`.
    pub(crate) async fn delete_document(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        document: Uuid,
    ) -> Result<(), Error> {
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        // Khoá sản phẩm như `product_for_write`: mở bán (kiểm giấy tờ) chờ lệnh này xong.
        let before = autoxide_kernel::rows::lock_one(
            &transaction,
            product::Entity::find_by_id(id).scoped(tenant),
        )
        .await?
        .ok_or_else(|| errors::product_not_found(id))?;
        writable(principal, &transaction, &PRODUCTS_READ, &DOCUMENTS_MANAGE, before.org_id, || {
            errors::product_not_found(id)
        })
        .await?;
        let deleted = product_document::Entity::delete_many()
            .filter(product_document::Column::Id.eq(document))
            .filter(product_document::Column::ProductId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if deleted.rows_affected == 0 {
            return Err(document_not_found(document));
        }
        self.refresh_readiness(&transaction, tenant, &[id]).await?;
        self.touch(&transaction, tenant, actor, &before, None).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Số sản phẩm trong phạm vi đọc, theo `status` hay `readiness` (số liệu đầu trang).
    pub(crate) async fn count_products(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        group_by: &str,
        page: &PageRequest,
    ) -> Result<Page<(String, u64)>, Error> {
        let (column, keys): (product::Column, &[&str]) = match group_by {
            "status" => (product::Column::Status, &["draft", "active", "inactive", "archived"]),
            "readiness" => (product::Column::Readiness, &readiness::ALL),
            _ => {
                return Err(Error::validation(vec![FieldError::new(
                    "group_by",
                    "one_of",
                    "must be `status` or `readiness`",
                )]));
            }
        };
        let scope = principal.scope_condition(&PRODUCTS_READ, product::Column::OrgId, None)?;
        let rows: Vec<(String, i64)> = product::Entity::find()
            .select_only()
            .column(column)
            .column_as(product::Column::Id.count(), "count")
            .filter(scope)
            .scoped(tenant)
            .group_by(column)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        let counts: Vec<(usize, String, u64)> = keys
            .iter()
            .enumerate()
            .map(|(index, key)| {
                let count = rows
                    .iter()
                    .find(|(value, _)| value == key)
                    .map_or(0, |(_, count)| u64::try_from(*count).unwrap_or(0));
                (index, (*key).to_owned(), count)
            })
            .collect();
        // Một trang (`page_of`): một dòng cho mỗi khoá của tập cố định trong code (trạng thái hay mức sẵn sàng),
        // giữ thứ tự khai báo (khoá trang là vị trí, đệm 0).
        let found = page_of(counts, |(index, _, _)| format!("{index:04}"), page)?;
        Ok(found.map(|(_, key, count)| (key, count)))
    }

    /// Ghi event `catalog.category_changed` cho danh mục `id` (thuộc tính hay quy tắc đổi).
    async fn record_category_change<C: ConnectionTrait>(
        &self,
        transaction: &C,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
    ) -> Result<(), Error> {
        let event = crate::events::CategoryChanged(crate::events::Change {
            before: None,
            after: Some(serde_json::json!({"id": id, "rules_changed": true})),
        });
        self.outbox
            .record(transaction, tenant, actor, crate::events::category_subject(id), &event)
            .await
    }
}
