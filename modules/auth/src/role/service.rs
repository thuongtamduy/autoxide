//! Service của role: tạo, sửa, xoá role; gán role cho người dùng; tính permission
//! của một người dùng.

use autoxide_kernel::time::Timestamp;
use std::collections::{BTreeSet, HashMap};

use autoxide_kernel::access::{Grant, PermissionCatalog, Principal, Scope};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{
    Order, Page, PageRequest, Pagination, SortValue, fetch_page, fetch_page_by, text_key_cursor,
};
use sea_orm::sea_query::{Expr, ExprTrait as _, Query};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Set, SqlErr,
};
use uuid::Uuid;

use super::entity::{role, role_permission, user_role};
use crate::api::User;
use crate::events;
use crate::user::entity as user;
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext, root_of};
use serde_json::json;

mod admins;
mod grants;
mod members;
mod principal;
mod scim_grants;
pub(crate) use admins::*;
pub(crate) use grants::*;
pub(crate) use members::*;
pub(crate) use principal::*;
pub(crate) use scim_grants::forget_scim_grants;

/// Role đã kiểm: tên, mô tả, permission cấp (kèm độ sâu) và permission cấm.
type CheckedRole = (String, String, RoleRules);

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
    /// Permission role cấm (bước 54), theo thứ tự tên: thắng mọi role khác của người dùng.
    pub denies: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
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
    /// Permission cấm (bước 54); không trùng permission cấp của chính role.
    pub denies: Vec<String>,
}

/// Bộ lọc danh sách membership.
#[derive(Debug, Clone, Default)]
pub(crate) struct MembershipFilter {
    pub org_id: Option<Uuid>,
    pub role_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    pub inherited: bool,
}

/// Số người ở một nút (xem `member_counts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemberCount {
    pub org_id: Uuid,
    pub direct: usize,
    pub total: usize,
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
    pub created_at: Timestamp,
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
    /// Hạn mức số nhân sự theo gói (bước 35).
    entitlements: autoxide_kernel::entitlement::EntitlementsHandle,
}

/// Hạn mức số nhân sự của một khách (bước 35): người khác nhau có ít nhất một role ở khách.
pub(crate) const MEMBERS_LIMIT: autoxide_kernel::entitlement::Limit =
    autoxide_kernel::entitlement::Limit::count(
        "auth.members",
        "People who hold at least one role in the tenant (staff accounts with access).",
    );

/// Mã giữ chỗ hạn mức của người dùng `user_id`.
pub(crate) fn member_reference(user_id: Uuid) -> String {
    format!("auth.member:{user_id}")
}

/// Người dùng `user_id` có giữ role nào ở khách `tenant` không.
pub(crate) async fn has_membership(tenant: &TenantContext, user_id: Uuid) -> Result<bool, Error> {
    // Đọc qua kết nối GHI, không qua `read()`: kết quả quyết định giữ hay trả chỗ trong hạn mức số
    // nhân sự ngay sau một lần ghi; kết nối đọc có thể là read replica trễ hơn bản chính — đọc ở đó
    // thì vừa gán role xong vẫn thấy "chưa giữ" và trả chỗ (gói bị vượt hạn mức).
    let found = user_role::Entity::find()
        .filter(user_role::Column::UserId.eq(user_id))
        .filter(user_role::Column::TenantId.eq(tenant.id().as_uuid()))
        .one(tenant.database().write())
        .await?;
    Ok(found.is_some())
}

/// Người dùng `user_id` có giữ một trong các role `roles` ở khách `tenant` không (ở nút nào cũng được).
pub(crate) async fn holds_any_role(
    tenant: &TenantContext,
    user_id: Uuid,
    roles: &[Uuid],
) -> Result<bool, Error> {
    if roles.is_empty() {
        return Ok(false);
    }
    let found = user_role::Entity::find()
        .filter(user_role::Column::UserId.eq(user_id))
        .filter(user_role::Column::RoleId.is_in(roles.iter().copied()))
        .scoped(tenant)
        .one(tenant.database().read())
        .await?;
    Ok(found.is_some())
}

/// Bộ đếm thật cho job đối soát của `plans`: số người khác nhau giữ role ở khách.
pub(crate) struct MemberCounter;

#[async_trait::async_trait]
impl autoxide_kernel::entitlement::UsageCounter for MemberCounter {
    async fn count(&self, tenant: &TenantContext) -> Result<i64, Error> {
        let users: BTreeSet<Uuid> = user_role::Entity::find()
            .select_only()
            .column(user_role::Column::UserId)
            .filter(user_role::Column::TenantId.eq(tenant.id().as_uuid()))
            .into_tuple::<Uuid>()
            .all(tenant.database().read())
            .await?
            .into_iter()
            .collect();
        Ok(i64::try_from(users.len()).unwrap_or(i64::MAX))
    }
}

impl RoleService {
    pub(crate) fn new(catalog: PermissionCatalog, outbox: Outbox) -> Self {
        RoleService {
            catalog,
            outbox,
            translations: Default::default(),
            entitlements: Default::default(),
        }
    }

    /// Hạn mức số nhân sự theo gói (bước 35).
    pub(crate) fn with_entitlements(
        mut self,
        entitlements: autoxide_kernel::entitlement::EntitlementsHandle,
    ) -> Self {
        self.entitlements = entitlements;
        self
    }

    pub(crate) fn entitlements(&self) -> &autoxide_kernel::entitlement::EntitlementsHandle {
        &self.entitlements
    }

    /// Bọc một lần đổi role của `user_id` bằng hạn mức số nhân sự: người chưa giữ role nào mà có thể
    /// sắp có (`will_hold`) → giữ chỗ TRƯỚC (ngoài transaction của module). `change` trả kết quả kèm
    /// "có đổi gì không". Có đổi: đối chiếu với trạng thái THẬT sau khi đổi (người đó còn giữ role
    /// nào không, đọc qua kết nối ghi — xem `has_membership`), không với ý định: lần đồng bộ SCIM chỉ
    /// áp phần mình được phép (D-20261125-scim-grant-cover), nên "muốn giữ" và "đang giữ" có thể khác
    /// — giữ role thì xác nhận chỗ, không giữ thì trả chỗ. Không đổi gì: không đụng tới chỗ — mã giữ
    /// chỗ (`auth.member:<id>`) dùng chung cho mọi lần đổi role của người đó, một lần chạy đồng thời
    /// khác có thể đang dựa vào nó; chỗ giữ mà không ai xác nhận tự hết hạn (D-20261125-scim-sync-report).
    async fn with_member_limit<T, F>(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        will_hold: bool,
        change: F,
    ) -> Result<T, Error>
    where
        F: std::future::Future<Output = Result<(T, bool), Error>>,
    {
        let held = has_membership(tenant, user_id).await?;
        let reference = member_reference(user_id);
        let joining = !held && will_hold;
        if joining {
            self.entitlements.reserve(tenant.id(), &MEMBERS_LIMIT, 1, &reference).await?;
        }
        let (value, changed) = match change.await {
            Ok(done) => done,
            Err(error) => {
                if joining {
                    self.entitlements.release(tenant.id(), &MEMBERS_LIMIT, &reference).await?;
                }
                return Err(error);
            }
        };
        if !changed {
            return Ok(value);
        }
        let holds = has_membership(tenant, user_id).await?;
        match (joining, holds) {
            (true, true) => {
                self.entitlements.confirm(tenant.id(), &MEMBERS_LIMIT, &reference).await?
            }
            (true, false) => {
                self.entitlements.release(tenant.id(), &MEMBERS_LIMIT, &reference).await?
            }
            (false, false) if held => {
                self.entitlements.release(tenant.id(), &MEMBERS_LIMIT, &reference).await?
            }
            _ => {}
        }
        Ok(value)
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

    /// Thay toàn bộ tên, mô tả và permission của role `id` (đường hệ thống, không kiểm người sửa) — chỉ để test
    /// dựng dữ liệu; code chạy thật dùng [`update_as`](Self::update_as).
    #[cfg(test)]
    pub(crate) async fn update(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        input: RoleInput,
        expected: Option<Timestamp>,
    ) -> Result<Role, Error> {
        self.update_checked(tenant, actor, None, id, input, expected).await
    }

    /// Như `update` (đường chỉ có trong test), do `principal` làm (REST, MCP): không thêm, nới hay mở cấm permission mà
    /// chính họ không có (PT-9, [`grants::rules_beyond`]).
    pub(crate) async fn update_as(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        input: RoleInput,
        expected: Option<Timestamp>,
    ) -> Result<Role, Error> {
        self.update_checked(tenant, Actor::from(principal), Some(principal), id, input, expected)
            .await
    }

    async fn update_checked(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        authority: Option<&Principal>,
        id: Uuid,
        input: RoleInput,
        expected: Option<Timestamp>,
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
            && permissions.grants.is_empty()
            && permissions.denies.is_empty();
        if current.all_permissions && !only_mfa {
            return Err(builtin_role());
        }
        // Nới yêu cầu MFA của role (kể cả role `admin`) là mở quyền cho người giữ role chưa bật MFA: người sửa phải
        // trao được role đó (D-20261126-no-self-grant). Siết thì không cần.
        if let Some(authority) = authority
            && current.requires_mfa
            && !requires_mfa
            && !grantable(
                &transaction,
                tenant,
                authority,
                (root_of(tenant.id()).as_uuid(), id),
                true,
            )
            .await?
        {
            transaction.rollback().await?;
            return Err(role_beyond_you(root_of(tenant.id()).as_uuid(), id));
        }
        if let Some(authority) = authority {
            let mut rules = permissions_of(&transaction, tenant, [id]).await?;
            let held = rules.remove(&id).unwrap_or_default();
            if let Some(permission) =
                grants::rules_beyond(authority, root_of(tenant.id()), &held, &permissions)
            {
                transaction.rollback().await?;
                tracing::warn!(role_id = %id, permission, "role change beyond the editor refused");
                return Err(grants::permission_beyond_you(&permission));
            }
        }
        let before = role_snapshot(&transaction, tenant, &current).await?;
        // Thêm permission cấm cho role đang được quản trị viên cuối cùng giữ thì không còn ai đủ
        // mọi permission (bước 54): chặn như bỏ role `admin` của họ.
        let admins_before = if permissions.denies.is_empty() {
            0
        } else {
            count_active_admins(&transaction, tenant).await?
        };
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
        if admins_before > 0 && count_active_admins(&transaction, tenant).await? == 0 {
            transaction.rollback().await?;
            return Err(last_admin());
        }
        let mut after = role_json(&name, &description, &permissions);
        after["requires_mfa"] = json!(requires_mfa);
        let event = events::RoleUpdated { before: Some(before), after: Some(after) };
        self.outbox.record(&transaction, tenant, actor, role_subject(id), &event).await?;
        transaction.commit().await?;
        tracing::info!(role_id = %id, "role changed");
        self.get(tenant, id).await
    }

    /// Xoá role `id`; người dùng đang có role này mất nó (đường hệ thống, không kiểm người xoá) — chỉ để test;
    /// code chạy thật dùng [`delete_as`](Self::delete_as).
    #[cfg(test)]
    pub(crate) async fn delete(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
    ) -> Result<(), Error> {
        self.delete_checked(tenant, actor, None, id).await
    }

    /// Như `delete` (đường chỉ có trong test), do `principal` làm (REST, MCP): xoá role là gỡ nó khỏi mọi người giữ —
    /// cùng luật với gỡ role khỏi một người (`grantable`, kể cả permission cấm; D-20261126-no-self-grant).
    pub(crate) async fn delete_as(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        self.delete_checked(tenant, Actor::from(principal), Some(principal), id).await
    }

    async fn delete_checked(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        authority: Option<&Principal>,
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
        if let Some(authority) = authority {
            let take = [(root_of(tenant.id()).as_uuid(), id)];
            if let Err(error) = require_grantable(&transaction, tenant, authority, &[], &take).await
            {
                transaction.rollback().await?;
                return Err(error);
            }
        }
        let before = role_snapshot(&transaction, tenant, &current).await?;
        role::Entity::delete_by_id(id).scoped(tenant).exec(&transaction).await?;
        // Luật ánh xạ nhóm SCIM trỏ tới role này và dấu "do SCIM cấp" của nó (bước 66): không có
        // foreign key, xoá cùng transaction (membership xoá theo foreign key của `auth_user_role`).
        scim_grants::forget_role(&transaction, tenant, id).await?;
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
        if input.permissions.len() + input.grants.len() + input.denies.len() > MAX_PERMISSIONS {
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
        let mut denied = BTreeSet::new();
        for (index, permission) in input.denies.iter().enumerate() {
            if !self.catalog.contains(permission) {
                problems.push(FieldError::new(
                    format!("denies[{index}]"),
                    "unknown",
                    "is not a permission of this system",
                ));
            } else if input.permissions.contains(permission) || seen.contains(permission.as_str()) {
                problems.push(
                    FieldError::new(
                        format!("denies[{index}]"),
                        "conflict",
                        "`{permission}` is also granted by this role; a role either grants or \
                         denies a permission",
                    )
                    .with_param("permission", permission),
                );
            }
            denied.insert(permission.clone());
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let mut granted: std::collections::BTreeMap<String, Scope> =
            input.permissions.into_iter().map(|permission| (permission, Scope::Tenant)).collect();
        granted.extend(input.grants);
        let rules = RoleRules {
            grants: granted.into_iter().collect(),
            denies: denied.into_iter().collect(),
        };
        Ok((name, description, rules))
    }
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
fn role_json(name: &str, description: &str, rules: &RoleRules) -> serde_json::Value {
    let names: Vec<&str> = rules.grants.iter().map(|(permission, _)| permission.as_str()).collect();
    let scopes: serde_json::Map<String, serde_json::Value> = rules
        .grants
        .iter()
        .map(|(permission, scope)| (permission.clone(), json!(scope.as_str())))
        .collect();
    let mut json =
        json!({"name": name, "description": description, "permissions": names, "scopes": scopes});
    if !rules.denies.is_empty() {
        json["denies"] = json!(rules.denies);
    }
    json
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
    rules: &RoleRules,
) -> Result<(), Error> {
    let allows = rules.grants.iter().map(|(permission, scope)| (permission, *scope, ALLOW));
    let denies = rules.denies.iter().map(|permission| (permission, Scope::Tenant, DENY));
    for (permission, scope, effect) in allows.chain(denies) {
        role_permission::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            role_id: Set(role_id),
            permission: Set(permission.clone()),
            scope: Set(scope.as_str().to_owned()),
            effect: Set(effect.to_owned()),
        }
        .insert(transaction)
        .await?;
    }
    Ok(())
}

fn to_role(row: role::Model, rules: RoleRules) -> Role {
    let RoleRules { grants, denies } = rules;
    Role {
        id: row.id,
        name: row.name,
        description: row.description,
        all_permissions: row.all_permissions,
        requires_mfa: row.requires_mfa,
        permissions: grants.iter().map(|(permission, _)| permission.clone()).collect(),
        grants,
        denies,
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

fn now() -> Timestamp {
    Timestamp::now()
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

#[cfg(test)]
mod self_grant_tests;
#[cfg(test)]
mod tests;
