//! Liên kết tài khoản với danh tính ở nhà cung cấp ngoài (bảng `auth_identity`, bước 40 P3): một
//! dòng cho mỗi `(nhà cung cấp, subject)`. Email và cờ xác minh là bản chụp lúc đăng nhập gần
//! nhất — dữ liệu cá nhân: có trong bản xuất, xoá cùng tài khoản.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
    SqlErr,
};
use uuid::Uuid;

pub(crate) mod entity {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_identity")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub user_id: Uuid,
        pub provider_id: Uuid,
        pub subject: String,
        pub email: Option<String>,
        pub email_verified: bool,
        pub created_at: Timestamp,
        pub last_login_at: Option<Timestamp>,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

pub(crate) type Identity = entity::Model;

fn now() -> Timestamp {
    Timestamp::now()
}

/// Danh tính `subject` ở nhà cung cấp `provider_id`.
pub(crate) async fn find(
    tenant: &TenantContext,
    provider_id: Uuid,
    subject: &str,
) -> Result<Option<Identity>, Error> {
    Ok(entity::Entity::find()
        .filter(entity::Column::ProviderId.eq(provider_id))
        .filter(entity::Column::Subject.eq(subject))
        .scoped(tenant)
        .one(tenant.database().read())
        .await?)
}

/// Mọi liên kết của người dùng `user_id`, cũ trước.
pub(crate) async fn for_user(
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<Vec<Identity>, Error> {
    for_user_in(tenant.database().read(), tenant, user_id).await
}

/// Như [`for_user`], đọc qua `connection` của bên gọi (vd transaction đã khoá dòng người dùng).
pub(crate) async fn for_user_in<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<Vec<Identity>, Error> {
    Ok(entity::Entity::find()
        .filter(entity::Column::UserId.eq(user_id))
        .scoped(tenant)
        .order_by_asc(entity::Column::Id)
        .all(connection)
        .await?)
}

/// Danh tính mới nhận được từ nhà cung cấp (đã kiểm).
pub(crate) struct External {
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub name: Option<String>,
}

/// Thêm liên kết trong transaction của bên gọi. Danh tính đã thuộc người khác (hai request đồng
/// thời): `auth.identity_taken`.
pub(crate) async fn insert<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    user_id: Uuid,
    provider_id: Uuid,
    external: &External,
) -> Result<Identity, Error> {
    let now = now();
    entity::ActiveModel {
        tenant_id: Set(tenant.id().as_uuid()),
        id: Set(Uuid::now_v7()),
        user_id: Set(user_id),
        provider_id: Set(provider_id),
        subject: Set(external.subject.clone()),
        email: Set(external.email.clone()),
        email_verified: Set(external.email_verified),
        created_at: Set(now),
        last_login_at: Set(Some(now)),
    }
    .insert(connection)
    .await
    .map_err(|error| match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => taken(),
        _ => Error::from(error),
    })
}

/// Ghi lần đăng nhập và email mới nhất của danh tính `id`.
pub(crate) async fn touch(
    tenant: &TenantContext,
    id: Uuid,
    external: &External,
) -> Result<(), Error> {
    entity::Entity::update_many()
        .col_expr(entity::Column::LastLoginAt, Expr::value(Some(now())))
        .col_expr(entity::Column::Email, Expr::value(external.email.clone()))
        .col_expr(entity::Column::EmailVerified, Expr::value(external.email_verified))
        .filter(entity::Column::Id.eq(id))
        .scoped(tenant)
        .exec(tenant.database().write())
        .await?;
    Ok(())
}

/// Gỡ liên kết `id` của người dùng `user_id`; `false` khi không có.
pub(crate) async fn delete_one<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    user_id: Uuid,
    id: Uuid,
) -> Result<bool, Error> {
    let deleted = entity::Entity::delete_many()
        .filter(entity::Column::Id.eq(id))
        .filter(entity::Column::UserId.eq(user_id))
        .scoped(tenant)
        .exec(connection)
        .await?;
    Ok(deleted.rows_affected > 0)
}

/// Xoá mọi liên kết của người dùng `user_id` (xoá tài khoản).
pub(crate) async fn delete_for_user<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<(), Error> {
    entity::Entity::delete_many()
        .filter(entity::Column::UserId.eq(user_id))
        .scoped(tenant)
        .exec(connection)
        .await?;
    Ok(())
}

/// Xoá mọi liên kết tới nhà cung cấp `provider_id` (xoá nhà cung cấp).
pub(crate) async fn delete_for_provider<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    provider_id: Uuid,
) -> Result<(), Error> {
    entity::Entity::delete_many()
        .filter(entity::Column::ProviderId.eq(provider_id))
        .scoped(tenant)
        .exec(connection)
        .await?;
    Ok(())
}

pub(crate) fn taken() -> Error {
    Error::conflict(
        "auth.identity_taken",
        "This sign-in is already linked to another account. Remove it there first.",
    )
}

pub(crate) fn not_found(id: Uuid) -> Error {
    Error::not_found("auth.identity_not_found", "You have no sign-in link with id `{id}`.")
        .with_param("id", id)
}
