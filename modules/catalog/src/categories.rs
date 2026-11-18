//! Danh mục dạng cây của cả khách: bảng `catalog_categories` (cha, độ sâu, thứ tự) và bảng tổ
//! tiên – con cháu `catalog_category_paths` (gồm chính nó) để lọc "danh mục và mọi danh mục con"
//! bằng một câu. Đổi cây (tạo, dời, xoá) khoá mọi dòng danh mục của khách trong transaction: hai
//! lần dời chéo nhau chạy nối đuôi, nên cây không bao giờ có vòng (cả hai database).

use std::collections::BTreeMap;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, QueryOrder, Set,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{category, category_path, category_translation, product_category};
use crate::errors;
use crate::events::{self, Change};
use crate::permissions::CATEGORIES_MANAGE;
use crate::service::{CatalogService, check_length, fail_if_any, now, unique_violation};

/// Độ sâu tối đa của cây (gốc là 0).
const MAX_DEPTH: i32 = 6;

/// Tên và slug của danh mục bằng một ngôn ngữ.
#[derive(Debug, Clone)]
pub(crate) struct CategoryText {
    pub name: String,
    /// Trống: sinh từ tên.
    pub slug: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct NewCategory {
    pub parent_id: Option<Uuid>,
    pub position: Option<i32>,
    pub visible: bool,
    pub texts: BTreeMap<String, CategoryText>,
}

/// Một danh mục kèm tên theo mọi ngôn ngữ.
#[derive(Debug, Clone)]
pub(crate) struct CategoryDetail {
    pub category: category::Model,
    pub translations: Vec<category_translation::Model>,
}

fn snapshot(row: &category::Model) -> Value {
    json!({
        "id": row.id,
        "parent_id": row.parent_id,
        "depth": row.depth,
        "position": row.position,
        "visible": row.visible,
    })
}

fn slug_conflict(error: sea_orm::DbErr, slug: &str) -> Error {
    match unique_violation(&error) {
        Some(_) => errors::slug_taken(slug),
        None => Error::from(error),
    }
}

impl CatalogService {
    /// Kiểm tên theo ngôn ngữ của danh mục, trả (ngôn ngữ, tên, slug) đã chuẩn hoá.
    fn check_category_texts(
        &self,
        texts: &BTreeMap<String, CategoryText>,
        id: Uuid,
    ) -> Result<Vec<(String, String, String)>, Error> {
        let mut problems = Vec::new();
        if texts.is_empty() {
            problems.push(FieldError::new("texts", "required", "name the category in a language"));
        }
        let mut out = Vec::new();
        for (locale, text) in texts {
            let locale = self.locale(locale)?;
            let name = text.name.trim().to_owned();
            check_length(&mut problems, &format!("texts.{locale}.name"), &name, 1, 200);
            let slug = match text.slug.as_deref().map(str::trim).filter(|slug| !slug.is_empty()) {
                Some(slug) => {
                    if !autoxide_kernel::slug::is_valid(slug) {
                        problems.push(FieldError::new(
                            format!("texts.{locale}.slug"),
                            "format",
                            "must be lower-case letters, digits and single `-` between them",
                        ));
                    }
                    slug.to_owned()
                }
                None => autoxide_kernel::slug::from_title(&name, id),
            };
            out.push((locale, name, slug));
        }
        fail_if_any(problems)?;
        Ok(out)
    }

    /// Khoá mọi dòng danh mục của khách tới hết transaction (Postgres `FOR UPDATE`; SQLite đã
    /// giữ quyền ghi cả database).
    async fn lock_tree<C: ConnectionTrait>(
        &self,
        transaction: &C,
        tenant: &TenantContext,
    ) -> Result<Vec<category::Model>, Error> {
        Ok(rows::lock(transaction, category::Entity::find().scoped(tenant), category::Column::Id)
            .await?)
    }

    async fn write_texts<C: ConnectionTrait>(
        &self,
        transaction: &C,
        tenant: &TenantContext,
        id: Uuid,
        texts: Vec<(String, String, String)>,
    ) -> Result<(), Error> {
        for (locale, name, slug) in texts {
            category_translation::Entity::delete_many()
                .filter(category_translation::Column::CategoryId.eq(id))
                .filter(category_translation::Column::Locale.eq(locale.as_str()))
                .scoped(tenant)
                .exec(transaction)
                .await?;
            category_translation::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                category_id: Set(id),
                locale: Set(locale),
                name: Set(name),
                slug: Set(slug.clone()),
            }
            .insert(transaction)
            .await
            .map_err(|error| slug_conflict(error, &slug))?;
        }
        Ok(())
    }

    /// Tạo danh mục (dưới `parent_id`, hay ở gốc).
    pub(crate) async fn create_category(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewCategory,
    ) -> Result<CategoryDetail, Error> {
        principal.require(&CATEGORIES_MANAGE)?;
        let id = Uuid::now_v7();
        let texts = self.check_category_texts(&input.texts, id)?;
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let tree = self.lock_tree(&transaction, tenant).await?;
        let depth = match input.parent_id {
            Some(parent) => {
                let parent = tree
                    .iter()
                    .find(|row| row.id == parent)
                    .ok_or_else(|| errors::category_not_found(parent))?;
                parent.depth + 1
            }
            None => 0,
        };
        if depth > MAX_DEPTH {
            return Err(Error::validation(vec![FieldError::new(
                "parent_id",
                "depth",
                "categories nest at most 7 levels deep",
            )]));
        }
        let siblings = tree.iter().filter(|row| row.parent_id == input.parent_id).count();
        let now = now();
        let row = category::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            parent_id: Set(input.parent_id),
            depth: Set(depth),
            position: Set(input.position.unwrap_or(i32::try_from(siblings).unwrap_or(i32::MAX))),
            visible: Set(input.visible),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await?;
        // Đường tổ tiên: chính nó, rồi mọi tổ tiên của cha.
        let mut ancestors = vec![(id, 0)];
        if let Some(parent) = input.parent_id {
            let above = category_path::Entity::find()
                .filter(category_path::Column::DescendantId.eq(parent))
                .scoped(tenant)
                .all(&transaction)
                .await?;
            ancestors.extend(above.into_iter().map(|path| (path.ancestor_id, path.depth + 1)));
        }
        for (ancestor, distance) in ancestors {
            category_path::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                ancestor_id: Set(ancestor),
                descendant_id: Set(id),
                depth: Set(distance),
            }
            .insert(&transaction)
            .await?;
        }
        self.write_texts(&transaction, tenant, id, texts).await?;
        let event = events::CategoryChanged(Change { before: None, after: Some(snapshot(&row)) });
        self.outbox
            .record(&transaction, tenant, actor, events::category_subject(id), &event)
            .await?;
        transaction.commit().await?;
        self.category_detail(tenant, id).await
    }

    /// Mọi danh mục của khách, theo độ sâu rồi thứ tự hiển thị.
    pub(crate) async fn list_categories(
        &self,
        tenant: &TenantContext,
    ) -> Result<Vec<CategoryDetail>, Error> {
        let read = tenant.database().read();
        let rows = category::Entity::find()
            .scoped(tenant)
            .order_by_asc(category::Column::Depth)
            .order_by_asc(category::Column::Position)
            .order_by_asc(category::Column::Id)
            .all(read)
            .await?;
        let texts = category_translation::Entity::find()
            .scoped(tenant)
            .order_by_asc(category_translation::Column::Locale)
            .all(read)
            .await?;
        Ok(rows
            .into_iter()
            .map(|category| {
                let translations =
                    texts.iter().filter(|text| text.category_id == category.id).cloned().collect();
                CategoryDetail { category, translations }
            })
            .collect())
    }

    pub(crate) async fn category_detail(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<CategoryDetail, Error> {
        let read = tenant.database().read();
        let category = category::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| errors::category_not_found(id))?;
        let translations = category_translation::Entity::find()
            .filter(category_translation::Column::CategoryId.eq(id))
            .scoped(tenant)
            .order_by_asc(category_translation::Column::Locale)
            .all(read)
            .await?;
        Ok(CategoryDetail { category, translations })
    }

    /// Đổi hiện / ẩn, thứ tự hay tên của danh mục.
    pub(crate) async fn update_category(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        visible: Option<bool>,
        position: Option<i32>,
        texts: BTreeMap<String, CategoryText>,
    ) -> Result<CategoryDetail, Error> {
        principal.require(&CATEGORIES_MANAGE)?;
        let texts =
            if texts.is_empty() { Vec::new() } else { self.check_category_texts(&texts, id)? };
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let before = category::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| errors::category_not_found(id))?;
        let mut row = before.clone().into_active_model();
        if let Some(visible) = visible {
            row.visible = Set(visible);
        }
        if let Some(position) = position {
            row.position = Set(position);
        }
        row.updated_at = Set(now());
        let after = row.update(&transaction).await?;
        self.write_texts(&transaction, tenant, id, texts).await?;
        let event = events::CategoryChanged(Change {
            before: Some(snapshot(&before)),
            after: Some(snapshot(&after)),
        });
        self.outbox
            .record(&transaction, tenant, actor, events::category_subject(id), &event)
            .await?;
        transaction.commit().await?;
        self.category_detail(tenant, id).await
    }

    /// Dời danh mục `id` (cả nhánh của nó) dưới `parent_id` (hay lên gốc). Dời vào chính nhánh
    /// của nó → `catalog.category_cycle`.
    pub(crate) async fn move_category(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        parent_id: Option<Uuid>,
    ) -> Result<CategoryDetail, Error> {
        principal.require(&CATEGORIES_MANAGE)?;
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        // Khoá cả cây TRƯỚC khi đọc đường: lần dời đồng thời chờ tới khi lần này commit, rồi đọc
        // cây mới (Postgres đọc lại dòng sau khi chờ khoá).
        let tree = self.lock_tree(&transaction, tenant).await?;
        let before = tree
            .iter()
            .find(|row| row.id == id)
            .cloned()
            .ok_or_else(|| errors::category_not_found(id))?;
        let paths = category_path::Entity::find().scoped(tenant).all(&transaction).await?;
        let subtree: Vec<&category_path::Model> =
            paths.iter().filter(|path| path.ancestor_id == id).collect();
        let parent_depth = match parent_id {
            Some(parent) => {
                if subtree.iter().any(|path| path.descendant_id == parent) {
                    return Err(errors::category_cycle());
                }
                tree.iter()
                    .find(|row| row.id == parent)
                    .ok_or_else(|| errors::category_not_found(parent))?
                    .depth
            }
            None => -1,
        };
        let height = subtree.iter().map(|path| path.depth).max().unwrap_or(0);
        if parent_depth + 1 + height > MAX_DEPTH {
            return Err(Error::validation(vec![FieldError::new(
                "parent_id",
                "depth",
                "categories nest at most 7 levels deep",
            )]));
        }
        let descendants: Vec<Uuid> = subtree.iter().map(|path| path.descendant_id).collect();
        let old_ancestors: Vec<Uuid> = paths
            .iter()
            .filter(|path| path.descendant_id == id && path.ancestor_id != id)
            .map(|path| path.ancestor_id)
            .collect();
        // Bỏ đường từ tổ tiên cũ tới mọi nút của nhánh, rồi nối tổ tiên mới.
        category_path::Entity::delete_many()
            .filter(category_path::Column::DescendantId.is_in(descendants.clone()))
            .filter(category_path::Column::AncestorId.is_in(old_ancestors))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if let Some(parent) = parent_id {
            let above: Vec<&category_path::Model> =
                paths.iter().filter(|path| path.descendant_id == parent).collect();
            for below in &subtree {
                for over in &above {
                    category_path::ActiveModel {
                        id: Set(Uuid::now_v7()),
                        tenant_id: Set(tenant.id().as_uuid()),
                        ancestor_id: Set(over.ancestor_id),
                        descendant_id: Set(below.descendant_id),
                        depth: Set(over.depth + 1 + below.depth),
                    }
                    .insert(&transaction)
                    .await?;
                }
            }
        }
        // Độ sâu mới của mọi nút trong nhánh.
        for below in &subtree {
            if let Some(node) = tree.iter().find(|row| row.id == below.descendant_id) {
                let mut row = node.clone().into_active_model();
                row.depth = Set(parent_depth + 1 + below.depth);
                if node.id == id {
                    row.parent_id = Set(parent_id);
                }
                row.updated_at = Set(now());
                row.update(&transaction).await?;
            }
        }
        let after = category::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| errors::category_not_found(id))?;
        let event = events::CategoryChanged(Change {
            before: Some(snapshot(&before)),
            after: Some(snapshot(&after)),
        });
        self.outbox
            .record(&transaction, tenant, actor, events::category_subject(id), &event)
            .await?;
        transaction.commit().await?;
        self.category_detail(tenant, id).await
    }

    /// Xoá danh mục không còn danh mục con hay sản phẩm (`catalog.category_not_empty`).
    pub(crate) async fn delete_category(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&CATEGORIES_MANAGE)?;
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let tree = self.lock_tree(&transaction, tenant).await?;
        let before = tree
            .iter()
            .find(|row| row.id == id)
            .cloned()
            .ok_or_else(|| errors::category_not_found(id))?;
        let products = product_category::Entity::find()
            .filter(product_category::Column::CategoryId.eq(id))
            .scoped(tenant)
            .count(&transaction)
            .await?;
        if products > 0 || tree.iter().any(|row| row.parent_id == Some(id)) {
            return Err(errors::category_not_empty());
        }
        category_translation::Entity::delete_many()
            .filter(category_translation::Column::CategoryId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        category_path::Entity::delete_many()
            .filter(category_path::Column::DescendantId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        category::Entity::delete_many()
            .filter(category::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let event =
            events::CategoryChanged(Change { before: Some(snapshot(&before)), after: None });
        self.outbox
            .record(&transaction, tenant, actor, events::category_subject(id), &event)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// `ids` lọc còn các danh mục có thật của khách, giữ thứ tự, bỏ trùng; ID lạ → 404.
    pub(crate) async fn existing_categories<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<Vec<Uuid>, Error> {
        let mut unique = Vec::new();
        for id in ids {
            if !unique.contains(id) {
                unique.push(*id);
            }
        }
        if unique.is_empty() {
            return Ok(unique);
        }
        let found: Vec<Uuid> = category::Entity::find()
            .filter(category::Column::Id.is_in(unique.clone()))
            .scoped(tenant)
            .all(connection)
            .await?
            .into_iter()
            .map(|row| row.id)
            .collect();
        match unique.iter().find(|id| !found.contains(id)) {
            Some(missing) => Err(errors::category_not_found(*missing)),
            None => Ok(unique),
        }
    }

    /// Danh mục `id` và mọi danh mục con cháu.
    pub(crate) async fn descendants<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Vec<Uuid>, Error> {
        Ok(category_path::Entity::find()
            .filter(category_path::Column::AncestorId.eq(id))
            .scoped(tenant)
            .all(connection)
            .await?
            .into_iter()
            .map(|path| path.descendant_id)
            .collect())
    }

    /// Mọi tổ tiên (gồm chính nó) của các danh mục `ids`, bỏ trùng.
    pub(crate) async fn with_ancestors<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<Vec<(Uuid, Uuid, i32)>, Error> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(category_path::Entity::find()
            .filter(category_path::Column::DescendantId.is_in(ids.iter().copied()))
            .scoped(tenant)
            .all(connection)
            .await?
            .into_iter()
            .map(|path| (path.descendant_id, path.ancestor_id, path.depth))
            .collect())
    }
}
