//! Service của role: tạo, sửa, xoá role; gán role cho người dùng; tính permission
//! của một người dùng.

use std::collections::{BTreeSet, HashMap};

use autoxide_kernel::access::{PermissionCatalog, Principal};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::{Expr, Query};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder, Set, SqlErr,
};
use uuid::Uuid;

use super::entity::{role, role_permission, user_role};
use crate::events;
use crate::user::entity as user;
use autoxide_kernel::event::{Actor, Outbox};
use serde_json::json;

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
    /// Theo thứ tự tên.
    pub permissions: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Dữ liệu để tạo hay thay toàn bộ một role.
#[derive(Debug, Clone)]
pub(crate) struct RoleInput {
    pub name: String,
    pub description: String,
    pub permissions: Vec<String>,
}

pub(crate) struct RoleService {
    database: Database,
    catalog: PermissionCatalog,
    outbox: Outbox,
}

impl RoleService {
    pub(crate) fn new(database: Database, catalog: PermissionCatalog, outbox: Outbox) -> Self {
        RoleService { database, catalog, outbox }
    }

    /// Mọi permission của chương trình.
    pub(crate) fn catalog(&self) -> &PermissionCatalog {
        &self.catalog
    }

    /// Một trang role, mới tạo trước (mặc định).
    pub(crate) async fn list(&self, page: &PageRequest) -> Result<Page<Role>, Error> {
        let connection = self.database.read();
        let page =
            fetch_page(role::Entity::find(), role::Column::Id, |row| row.id, page, connection)
                .await?;
        let mut permissions = permissions_of(connection, page.data.iter().map(|r| r.id)).await?;
        Ok(page.map(|row| {
            let granted = permissions.remove(&row.id).unwrap_or_default();
            to_role(row, granted)
        }))
    }

    pub(crate) async fn get(&self, id: Uuid) -> Result<Role, Error> {
        let connection = self.database.read();
        let row = role::Entity::find_by_id(id)
            .one(connection)
            .await?
            .ok_or_else(|| role_not_found(id))?;
        let mut permissions = permissions_of(connection, [id]).await?;
        Ok(to_role(row, permissions.remove(&id).unwrap_or_default()))
    }

    pub(crate) async fn create(&self, actor: Actor, input: RoleInput) -> Result<Role, Error> {
        let (name, description, permissions) = self.check(input)?;
        let now = now();
        let id = Uuid::now_v7();
        let transaction = self.database.begin_write().await?;
        role::ActiveModel {
            id: Set(id),
            name: Set(name.clone()),
            description: Set(description.clone()),
            all_permissions: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await
        .map_err(|error| name_taken_or(error, &name))?;
        insert_permissions(&transaction, id, &permissions).await?;
        let after =
            Some(json!({"name": name, "description": description, "permissions": permissions}));
        let event = events::RoleCreated { before: None, after };
        self.outbox.record(&transaction, actor, role_subject(id), &event).await?;
        transaction.commit().await?;
        tracing::info!(role_id = %id, "role created");
        self.get(id).await
    }

    /// Thay toàn bộ tên, mô tả và permission của role `id`.
    pub(crate) async fn update(
        &self,
        actor: Actor,
        id: Uuid,
        input: RoleInput,
    ) -> Result<Role, Error> {
        let (name, description, permissions) = self.check(input)?;
        let transaction = self.database.begin_write().await?;
        let current = role::Entity::find_by_id(id)
            .one(&transaction)
            .await?
            .ok_or_else(|| role_not_found(id))?;
        if current.all_permissions {
            return Err(builtin_role());
        }
        let before = role_snapshot(&transaction, &current).await?;
        role::Entity::update_many()
            .col_expr(role::Column::Name, Expr::value(name.clone()))
            .col_expr(role::Column::Description, Expr::value(description.clone()))
            .col_expr(role::Column::UpdatedAt, Expr::value(now()))
            .filter(role::Column::Id.eq(id))
            .exec(&transaction)
            .await
            .map_err(|error| name_taken_or(error, &name))?;
        role_permission::Entity::delete_many()
            .filter(role_permission::Column::RoleId.eq(id))
            .exec(&transaction)
            .await?;
        insert_permissions(&transaction, id, &permissions).await?;
        let after = json!({"name": name, "description": description, "permissions": permissions});
        let event = events::RoleUpdated { before: Some(before), after: Some(after) };
        self.outbox.record(&transaction, actor, role_subject(id), &event).await?;
        transaction.commit().await?;
        tracing::info!(role_id = %id, "role changed");
        self.get(id).await
    }

    /// Xoá role `id`; người dùng đang có role này mất nó.
    pub(crate) async fn delete(&self, actor: Actor, id: Uuid) -> Result<(), Error> {
        let transaction = self.database.begin_write().await?;
        let current = role::Entity::find_by_id(id)
            .one(&transaction)
            .await?
            .ok_or_else(|| role_not_found(id))?;
        if current.all_permissions {
            return Err(builtin_role());
        }
        let before = role_snapshot(&transaction, &current).await?;
        role::Entity::delete_by_id(id).exec(&transaction).await?;
        let event = events::RoleDeleted { before: Some(before), after: None };
        self.outbox.record(&transaction, actor, role_subject(id), &event).await?;
        transaction.commit().await?;
        tracing::info!(role_id = %id, "role deleted");
        Ok(())
    }

    /// Các role của người dùng `user_id`, theo thứ tự tên.
    pub(crate) async fn user_roles(&self, user_id: Uuid) -> Result<Vec<Role>, Error> {
        let connection = self.database.read();
        if user::Entity::find_by_id(user_id).one(connection).await?.is_none() {
            return Err(crate::user::user_not_found(user_id));
        }
        roles_of_user(connection, user_id).await
    }

    /// Thay toàn bộ role của người dùng `user_id` bằng `role_ids`.
    pub(crate) async fn set_user_roles(
        &self,
        actor: Actor,
        user_id: Uuid,
        role_ids: Vec<Uuid>,
    ) -> Result<Vec<Role>, Error> {
        let role_ids: BTreeSet<Uuid> = role_ids.into_iter().collect();
        if role_ids.len() > MAX_ROLES_PER_USER {
            return Err(Error::validation(vec![FieldError::new(
                "role_ids",
                "length",
                format!("must list at most {MAX_ROLES_PER_USER} roles"),
            )]));
        }
        let transaction = self.database.begin_write().await?;
        // Cập nhật dòng người dùng trước: khoá dòng đó tới hết transaction, nên hai
        // request gán role cho cùng một người chạy lần lượt (Postgres); SQLite vốn
        // chỉ một bên ghi. Đồng thời biết người dùng có tồn tại không.
        let touched = user::Entity::update_many()
            .col_expr(user::Column::UpdatedAt, Expr::value(now()))
            .filter(user::Column::Id.eq(user_id))
            .exec(&transaction)
            .await?;
        if touched.rows_affected == 0 {
            return Err(crate::user::user_not_found(user_id));
        }
        let found = role::Entity::find()
            .filter(role::Column::Id.is_in(role_ids.iter().copied()))
            .count(&transaction)
            .await?;
        if found != role_ids.len() as u64 {
            return Err(Error::validation(vec![FieldError::new(
                "role_ids",
                "unknown",
                "every entry must be the id of an existing role",
            )]));
        }
        let admins_before = count_active_admins(&transaction).await?;
        let before = role_names(&roles_of_user(&transaction, user_id).await?);
        user_role::Entity::delete_many()
            .filter(user_role::Column::UserId.eq(user_id))
            .exec(&transaction)
            .await?;
        let now = now();
        for role_id in &role_ids {
            user_role::ActiveModel {
                id: Set(Uuid::now_v7()),
                user_id: Set(user_id),
                role_id: Set(*role_id),
                created_at: Set(now),
            }
            .insert(&transaction)
            .await?;
        }
        // Chỉ chặn khi thay đổi này lấy đi admin cuối cùng. Lúc mới cài chưa có admin
        // nào thì việc gán role vẫn được phép.
        if admins_before > 0 && count_active_admins(&transaction).await? == 0 {
            transaction.rollback().await?;
            return Err(last_admin());
        }
        let after = roles_of_user(&transaction, user_id).await?;
        let event =
            events::UserRolesChanged { before: Some(before), after: Some(role_names(&after)) };
        self.outbox.record(&transaction, actor, events::user_subject(user_id), &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, roles = role_ids.len(), "user roles changed");
        Ok(after)
    }

    /// Bên gọi là người dùng `user_id`: permission của họ là hợp các permission của
    /// mọi role của họ. Hai câu truy vấn, không phụ thuộc số role.
    pub(crate) async fn principal(&self, user_id: Uuid) -> Result<Principal, Error> {
        use sea_orm::sea_query::ExprTrait as _;
        // MỘT câu cho mọi request đã đăng nhập (đường nóng): mỗi dòng là một role của
        // người dùng kèm một permission của role đó (NULL nếu role không có permission
        // nào). Trước đây là hai câu — trên Postgres mỗi câu thêm một vòng mạng (D-076).
        let mut select = Query::select();
        select
            .column((role::Entity, role::Column::AllPermissions))
            .column((role_permission::Entity, role_permission::Column::Permission))
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
            .and_where(Expr::col((user_role::Entity, user_role::Column::UserId)).eq(user_id));
        let rows = self.database.read().query_all(&select).await?;
        let mut permissions = Vec::with_capacity(rows.len());
        for row in rows {
            if row.try_get::<bool>("", "all_permissions")? {
                return Ok(Principal::with_all_permissions(user_id));
            }
            if let Some(permission) = row.try_get::<Option<String>>("", "permission")? {
                permissions.push(permission);
            }
        }
        Ok(Principal::new(user_id, permissions))
    }

    /// Kiểm và chuẩn hoá dữ liệu vào; báo mọi trường sai cùng lúc.
    fn check(&self, input: RoleInput) -> Result<(String, String, Vec<String>), Error> {
        let name = input.name.trim().to_owned();
        let description = input.description.trim().to_owned();
        let mut problems = Vec::new();
        if !is_role_name(&name) {
            problems.push(FieldError::new(
                "name",
                "pattern",
                format!(
                    "must be 1 to {NAME_MAX_LENGTH} characters: lowercase letters, digits, `_` \
                     and `-`, starting with a letter"
                ),
            ));
        }
        if description.chars().count() > DESCRIPTION_MAX_LENGTH {
            problems.push(FieldError::new(
                "description",
                "length",
                format!("must be at most {DESCRIPTION_MAX_LENGTH} characters long"),
            ));
        }
        if input.permissions.len() > MAX_PERMISSIONS {
            problems.push(FieldError::new(
                "permissions",
                "length",
                format!("must list at most {MAX_PERMISSIONS} permissions"),
            ));
        }
        for (index, permission) in input.permissions.iter().enumerate() {
            if !self.catalog.contains(permission) {
                problems.push(FieldError::new(
                    format!("permissions[{index}]"),
                    "unknown",
                    "is not a permission of this system; see `GET /api/v1/auth/permissions`",
                ));
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let permissions: BTreeSet<String> = input.permissions.into_iter().collect();
        Ok((name, description, permissions.into_iter().collect()))
    }
}

/// Gán role `admin` (role có mọi permission) cho người dùng `user_id`, trong
/// transaction của bên gọi.
pub(crate) async fn grant_admin<C: ConnectionTrait>(
    connection: &C,
    user_id: Uuid,
) -> Result<(), Error> {
    let admin = role::Entity::find()
        .filter(role::Column::AllPermissions.eq(true))
        .one(connection)
        .await?
        .ok_or_else(|| {
            Error::internal("the built-in `admin` role is missing; run the migrations")
        })?;
    user_role::ActiveModel {
        id: Set(Uuid::now_v7()),
        user_id: Set(user_id),
        role_id: Set(admin.id),
        created_at: Set(now()),
    }
    .insert(connection)
    .await?;
    Ok(())
}

/// Chủ thể (subject) của event về role `id`.
fn role_subject(id: Uuid) -> Option<String> {
    Some(format!("auth.role:{id}"))
}

/// Ảnh chụp một role cho audit: tên, mô tả, permission.
async fn role_snapshot<C: ConnectionTrait>(
    connection: &C,
    row: &role::Model,
) -> Result<serde_json::Value, Error> {
    let mut permissions = permissions_of(connection, [row.id]).await?;
    let granted = permissions.remove(&row.id).unwrap_or_default();
    Ok(json!({"name": row.name, "description": row.description, "permissions": granted}))
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
    role_id: Uuid,
    permissions: &[String],
) -> Result<(), Error> {
    for permission in permissions {
        role_permission::ActiveModel {
            id: Set(Uuid::now_v7()),
            role_id: Set(role_id),
            permission: Set(permission.clone()),
        }
        .insert(transaction)
        .await?;
    }
    Ok(())
}

/// Permission của từng role trong `role_ids`, một câu truy vấn cho cả danh sách.
async fn permissions_of<C: ConnectionTrait>(
    connection: &C,
    role_ids: impl IntoIterator<Item = Uuid>,
) -> Result<HashMap<Uuid, Vec<String>>, Error> {
    let rows = role_permission::Entity::find()
        .filter(role_permission::Column::RoleId.is_in(role_ids))
        .order_by_asc(role_permission::Column::Permission)
        .all(connection)
        .await?;
    let mut by_role: HashMap<Uuid, Vec<String>> = HashMap::new();
    for row in rows {
        by_role.entry(row.role_id).or_default().push(row.permission);
    }
    Ok(by_role)
}

async fn roles_of_user<C: ConnectionTrait>(
    connection: &C,
    user_id: Uuid,
) -> Result<Vec<Role>, Error> {
    let rows = role::Entity::find()
        .filter(
            role::Column::Id.in_subquery(
                Query::select()
                    .column(user_role::Column::RoleId)
                    .from(user_role::Entity)
                    .and_where(user_role::Column::UserId.eq(user_id))
                    .to_owned(),
            ),
        )
        .order_by_asc(role::Column::Name)
        .all(connection)
        .await?;
    let mut permissions = permissions_of(connection, rows.iter().map(|row| row.id)).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let granted = permissions.remove(&row.id).unwrap_or_default();
            to_role(row, granted)
        })
        .collect())
}

/// Số người dùng CÒN HOẠT ĐỘNG (chưa bị khoá) đang có một role mang mọi permission.
pub(crate) async fn count_active_admins<C: ConnectionTrait>(connection: &C) -> Result<u64, Error> {
    let count = user_role::Entity::find()
        .filter(
            user_role::Column::RoleId.in_subquery(
                Query::select()
                    .column(role::Column::Id)
                    .from(role::Entity)
                    .and_where(role::Column::AllPermissions.eq(true))
                    .to_owned(),
            ),
        )
        .filter(
            user_role::Column::UserId.in_subquery(
                Query::select()
                    .column(user::Column::Id)
                    .from(user::Entity)
                    .and_where(user::Column::DisabledAt.is_null())
                    .to_owned(),
            ),
        )
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

fn to_role(row: role::Model, permissions: Vec<String>) -> Role {
    Role {
        id: row.id,
        name: row.name,
        description: row.description,
        all_permissions: row.all_permissions,
        permissions,
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
            name: name.to_owned(),
            description: String::new(),
            permissions: permissions.iter().map(|p| (*p).to_owned()).collect(),
        }
    }

    async fn user(services: &Services, email: &str) -> Uuid {
        let input = NewUser { email: email.to_owned(), name: "U".to_owned(), password: None };
        services.users.create(Actor::system(), input).await.unwrap().id
    }

    async fn admin_role(roles: &RoleService) -> Role {
        let page = roles.list(&PageRequest::first(200, Order::Asc)).await.unwrap();
        page.data.into_iter().find(|role| role.all_permissions).unwrap()
    }

    #[tokio::test]
    async fn role_admin_dung_san_va_khong_sua_xoa_duoc() {
        TestDatabase::run(async |test| {
            let roles = crate::testing::services(test).await.roles;

            let admin = admin_role(&roles).await;

            assert_eq!(admin.name, "admin");
            assert!(admin.permissions.is_empty());
            let changed =
                roles.update(Actor::system(), admin.id, input("boss", &[])).await.unwrap_err();
            let deleted = roles.delete(Actor::system(), admin.id).await.unwrap_err();
            assert_eq!(changed.code(), "auth.role_builtin");
            assert_eq!(deleted.code(), "auth.role_builtin");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tao_sua_xoa_role() {
        TestDatabase::run(async |test| {
            let roles = crate::testing::services(test).await.roles;

            let created = roles
                .create(
                    Actor::system(),
                    input(" editor ", &["notes.write", "auth.roles.read", "notes.write"]),
                )
                .await
                .unwrap();
            assert_eq!(created.name, "editor");
            assert_eq!(created.permissions, ["auth.roles.read", "notes.write"]);
            assert!(!created.all_permissions);

            let changed = roles
                .update(Actor::system(), created.id, input("writer", &["notes.write"]))
                .await
                .unwrap();
            assert_eq!(changed.name, "writer");
            assert_eq!(changed.permissions, ["notes.write"]);
            assert_eq!(roles.get(created.id).await.unwrap(), changed);

            roles.delete(Actor::system(), created.id).await.unwrap();
            assert_eq!(roles.get(created.id).await.unwrap_err().code(), "auth.role_not_found");
            assert_eq!(
                roles.delete(Actor::system(), created.id).await.unwrap_err().code(),
                "auth.role_not_found"
            );
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn du_lieu_sai_va_ten_trung() {
        TestDatabase::run(async |test| {
            let roles = crate::testing::services(test).await.roles;

            let invalid = roles
                .create(Actor::system(), input("Bad Name", &["notes.write", "nope.read"]))
                .await
                .unwrap_err();
            let fields: Vec<(&str, &str)> =
                invalid.fields().iter().map(|f| (f.field.as_str(), f.code.as_str())).collect();
            assert_eq!(fields, [("name", "pattern"), ("permissions[1]", "unknown")]);

            roles.create(Actor::system(), input("editor", &[])).await.unwrap();
            let taken = roles.create(Actor::system(), input("editor", &[])).await.unwrap_err();
            assert_eq!(taken.kind(), ErrorKind::Conflict);
            assert_eq!(taken.code(), "auth.role_name_taken");
            let admin = roles.create(Actor::system(), input("admin", &[])).await.unwrap_err();
            assert_eq!(admin.code(), "auth.role_name_taken");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn quyen_cua_nguoi_dung_la_hop_cac_role() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let roles = &services.roles;
            let reader =
                roles.create(Actor::system(), input("reader", &["auth.roles.read"])).await.unwrap();
            let writer =
                roles.create(Actor::system(), input("writer", &["notes.write"])).await.unwrap();
            let ada = user(&services, "ada@example.com").await;

            assert_eq!(roles.principal(ada).await.unwrap().permissions(), Vec::<String>::new());
            let assigned = roles
                .set_user_roles(Actor::system(), ada, vec![writer.id, reader.id])
                .await
                .unwrap();

            let names: Vec<&str> = assigned.iter().map(|role| role.name.as_str()).collect();
            assert_eq!(names, ["reader", "writer"]);
            let principal = roles.principal(ada).await.unwrap();
            assert_eq!(principal.permissions(), ["auth.roles.read", "notes.write"]);
            assert!(principal.has(&NOTES_WRITE));
            assert_eq!(roles.user_roles(ada).await.unwrap(), assigned);

            // Sửa role: người có role nhận permission mới ngay.
            roles.update(Actor::system(), writer.id, input("writer", &[])).await.unwrap();
            assert!(!roles.principal(ada).await.unwrap().has(&NOTES_WRITE));

            let admin = admin_role(roles).await;
            roles.set_user_roles(Actor::system(), ada, vec![admin.id]).await.unwrap();
            assert!(roles.principal(ada).await.unwrap().has_all_permissions());
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn gan_role_cho_nguoi_hay_role_khong_ton_tai() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let ada = user(&services, "ada@example.com").await;

            let no_user =
                services.roles.set_user_roles(Actor::system(), Uuid::now_v7(), vec![]).await;
            let no_role =
                services.roles.set_user_roles(Actor::system(), ada, vec![Uuid::now_v7()]).await;
            let read_no_user = services.roles.user_roles(Uuid::now_v7()).await;

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
            let roles = &services.roles;
            let admin = admin_role(roles).await;
            let ada = user(&services, "ada@example.com").await;
            let bob = user(&services, "bob@example.com").await;
            roles.set_user_roles(Actor::system(), ada, vec![admin.id]).await.unwrap();

            let error = roles.set_user_roles(Actor::system(), ada, vec![]).await.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Conflict);
            assert_eq!(error.code(), "auth.last_admin");
            assert!(roles.principal(ada).await.unwrap().has_all_permissions());

            // Có admin khác thì bỏ được.
            roles.set_user_roles(Actor::system(), bob, vec![admin.id]).await.unwrap();
            roles.set_user_roles(Actor::system(), ada, vec![]).await.unwrap();
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
            let roles = services.roles.clone();
            let ada = user(&services, "ada@example.com").await;
            let mut sets = Vec::new();
            for index in 0..4 {
                let first = roles
                    .create(Actor::system(), input(&format!("a{index}"), &[]))
                    .await
                    .unwrap()
                    .id;
                let second = roles
                    .create(Actor::system(), input(&format!("b{index}"), &[]))
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
                    tokio::spawn(
                        async move { roles.set_user_roles(Actor::system(), ada, ids).await },
                    )
                })
                .collect();
            for attempt in attempts {
                attempt.await.unwrap().unwrap();
            }

            let last: BTreeSet<Uuid> =
                roles.user_roles(ada).await.unwrap().into_iter().map(|role| role.id).collect();
            assert!(sets.contains(&last), "{last:?}");
        })
        .await
        .unwrap();
    }
}
