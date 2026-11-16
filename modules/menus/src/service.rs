//! Service của menu: MỌI logic nằm ở đây. Handler REST và tool MCP chỉ lấy dữ liệu vào và gọi
//! service — nên hai đường cho cùng một kết quả. Service kiểm permission.
//!
//! Mỗi lệnh ghi đi cùng một event trong CÙNG transaction (luật kiến trúc 3). Kiểm đối tượng đích
//! (sổ đối tượng, có thể gọi module khác) chạy TRƯỚC transaction: transaction ghi phải ngắn.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use autoxide_content::api::LegacyMenus;
use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::resource::ResourceDirectory;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, Site, TenantContext, root_of};
use autoxide_media::api::Media;
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, SqlErr,
};
use serde_json::json;
use uuid::Uuid;

use crate::config::{MenusConfig, canonical_locale};
use crate::entity::{item, menu};
use crate::events::{self, Change};
use crate::items::{Flat, ItemInput, Node, flatten, micros, tree};
use crate::permissions::{MENUS_MANAGE, MENUS_READ};
use crate::public::PublicCache;

/// Một menu (dòng của `menus_menus`).
pub(crate) type Menu = menu::Model;

/// Dữ liệu để tạo menu.
#[derive(Debug, Clone, Default)]
pub(crate) struct NewMenu {
    pub key: String,
    pub locale: Option<String>,
    /// Website (nút) của menu; `None`: website của cả khách (nút gốc).
    pub org_id: Option<Uuid>,
    pub items: Vec<ItemInput>,
}

/// Những thứ service dùng từ bên ngoài module.
pub(crate) struct Dependencies {
    /// Sổ đối tượng của kernel: kiểm và giải đích của mục.
    pub resources: ResourceDirectory,
    /// Ảnh của ô gợi ý.
    pub media: Arc<dyn Media>,
    /// Menu cũ của `content`, khi có module đó (lệnh nhập, D-238).
    pub legacy: Option<Arc<dyn LegacyMenus>>,
}

pub(crate) struct MenuService {
    outbox: Outbox,
    pub(crate) resources: ResourceDirectory,
    pub(crate) media: Arc<dyn Media>,
    pub(crate) legacy: Option<Arc<dyn LegacyMenus>>,
    pub(crate) config: MenusConfig,
    pub(crate) cache: PublicCache,
}

impl MenuService {
    pub(crate) fn new(outbox: Outbox, dependencies: Dependencies, config: MenusConfig) -> Self {
        MenuService {
            outbox,
            resources: dependencies.resources,
            media: dependencies.media,
            legacy: dependencies.legacy,
            config,
            cache: PublicCache::default(),
        }
    }

    /// Một trang menu, theo ID (mặc định mới tạo trước), kèm số mục của từng menu (một câu
    /// đếm cho cả trang, không đếm từng menu).
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        owner_org: Option<Uuid>,
        page: &PageRequest,
    ) -> Result<Page<(Menu, u64)>, Error> {
        principal.require(&MENUS_READ)?;
        let read = tenant.database().read();
        let scope = principal.scope_condition(&MENUS_READ, menu::Column::OrgId, None)?;
        let mut select = menu::Entity::find().filter(scope).scoped(tenant);
        if let Some(org) = owner_org {
            select = select.filter(menu::Column::OrgId.eq(org));
        }
        let page = fetch_page(select, menu::Column::Id, |row| row.id, page, read).await?;
        let ids: Vec<Uuid> = page.data.iter().map(|menu| menu.id).collect();
        let counts: HashMap<Uuid, i64> = if ids.is_empty() {
            HashMap::new()
        } else {
            item::Entity::find()
                .scoped(tenant)
                .select_only()
                .column(item::Column::MenuId)
                .column_as(item::Column::Id.count(), "items")
                .filter(item::Column::MenuId.is_in(ids))
                .group_by(item::Column::MenuId)
                .into_tuple::<(Uuid, i64)>()
                .all(read)
                .await?
                .into_iter()
                .collect()
        };
        Ok(page.map(|menu| {
            let count = counts.get(&menu.id).copied().unwrap_or(0);
            (menu, u64::try_from(count).unwrap_or(0))
        }))
    }

    /// Menu `id` kèm cây mục như đã lưu.
    pub(crate) async fn get(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(Menu, Vec<Node>), Error> {
        principal.require(&MENUS_READ)?;
        let found = self.find(tenant, id).await?;
        let read = tenant.database().read();
        hidden(principal.require_at(read, &MENUS_READ, target(tenant, &found)).await, id)?;
        let items = self.items_of(read, tenant, id).await?;
        Ok((found, tree(items)))
    }

    /// Tạo menu (`key`, `locale`) với cây mục `items`. Đã có menu cùng khoá và ngôn ngữ thì
    /// `menus.menu_exists`.
    pub(crate) async fn create(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewMenu,
    ) -> Result<(Menu, Vec<Node>), Error> {
        principal.require(&MENUS_MANAGE)?;
        self.create_checked(tenant, Some(principal), input, true).await
    }

    /// Như [`create`](Self::create), không kiểm permission. `check_targets`: hỏi sổ đối tượng
    /// xem đích có thật không (lệnh nhập menu cũ bỏ bước này: đích đã mất thì menu công khai tự
    /// ẩn mục đó, không làm hỏng cả lần nhập).
    pub(crate) async fn create_checked(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        input: NewMenu,
        check_targets: bool,
    ) -> Result<(Menu, Vec<Node>), Error> {
        let actor = principal.map_or_else(Actor::system, Actor::from);
        let org = input.org_id.map_or(root_of(tenant.id()), OrgId::from_uuid);
        let mut problems = Vec::new();
        let locale = self.check_locale(input.locale.as_deref(), &mut problems);
        let key = input.key.trim().to_owned();
        if !is_key(&key) {
            problems.push(FieldError::new(
                "key",
                "invalid",
                "must be 1 to 64 lower-case letters, digits, `-` or `_`",
            ));
        }
        let flat = flatten(&input.items, &mut problems);
        // Menu mới chưa có mục nào để giữ id.
        for entry in flat.iter().filter(|entry| entry.kept) {
            problems.push(FieldError::new(
                format!("{}.id", entry.path),
                "invalid",
                "must be left out when creating a menu",
            ));
        }
        self.check_targets(tenant, &flat, &locale, check_targets, &mut problems).await?;
        if !problems.is_empty() {
            return Err(invalid(problems));
        }
        let now = micros(Utc::now());
        let transaction = tenant.database().begin_write().await?;
        // Website do người gọi nêu: ngoài phạm vi → 403; nút đang khoá → `org_frozen`.
        if let Some(principal) = principal {
            let at = OrgTarget::new(org, None);
            principal.require_write_at(&transaction, &MENUS_MANAGE, at).await?;
        }
        let saved = menu::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant.id().as_uuid(),
            org_id: Some(org.as_uuid()),
            key: key.clone(),
            locale: locale.clone(),
            created_at: now,
            updated_at: now,
            updated_by: actor.user_id,
        }
        .into_active_model()
        .insert(&transaction)
        .await
        .map_err(|error| {
            if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) {
                menu_exists(&key, &locale)
            } else {
                error.into()
            }
        })?;
        let rows = insert_items(&transaction, tenant, saved.id, flat).await?;
        let event =
            events::MenuCreated(Change { before: None, after: Some(snapshot(&saved, rows.len())) });
        self.outbox
            .record(&transaction, tenant, actor, events::menu_subject(saved.id), &event)
            .await?;
        transaction.commit().await?;
        self.cache.clear(tenant.id());
        tracing::info!(menu_id = %saved.id, items = rows.len(), "menu created");
        Ok((saved, tree(rows)))
    }

    /// Thay cả cây mục của menu `id`. `expected`: phiên bản người gọi đã đọc (`If-Match`);
    /// `None` khi họ gửi `*`. Menu đã bị sửa từ lúc đó thì 412.
    pub(crate) async fn replace(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        items: Vec<ItemInput>,
        expected: Option<DateTime<Utc>>,
    ) -> Result<(Menu, Vec<Node>), Error> {
        principal.require(&MENUS_MANAGE)?;
        let found = self.find(tenant, id).await?;
        let read = tenant.database().read();
        hidden(principal.require_at(read, &MENUS_READ, target(tenant, &found)).await, id)?;
        let mut problems = Vec::new();
        let flat = flatten(&items, &mut problems);
        // Chỉ giữ được id của mục đang thuộc chính menu này.
        if flat.iter().any(|entry| entry.kept) {
            let current: std::collections::HashSet<Uuid> = item::Entity::find()
                .scoped(tenant)
                .select_only()
                .column(item::Column::Id)
                .filter(item::Column::MenuId.eq(id))
                .into_tuple::<Uuid>()
                .all(tenant.database().read())
                .await?
                .into_iter()
                .collect();
            for entry in flat.iter().filter(|entry| entry.kept && !current.contains(&entry.row.id))
            {
                problems.push(FieldError::new(
                    format!("{}.id", entry.path),
                    "not_found",
                    "is not an item of this menu; leave `id` out for a new item",
                ));
            }
        }
        self.check_targets(tenant, &flat, &found.locale, true, &mut problems).await?;
        if !problems.is_empty() {
            return Err(invalid(problems));
        }
        let now = micros(Utc::now());
        let transaction = tenant.database().begin_write().await?;
        principal.require_write_at(&transaction, &MENUS_MANAGE, target(tenant, &found)).await?;
        // Kiểm phiên bản trong chính câu UPDATE: hai người sửa cùng lúc thì một người nhận 412.
        let editor = Actor::from(principal).user_id;
        let updated = menu::Entity::update_many()
            .col_expr(menu::Column::UpdatedAt, sea_orm::sea_query::Expr::value(now))
            .col_expr(menu::Column::UpdatedBy, sea_orm::sea_query::Expr::value(editor))
            .scoped(tenant)
            .filter(menu::Column::Id.eq(id))
            .filter(menu::Column::UpdatedAt.eq(expected.unwrap_or(found.updated_at)))
            .exec(&transaction)
            .await?;
        if updated.rows_affected == 0 {
            // Không còn menu (vừa bị xoá) hoặc đã bị sửa.
            return Err(
                match menu::Entity::find_by_id(id).scoped(tenant).one(&transaction).await? {
                    Some(_) => autoxide_kernel::version::changed(),
                    None => menu_not_found(id),
                },
            );
        }
        let before = item::Entity::find()
            .scoped(tenant)
            .filter(item::Column::MenuId.eq(id))
            .count(&transaction)
            .await?;
        item::Entity::delete_many()
            .scoped(tenant)
            .filter(item::Column::MenuId.eq(id))
            .exec(&transaction)
            .await?;
        let rows = insert_items(&transaction, tenant, id, flat).await?;
        let saved = Menu { updated_at: now, updated_by: editor, ..found };
        let event = events::MenuReplaced(Change {
            before: Some(snapshot(&saved, usize::try_from(before).unwrap_or(usize::MAX))),
            after: Some(snapshot(&saved, rows.len())),
        });
        self.outbox
            .record(&transaction, tenant, Actor::from(principal), events::menu_subject(id), &event)
            .await?;
        transaction.commit().await?;
        self.cache.clear(tenant.id());
        tracing::info!(menu_id = %id, items = rows.len(), "menu replaced");
        Ok((saved, tree(rows)))
    }

    /// Xoá menu `id` cùng mọi mục của nó.
    pub(crate) async fn delete(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&MENUS_MANAGE)?;
        let transaction = tenant.database().begin_write().await?;
        let found = menu::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| menu_not_found(id))?;
        hidden(principal.require_at(&transaction, &MENUS_READ, target(tenant, &found)).await, id)?;
        principal.require_write_at(&transaction, &MENUS_MANAGE, target(tenant, &found)).await?;
        let count = item::Entity::delete_many()
            .scoped(tenant)
            .filter(item::Column::MenuId.eq(id))
            .exec(&transaction)
            .await?
            .rows_affected;
        menu::Entity::delete_many()
            .scoped(tenant)
            .filter(menu::Column::Id.eq(id))
            .exec(&transaction)
            .await?;
        let event = events::MenuDeleted(Change {
            before: Some(snapshot(&found, usize::try_from(count).unwrap_or(usize::MAX))),
            after: None,
        });
        self.outbox
            .record(&transaction, tenant, Actor::from(principal), events::menu_subject(id), &event)
            .await?;
        transaction.commit().await?;
        self.cache.clear(tenant.id());
        tracing::info!(menu_id = %id, "menu deleted");
        Ok(())
    }

    /// Menu theo khoá và ngôn ngữ của website `site` (đọc công khai, nhập menu cũ): menu của
    /// chính website, không có thì của website cấp trên gần nhất (menu của cả khách dùng chung cho
    /// mọi website chưa có menu riêng).
    pub(crate) async fn find_by_key(
        &self,
        tenant: &TenantContext,
        site: &Site,
        key: &str,
        locale: &str,
    ) -> Result<Option<Menu>, Error> {
        let found = menu::Entity::find()
            .scoped(tenant)
            .filter(menu::Column::Key.eq(key))
            .filter(menu::Column::Locale.eq(locale))
            .filter(menu::Column::OrgId.is_in(site.lineage_ids()))
            .all(tenant.database().read())
            .await?;
        let root = root_of(tenant.id()).as_uuid();
        let nearest = autoxide_kernel::tenancy::nearest(
            site.lineage(),
            found.into_iter().map(|menu| (OrgId::from_uuid(menu.org_id.unwrap_or(root)), menu)),
        );
        Ok(nearest.map(|(_, menu)| menu))
    }

    pub(crate) async fn items_of<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        menu_id: Uuid,
    ) -> Result<Vec<item::Model>, Error> {
        Ok(item::Entity::find()
            .scoped(tenant)
            .filter(item::Column::MenuId.eq(menu_id))
            .order_by_asc(item::Column::Position)
            .all(connection)
            .await?)
    }

    async fn find(&self, tenant: &TenantContext, id: Uuid) -> Result<Menu, Error> {
        menu::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| menu_not_found(id))
    }

    /// Ngôn ngữ đã chuẩn hoá; không gửi thì ngôn ngữ mặc định.
    pub(crate) fn check_locale(&self, raw: Option<&str>, problems: &mut Vec<FieldError>) -> String {
        let Some(raw) = raw else {
            return self.config.default_locale.clone();
        };
        match canonical_locale(raw.trim()).filter(|locale| self.config.locales.contains(locale)) {
            Some(locale) => locale,
            None => {
                problems.push(
                    FieldError::new("locale", "locale_not_supported", "must be one of: {allowed}")
                        .with_param("allowed", self.config.locales.join(", ")),
                );
                self.config.default_locale.clone()
            }
        }
    }

    /// Đích và ô gợi ý của mọi mục: loại đã đăng ký, đối tượng có thật (khi `check_exists`),
    /// nhóm con tự lấy chỉ trên loại có cây. Gom lỗi vào `problems`.
    async fn check_targets(
        &self,
        tenant: &TenantContext,
        flat: &[Flat],
        locale: &str,
        check_exists: bool,
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        // (đường dẫn trong JSON, loại, ID)
        let mut wanted: Vec<(String, &str, &str)> = Vec::new();
        for entry in flat {
            let row = &entry.row;
            if let (Some(kind), Some(id)) = (&row.ref_kind, &row.ref_id) {
                wanted.push((format!("{}.ref", entry.path), kind, id));
                if row.auto_children
                    && self.resources.knows(kind)
                    && !self.resources.has_children(kind)
                {
                    problems.push(
                        FieldError::new(
                            format!("{}.auto_children", entry.path),
                            "children_unsupported",
                            "`{kind}` has no sub-items to take",
                        )
                        .with_param("kind", kind),
                    );
                }
            }
            if let (Some(kind), Some(id)) = (&row.feature_kind, &row.feature_id) {
                wanted.push((format!("{}.feature", entry.path), kind, id));
            }
        }
        let mut by_kind: HashMap<&str, Vec<String>> = HashMap::new();
        for (path, kind, id) in &wanted {
            if self.resources.knows(kind) {
                by_kind.entry(kind).or_default().push((*id).to_owned());
            } else {
                problems.push(
                    FieldError::new(
                        format!("{path}.kind"),
                        "ref_unknown_kind",
                        "no module provides `{kind}`; known: {known}",
                    )
                    .with_param("kind", kind)
                    .with_param("known", self.resources.types().join(", ")),
                );
            }
        }
        if !check_exists {
            return Ok(());
        }
        let mut found: HashSet<(String, String)> = HashSet::new();
        for (kind, ids) in by_kind {
            let resolved = self.resources.resolve_many(tenant, kind, &ids, Some(locale)).await?;
            found.extend(resolved.into_keys().map(|id| (kind.to_owned(), id)));
        }
        for (path, kind, id) in &wanted {
            if self.resources.knows(kind)
                && !found.contains(&((*kind).to_owned(), (*id).to_owned()))
            {
                problems.push(
                    FieldError::new(
                        format!("{path}.id"),
                        "ref_not_found",
                        "no `{kind}` has this id",
                    )
                    .with_param("kind", kind),
                );
            }
        }
        Ok(())
    }
}

/// Ghi các dòng mục của menu `menu_id` của khách `tenant` (trong transaction của người gọi).
async fn insert_items<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    menu_id: Uuid,
    flat: Vec<Flat>,
) -> Result<Vec<item::Model>, Error> {
    let tenant_id = tenant.id().as_uuid();
    let rows: Vec<item::Model> =
        flat.into_iter().map(|entry| item::Model { tenant_id, menu_id, ..entry.row }).collect();
    if !rows.is_empty() {
        item::Entity::insert_many(rows.iter().cloned().map(IntoActiveModel::into_active_model))
            .exec(connection)
            .await?;
    }
    Ok(rows)
}

/// Khoá menu: 1 tới 64 chữ thường, số, `-`, `_` (vd `main`, `footer`).
fn is_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 64
        && key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn snapshot(menu: &Menu, items: usize) -> serde_json::Value {
    json!({"key": menu.key, "locale": menu.locale, "items": items})
}

/// Lỗi kiểm dữ liệu với mã phù hợp: mọi lỗi cùng một loại đích thì mã riêng của loại đó, để máy
/// nhận ra ngay (D-083); còn lại `validation_failed`. Lỗi từng chỗ nằm trong `errors`.
fn invalid(problems: Vec<FieldError>) -> Error {
    let all = |code: &str| problems.iter().all(|problem| problem.code == code);
    // `ErrorKind` đứng ngay trước mã để `cargo xtask check` nhận ra đây là chỗ tạo lỗi.
    let (kind, code, message) = if all("ref_unknown_kind") {
        (
            ErrorKind::Validation,
            "menus.ref_unknown_kind",
            "An item points to a kind of object no module provides; see `errors`.",
        )
    } else if all("ref_not_found") {
        (
            ErrorKind::Validation,
            "menus.ref_not_found",
            "An item points to an object that does not exist; see `errors`.",
        )
    } else if all("children_unsupported") {
        (
            ErrorKind::Validation,
            "menus.children_unsupported",
            "An item takes sub-items from a kind of object that has none; see `errors`.",
        )
    } else {
        return Error::validation(problems);
    };
    let mut error = Error::new(kind, code, message);
    for problem in problems {
        error = error.with_field(problem);
    }
    error
}

/// Đích kiểm quyền của menu: website (nút) của nó.
fn target(tenant: &TenantContext, menu: &Menu) -> OrgTarget {
    OrgTarget::new(menu.org_id.map_or(root_of(tenant.id()), OrgId::from_uuid), None)
}

/// Menu ngoài phạm vi đọc báo như không có.
fn hidden(result: Result<(), Error>, id: Uuid) -> Result<(), Error> {
    result.map_err(
        |error| if error.code() == "org_out_of_scope" { menu_not_found(id) } else { error },
    )
}

pub(crate) fn menu_not_found(id: Uuid) -> Error {
    Error::not_found("menus.menu_not_found", "There is no menu `{id}`.").with_param("id", id)
}

fn menu_exists(key: &str, locale: &str) -> Error {
    Error::conflict(
        "menus.menu_exists",
        "A menu `{key}` in `{locale}` already exists; replace its items instead of creating it again.",
    )
    .with_param("key", key)
    .with_param("locale", locale)
}
