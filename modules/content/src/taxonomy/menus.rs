//! Menu điều hướng: theo (`key`, `locale`), nhiều tầng, mỗi mục trỏ tới một mục nội dung,
//! một term hay một địa chỉ. `PUT` thay cả cây (bắt buộc `If-Match`; menu mới: `If-Match: *`).
//! Bản công khai tính sẵn đường dẫn và tiêu đề; mục trỏ tới nội dung chưa công khai thì ẩn
//! (cả nhánh con của nó). Thiếu menu ở một ngôn ngữ thì dùng ngôn ngữ mặc định (D-102).

use std::collections::HashMap;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{entry, menu, menu_item, term};
use crate::events::{self, Change};
use crate::permissions::{CONTENT_MANAGE, CONTENT_READ};
use crate::service::{ContentService, invalid, now};

pub(crate) type Menu = menu::Model;
pub(crate) type MenuItem = menu_item::Model;

const MAX_ITEMS: usize = 200;
const MAX_DEPTH: usize = 4;

/// Một mục của menu như client gửi (cây).
#[derive(Debug, Clone, Default)]
pub(crate) struct ItemInput {
    pub label: String,
    pub entry_id: Option<Uuid>,
    pub term_id: Option<Uuid>,
    pub url: Option<String>,
    pub children: Vec<ItemInput>,
}

/// Một mục kèm các mục con.
#[derive(Debug, Clone)]
pub(crate) struct Node {
    pub item: MenuItem,
    pub children: Vec<Node>,
}

/// Một mục của menu công khai.
#[derive(Debug, Clone)]
pub(crate) struct PublicNode {
    pub label: String,
    /// Đường dẫn trong site (mục nội dung) hay địa chỉ (`url`).
    pub href: String,
    pub entry_id: Option<Uuid>,
    pub term: Option<crate::taxonomy::terms::TermRef>,
    pub children: Vec<PublicNode>,
}

impl ContentService {
    pub(crate) async fn list_menus(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<Vec<Menu>, Error> {
        principal.require(&CONTENT_READ)?;
        Ok(menu::Entity::find()
            .scoped(tenant)
            .order_by_asc(menu::Column::Key)
            .order_by_asc(menu::Column::Locale)
            .all(tenant.database().read())
            .await?)
    }

    pub(crate) async fn get_menu(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &str,
        locale: Option<&str>,
    ) -> Result<(Menu, Vec<Node>), Error> {
        principal.require(&CONTENT_READ)?;
        let mut problems = Vec::new();
        let locale =
            self.check_locale(&self.languages(tenant).await?, locale, "locale", &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let found = self
            .find_menu(tenant, key, &locale)
            .await?
            .ok_or_else(|| menu_not_found(key, &locale))?;
        let items = self.menu_items(tenant, found.id).await?;
        Ok((found, tree(items)))
    }

    /// Thay cả cây mục của menu (`key`, `locale`); chưa có thì tạo. Cần `content.manage`.
    pub(crate) async fn put_menu(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &str,
        locale: Option<&str>,
        items: Vec<ItemInput>,
        expected: Option<DateTime<Utc>>,
    ) -> Result<(Menu, Vec<Node>), Error> {
        principal.require(&CONTENT_MANAGE)?;
        let mut problems = Vec::new();
        let locale =
            self.check_locale(&self.languages(tenant).await?, locale, "locale", &mut problems);
        if key.is_empty()
            || key.len() > 64
            || !key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        {
            problems.push(FieldError::new(
                "key",
                "invalid",
                "must be 1 to 64 lower-case letters, digits, `-` or `_`",
            ));
        }
        let mut flat = Vec::new();
        flatten(tenant, &items, None, 1, "items", &mut flat, &mut problems);
        if flat.len() > MAX_ITEMS {
            problems.push(
                FieldError::new("items", "length", "at most {max_items} items")
                    .with_param("max_items", MAX_ITEMS),
            );
        }
        self.check_targets(tenant, &flat, &locale, &mut problems).await?;
        if !problems.is_empty() {
            return Err(invalid(problems));
        }
        let now = now();
        let transaction = tenant.database().begin_write().await?;
        let existing = menu::Entity::find()
            .scoped(tenant)
            .filter(menu::Column::Key.eq(key))
            .filter(menu::Column::Locale.eq(locale.as_str()))
            .one(&transaction)
            .await?;
        let (saved, before) = match existing {
            Some(existing) => {
                let updated = menu::Entity::update_many()
                    .scoped(tenant)
                    .col_expr(menu::Column::UpdatedAt, sea_orm::sea_query::Expr::value(now))
                    .filter(menu::Column::Id.eq(existing.id))
                    .filter(menu::Column::UpdatedAt.eq(expected.unwrap_or(existing.updated_at)))
                    .exec(&transaction)
                    .await?;
                if updated.rows_affected == 0 {
                    return Err(autoxide_kernel::version::changed());
                }
                let before = menu_item::Entity::find()
                    .scoped(tenant)
                    .filter(menu_item::Column::MenuId.eq(existing.id))
                    .count(&transaction)
                    .await?;
                menu_item::Entity::delete_many()
                    .scoped(tenant)
                    .filter(menu_item::Column::MenuId.eq(existing.id))
                    .exec(&transaction)
                    .await?;
                (Menu { updated_at: now, ..existing }, Some(before))
            }
            None => {
                if expected.is_some() {
                    // `If-Match` nêu một phiên bản cụ thể mà menu chưa có: không khớp.
                    return Err(autoxide_kernel::version::changed());
                }
                let row = menu::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    key: Set(key.to_owned()),
                    locale: Set(locale.clone()),
                    created_at: Set(now),
                    updated_at: Set(now),
                }
                .insert(&transaction)
                .await
                .map_err(|error| {
                    if crate::entries::is_unique_violation(&error) {
                        autoxide_kernel::version::changed()
                    } else {
                        error.into()
                    }
                })?;
                (row, None)
            }
        };
        for (index, (item, _)) in flat.iter().enumerate() {
            menu_item::ActiveModel {
                id: Set(item.id),
                tenant_id: Set(tenant.id().as_uuid()),
                menu_id: Set(saved.id),
                parent_id: Set(item.parent_id),
                sort_order: Set(i32::try_from(index).unwrap_or(i32::MAX)),
                label: Set(item.label.clone()),
                target_entry_id: Set(item.target_entry_id),
                target_term_id: Set(item.target_term_id),
                target_url: Set(item.target_url.clone()),
            }
            .insert(&transaction)
            .await?;
        }
        let change = Change {
            before: before.map(|count| json!({"key": key, "locale": locale, "items": count})),
            after: Some(json!({"key": key, "locale": locale, "items": flat.len()})),
            published: None,
        };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::subject("menu", saved.id),
                &events::MenuChanged(change),
            )
            .await?;
        transaction.commit().await?;
        let items = flat.into_iter().map(|(item, _)| item).collect();
        Ok((saved, tree(items)))
    }

    /// Menu công khai: ngôn ngữ `locale`, thiếu thì ngôn ngữ mặc định. Trả menu và cây mục đã
    /// tính đường dẫn; mục trỏ tới nội dung chưa công khai thì ẩn.
    pub(crate) async fn public_menu(
        &self,
        tenant: &TenantContext,
        key: &str,
        locale: &str,
    ) -> Result<(Menu, Vec<PublicNode>), Error> {
        let mut problems = Vec::new();
        let languages = self.languages(tenant).await?;
        let locale = self.check_locale(&languages, Some(locale), "locale", &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let found = match self.find_menu(tenant, key, &locale).await? {
            Some(found) => found,
            None => self
                .find_menu(tenant, key, &languages.default)
                .await?
                .ok_or_else(|| menu_not_found(key, &locale))?,
        };
        let items = self.menu_items(tenant, found.id).await?;
        let entry_ids: Vec<Uuid> = items.iter().filter_map(|item| item.target_entry_id).collect();
        let live: HashMap<Uuid, String> =
            crate::publishing::paths::paths(tenant.database().read(), tenant, &entry_ids).await?;
        let term_ids: Vec<Uuid> = items.iter().filter_map(|item| item.target_term_id).collect();
        let terms = self.term_refs(tenant, &term_ids, &found.locale).await?;
        let nodes = public_nodes(&tree(items), &live, &terms);
        Ok((found, nodes))
    }

    async fn find_menu(
        &self,
        tenant: &TenantContext,
        key: &str,
        locale: &str,
    ) -> Result<Option<Menu>, Error> {
        Ok(menu::Entity::find()
            .scoped(tenant)
            .filter(menu::Column::Key.eq(key))
            .filter(menu::Column::Locale.eq(locale))
            .one(tenant.database().read())
            .await?)
    }

    async fn menu_items(
        &self,
        tenant: &TenantContext,
        menu_id: Uuid,
    ) -> Result<Vec<MenuItem>, Error> {
        Ok(menu_item::Entity::find()
            .scoped(tenant)
            .filter(menu_item::Column::MenuId.eq(menu_id))
            .order_by_asc(menu_item::Column::SortOrder)
            .all(tenant.database().read())
            .await?)
    }

    /// Tên của các term (theo ngôn ngữ của menu, thiếu thì mặc định).
    async fn term_refs(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
        locale: &str,
    ) -> Result<HashMap<Uuid, crate::taxonomy::terms::TermRef>, Error> {
        let mut found = HashMap::new();
        for id in ids {
            if let Ok(term) = self.term_for_menu(tenant, *id, locale).await {
                found.insert(*id, term);
            }
        }
        Ok(found)
    }

    async fn term_for_menu(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        locale: &str,
    ) -> Result<crate::taxonomy::terms::TermRef, Error> {
        let texts = crate::entity::term_translation::Entity::find()
            .scoped(tenant)
            .filter(crate::entity::term_translation::Column::TermId.eq(id))
            .all(tenant.database().read())
            .await?;
        let default = self.languages(tenant).await?.default;
        let chosen = texts
            .iter()
            .find(|text| text.locale == locale)
            .or_else(|| texts.iter().find(|text| text.locale == default))
            .or_else(|| texts.first())
            .ok_or_else(|| crate::taxonomy::terms::term_not_found(id))?;
        Ok(crate::taxonomy::terms::TermRef {
            id,
            taxonomy: chosen.taxonomy.clone(),
            name: chosen.name.clone(),
            slug: chosen.slug.clone(),
            locale: chosen.locale.clone(),
        })
    }

    /// Đích của mọi mục có thật: mục nội dung cùng ngôn ngữ, term có thật.
    async fn check_targets(
        &self,
        tenant: &TenantContext,
        flat: &[(MenuItem, String)],
        locale: &str,
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        let entry_ids: Vec<Uuid> =
            flat.iter().filter_map(|(item, _)| item.target_entry_id).collect();
        let entries: HashMap<Uuid, String> = if entry_ids.is_empty() {
            HashMap::new()
        } else {
            entry::Entity::find()
                .scoped(tenant)
                .filter(entry::Column::Id.is_in(entry_ids))
                .all(tenant.database().read())
                .await?
                .into_iter()
                .map(|row| (row.id, row.locale))
                .collect()
        };
        let term_ids: Vec<Uuid> = flat.iter().filter_map(|(item, _)| item.target_term_id).collect();
        let terms: Vec<Uuid> = if term_ids.is_empty() {
            Vec::new()
        } else {
            term::Entity::find()
                .scoped(tenant)
                .filter(term::Column::Id.is_in(term_ids))
                .all(tenant.database().read())
                .await?
                .into_iter()
                .map(|row| row.id)
                .collect()
        };
        for (item, path) in flat {
            if let Some(id) = item.target_entry_id
                && entries.get(&id).map(String::as_str) != Some(locale)
            {
                problems.push(FieldError::new(
                    format!("{path}.entry_id"),
                    "not_found",
                    "no entry in this language has this id",
                ));
            }
            if let Some(id) = item.target_term_id
                && !terms.contains(&id)
            {
                problems.push(FieldError::new(
                    format!("{path}.term_id"),
                    "not_found",
                    "no category or tag has this id",
                ));
            }
        }
        Ok(())
    }
}

/// Dàn cây mục thành danh sách (cha trước con), kiểm từng mục; kèm vị trí trong JSON.
fn flatten(
    tenant: &TenantContext,
    items: &[ItemInput],
    parent: Option<Uuid>,
    depth: usize,
    path: &str,
    flat: &mut Vec<(MenuItem, String)>,
    problems: &mut Vec<FieldError>,
) {
    for (index, input) in items.iter().enumerate() {
        let here = format!("{path}[{index}]");
        if depth > MAX_DEPTH {
            problems.push(
                FieldError::new(here, "too_deep", "menus nest at most {max_depth} levels")
                    .with_param("max_depth", MAX_DEPTH),
            );
            continue;
        }
        let label = input.label.trim();
        if label.is_empty() || label.chars().count() > 200 {
            problems.push(FieldError::new(
                format!("{here}.label"),
                "length",
                "must be 1 to 200 characters",
            ));
        }
        let url = input.url.as_deref().map(str::trim).filter(|url| !url.is_empty());
        let targets = usize::from(input.entry_id.is_some())
            + usize::from(input.term_id.is_some())
            + usize::from(url.is_some());
        if targets != 1 {
            problems.push(FieldError::new(
                format!("{here}.entry_id"),
                "required",
                "give exactly one of `entry_id`, `term_id` and `url`",
            ));
        }
        if let Some(url) = url {
            let lower = url.to_ascii_lowercase();
            let valid = (lower.starts_with("https://")
                || lower.starts_with("http://")
                || lower.starts_with("mailto:")
                || crate::publishing::paths::is_site_link(url))
                && !url.chars().any(|c| c.is_whitespace() || c.is_control());
            if !valid {
                problems.push(FieldError::new(
                    format!("{here}.url"),
                    "invalid",
                    "must be `https://`, `mailto:` or a path starting with a single `/`",
                ));
            }
        }
        let id = Uuid::now_v7();
        flat.push((
            MenuItem {
                id,
                tenant_id: tenant.id().as_uuid(),
                menu_id: Uuid::nil(),
                parent_id: parent,
                sort_order: 0,
                label: label.to_owned(),
                target_entry_id: input.entry_id,
                target_term_id: input.term_id,
                target_url: url.map(str::to_owned),
            },
            here.clone(),
        ));
        flatten(
            tenant,
            &input.children,
            Some(id),
            depth + 1,
            &format!("{here}.children"),
            flat,
            problems,
        );
    }
}

/// Dựng cây từ danh sách mục (đã theo thứ tự).
fn tree(items: Vec<MenuItem>) -> Vec<Node> {
    fn children(parent: Option<Uuid>, items: &[MenuItem]) -> Vec<Node> {
        items
            .iter()
            .filter(|item| item.parent_id == parent)
            .map(|item| Node { item: item.clone(), children: children(Some(item.id), items) })
            .collect()
    }
    children(None, &items)
}

fn public_nodes(
    nodes: &[Node],
    live: &HashMap<Uuid, String>,
    terms: &HashMap<Uuid, crate::taxonomy::terms::TermRef>,
) -> Vec<PublicNode> {
    nodes
        .iter()
        .filter_map(|node| {
            let item = &node.item;
            let (href, term) = match (item.target_entry_id, item.target_term_id, &item.target_url) {
                (Some(id), _, _) => (live.get(&id)?.clone(), None),
                (_, Some(id), _) => {
                    let term = terms.get(&id)?.clone();
                    (String::new(), Some(term))
                }
                (_, _, Some(url)) => (url.clone(), None),
                _ => return None,
            };
            Some(PublicNode {
                label: item.label.clone(),
                href,
                entry_id: item.target_entry_id,
                term,
                children: public_nodes(&node.children, live, terms),
            })
        })
        .collect()
}

fn menu_not_found(key: &str, locale: &str) -> Error {
    Error::not_found("content.menu_not_found", "No menu `{key}` in `{locale}`.")
        .with_param("key", key)
        .with_param("locale", locale)
}

// Menu cũ cho module `menus` nhập sang (D-238). Không kiểm quyền: chỉ module khác gọi, qua lệnh
// của người vận hành máy chủ.
#[async_trait::async_trait]
impl crate::api::LegacyMenus for ContentService {
    async fn legacy_menus(
        &self,
        tenant: &TenantContext,
    ) -> Result<Vec<crate::api::LegacyMenu>, Error> {
        let menus = menu::Entity::find()
            .scoped(tenant)
            .order_by_asc(menu::Column::Key)
            .order_by_asc(menu::Column::Locale)
            .all(tenant.database().read())
            .await?;
        let mut found = Vec::with_capacity(menus.len());
        for row in menus {
            let items = legacy_items(tree(self.menu_items(tenant, row.id).await?));
            found.push(crate::api::LegacyMenu { key: row.key, locale: row.locale, items });
        }
        Ok(found)
    }
}

fn legacy_items(nodes: Vec<Node>) -> Vec<crate::api::LegacyMenuItem> {
    use crate::api::LegacyTarget;
    nodes
        .into_iter()
        .filter_map(|node| {
            let item = node.item;
            let target = match (item.target_entry_id, item.target_term_id, item.target_url) {
                (Some(id), _, _) => LegacyTarget::Entry(id),
                (_, Some(id), _) => LegacyTarget::Term(id),
                (_, _, Some(url)) => LegacyTarget::Url(url),
                _ => return None,
            };
            Some(crate::api::LegacyMenuItem {
                label: item.label,
                target,
                children: legacy_items(node.children),
            })
        })
        .collect()
}
