//! Quản trị bộ sưu tập: tạo, sửa, xoá, thay danh sách sản phẩm chọn tay. Phiên bản là
//! `updated_at` (ETag / `If-Match`). Kiểm ảnh (gọi `media`) chạy trước transaction ghi.

use autoxide_kernel::time::Timestamp;
use std::collections::{BTreeMap, HashSet};

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, root_of};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{MAX_ITEMS, SORTS, STATUSES, collection_not_found};
use crate::entity::{collection, collection_item, collection_text, product};
use crate::events::{self, Change};
use crate::permissions::{COLLECTIONS_MANAGE, PRODUCTS_READ};
use crate::service::{CatalogService, check_length, fail_if_any, micros, now, unique_violation};

/// Chữ của bộ sưu tập bằng một ngôn ngữ.
#[derive(Debug, Clone, Default)]
pub(crate) struct CollectionText {
    pub title: String,
    /// Trống: sinh từ tiêu đề.
    pub slug: Option<String>,
    pub description: String,
    pub meta_title: String,
    pub meta_description: String,
}

/// Bộ sưu tập mới.
#[derive(Debug, Clone, Default)]
pub(crate) struct NewCollection {
    /// Nút sở hữu; `None`: cả khách.
    pub org_id: Option<Uuid>,
    /// `manual` hay `rule`.
    pub kind: String,
    pub rules: Option<Value>,
    pub sort: Option<String>,
    pub media_id: Option<Uuid>,
    pub status: Option<String>,
    pub starts_at: Option<Timestamp>,
    pub ends_at: Option<Timestamp>,
    pub texts: BTreeMap<String, CollectionText>,
    /// Chữ soạn từ gợi ý của AI (Luật Trí tuệ nhân tạo — cần đối chiếu); tool MCP mặc định `true`.
    pub ai_assisted: bool,
}

/// Sửa một phần: trường không gửi giữ nguyên; `Some(None)` xoá giá trị. `texts` thay bản của các
/// ngôn ngữ được nêu.
#[derive(Debug, Clone, Default)]
pub(crate) struct CollectionPatch {
    pub rules: Option<Value>,
    pub sort: Option<String>,
    pub media_id: Option<Option<Uuid>>,
    pub status: Option<String>,
    pub starts_at: Option<Option<Timestamp>>,
    pub ends_at: Option<Option<Timestamp>>,
    pub texts: BTreeMap<String, CollectionText>,
    pub ai_assisted: Option<bool>,
}

/// Bộ sưu tập kèm chữ theo mọi ngôn ngữ và số sản phẩm chọn tay.
#[derive(Debug, Clone)]
pub(crate) struct CollectionDetail {
    pub collection: collection::Model,
    pub texts: Vec<collection_text::Model>,
    pub items: u64,
}

fn snapshot(row: &collection::Model) -> Value {
    json!({"kind": row.kind, "sort": row.sort, "status": row.status, "org_id": row.org_id})
}

impl CatalogService {
    pub(crate) async fn list_collections(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        status: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<CollectionDetail>, Error> {
        principal.require(&PRODUCTS_READ)?;
        let mut problems = Vec::new();
        if let Some(status) = status {
            check_choice(&mut problems, "status", status, &STATUSES);
        }
        fail_if_any(problems)?;
        let scope = principal.scope_condition(&PRODUCTS_READ, collection::Column::OrgId, None)?;
        let mut select = collection::Entity::find().filter(scope).scoped(tenant);
        if let Some(status) = status {
            select = select.filter(collection::Column::Status.eq(status));
        }
        let read = tenant.database().read();
        let page = fetch_page(select, collection::Column::Id, |row| row.id, page, read).await?;
        let ids: Vec<Uuid> = page.data.iter().map(|row| row.id).collect();
        let texts = collection_text::Entity::find()
            .filter(collection_text::Column::CollectionId.is_in(ids.clone()))
            .scoped(tenant)
            .all(read)
            .await?;
        let counts: Vec<(Uuid, i64)> = collection_item::Entity::find()
            .select_only()
            .column(collection_item::Column::CollectionId)
            .column_as(collection_item::Column::Id.count(), "items")
            .filter(collection_item::Column::CollectionId.is_in(ids))
            .group_by(collection_item::Column::CollectionId)
            .scoped(tenant)
            .into_tuple()
            .all(read)
            .await?;
        Ok(page.map(|row| {
            let items = counts.iter().find(|(id, _)| *id == row.id).map_or(0, |(_, n)| *n);
            CollectionDetail {
                texts: texts.iter().filter(|text| text.collection_id == row.id).cloned().collect(),
                items: u64::try_from(items).unwrap_or(0),
                collection: row,
            }
        }))
    }

    pub(crate) async fn get_collection(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<CollectionDetail, Error> {
        principal.require(&PRODUCTS_READ)?;
        let read = tenant.database().read();
        let row = find(read, tenant, id).await?;
        crate::service::readable(principal, read, &PRODUCTS_READ, row.org_id, || {
            collection_not_found(id)
        })
        .await?;
        self.collection_detail(read, tenant, row).await
    }

    async fn collection_detail<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        row: collection::Model,
    ) -> Result<CollectionDetail, Error> {
        let texts = collection_text::Entity::find()
            .filter(collection_text::Column::CollectionId.eq(row.id))
            .scoped(tenant)
            .order_by_asc(collection_text::Column::Locale)
            .all(connection)
            .await?;
        let items = collection_item::Entity::find()
            .filter(collection_item::Column::CollectionId.eq(row.id))
            .scoped(tenant)
            .count(connection)
            .await?;
        Ok(CollectionDetail { collection: row, texts, items })
    }

    pub(crate) async fn create_collection(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewCollection,
    ) -> Result<CollectionDetail, Error> {
        principal.require(&COLLECTIONS_MANAGE)?;
        let id = Uuid::now_v7();
        let mut problems = Vec::new();
        check_choice(&mut problems, "kind", &input.kind, &["manual", "rule"]);
        let texts = self.check_texts(tenant, &input.texts, id, true, &mut problems).await?;
        fail_if_any(problems)?;
        let rules = self.rules_for(tenant, &input.kind, input.rules.as_ref()).await?;
        let at = now();
        let row = collection::Model {
            id,
            tenant_id: tenant.id().as_uuid(),
            org_id: input.org_id.unwrap_or_else(|| root_of(tenant.id()).as_uuid()),
            kind: input.kind.clone(),
            rules,
            sort: input
                .sort
                .unwrap_or_else(|| if input.kind == "manual" { "manual" } else { "newest" }.into()),
            media_id: input.media_id,
            status: input.status.unwrap_or_else(|| "draft".to_owned()),
            starts_at: input.starts_at.map(micros),
            ends_at: input.ends_at.map(micros),
            created_at: at,
            updated_at: at,
            ai_assisted: input.ai_assisted,
        };
        self.check_row(tenant, &row).await?;
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(&transaction, &COLLECTIONS_MANAGE, crate::service::at(row.org_id))
            .await?;
        let saved = row.into_active_model().insert(&transaction).await?;
        write_texts(&transaction, tenant, id, texts).await?;
        self.record(&transaction, tenant, principal, None, Some(&saved)).await?;
        transaction.commit().await?;
        tracing::info!(collection_id = %id, "collection created");
        self.collection_detail(tenant.database().read(), tenant, saved).await
    }

    pub(crate) async fn update_collection(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        patch: CollectionPatch,
        expected: Option<Timestamp>,
    ) -> Result<CollectionDetail, Error> {
        // Kiểm permission trước khi tra bản ghi: người không quyền không biết bộ sưu tập nào tồn tại.
        principal.require(&COLLECTIONS_MANAGE)?;
        let found = self.get_collection(tenant, principal, id).await?.collection;
        let mut problems = Vec::new();
        let texts = self.check_texts(tenant, &patch.texts, id, false, &mut problems).await?;
        fail_if_any(problems)?;
        let rules = match &patch.rules {
            Some(raw) => self.rules_for(tenant, &found.kind, Some(raw)).await?,
            None => found.rules.clone(),
        };
        let row = collection::Model {
            rules,
            sort: patch.sort.unwrap_or_else(|| found.sort.clone()),
            media_id: patch.media_id.unwrap_or(found.media_id),
            status: patch.status.unwrap_or_else(|| found.status.clone()),
            starts_at: patch.starts_at.unwrap_or(found.starts_at).map(micros),
            ends_at: patch.ends_at.unwrap_or(found.ends_at).map(micros),
            ai_assisted: patch.ai_assisted.unwrap_or(found.ai_assisted),
            updated_at: now(),
            ..found.clone()
        };
        self.check_row(tenant, &row).await?;
        let transaction = tenant.database().begin_write().await?;
        crate::service::writable(
            principal,
            &transaction,
            &PRODUCTS_READ,
            &COLLECTIONS_MANAGE,
            found.org_id,
            || collection_not_found(id),
        )
        .await?;
        // `reset_all`: mọi cột thành `Set` để câu UPDATE ghi hết; kiểm phiên bản trong chính câu đó.
        let updated = collection::Entity::update_many()
            .set(row.clone().into_active_model().reset_all())
            .filter(collection::Column::Id.eq(id))
            .filter(collection::Column::UpdatedAt.eq(expected.unwrap_or(found.updated_at)))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if updated.rows_affected == 0 {
            return Err(autoxide_kernel::version::changed());
        }
        write_texts(&transaction, tenant, id, texts).await?;
        self.record(&transaction, tenant, principal, Some(&found), Some(&row)).await?;
        transaction.commit().await?;
        self.collection_detail(tenant.database().read(), tenant, row).await
    }

    pub(crate) async fn delete_collection(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&COLLECTIONS_MANAGE)?;
        let transaction = tenant.database().begin_write().await?;
        let found = find(&transaction, tenant, id).await?;
        crate::service::writable(
            principal,
            &transaction,
            &PRODUCTS_READ,
            &COLLECTIONS_MANAGE,
            found.org_id,
            || collection_not_found(id),
        )
        .await?;
        collection_item::Entity::delete_many()
            .filter(collection_item::Column::CollectionId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        collection_text::Entity::delete_many()
            .filter(collection_text::Column::CollectionId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        collection::Entity::delete_many()
            .filter(collection::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        self.record(&transaction, tenant, principal, Some(&found), None).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Thay cả danh sách sản phẩm chọn tay (theo thứ tự gửi). Chỉ bộ sưu tập `manual`.
    pub(crate) async fn set_collection_items(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        product_ids: Vec<Uuid>,
        expected: Option<Timestamp>,
    ) -> Result<CollectionDetail, Error> {
        // Kiểm permission trước khi tra bản ghi: người không quyền không biết bộ sưu tập nào tồn tại.
        principal.require(&COLLECTIONS_MANAGE)?;
        let found = self.get_collection(tenant, principal, id).await?.collection;
        let mut problems = Vec::new();
        if found.kind != "manual" {
            problems.push(FieldError::new(
                "product_ids",
                "invalid",
                "only a `manual` collection has chosen products",
            ));
        }
        if product_ids.len() > MAX_ITEMS {
            problems.push(
                FieldError::new("product_ids", "length", "must list at most {max} products")
                    .with_param("max", MAX_ITEMS),
            );
        }
        let mut seen = HashSet::new();
        for (index, product) in product_ids.iter().enumerate() {
            if !seen.insert(*product) {
                problems.push(FieldError::new(
                    format!("product_ids[{index}]"),
                    "duplicate",
                    "is listed twice",
                ));
            }
        }
        fail_if_any(problems)?;
        let known: Vec<Uuid> = product::Entity::find()
            .select_only()
            .column(product::Column::Id)
            .filter(product::Column::Id.is_in(product_ids.clone()))
            .scoped(tenant)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        let missing: Vec<FieldError> = product_ids
            .iter()
            .enumerate()
            .filter(|(_, product)| !known.contains(product))
            .map(|(index, _)| {
                FieldError::new(format!("product_ids[{index}]"), "not_found", "is not a product")
            })
            .collect();
        fail_if_any(missing)?;
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        crate::service::writable(
            principal,
            &transaction,
            &PRODUCTS_READ,
            &COLLECTIONS_MANAGE,
            found.org_id,
            || collection_not_found(id),
        )
        .await?;
        let updated = collection::Entity::update_many()
            .col_expr(collection::Column::UpdatedAt, Expr::value(at))
            .filter(collection::Column::Id.eq(id))
            .filter(collection::Column::UpdatedAt.eq(expected.unwrap_or(found.updated_at)))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if updated.rows_affected == 0 {
            return Err(autoxide_kernel::version::changed());
        }
        collection_item::Entity::delete_many()
            .filter(collection_item::Column::CollectionId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let rows: Vec<collection_item::ActiveModel> = product_ids
            .iter()
            .enumerate()
            .map(|(position, product_id)| {
                collection_item::Model {
                    id: Uuid::now_v7(),
                    tenant_id: tenant.id().as_uuid(),
                    collection_id: id,
                    product_id: *product_id,
                    position: i32::try_from(position).unwrap_or(i32::MAX),
                }
                .into_active_model()
            })
            .collect();
        if !rows.is_empty() {
            collection_item::Entity::insert_many(rows).exec(&transaction).await?;
        }
        let after = collection::Model { updated_at: at, ..found.clone() };
        let event = events::CollectionChanged(Change {
            before: None,
            after: Some(json!({"products": product_ids.len()})),
        });
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::collection_subject(id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        self.collection_detail(tenant.database().read(), tenant, after).await
    }

    /// Sản phẩm chọn tay của bộ sưu tập `id`, theo thứ tự.
    pub(crate) async fn collection_items(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Vec<Uuid>, Error> {
        self.get_collection(tenant, principal, id).await?;
        Ok(collection_item::Entity::find()
            .select_only()
            .column(collection_item::Column::ProductId)
            .filter(collection_item::Column::CollectionId.eq(id))
            .scoped(tenant)
            .order_by_asc(collection_item::Column::Position)
            .into_tuple()
            .all(tenant.database().read())
            .await?)
    }

    /// JSON quy tắc đã kiểm cho loại `kind`.
    async fn rules_for(
        &self,
        tenant: &TenantContext,
        kind: &str,
        raw: Option<&Value>,
    ) -> Result<String, Error> {
        if kind != "rule" {
            if raw.is_some_and(|raw| !raw.is_null() && raw != &json!({})) {
                return Err(Error::validation(vec![FieldError::new(
                    "rules",
                    "invalid",
                    "only a `rule` collection has rules",
                )]));
            }
            return Ok("{}".to_owned());
        }
        let raw = raw.cloned().unwrap_or(Value::Null);
        let rules = self.check_rules(tenant, &raw).await?;
        serde_json::to_string(&rules).map_err(Error::internal)
    }

    /// Kiểm các cột của dòng (sort, trạng thái, khung giờ, ảnh công khai).
    async fn check_row(
        &self,
        tenant: &TenantContext,
        row: &collection::Model,
    ) -> Result<(), Error> {
        let mut problems = Vec::new();
        check_choice(&mut problems, "sort", &row.sort, &SORTS);
        if row.sort == "manual" && row.kind != "manual" {
            problems.push(FieldError::new(
                "sort",
                "invalid",
                "`manual` order is only for a `manual` collection",
            ));
        }
        check_choice(&mut problems, "status", &row.status, &STATUSES);
        if let (Some(starts), Some(ends)) = (row.starts_at, row.ends_at)
            && ends <= starts
        {
            problems.push(FieldError::new("ends_at", "range", "must be after `starts_at`"));
        }
        fail_if_any(problems)?;
        if let Some(media) = row.media_id {
            self.check_public_media(tenant, &[media]).await?;
        }
        Ok(())
    }

    /// Kiểm chữ theo ngôn ngữ: (ngôn ngữ, chữ đã chuẩn hoá, slug).
    async fn check_texts(
        &self,
        tenant: &TenantContext,
        texts: &BTreeMap<String, CollectionText>,
        id: Uuid,
        required: bool,
        problems: &mut Vec<FieldError>,
    ) -> Result<Vec<(String, CollectionText, String)>, Error> {
        if required && texts.is_empty() {
            problems.push(FieldError::new(
                "texts",
                "required",
                "name the collection in a language",
            ));
        }
        let mut out = Vec::new();
        for (locale, text) in texts {
            let locale = self.locale(tenant, locale).await?;
            let at = |field: &str| format!("texts.{locale}.{field}");
            let title = text.title.trim().to_owned();
            check_length(problems, &at("title"), &title, 1, 200);
            check_length(problems, &at("description"), &text.description, 0, 5000);
            check_length(problems, &at("meta_title"), &text.meta_title, 0, 200);
            check_length(problems, &at("meta_description"), &text.meta_description, 0, 500);
            let slug = match text.slug.as_deref().map(str::trim).filter(|slug| !slug.is_empty()) {
                Some(slug) => {
                    if !autoxide_kernel::slug::is_valid(slug) {
                        problems.push(FieldError::new(
                            at("slug"),
                            "format",
                            "must be lower-case letters, digits and single `-` between them",
                        ));
                    }
                    slug.to_owned()
                }
                None => autoxide_kernel::slug::from_title(&title, id),
            };
            out.push((locale, CollectionText { title, ..text.clone() }, slug));
        }
        Ok(out)
    }

    async fn record<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        principal: &Principal,
        before: Option<&collection::Model>,
        after: Option<&collection::Model>,
    ) -> Result<(), Error> {
        let id = after.or(before).map(|row| row.id).unwrap_or_default();
        let event = events::CollectionChanged(Change {
            before: before.map(snapshot),
            after: after.map(snapshot),
        });
        self.outbox
            .record(
                connection,
                tenant,
                Actor::from(principal),
                events::collection_subject(id),
                &event,
            )
            .await
    }
}

async fn find<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<collection::Model, Error> {
    collection::Entity::find_by_id(id)
        .scoped(tenant)
        .one(connection)
        .await?
        .ok_or_else(|| collection_not_found(id))
}

async fn write_texts<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    id: Uuid,
    texts: Vec<(String, CollectionText, String)>,
) -> Result<(), Error> {
    for (locale, text, slug) in texts {
        collection_text::Entity::delete_many()
            .filter(collection_text::Column::CollectionId.eq(id))
            .filter(collection_text::Column::Locale.eq(locale.as_str()))
            .scoped(tenant)
            .exec(transaction)
            .await?;
        collection_text::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant.id().as_uuid(),
            collection_id: id,
            locale,
            title: text.title,
            slug: slug.clone(),
            description: text.description.trim().to_owned(),
            meta_title: text.meta_title.trim().to_owned(),
            meta_description: text.meta_description.trim().to_owned(),
        }
        .into_active_model()
        .insert(transaction)
        .await
        .map_err(|error| match unique_violation(&error) {
            Some(_) => crate::errors::slug_taken(&slug),
            None => Error::from(error),
        })?;
    }
    Ok(())
}

fn check_choice(problems: &mut Vec<FieldError>, field: &str, value: &str, allowed: &[&str]) {
    if !allowed.contains(&value) {
        problems.push(
            FieldError::new(field, "invalid", "must be one of {allowed}")
                .with_param("allowed", allowed.join(", ")),
        );
    }
}
