//! Role `admin` dựng sẵn: gán, đếm quản trị viên còn hoạt động, chặn bỏ quản trị viên cuối cùng.

use super::*;

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

/// ID của người dùng còn hoạt động có role mang mọi permission, theo thứ tự ID (chỉ đọc).
pub(crate) async fn active_admin_ids<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
) -> Result<Vec<Uuid>, Error> {
    let id = tenant.id().as_uuid();
    let rows = user_role::Entity::find()
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
        .all(connection)
        .await?;
    let mut ids: Vec<Uuid> = rows.into_iter().map(|row| row.user_id).collect();
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// Lỗi khi một thay đổi sẽ để hệ thống không còn admin nào hoạt động.
pub(crate) fn last_admin() -> Error {
    Error::conflict(
        "auth.last_admin",
        "This change would leave no active user with the `admin` role, and nobody could manage \
         the system any more. Give the `admin` role to another user first.",
    )
}
