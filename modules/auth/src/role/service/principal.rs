//! Permission của một người dùng: hợp các role, theo nút; điều kiện "đã có yếu tố thứ hai".

use super::*;

impl RoleService {
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
        principal_from_rows(tenant, user_id, &rows).await
    }

    /// Người dùng `user_id` cùng quyền của họ trong MỘT câu — đường nóng của mọi request đã đăng
    /// nhập (access token, API key): trước đây là hai câu, đọc dòng người dùng rồi mới đọc role
    /// (D-319). `None` khi không có người dùng đó ở khách `tenant`. Không kiểm tài khoản có bị
    /// khoá hay là người mua: bên gọi quyết.
    pub(crate) async fn user_principal(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Option<(User, Principal)>, Error> {
        use sea_orm::FromQueryResult as _;
        use sea_orm::sea_query::{Asterisk, ExprTrait as _};
        let tenant_id = tenant.id().as_uuid();
        // Mỗi dòng: cả dòng người dùng, kèm một role và một permission của role đó. Người dùng
        // không có role nào vẫn ra một dòng (LEFT JOIN), cột role là NULL.
        let mut select = Query::select();
        select
            .column((user::Entity, Asterisk))
            .column((role::Entity, role::Column::AllPermissions))
            .column((role::Entity, role::Column::RequiresMfa))
            .column((user_role::Entity, user_role::Column::OrgId))
            .column((role_permission::Entity, role_permission::Column::Permission))
            .column((role_permission::Entity, role_permission::Column::Scope))
            .from(user::Entity)
            .left_join(
                user_role::Entity,
                Expr::col((user_role::Entity, user_role::Column::UserId))
                    .equals((user::Entity, user::Column::Id))
                    .and(Expr::col((user_role::Entity, user_role::Column::TenantId)).eq(tenant_id)),
            )
            .left_join(
                role::Entity,
                Expr::col((role::Entity, role::Column::Id))
                    .equals((user_role::Entity, user_role::Column::RoleId)),
            )
            .left_join(
                role_permission::Entity,
                Expr::col((role_permission::Entity, role_permission::Column::RoleId))
                    .equals((role::Entity, role::Column::Id)),
            )
            .and_where(Expr::col((user::Entity, user::Column::Id)).eq(user_id))
            .and_where(Expr::col((user::Entity, user::Column::TenantId)).eq(tenant_id));
        let rows = tenant.database().read().query_all(&select).await?;
        let Some(first) = rows.first() else {
            return Ok(None);
        };
        let user = User::from(user::Model::from_query_result(first, "")?);
        let principal = principal_from_rows(tenant, user_id, &rows).await?;
        Ok(Some((user, principal)))
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
}

/// Permission (kèm độ sâu) của từng role trong `role_ids`, một câu truy vấn cho cả danh sách.
pub(super) async fn permissions_of<C: ConnectionTrait>(
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

/// Quyền của người dùng `user_id` từ các dòng (role, permission) của [`RoleService::principal`]
/// hay [`RoleService::user_principal`]. Dòng không có role (`all_permissions` NULL, do LEFT JOIN)
/// được bỏ qua.
pub(super) async fn principal_from_rows(
    tenant: &TenantContext,
    user_id: Uuid,
    rows: &[sea_orm::QueryResult],
) -> Result<Principal, Error> {
    // Role bắt buộc MFA (D-308): chỉ khi có role như vậy mới tra MFA của tài khoản — đường
    // nóng của mọi người khác giữ một câu.
    let mut has_mfa = None;
    for row in rows {
        if row.try_get::<Option<bool>>("", "requires_mfa")? == Some(true) {
            has_mfa = Some(has_second_factor(tenant, user_id).await?);
            break;
        }
    }
    let root = root_of(tenant.id());
    let mut grants = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(all_permissions) = row.try_get::<Option<bool>>("", "all_permissions")? else {
            continue;
        };
        if row.try_get::<Option<bool>>("", "requires_mfa")? == Some(true) && has_mfa == Some(false)
        {
            continue;
        }
        // Role `admin` chỉ gán được ở nút gốc (`set_memberships` chặn chỗ khác).
        if all_permissions {
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

/// Điều kiện SQL đặt trong câu đọc `auth_user`: người dùng của dòng đang đọc giữ ít nhất một
/// role bắt buộc MFA ở khách `tenant` (dùng cho cờ `mfa_required` của `GET /me`, không tốn câu riêng).
pub(crate) fn holds_mfa_role(tenant: &TenantContext) -> Expr {
    let tenant_id = tenant.id().as_uuid();
    let held = Query::select()
        .expr(Expr::val(1))
        .from(user_role::Entity)
        .inner_join(
            role::Entity,
            Expr::col((role::Entity, role::Column::Id))
                .equals((user_role::Entity, user_role::Column::RoleId)),
        )
        .and_where(
            Expr::col((user_role::Entity, user_role::Column::UserId))
                .equals((user::Entity, user::Column::Id)),
        )
        .and_where(Expr::col((user_role::Entity, user_role::Column::TenantId)).eq(tenant_id))
        .and_where(Expr::col((role::Entity, role::Column::TenantId)).eq(tenant_id))
        .and_where(Expr::col((role::Entity, role::Column::RequiresMfa)).eq(true))
        .to_owned();
    Expr::exists(held)
}

/// Điều kiện SQL đặt trong câu đọc `auth_user`: người dùng của dòng đang đọc đã bật MFA (TOTP đã
/// xác nhận, hay có passkey) — cùng nghĩa với [`has_second_factor`].
pub(crate) fn has_second_factor_condition(tenant: &TenantContext) -> Expr {
    use crate::mfa::entity::totp;
    use crate::passkey::entity::passkey;
    let tenant_id = tenant.id().as_uuid();
    let totp = Query::select()
        .expr(Expr::val(1))
        .from(totp::Entity)
        .and_where(
            Expr::col((totp::Entity, totp::Column::UserId))
                .equals((user::Entity, user::Column::Id)),
        )
        .and_where(Expr::col((totp::Entity, totp::Column::TenantId)).eq(tenant_id))
        .and_where(Expr::col((totp::Entity, totp::Column::ConfirmedAt)).is_not_null())
        .to_owned();
    let passkey = Query::select()
        .expr(Expr::val(1))
        .from(passkey::Entity)
        .and_where(
            Expr::col((passkey::Entity, passkey::Column::UserId))
                .equals((user::Entity, user::Column::Id)),
        )
        .and_where(Expr::col((passkey::Entity, passkey::Column::TenantId)).eq(tenant_id))
        .to_owned();
    Expr::exists(totp).or(Expr::exists(passkey))
}
