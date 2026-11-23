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
            .column((role_permission::Entity, role_permission::Column::Effect))
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
        principal_from_rows(tenant, user_id, &rows, &self.catalog).await
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
        let found = self.user_principal_in(tenant, user_id, None).await?;
        Ok(found.map(|(user, principal, _)| (user, principal)))
    }

    /// Như [`Self::user_principal`], kèm phiên `session` (họ refresh token cấp access token) còn
    /// sống không — vẫn MỘT câu: đăng xuất, thu hồi phiên, đổi mật khẩu có hiệu lực ngay với access
    /// token đã cấp, không chờ token hết hạn (D-20261121-access-token-revocation).
    pub(crate) async fn session_principal(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        session: Uuid,
    ) -> Result<Option<(User, Principal, bool)>, Error> {
        self.user_principal_in(tenant, user_id, Some(session)).await
    }

    async fn user_principal_in(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        session: Option<Uuid>,
    ) -> Result<Option<(User, Principal, bool)>, Error> {
        use crate::session::entity as refresh;
        use sea_orm::FromQueryResult as _;
        use sea_orm::sea_query::{Alias, Asterisk, ExprTrait as _};
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
            .column((role_permission::Entity, role_permission::Column::Effect))
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
        if let Some(session) = session {
            // Phiên còn sống = họ còn refresh token chưa dùng, chưa thu hồi. Đăng xuất thu hồi cả
            // họ; đổi mật khẩu, khoá tài khoản thu hồi token chưa dùng của mọi họ. Index
            // `auth_refresh_token_family_idx` (tenant_id, family_id).
            let mut live = Query::select();
            live.expr(Expr::val(1))
                .from(refresh::Entity)
                .and_where(Expr::col((refresh::Entity, refresh::Column::TenantId)).eq(tenant_id))
                .and_where(Expr::col((refresh::Entity, refresh::Column::FamilyId)).eq(session))
                .and_where(Expr::col((refresh::Entity, refresh::Column::UsedAt)).is_null())
                .and_where(Expr::col((refresh::Entity, refresh::Column::RevokedAt)).is_null());
            select.expr_as(Expr::exists(live), Alias::new(SESSION_LIVE));
        }
        let rows = tenant.database().read().query_all(&select).await?;
        let Some(first) = rows.first() else {
            return Ok(None);
        };
        let user = User::from(user::Model::from_query_result(first, "")?);
        let live = match session {
            Some(_) => first.try_get::<bool>("", SESSION_LIVE)?,
            None => true,
        };
        let principal = principal_from_rows(tenant, user_id, &rows, &self.catalog).await?;
        Ok(Some((user, principal, live)))
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

/// Tên cột "phiên còn sống" trong câu của [`RoleService::session_principal`].
const SESSION_LIVE: &str = "session_live";

/// Giá trị cột `effect` của `auth_role_permission`.
pub(super) const ALLOW: &str = "allow";
pub(super) const DENY: &str = "deny";

/// Luật của một role: permission cấp (kèm độ sâu) và permission cấm, theo thứ tự tên.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RoleRules {
    pub grants: Vec<(String, Scope)>,
    pub denies: Vec<String>,
}

/// Luật của từng role trong `role_ids`, một câu truy vấn cho cả danh sách.
pub(super) async fn permissions_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    role_ids: impl IntoIterator<Item = Uuid>,
) -> Result<HashMap<Uuid, RoleRules>, Error> {
    let rows = role_permission::Entity::find()
        .filter(role_permission::Column::RoleId.is_in(role_ids))
        .scoped(tenant)
        .order_by_asc(role_permission::Column::Permission)
        .all(connection)
        .await?;
    let mut by_role: HashMap<Uuid, RoleRules> = HashMap::new();
    for row in rows {
        let rules = by_role.entry(row.role_id).or_default();
        if row.effect == DENY {
            rules.denies.push(row.permission);
            continue;
        }
        let scope = Scope::parse(&row.scope).unwrap_or(Scope::Tenant);
        rules.grants.push((row.permission, scope));
    }
    Ok(by_role)
}

/// Role có `all_permissions` không và permission (kèm độ sâu) của từng role trong `ids` của khách
/// `tenant`. Role không có: vắng.
pub(crate) async fn role_grants(
    tenant: &TenantContext,
    ids: &[Uuid],
) -> Result<std::collections::HashMap<Uuid, (bool, RoleRules)>, Error> {
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
/// được bỏ qua. Permission bị một role cấm (`effect = deny`) bị bỏ khỏi kết quả, kể cả khi role
/// khác cấp hay người dùng có role toàn quyền (bước 54) — kể cả role cấm đang chờ MFA: cấm luôn
/// có hiệu lực.
pub(super) async fn principal_from_rows(
    tenant: &TenantContext,
    user_id: Uuid,
    rows: &[sea_orm::QueryResult],
    catalog: &PermissionCatalog,
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
    let mut denied = Vec::new();
    let mut everything = false;
    for row in rows {
        let Some(all_permissions) = row.try_get::<Option<bool>>("", "all_permissions")? else {
            continue;
        };
        let permission = row.try_get::<Option<String>>("", "permission")?;
        if row.try_get::<Option<String>>("", "effect")?.as_deref() == Some(DENY) {
            denied.extend(permission);
            continue;
        }
        if row.try_get::<Option<bool>>("", "requires_mfa")? == Some(true) && has_mfa == Some(false)
        {
            continue;
        }
        // Role `admin` chỉ gán được ở nút gốc (`set_memberships` chặn chỗ khác).
        if all_permissions {
            everything = true;
            continue;
        }
        let Some(permission) = permission else {
            continue;
        };
        let org = row.try_get::<Option<Uuid>>("", "org_id")?.map_or(root, OrgId::from_uuid);
        let scope = row
            .try_get::<Option<String>>("", "scope")?
            .and_then(|scope| Scope::parse(&scope))
            .unwrap_or(Scope::Tenant);
        grants.push((permission, Grant::new(org, scope)));
    }
    let principal = if everything {
        Principal::with_all_permissions(user_id)
    } else {
        Principal::with_grants(user_id, grants)
    };
    // `in_tenant` trước `without`: người toàn quyền mở ra grant ở nút gốc của đúng khách.
    Ok(principal.in_tenant(tenant.id()).without(denied, catalog))
}

/// Role trong `ids` (của khách `tenant`) bắt buộc MFA (D-308) mà người dùng `user_id` của khách đó
/// chưa bật MFA. Quyền dựng ở khách khác (uỷ quyền xuống khách con, quyền hỗ trợ) bỏ các role này
/// như [`principal_from_rows`] bỏ ở khách nhà — trước đây đi đường vòng qua khách khác thì vẫn dùng
/// được quyền của role đòi MFA (rà soát bảo mật 2026-11).
pub(crate) async fn roles_blocked_by_mfa(
    tenant: &TenantContext,
    user_id: Uuid,
    ids: &[Uuid],
) -> Result<std::collections::HashSet<Uuid>, Error> {
    if ids.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    let strict: std::collections::HashSet<Uuid> = role::Entity::find()
        .filter(role::Column::Id.is_in(ids.iter().copied()))
        .filter(role::Column::RequiresMfa.eq(true))
        .scoped(tenant)
        .all(tenant.database().read())
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect();
    if strict.is_empty() || has_second_factor(tenant, user_id).await? {
        return Ok(std::collections::HashSet::new());
    }
    Ok(strict)
}

/// Người dùng `user_id` giữ role đòi MFA (D-308) mà yếu tố thứ hai duy nhất là passkey (chưa bật
/// TOTP). Role đòi MFA được tính theo tài khoản *đã bật* yếu tố thứ hai; đăng nhập bằng mật khẩu chỉ hỏi
/// mã TOTP, nên tài khoản như vậy mở được phiên mang quyền của role chỉ bằng mật khẩu. Các cách đăng
/// nhập không qua yếu tố thứ hai từ chối tài khoản này (`auth.passkey_required`): chỉ passkey mở phiên
/// (rà soát bảo mật 2026-11, M-8 của `docs/security/review-2026-11.md`).
pub(crate) async fn passkey_required(tenant: &TenantContext, user_id: Uuid) -> Result<bool, Error> {
    let read = tenant.database().read();
    let holder = user::Entity::find_by_id(user_id)
        .filter(holds_mfa_role(tenant))
        .scoped(tenant)
        .one(read)
        .await?;
    if holder.is_none() {
        return Ok(false);
    }
    let totp = crate::mfa::entity::totp::Entity::find()
        .filter(crate::mfa::entity::totp::Column::UserId.eq(user_id))
        .filter(crate::mfa::entity::totp::Column::ConfirmedAt.is_not_null())
        .scoped(tenant)
        .one(read)
        .await?;
    if totp.is_some() {
        return Ok(false);
    }
    let passkey = crate::passkey::entity::passkey::Entity::find()
        .filter(crate::passkey::entity::passkey::Column::UserId.eq(user_id))
        .scoped(tenant)
        .one(read)
        .await?;
    Ok(passkey.is_some())
}

/// Lỗi khi đăng nhập không qua yếu tố thứ hai mà tài khoản phải dùng passkey ([`passkey_required`]).
pub(crate) fn passkey_required_error() -> Error {
    Error::forbidden(
        "auth.passkey_required",
        "This account must sign in with its passkey: one of its roles requires two-step sign-in, \
         and a passkey is its only second step.",
    )
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
