//! Service của role: tạo, sửa, xoá role; gán role cho người dùng; tính permission
//! của một người dùng.

use std::collections::{BTreeSet, HashMap};

use autoxide_kernel::access::{Grant, PermissionCatalog, Principal, Scope};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::{Expr, ExprTrait as _, Query};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder, Set, SqlErr,
};
use uuid::Uuid;

use super::entity::{role, role_permission, user_role};
use crate::events;
use crate::user::entity as user;
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext, root_of};
use serde_json::json;

/// Role đã kiểm: tên, mô tả, permission kèm độ sâu.
type CheckedRole = (String, String, Vec<(String, Scope)>);

const NAME_MAX_LENGTH: usize = 64;
const DESCRIPTION_MAX_LENGTH: usize = 500;
/// Đủ cho mọi permission của một hệ thống lớn; chặn danh sách khổng lồ.
const MAX_PERMISSIONS: usize = 1000;
/// Số role tối đa gán cho một người dùng trong một lần.
const MAX_ROLES_PER_USER: usize = 100;

/// Một role kèm các permission của nó.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Role {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    /// Có mọi permission (role `admin`); khi đó `permissions` rỗng.
    pub all_permissions: bool,
    /// Quyền của role chỉ có hiệu lực khi tài khoản đã bật MFA (D-308).
    pub requires_mfa: bool,
    /// Theo thứ tự tên, mọi độ sâu.
    pub permissions: Vec<String>,
    /// Permission kèm độ sâu (bước 34), theo thứ tự tên.
    pub grants: Vec<(String, Scope)>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Dữ liệu để tạo hay thay toàn bộ một role.
#[derive(Debug, Clone)]
pub(crate) struct RoleInput {
    pub name: String,
    pub description: String,
    /// Permission ở độ sâu `tenant`.
    pub permissions: Vec<String>,
    /// Permission kèm độ sâu (bước 34); một permission chỉ được nêu một lần trong cả hai danh sách.
    pub grants: Vec<(String, Scope)>,
    /// Xem [`Role::requires_mfa`].
    pub requires_mfa: bool,
}

/// Bộ lọc danh sách membership.
#[derive(Debug, Clone, Default)]
pub(crate) struct MembershipFilter {
    pub org_id: Option<Uuid>,
    pub role_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    pub inherited: bool,
}

/// Một dòng membership (người, nút, role).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MembershipRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub org_id: Uuid,
    pub role_id: Uuid,
    pub role_name: String,
    /// Membership ở một nút trên nút đang hỏi.
    pub inherited: bool,
    pub created_at: DateTime<Utc>,
}

/// Một membership: người làm ở nút `org_id` với các role này.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Membership {
    pub user_id: Uuid,
    pub org_id: Uuid,
    pub roles: Vec<Role>,
}

pub(crate) struct RoleService {
    catalog: PermissionCatalog,
    outbox: Outbox,
    /// Bản dịch mô tả permission (D-264); mặc định rỗng (tiếng Anh).
    translations: autoxide_kernel::i18n::Translations,
}

impl RoleService {
    pub(crate) fn new(catalog: PermissionCatalog, outbox: Outbox) -> Self {
        RoleService { catalog, outbox, translations: Default::default() }
    }

    /// Dịch mô tả permission bằng `translations`.
    pub(crate) fn with_translations(
        mut self,
        translations: autoxide_kernel::i18n::Translations,
    ) -> Self {
        self.translations = translations;
        self
    }

    /// Mô tả permission `english` bằng `language`.
    pub(crate) fn describe(
        &self,
        english: &str,
        language: autoxide_kernel::i18n::Language,
    ) -> String {
        self.translations.describe(english, language)
    }

    /// Mọi permission của chương trình.
    pub(crate) fn catalog(&self) -> &PermissionCatalog {
        &self.catalog
    }

    /// Một trang role, mới tạo trước (mặc định).
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        page: &PageRequest,
    ) -> Result<Page<Role>, Error> {
        let connection = tenant.database().read();
        let select = role::Entity::find().scoped(tenant);
        let page = fetch_page(select, role::Column::Id, |row| row.id, page, connection).await?;
        let mut permissions =
            permissions_of(connection, tenant, page.data.iter().map(|r| r.id)).await?;
        Ok(page.map(|row| {
            let granted = permissions.remove(&row.id).unwrap_or_default();
            to_role(row, granted)
        }))
    }

    pub(crate) async fn get(&self, tenant: &TenantContext, id: Uuid) -> Result<Role, Error> {
        let connection = tenant.database().read();
        let row = role::Entity::find_by_id(id)
            .scoped(tenant)
            .one(connection)
            .await?
            .ok_or_else(|| role_not_found(id))?;
        let mut permissions = permissions_of(connection, tenant, [id]).await?;
        Ok(to_role(row, permissions.remove(&id).unwrap_or_default()))
    }

    pub(crate) async fn create(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        input: RoleInput,
    ) -> Result<Role, Error> {
        let requires_mfa = input.requires_mfa;
        let (name, description, permissions) = self.check(input)?;
        let now = now();
        let id = Uuid::now_v7();
        let transaction = tenant.database().begin_write().await?;
        role::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            name: Set(name.clone()),
            description: Set(description.clone()),
            all_permissions: Set(false),
            requires_mfa: Set(requires_mfa),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await
        .map_err(|error| name_taken_or(error, &name))?;
        insert_permissions(&transaction, tenant, id, &permissions).await?;
        let mut after = role_json(&name, &description, &permissions);
        after["requires_mfa"] = json!(requires_mfa);
        let after = Some(after);
        let event = events::RoleCreated { before: None, after };
        self.outbox.record(&transaction, tenant, actor, role_subject(id), &event).await?;
        transaction.commit().await?;
        tracing::info!(role_id = %id, "role created");
        self.get(tenant, id).await
    }

    /// Thay toàn bộ tên, mô tả và permission của role `id`.
    pub(crate) async fn update(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        input: RoleInput,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Role, Error> {
        let requires_mfa = input.requires_mfa;
        let (name, description, permissions) = self.check(input)?;
        let transaction = tenant.database().begin_write().await?;
        let current = role::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| role_not_found(id))?;
        // Role `admin` dựng sẵn: chỉ đổi được `requires_mfa` (tên, mô tả giữ nguyên, không
        // permission nào) — bảo vệ tài khoản quyền cao nhất là chỗ cần MFA nhất (D-308).
        let only_mfa = current.all_permissions
            && name == current.name
            && description == current.description
            && permissions.is_empty();
        if current.all_permissions && !only_mfa {
            return Err(builtin_role());
        }
        let before = role_snapshot(&transaction, tenant, &current).await?;
        let mut update = role::Entity::update_many()
            .col_expr(role::Column::RequiresMfa, Expr::value(requires_mfa))
            .col_expr(role::Column::Name, Expr::value(name.clone()))
            .col_expr(role::Column::Description, Expr::value(description.clone()))
            .col_expr(role::Column::UpdatedAt, Expr::value(now()))
            .filter(role::Column::Id.eq(id))
            .scoped(tenant);
        if let Some(expected) = expected {
            // Kiểm phiên bản ngay trong câu `UPDATE` (D-084).
            update = update.filter(role::Column::UpdatedAt.eq(expected));
        }
        let updated =
            update.exec(&transaction).await.map_err(|error| name_taken_or(error, &name))?;
        if updated.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(autoxide_kernel::version::changed());
        }
        role_permission::Entity::delete_many()
            .filter(role_permission::Column::RoleId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        insert_permissions(&transaction, tenant, id, &permissions).await?;
        let mut after = role_json(&name, &description, &permissions);
        after["requires_mfa"] = json!(requires_mfa);
        let event = events::RoleUpdated { before: Some(before), after: Some(after) };
        self.outbox.record(&transaction, tenant, actor, role_subject(id), &event).await?;
        transaction.commit().await?;
        tracing::info!(role_id = %id, "role changed");
        self.get(tenant, id).await
    }

    /// Xoá role `id`; người dùng đang có role này mất nó.
    pub(crate) async fn delete(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let current = role::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| role_not_found(id))?;
        if current.all_permissions {
            return Err(builtin_role());
        }
        let before = role_snapshot(&transaction, tenant, &current).await?;
        role::Entity::delete_by_id(id).scoped(tenant).exec(&transaction).await?;
        let event = events::RoleDeleted { before: Some(before), after: None };
        self.outbox.record(&transaction, tenant, actor, role_subject(id), &event).await?;
        transaction.commit().await?;
        tracing::info!(role_id = %id, "role deleted");
        Ok(())
    }

    /// Role (id, tên) của từng người trong `user_ids`, theo tên — một câu truy vấn cho cả trang
    /// (`GET /auth/users?include=roles`, đề bài giao diện B1).
    pub(crate) async fn names_of_users(
        &self,
        tenant: &TenantContext,
        user_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<(Uuid, String)>>, Error> {
        let mut by_user: HashMap<Uuid, Vec<(Uuid, String)>> = HashMap::new();
        if user_ids.is_empty() {
            return Ok(by_user);
        }
        let select = Query::select()
            .column((user_role::Entity, user_role::Column::UserId))
            .column((role::Entity, role::Column::Id))
            .column((role::Entity, role::Column::Name))
            .from(user_role::Entity)
            .inner_join(
                role::Entity,
                Expr::col((role::Entity, role::Column::Id))
                    .equals((user_role::Entity, user_role::Column::RoleId)),
            )
            .and_where(
                Expr::col((user_role::Entity, user_role::Column::UserId))
                    .is_in(user_ids.iter().copied()),
            )
            .and_where(
                Expr::col((user_role::Entity, user_role::Column::TenantId))
                    .eq(tenant.id().as_uuid()),
            )
            .order_by((role::Entity, role::Column::Name), sea_orm::sea_query::Order::Asc)
            .to_owned();
        for row in tenant.database().read().query_all(&select).await? {
            let user_id: Uuid = row.try_get("", "user_id")?;
            let role_id: Uuid = row.try_get("", "id")?;
            let name: String = row.try_get("", "name")?;
            by_user.entry(user_id).or_default().push((role_id, name));
        }
        Ok(by_user)
    }

    /// Các role của người dùng `user_id`, theo thứ tự tên.
    pub(crate) async fn user_roles(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Vec<Role>, Error> {
        let connection = tenant.database().read();
        if user::Entity::find_by_id(user_id).scoped(tenant).one(connection).await?.is_none() {
            return Err(crate::user::user_not_found(user_id));
        }
        roles_of_user(connection, tenant, user_id).await
    }

    /// Thay toàn bộ role của người dùng `user_id` bằng `role_ids`.
    pub(crate) async fn set_user_roles(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        user_id: Uuid,
        role_ids: Vec<Uuid>,
    ) -> Result<Vec<Role>, Error> {
        let role_ids: BTreeSet<Uuid> = role_ids.into_iter().collect();
        if role_ids.len() > MAX_ROLES_PER_USER {
            return Err(Error::validation(vec![
                FieldError::new(
                    "role_ids",
                    "length",
                    "must list at most {max_roles_per_user} roles",
                )
                .with_param("max_roles_per_user", MAX_ROLES_PER_USER),
            ]));
        }
        let transaction = tenant.database().begin_write().await?;
        // Cập nhật dòng người dùng trước: khoá dòng đó tới hết transaction, nên hai
        // request gán role cho cùng một người chạy lần lượt (Postgres); SQLite vốn
        // chỉ một bên ghi. Đồng thời biết người dùng có tồn tại không.
        let touched = user::Entity::update_many()
            .col_expr(user::Column::UpdatedAt, Expr::value(now()))
            .filter(user::Column::Id.eq(user_id))
            // Người mua không có role (D-309): như không có người dùng này.
            .filter(user::Column::Realm.eq("staff"))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if touched.rows_affected == 0 {
            return Err(crate::user::user_not_found(user_id));
        }
        let found = role::Entity::find()
            .filter(role::Column::Id.is_in(role_ids.iter().copied()))
            .scoped(tenant)
            .count(&transaction)
            .await?;
        if found != role_ids.len() as u64 {
            return Err(Error::validation(vec![FieldError::new(
                "role_ids",
                "unknown",
                "every entry must be the id of an existing role",
            )]));
        }
        let admins_before = count_active_admins(&transaction, tenant).await?;
        let before = role_names(&roles_of_user(&transaction, tenant, user_id).await?);
        // Đường cũ (trước bước 34): role ở nút gốc. Membership ở nút khác giữ nguyên.
        let root = root_of(tenant.id()).as_uuid();
        user_role::Entity::delete_many()
            .filter(user_role::Column::UserId.eq(user_id))
            .filter(user_role::Column::OrgId.eq(root))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let now = now();
        for role_id in &role_ids {
            user_role::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                user_id: Set(user_id),
                role_id: Set(*role_id),
                org_id: Set(Some(root)),
                created_at: Set(now),
            }
            .insert(&transaction)
            .await?;
        }
        // Chỉ chặn khi thay đổi này lấy đi admin cuối cùng. Lúc mới cài chưa có admin
        // nào thì việc gán role vẫn được phép.
        if admins_before > 0 && count_active_admins(&transaction, tenant).await? == 0 {
            transaction.rollback().await?;
            return Err(last_admin());
        }
        let after = roles_of_user(&transaction, tenant, user_id).await?;
        let event =
            events::UserRolesChanged { before: Some(before), after: Some(role_names(&after)) };
        self.outbox
            .record(&transaction, tenant, actor, events::user_subject(user_id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, roles = role_ids.len(), "user roles changed");
        Ok(after)
    }

    /// Mọi membership của người dùng `user_id`: mỗi nút một dòng, kèm role ở nút đó; nút theo ID.
    pub(crate) async fn memberships(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Vec<Membership>, Error> {
        let connection = tenant.database().read();
        if user::Entity::find_by_id(user_id).scoped(tenant).one(connection).await?.is_none() {
            return Err(crate::user::user_not_found(user_id));
        }
        memberships_of(connection, tenant, user_id).await
    }

    /// Thay mọi membership của người dùng `user_id` bằng `memberships` (nút → role). Người gán
    /// phải làm việc ở từng nút và có mọi permission của role mình gán; role `admin` chỉ gán ở
    /// nút gốc.
    pub(crate) async fn set_memberships(
        &self,
        tenant: &TenantContext,
        actor: &Principal,
        user_id: Uuid,
        memberships: Vec<(Uuid, Vec<Uuid>)>,
    ) -> Result<Vec<Membership>, Error> {
        let mut problems = Vec::new();
        let mut wanted: Vec<(Uuid, BTreeSet<Uuid>)> = Vec::new();
        for (index, (org_id, role_ids)) in memberships.into_iter().enumerate() {
            if wanted.iter().any(|(org, _)| *org == org_id) {
                problems.push(FieldError::new(
                    format!("memberships[{index}].org_id"),
                    "duplicate",
                    "each unit may appear once; list all its roles together",
                ));
            }
            let roles: BTreeSet<Uuid> = role_ids.into_iter().collect();
            if roles.len() > MAX_ROLES_PER_USER {
                problems.push(
                    FieldError::new(
                        format!("memberships[{index}].role_ids"),
                        "length",
                        "must list at most {max_roles_per_user} roles",
                    )
                    .with_param("max_roles_per_user", MAX_ROLES_PER_USER),
                );
            }
            wanted.push((org_id, roles));
        }
        if wanted.len() > MAX_ROLES_PER_USER {
            problems.push(
                FieldError::new("memberships", "length", "must list at most {max} units")
                    .with_param("max", MAX_ROLES_PER_USER),
            );
        }
        // Lỗi ở trên chưa trả ngay: kiểm tiếp nút và role để báo mọi lỗi một lần (D-083).
        let transaction = tenant.database().begin_write().await?;
        // Khoá dòng người dùng (như `set_user_roles`): hai lần gán cho cùng người nối đuôi nhau.
        let touched = user::Entity::update_many()
            .col_expr(user::Column::UpdatedAt, Expr::value(now()))
            .filter(user::Column::Id.eq(user_id))
            // Người mua không có role (D-309): như không có người dùng này.
            .filter(user::Column::Realm.eq("staff"))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if touched.rows_affected == 0 {
            return Err(crate::user::user_not_found(user_id));
        }
        for (index, (org_id, roles)) in wanted.iter().enumerate() {
            let field = format!("memberships[{index}].");
            problems
                .extend(grant_problems(&transaction, tenant, actor, &field, *org_id, roles).await?);
        }
        if !problems.is_empty() {
            transaction.rollback().await?;
            return Err(Error::validation(problems));
        }
        let admins_before = count_active_admins(&transaction, tenant).await?;
        let before = memberships_json(&memberships_of(&transaction, tenant, user_id).await?);
        user_role::Entity::delete_many()
            .filter(user_role::Column::UserId.eq(user_id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let now = now();
        for (org_id, roles) in &wanted {
            for role_id in roles {
                user_role::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    user_id: Set(user_id),
                    role_id: Set(*role_id),
                    org_id: Set(Some(*org_id)),
                    created_at: Set(now),
                }
                .insert(&transaction)
                .await?;
            }
        }
        if admins_before > 0 && count_active_admins(&transaction, tenant).await? == 0 {
            transaction.rollback().await?;
            return Err(last_admin());
        }
        let after = memberships_of(&transaction, tenant, user_id).await?;
        let event = events::MembershipsChanged {
            before: Some(before),
            after: Some(memberships_json(&after)),
        };
        self.outbox
            .record(&transaction, tenant, Actor::from(actor), events::user_subject(user_id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, units = after.len(), "memberships changed");
        Ok(after)
    }

    /// Một trang membership (người, nút, role) của khách, lọc theo nút / role. `inherited`: kể
    /// cả membership ở các nút TRÊN `org_id` (người có quyền ở nút đó nhờ nút cha).
    pub(crate) async fn list_memberships(
        &self,
        tenant: &TenantContext,
        filter: &MembershipFilter,
        page: &PageRequest,
    ) -> Result<Page<MembershipRow>, Error> {
        let connection = tenant.database().read();
        let root = root_of(tenant.id()).as_uuid();
        let mut select = user_role::Entity::find().scoped(tenant);
        if let Some(org_id) = filter.org_id {
            let orgs: Vec<Uuid> = if filter.inherited {
                let lineage =
                    OrgTree::ancestors(connection, tenant.id(), OrgId::from_uuid(org_id)).await?;
                let mut orgs: Vec<Uuid> = lineage.iter().map(OrgId::as_uuid).collect();
                if !orgs.contains(&root) {
                    orgs.push(root);
                }
                if !orgs.contains(&org_id) {
                    orgs.push(org_id);
                }
                orgs
            } else {
                vec![org_id]
            };
            select = select.filter(user_role::Column::OrgId.is_in(orgs));
        }
        if let Some(role_id) = filter.role_id {
            select = select.filter(user_role::Column::RoleId.eq(role_id));
        }
        if let Some(user_id) = filter.user_id {
            select = select.filter(user_role::Column::UserId.eq(user_id));
        }
        let page =
            fetch_page(select, user_role::Column::Id, |row| row.id, page, connection).await?;
        let names: HashMap<Uuid, String> = role::Entity::find()
            .filter(role::Column::Id.is_in(page.data.iter().map(|row| row.role_id)))
            .scoped(tenant)
            .all(connection)
            .await?
            .into_iter()
            .map(|row| (row.id, row.name))
            .collect();
        Ok(page.map(|row| {
            let org_id = row.org_id.unwrap_or(root);
            MembershipRow {
                id: row.id,
                user_id: row.user_id,
                org_id,
                role_id: row.role_id,
                role_name: names.get(&row.role_id).cloned().unwrap_or_default(),
                inherited: filter.org_id.is_some_and(|wanted| wanted != org_id),
                created_at: row.created_at,
            }
        }))
    }

    /// Bên gọi là người dùng `user_id`: permission của họ là hợp các permission của
    /// mọi role của họ. Hai câu truy vấn, không phụ thuộc số role. Bên gọi thuộc khách `tenant`.
    pub(crate) async fn principal(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Principal, Error> {
        use sea_orm::sea_query::ExprTrait as _;
        // MỘT câu cho mọi request đã đăng nhập (đường nóng): mỗi dòng là một role của
        // người dùng kèm một permission của role đó (NULL nếu role không có permission
        // nào). Trước đây là hai câu — trên Postgres mỗi câu thêm một vòng mạng (D-076).
        let mut select = Query::select();
        select
            .column((role::Entity, role::Column::AllPermissions))
            .column((role::Entity, role::Column::RequiresMfa))
            .column((user_role::Entity, user_role::Column::OrgId))
            .column((role_permission::Entity, role_permission::Column::Permission))
            .column((role_permission::Entity, role_permission::Column::Scope))
            .from(user_role::Entity)
            .inner_join(
                role::Entity,
                Expr::col((role::Entity, role::Column::Id))
                    .equals((user_role::Entity, user_role::Column::RoleId)),
            )
            .left_join(
                role_permission::Entity,
                Expr::col((role_permission::Entity, role_permission::Column::RoleId))
                    .equals((role::Entity, role::Column::Id)),
            )
            .and_where(Expr::col((user_role::Entity, user_role::Column::UserId)).eq(user_id))
            .and_where(
                Expr::col((user_role::Entity, user_role::Column::TenantId))
                    .eq(tenant.id().as_uuid()),
            );
        let rows = tenant.database().read().query_all(&select).await?;
        // Role bắt buộc MFA (D-308): chỉ khi có role như vậy mới tra MFA của tài khoản — đường
        // nóng của mọi người khác giữ một câu.
        let mut has_mfa = None;
        for row in &rows {
            if row.try_get::<bool>("", "requires_mfa")? {
                has_mfa = Some(has_second_factor(tenant, user_id).await?);
                break;
            }
        }
        let root = root_of(tenant.id());
        let mut grants = Vec::with_capacity(rows.len());
        for row in rows {
            if row.try_get::<bool>("", "requires_mfa")? && has_mfa == Some(false) {
                continue;
            }
            // Role `admin` chỉ gán được ở nút gốc (`set_memberships` chặn chỗ khác).
            if row.try_get::<bool>("", "all_permissions")? {
                return Ok(Principal::with_all_permissions(user_id).in_tenant(tenant.id()));
            }
            let Some(permission) = row.try_get::<Option<String>>("", "permission")? else {
                continue;
            };
            let org = row.try_get::<Option<Uuid>>("", "org_id")?.map_or(root, OrgId::from_uuid);
            let scope = row
                .try_get::<Option<String>>("", "scope")?
                .and_then(|scope| Scope::parse(&scope))
                .unwrap_or(Scope::Tenant);
            grants.push((permission, Grant::new(org, scope)));
        }
        Ok(Principal::with_grants(user_id, grants).in_tenant(tenant.id()))
    }

    /// `Ok` nếu `actor` có mọi permission người dùng `target` có. Dùng trước khi một
    /// quản trị viên tác động lên tài khoản người khác (gỡ MFA, khoá, đặt mật khẩu đầu):
    /// người chỉ có `auth.users.manage` không được làm yếu tài khoản của người quyền cao
    /// hơn mình, rồi chiếm nó (D-079).
    pub(crate) async fn require_covers(
        &self,
        tenant: &TenantContext,
        actor: &Principal,
        target: Uuid,
    ) -> Result<(), Error> {
        // Người mua không quản lý qua đây (D-309): `GET /customers`, `PATCH /customers/{id}`.
        let staff = user::Entity::find_by_id(target)
            .filter(user::Column::Realm.eq("staff"))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        if staff.is_none() {
            return Err(crate::user::user_not_found(target));
        }
        let target = self.principal(tenant, target).await?;
        let covered = if target.has_all_permissions() {
            actor.has_all_permissions()
        } else {
            target.permissions().iter().all(|permission| actor.allows(permission))
        };
        if covered {
            return Ok(());
        }
        Err(Error::forbidden(
            "auth.target_has_more_permissions",
            "This user has permissions that you do not have, so you cannot change their \
             account this way. Ask someone who has all of their permissions.",
        ))
    }

    /// Kiểm và chuẩn hoá dữ liệu vào; báo mọi trường sai cùng lúc.
    fn check(&self, input: RoleInput) -> Result<CheckedRole, Error> {
        let name = input.name.trim().to_owned();
        let description = input.description.trim().to_owned();
        let mut problems = Vec::new();
        if !is_role_name(&name) {
            problems.push(
                FieldError::new(
                    "name",
                    "pattern",
                    "must be 1 to {name_max_length} characters: lowercase letters, digits, `_` \
                     and `-`, starting with a letter",
                )
                .with_param("name_max_length", NAME_MAX_LENGTH),
            );
        }
        if description.chars().count() > DESCRIPTION_MAX_LENGTH {
            problems.push(
                FieldError::new(
                    "description",
                    "length",
                    "must be at most {description_max_length} characters long",
                )
                .with_param("description_max_length", DESCRIPTION_MAX_LENGTH),
            );
        }
        if input.permissions.len() + input.grants.len() > MAX_PERMISSIONS {
            problems.push(
                FieldError::new(
                    "permissions",
                    "length",
                    "must list at most {max_permissions} permissions",
                )
                .with_param("max_permissions", MAX_PERMISSIONS),
            );
        }
        for (index, permission) in input.permissions.iter().enumerate() {
            if !self.catalog.contains(permission) {
                problems.push(FieldError::new(
                    format!("permissions[{index}]"),
                    "unknown",
                    "is not a permission of this system",
                ));
            }
        }
        let mut seen = BTreeSet::new();
        for (index, (permission, scope)) in input.grants.iter().enumerate() {
            match self.catalog.iter().find(|known| known.name() == permission) {
                None => problems.push(FieldError::new(
                    format!("grants[{index}].permission"),
                    "unknown",
                    "is not a permission of this system",
                )),
                // Permission không theo nút chỉ cấp được cho cả khách (D-303 mục 3).
                Some(known) if !known.is_scoped() && *scope != Scope::Tenant => {
                    problems.push(
                        FieldError::new(
                            format!("grants[{index}].scope"),
                            "scope_not_supported",
                            "`{permission}` can only be granted for the whole tenant (`tenant`)",
                        )
                        .with_param("permission", permission),
                    );
                }
                Some(_) => {}
            }
            if input.permissions.contains(permission) || !seen.insert(permission.as_str()) {
                problems.push(
                    FieldError::new(
                        format!("grants[{index}].permission"),
                        "duplicate",
                        "`{permission}` is listed more than once; give each permission one scope",
                    )
                    .with_param("permission", permission),
                );
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let mut granted: std::collections::BTreeMap<String, Scope> =
            input.permissions.into_iter().map(|permission| (permission, Scope::Tenant)).collect();
        granted.extend(input.grants);
        Ok((name, description, granted.into_iter().collect()))
    }
}

/// Gán role `admin` (role có mọi permission) cho người dùng `user_id`, trong
/// transaction của bên gọi.
pub(crate) async fn grant_admin<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<(), Error> {
    let admin = role::Entity::find()
        .filter(role::Column::AllPermissions.eq(true))
        .scoped(tenant)
        .one(connection)
        .await?
        .ok_or_else(|| {
            Error::internal(
                "the built-in `admin` role of this tenant is missing; run the migrations, or \
                 provision the tenant again",
            )
        })?;
    user_role::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        user_id: Set(user_id),
        role_id: Set(admin.id),
        org_id: Set(Some(root_of(tenant.id()).as_uuid())),
        created_at: Set(now()),
    }
    .insert(connection)
    .await?;
    Ok(())
}

/// Mô tả của role `admin` dựng sẵn (như migration m0003 gieo cho khách mặc định).
const ADMIN_DESCRIPTION: &str =
    "Every permission, including permissions added later. Built in: cannot be changed or deleted.";

/// Gieo role `admin` (mọi permission) cho khách mới (bước 33, `Module::seed_tenant`). Chạy lại
/// được: khách đã có role toàn quyền thì thôi; hai lần gieo cùng lúc thì index unique
/// `(tenant_id, name)` giữ một dòng. Khách mặc định có sẵn role này từ migration (m0003, m0026).
pub(crate) struct AdminRoleSeed;

#[async_trait::async_trait]
impl autoxide_kernel::tenancy::TenantSeed for AdminRoleSeed {
    async fn seed(&self, tenant: &TenantContext) -> Result<(), Error> {
        let write = tenant.database().write();
        let existing = role::Entity::find()
            .filter(role::Column::AllPermissions.eq(true))
            .scoped(tenant)
            .one(write)
            .await?;
        if existing.is_some() {
            return Ok(());
        }
        let now = now();
        let row = role::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            name: Set("admin".to_owned()),
            description: Set(ADMIN_DESCRIPTION.to_owned()),
            all_permissions: Set(true),
            requires_mfa: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
        };
        role::Entity::insert(row)
            .on_conflict(
                sea_orm::sea_query::OnConflict::columns([
                    role::Column::TenantId,
                    role::Column::Name,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(write)
            .await?;
        Ok(())
    }
}

/// Chủ thể (subject) của event về role `id`.
fn role_subject(id: Uuid) -> Option<String> {
    Some(format!("auth.role:{id}"))
}

/// Ảnh chụp một role cho audit: tên, mô tả, permission.
async fn role_snapshot<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    row: &role::Model,
) -> Result<serde_json::Value, Error> {
    let mut permissions = permissions_of(connection, tenant, [row.id]).await?;
    let granted = permissions.remove(&row.id).unwrap_or_default();
    let mut snapshot = role_json(&row.name, &row.description, &granted);
    snapshot["requires_mfa"] = json!(row.requires_mfa);
    Ok(snapshot)
}

/// Role cho audit: tên, mô tả, permission (tên) và độ sâu của từng permission.
fn role_json(name: &str, description: &str, grants: &[(String, Scope)]) -> serde_json::Value {
    let names: Vec<&str> = grants.iter().map(|(permission, _)| permission.as_str()).collect();
    let scopes: serde_json::Map<String, serde_json::Value> = grants
        .iter()
        .map(|(permission, scope)| (permission.clone(), json!(scope.as_str())))
        .collect();
    json!({"name": name, "description": description, "permissions": names, "scopes": scopes})
}

/// Tên các role, cho audit.
fn role_names(roles: &[Role]) -> serde_json::Value {
    json!(roles.iter().map(|role| role.name.as_str()).collect::<Vec<_>>())
}

fn is_role_name(name: &str) -> bool {
    name.chars().count() <= NAME_MAX_LENGTH
        && name.chars().next().is_some_and(|first| first.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

async fn insert_permissions(
    transaction: &DatabaseTransaction,
    tenant: &TenantContext,
    role_id: Uuid,
    permissions: &[(String, Scope)],
) -> Result<(), Error> {
    for (permission, scope) in permissions {
        role_permission::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            role_id: Set(role_id),
            permission: Set(permission.clone()),
            scope: Set(scope.as_str().to_owned()),
        }
        .insert(transaction)
        .await?;
    }
    Ok(())
}

/// Permission (kèm độ sâu) của từng role trong `role_ids`, một câu truy vấn cho cả danh sách.
async fn permissions_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    role_ids: impl IntoIterator<Item = Uuid>,
) -> Result<HashMap<Uuid, Vec<(String, Scope)>>, Error> {
    let rows = role_permission::Entity::find()
        .filter(role_permission::Column::RoleId.is_in(role_ids))
        .scoped(tenant)
        .order_by_asc(role_permission::Column::Permission)
        .all(connection)
        .await?;
    let mut by_role: HashMap<Uuid, Vec<(String, Scope)>> = HashMap::new();
    for row in rows {
        let scope = Scope::parse(&row.scope).unwrap_or(Scope::Tenant);
        by_role.entry(row.role_id).or_default().push((row.permission, scope));
    }
    Ok(by_role)
}

async fn roles_of_user<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<Vec<Role>, Error> {
    let rows = role::Entity::find()
        .filter(
            role::Column::Id.in_subquery(
                Query::select()
                    .column(user_role::Column::RoleId)
                    .from(user_role::Entity)
                    .and_where(user_role::Column::TenantId.eq(tenant.id().as_uuid()))
                    .and_where(user_role::Column::UserId.eq(user_id))
                    .to_owned(),
            ),
        )
        .scoped(tenant)
        .order_by_asc(role::Column::Name)
        .all(connection)
        .await?;
    let mut permissions = permissions_of(connection, tenant, rows.iter().map(|row| row.id)).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let granted = permissions.remove(&row.id).unwrap_or_default();
            to_role(row, granted)
        })
        .collect())
}

/// Số người dùng CÒN HOẠT ĐỘNG (chưa bị khoá) đang có một role mang mọi permission.
pub(crate) async fn count_active_admins<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
) -> Result<u64, Error> {
    // Khoá các role toàn quyền (câu `UPDATE` không đổi gì vẫn giữ khoá dòng tới hết
    // transaction): mọi thay đổi có thể làm mất admin nối đuôi nhau. Thiếu khoá này, trên
    // Postgres hai admin cuối cùng xoá nhau cùng lúc thì mỗi bên vẫn đếm thấy người kia còn
    // và cả hai cùng thành công (D-149). SQLite chỉ có một bên ghi nên vốn đã nối đuôi.
    role::Entity::update_many()
        .col_expr(role::Column::AllPermissions, Expr::col(role::Column::AllPermissions))
        .filter(role::Column::AllPermissions.eq(true))
        .scoped(tenant)
        .exec(connection)
        .await?;
    count_active_admins_unlocked(connection, tenant).await
}

/// Như [`count_active_admins`] nhưng không khoá: chỉ đọc, dùng được trên kết nối đọc (vd lệnh
/// `autoxide doctor`). Không dùng để quyết một thay đổi về quyền — đếm này có thể cũ ngay sau đó.
pub(crate) async fn count_active_admins_unlocked<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
) -> Result<u64, Error> {
    let id = tenant.id().as_uuid();
    let count = user_role::Entity::find()
        .filter(
            user_role::Column::RoleId.in_subquery(
                Query::select()
                    .column(role::Column::Id)
                    .from(role::Entity)
                    .and_where(role::Column::TenantId.eq(id))
                    .and_where(role::Column::AllPermissions.eq(true))
                    .to_owned(),
            ),
        )
        .filter(
            user_role::Column::UserId.in_subquery(
                Query::select()
                    .column(user::Column::Id)
                    .from(user::Entity)
                    .and_where(user::Column::TenantId.eq(id))
                    .and_where(user::Column::DisabledAt.is_null())
                    .to_owned(),
            ),
        )
        .scoped(tenant)
        .count(connection)
        .await?;
    Ok(count)
}

/// Lỗi khi một thay đổi sẽ để hệ thống không còn admin nào hoạt động.
pub(crate) fn last_admin() -> Error {
    Error::conflict(
        "auth.last_admin",
        "This change would leave no active user with the `admin` role, and nobody could manage \
         the system any more. Give the `admin` role to another user first.",
    )
}

/// Membership của người `user_id`, gom theo nút (nút theo ID; role theo tên).
async fn memberships_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<Vec<Membership>, Error> {
    let root = root_of(tenant.id()).as_uuid();
    let rows = user_role::Entity::find()
        .filter(user_role::Column::UserId.eq(user_id))
        .scoped(tenant)
        .all(connection)
        .await?;
    let roles = role::Entity::find()
        .filter(role::Column::Id.is_in(rows.iter().map(|row| row.role_id)))
        .scoped(tenant)
        .order_by_asc(role::Column::Name)
        .all(connection)
        .await?;
    let mut grants = permissions_of(connection, tenant, roles.iter().map(|row| row.id)).await?;
    let roles: Vec<Role> = roles
        .into_iter()
        .map(|row| {
            let granted = grants.remove(&row.id).unwrap_or_default();
            to_role(row, granted)
        })
        .collect();
    let mut by_org: std::collections::BTreeMap<Uuid, Vec<Role>> = std::collections::BTreeMap::new();
    for role in &roles {
        for row in rows.iter().filter(|row| row.role_id == role.id) {
            by_org.entry(row.org_id.unwrap_or(root)).or_default().push(role.clone());
        }
    }
    Ok(by_org.into_iter().map(|(org_id, roles)| Membership { user_id, org_id, roles }).collect())
}

/// Membership cho audit: nút → tên role.
fn memberships_json(memberships: &[Membership]) -> serde_json::Value {
    let map: serde_json::Map<String, serde_json::Value> = memberships
        .iter()
        .map(|membership| (membership.org_id.to_string(), role_names(&membership.roles)))
        .collect();
    serde_json::Value::Object(map)
}

fn to_role(row: role::Model, grants: Vec<(String, Scope)>) -> Role {
    Role {
        id: row.id,
        name: row.name,
        description: row.description,
        all_permissions: row.all_permissions,
        requires_mfa: row.requires_mfa,
        permissions: grants.iter().map(|(permission, _)| permission.clone()).collect(),
        grants,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn name_taken_or(error: sea_orm::DbErr, name: &str) -> Error {
    match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => Error::conflict(
            "auth.role_name_taken",
            "A role named `{name}` already exists. Choose another name.",
        )
        .with_param("name", name),
        _ => Error::from(error),
    }
}

fn role_not_found(id: Uuid) -> Error {
    Error::not_found("auth.role_not_found", "Role `{id}` does not exist.").with_param("id", id)
}

fn builtin_role() -> Error {
    Error::conflict(
        "auth.role_builtin",
        "The built-in `admin` role cannot be changed or deleted. Create another role instead.",
    )
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// ID của role tên `name` của khách `tenant` (lệnh CLI nêu role bằng tên).
pub(crate) async fn role_by_name(
    tenant: &TenantContext,
    name: &str,
) -> Result<Option<Uuid>, Error> {
    Ok(role::Entity::find()
        .filter(role::Column::Name.eq(name.trim()))
        .scoped(tenant)
        .one(tenant.database().read())
        .await?
        .map(|row| row.id))
}

/// Vì sao `actor` không gán được các role `roles` ở nút `org_id` (rỗng: gán được): nút phải có
/// trong khách và trong phạm vi của người gán, role phải có, `admin` chỉ ở gốc, và người gán phải
/// có mọi permission của role (không tự nâng quyền). `field`: tiền tố tên ô (`memberships[0].`).
/// Dùng cho membership và lời mời.
pub(crate) async fn grant_problems<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    actor: &Principal,
    field: &str,
    org_id: Uuid,
    roles: &BTreeSet<Uuid>,
) -> Result<Vec<FieldError>, Error> {
    let mut problems = Vec::new();
    let root = root_of(tenant.id()).as_uuid();
    let org = OrgId::from_uuid(org_id);
    if org_id != root && OrgTree::ancestors(connection, tenant.id(), org).await?.is_empty() {
        problems.push(FieldError::new(
            format!("{field}org_id"),
            "unknown",
            "is not an organization unit of this tenant",
        ));
        return Ok(problems);
    }
    if !actor.reaches(connection, org).await? {
        problems.push(FieldError::new(
            format!("{field}org_id"),
            "out_of_scope",
            "must be a unit you work in (one of your roles covers it)",
        ));
    }
    let found = role::Entity::find()
        .filter(role::Column::Id.is_in(roles.iter().copied()))
        .scoped(tenant)
        .all(connection)
        .await?;
    let mut grants = permissions_of(connection, tenant, found.iter().map(|row| row.id)).await?;
    for role_id in roles {
        let Some(row) = found.iter().find(|row| row.id == *role_id) else {
            problems.push(FieldError::new(
                format!("{field}role_ids"),
                "unknown",
                "every entry must be the id of an existing role",
            ));
            continue;
        };
        if row.all_permissions && org_id != root {
            problems.push(FieldError::new(
                format!("{field}role_ids"),
                "root_only",
                "the `admin` role can only be given at the root unit",
            ));
        }
        let covers = if row.all_permissions {
            actor.has_all_permissions()
        } else {
            grants
                .remove(&row.id)
                .unwrap_or_default()
                .iter()
                .all(|(permission, _)| actor.allows(permission))
        };
        if !covers {
            problems.push(
                FieldError::new(
                    format!("{field}role_ids"),
                    "more_than_you_have",
                    "role `{role}` has permissions you do not have; you cannot give it",
                )
                .with_param("role", &row.name),
            );
        }
    }
    Ok(problems)
}

/// Thêm các role `roles` ở nút `org_id` cho người dùng `user_id` (bỏ qua role đã có ở nút đó),
/// trong `transaction` của bên gọi (chấp nhận lời mời).
pub(crate) async fn add_roles<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    user_id: Uuid,
    org_id: Uuid,
    roles: &BTreeSet<Uuid>,
) -> Result<(), Error> {
    let held: BTreeSet<Uuid> = user_role::Entity::find()
        .filter(user_role::Column::UserId.eq(user_id))
        .filter(user_role::Column::OrgId.eq(org_id))
        .scoped(tenant)
        .all(transaction)
        .await?
        .into_iter()
        .map(|row| row.role_id)
        .collect();
    let now = now();
    for role_id in roles.difference(&held) {
        user_role::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            user_id: Set(user_id),
            role_id: Set(*role_id),
            org_id: Set(Some(org_id)),
            created_at: Set(now),
        }
        .insert(transaction)
        .await?;
    }
    Ok(())
}

/// Role người dùng `user_id` giữ ở nút gốc của khách `tenant` (uỷ quyền theo cây khách hàng,
/// D-310).
pub(crate) async fn root_roles(tenant: &TenantContext, user_id: Uuid) -> Result<Vec<Uuid>, Error> {
    let root = root_of(tenant.id()).as_uuid();
    Ok(user_role::Entity::find()
        .filter(user_role::Column::UserId.eq(user_id))
        .filter(user_role::Column::OrgId.eq(root))
        .scoped(tenant)
        .all(tenant.database().read())
        .await?
        .into_iter()
        .map(|row| row.role_id)
        .collect())
}

/// Role có `all_permissions` không và permission (kèm độ sâu) của từng role trong `ids` của khách
/// `tenant`. Role không có: vắng.
pub(crate) async fn role_grants(
    tenant: &TenantContext,
    ids: &[Uuid],
) -> Result<std::collections::HashMap<Uuid, (bool, Vec<(String, Scope)>)>, Error> {
    if ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let read = tenant.database().read();
    let rows = role::Entity::find()
        .filter(role::Column::Id.is_in(ids.iter().copied()))
        .scoped(tenant)
        .all(read)
        .await?;
    let mut permissions = permissions_of(read, tenant, rows.iter().map(|row| row.id)).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let granted = permissions.remove(&row.id).unwrap_or_default();
            (row.id, (row.all_permissions, granted))
        })
        .collect())
}

/// Tài khoản `user_id` đã bật MFA: TOTP đã xác nhận, hay có ít nhất một passkey.
pub(crate) async fn has_second_factor(
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<bool, Error> {
    let read = tenant.database().read();
    let totp = crate::mfa::entity::totp::Entity::find()
        .filter(crate::mfa::entity::totp::Column::UserId.eq(user_id))
        .filter(crate::mfa::entity::totp::Column::ConfirmedAt.is_not_null())
        .scoped(tenant)
        .one(read)
        .await?;
    if totp.is_some() {
        return Ok(true);
    }
    let passkey = crate::passkey::entity::passkey::Entity::find()
        .filter(crate::passkey::entity::passkey::Column::UserId.eq(user_id))
        .scoped(tenant)
        .one(read)
        .await?;
    Ok(passkey.is_some())
}

/// Tài khoản `user_id` có role bắt buộc MFA mà chưa bật MFA: quyền của các role đó đang bị giữ.
pub(crate) async fn mfa_pending(tenant: &TenantContext, user_id: Uuid) -> Result<bool, Error> {
    let read = tenant.database().read();
    let held: Vec<Uuid> = user_role::Entity::find()
        .filter(user_role::Column::UserId.eq(user_id))
        .scoped(tenant)
        .all(read)
        .await?
        .into_iter()
        .map(|row| row.role_id)
        .collect();
    if held.is_empty() {
        return Ok(false);
    }
    let flagged = role::Entity::find()
        .filter(role::Column::Id.is_in(held))
        .filter(role::Column::RequiresMfa.eq(true))
        .scoped(tenant)
        .one(read)
        .await?;
    Ok(flagged.is_some() && !has_second_factor(tenant, user_id).await?)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use autoxide_kernel::access::Permission;
    use autoxide_kernel::error::ErrorKind;
    use autoxide_kernel::pagination::Order;
    use autoxide_testkit::TestDatabase;

    use crate::testing::Services;
    use crate::user::NewUser;

    const NOTES_WRITE: Permission = Permission::new("notes.write", "Write notes.");

    fn input(name: &str, permissions: &[&str]) -> RoleInput {
        RoleInput {
            requires_mfa: false,
            grants: Vec::new(),
            name: name.to_owned(),
            description: String::new(),
            permissions: permissions.iter().map(|p| (*p).to_owned()).collect(),
        }
    }

    async fn user(services: &Services, email: &str) -> Uuid {
        let tenant = services.tenant.clone();
        let input = NewUser { email: email.to_owned(), name: "U".to_owned(), password: None };
        services.users.create(&tenant, Actor::system(), input).await.unwrap().id
    }

    async fn admin_role(roles: &RoleService, tenant: &TenantContext) -> Role {
        let page = roles.list(tenant, &PageRequest::first(200, Order::Asc)).await.unwrap();
        page.data.into_iter().find(|role| role.all_permissions).unwrap()
    }

    #[tokio::test]
    async fn role_admin_dung_san_va_khong_sua_xoa_duoc() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let (roles, tenant) = (services.roles.clone(), services.tenant.clone());

            let admin = admin_role(&roles, &tenant).await;

            assert_eq!(admin.name, "admin");
            assert!(admin.permissions.is_empty());
            let changed = roles
                .update(&tenant, Actor::system(), admin.id, input("boss", &[]), None)
                .await
                .unwrap_err();
            let deleted = roles.delete(&tenant, Actor::system(), admin.id).await.unwrap_err();
            assert_eq!(changed.code(), "auth.role_builtin");
            assert_eq!(deleted.code(), "auth.role_builtin");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tao_sua_xoa_role() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let (roles, tenant) = (services.roles.clone(), services.tenant.clone());

            let created = roles
                .create(
                    &tenant,
                    Actor::system(),
                    input(" editor ", &["notes.write", "auth.roles.read", "notes.write"]),
                )
                .await
                .unwrap();
            assert_eq!(created.name, "editor");
            assert_eq!(created.permissions, ["auth.roles.read", "notes.write"]);
            assert!(!created.all_permissions);

            let changed = roles
                .update(
                    &tenant,
                    Actor::system(),
                    created.id,
                    input("writer", &["notes.write"]),
                    None,
                )
                .await
                .unwrap();
            assert_eq!(changed.name, "writer");
            assert_eq!(changed.permissions, ["notes.write"]);
            assert_eq!(roles.get(&tenant, created.id).await.unwrap(), changed);

            roles.delete(&tenant, Actor::system(), created.id).await.unwrap();
            assert_eq!(
                roles.get(&tenant, created.id).await.unwrap_err().code(),
                "auth.role_not_found"
            );
            assert_eq!(
                roles.delete(&tenant, Actor::system(), created.id).await.unwrap_err().code(),
                "auth.role_not_found"
            );
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn du_lieu_sai_va_ten_trung() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let (roles, tenant) = (services.roles.clone(), services.tenant.clone());

            let invalid = roles
                .create(&tenant, Actor::system(), input("Bad Name", &["notes.write", "nope.read"]))
                .await
                .unwrap_err();
            let fields: Vec<(&str, &str)> =
                invalid.fields().iter().map(|f| (f.field.as_str(), f.code.as_str())).collect();
            assert_eq!(fields, [("name", "pattern"), ("permissions[1]", "unknown")]);

            roles.create(&tenant, Actor::system(), input("editor", &[])).await.unwrap();
            let taken =
                roles.create(&tenant, Actor::system(), input("editor", &[])).await.unwrap_err();
            assert_eq!(taken.kind(), ErrorKind::Conflict);
            assert_eq!(taken.code(), "auth.role_name_taken");
            let admin =
                roles.create(&tenant, Actor::system(), input("admin", &[])).await.unwrap_err();
            assert_eq!(admin.code(), "auth.role_name_taken");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn quyen_cua_nguoi_dung_la_hop_cac_role() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let tenant = services.tenant.clone();
            let roles = &services.roles;
            let reader = roles
                .create(&tenant, Actor::system(), input("reader", &["auth.roles.read"]))
                .await
                .unwrap();
            let writer = roles
                .create(&tenant, Actor::system(), input("writer", &["notes.write"]))
                .await
                .unwrap();
            let ada = user(&services, "ada@example.com").await;

            assert_eq!(
                roles.principal(&tenant, ada).await.unwrap().permissions(),
                Vec::<String>::new()
            );
            let assigned = roles
                .set_user_roles(&tenant, Actor::system(), ada, vec![writer.id, reader.id])
                .await
                .unwrap();

            let names: Vec<&str> = assigned.iter().map(|role| role.name.as_str()).collect();
            assert_eq!(names, ["reader", "writer"]);
            let principal = roles.principal(&tenant, ada).await.unwrap();
            assert_eq!(principal.permissions(), ["auth.roles.read", "notes.write"]);
            assert!(principal.has(&NOTES_WRITE));
            assert_eq!(roles.user_roles(&tenant, ada).await.unwrap(), assigned);

            // Sửa role: người có role nhận permission mới ngay.
            roles
                .update(&tenant, Actor::system(), writer.id, input("writer", &[]), None)
                .await
                .unwrap();
            assert!(!roles.principal(&tenant, ada).await.unwrap().has(&NOTES_WRITE));

            let admin = admin_role(roles, &tenant).await;
            roles.set_user_roles(&tenant, Actor::system(), ada, vec![admin.id]).await.unwrap();
            assert!(roles.principal(&tenant, ada).await.unwrap().has_all_permissions());
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn gan_role_cho_nguoi_hay_role_khong_ton_tai() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let tenant = services.tenant.clone();
            let ada = user(&services, "ada@example.com").await;

            let no_user = services
                .roles
                .set_user_roles(&tenant, Actor::system(), Uuid::now_v7(), vec![])
                .await;
            let no_role = services
                .roles
                .set_user_roles(&tenant, Actor::system(), ada, vec![Uuid::now_v7()])
                .await;
            let read_no_user = services.roles.user_roles(&tenant, Uuid::now_v7()).await;

            assert_eq!(no_user.unwrap_err().code(), "auth.user_not_found");
            assert_eq!(no_role.unwrap_err().fields()[0].code, "unknown");
            assert_eq!(read_no_user.unwrap_err().code(), "auth.user_not_found");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn khong_bo_duoc_admin_cuoi_cung() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let tenant = services.tenant.clone();
            let roles = &services.roles;
            let admin = admin_role(roles, &tenant).await;
            let ada = user(&services, "ada@example.com").await;
            let bob = user(&services, "bob@example.com").await;
            roles.set_user_roles(&tenant, Actor::system(), ada, vec![admin.id]).await.unwrap();

            let error =
                roles.set_user_roles(&tenant, Actor::system(), ada, vec![]).await.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Conflict);
            assert_eq!(error.code(), "auth.last_admin");
            assert!(roles.principal(&tenant, ada).await.unwrap().has_all_permissions());

            // Có admin khác thì bỏ được.
            roles.set_user_roles(&tenant, Actor::system(), bob, vec![admin.id]).await.unwrap();
            roles.set_user_roles(&tenant, Actor::system(), ada, vec![]).await.unwrap();
        })
        .await
        .unwrap();
    }

    /// Nhiều request thay role của CÙNG một người dùng cùng lúc: mọi request thành
    /// công, và kết quả cuối là đúng danh sách của một request nào đó (không trộn) —
    /// trên cả hai database.
    #[tokio::test]
    async fn gan_role_dong_thoi_khong_tron_lan() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let tenant = services.tenant.clone();
            let roles = services.roles.clone();
            let ada = user(&services, "ada@example.com").await;
            let mut sets = Vec::new();
            for index in 0..4 {
                let first = roles
                    .create(&tenant, Actor::system(), input(&format!("a{index}"), &[]))
                    .await
                    .unwrap()
                    .id;
                let second = roles
                    .create(&tenant, Actor::system(), input(&format!("b{index}"), &[]))
                    .await
                    .unwrap()
                    .id;
                sets.push(BTreeSet::from([first, second]));
            }

            let attempts: Vec<_> = sets
                .iter()
                .map(|set| {
                    let roles: Arc<RoleService> = roles.clone();
                    let ids: Vec<Uuid> = set.iter().copied().collect();
                    let tenant = tenant.clone();
                    tokio::spawn(async move {
                        roles.set_user_roles(&tenant, Actor::system(), ada, ids).await
                    })
                })
                .collect();
            for attempt in attempts {
                attempt.await.unwrap().unwrap();
            }

            let last: BTreeSet<Uuid> = roles
                .user_roles(&tenant, ada)
                .await
                .unwrap()
                .into_iter()
                .map(|role| role.id)
                .collect();
            assert!(sets.contains(&last), "{last:?}");
        })
        .await
        .unwrap();
    }
}
