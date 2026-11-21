//! Membership do SCIM cấp (bước 66): nhóm của hệ thống danh tính → role ở nút theo luật ánh xạ
//! của khách. Chỉ dòng membership SCIM tự tạo (ID ghi ở `auth_scim_grants.user_role_id`) bị SCIM gỡ;
//! membership gán tay — kể cả khi trùng (nút, role), kể cả khi người quản trị gán lại đúng role SCIM
//! đã cấp — không bao giờ bị đụng.

use super::*;
use crate::scim::entity::grant;

impl RoleService {
    /// Đặt membership do SCIM cấp của người `user_id` thành đúng `wanted` (nút, role). Trả `true`
    /// khi có thay đổi. Giữ hạn mức số nhân sự của gói như mọi lần gán role; không gỡ được admin
    /// cuối cùng (`auth.last_admin`). Tài khoản không có (vừa bị xoá) hay là người mua: bỏ qua.
    pub(crate) async fn sync_scim_grants(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        user_id: Uuid,
        wanted: BTreeSet<(Uuid, Uuid)>,
    ) -> Result<bool, Error> {
        let read = tenant.database().read();
        let grants: BTreeSet<Uuid> = grant::Entity::find()
            .filter(grant::Column::UserId.eq(user_id))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .map(|row| row.user_role_id)
            .collect();
        // Còn giữ role nào sau lần này không (cho hạn mức số nhân sự): role SCIM muốn, hay
        // membership không do SCIM tạo. Đọc ngoài transaction như `set_memberships`.
        let manual = user_role::Entity::find()
            .filter(user_role::Column::UserId.eq(user_id))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .any(|row| !grants.contains(&row.id));
        let will_hold = !wanted.is_empty() || manual;
        let change = self.write_scim_grants(tenant, actor, user_id, wanted);
        self.with_member_limit(tenant, user_id, will_hold, change).await
    }

    async fn write_scim_grants(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        user_id: Uuid,
        wanted: BTreeSet<(Uuid, Uuid)>,
    ) -> Result<bool, Error> {
        let root = root_of(tenant.id()).as_uuid();
        let transaction = tenant.database().begin_write().await?;
        // Khoá dòng người dùng trước, role toàn quyền sau (cùng thứ tự với `set_memberships`).
        let locked = autoxide_kernel::rows::lock_one(
            &transaction,
            user::Entity::find_by_id(user_id)
                .filter(user::Column::Realm.eq("staff"))
                .scoped(tenant),
        )
        .await?;
        if locked.is_none() {
            transaction.rollback().await?;
            return Ok(false);
        }
        let held: HashMap<Uuid, (Uuid, Uuid)> = user_role::Entity::find()
            .filter(user_role::Column::UserId.eq(user_id))
            .scoped(tenant)
            .all(&transaction)
            .await?
            .into_iter()
            .map(|row| (row.id, (row.org_id.unwrap_or(root), row.role_id)))
            .collect();
        let grants = grant::Entity::find()
            .filter(grant::Column::UserId.eq(user_id))
            .scoped(tenant)
            .all(&transaction)
            .await?;
        // Dấu còn hiệu lực: dòng membership SCIM tạo vẫn còn. Dòng đã bị gỡ hay thay bằng tay thì
        // dấu hết hiệu lực — SCIM không gỡ thứ người khác đã đặt lại.
        let (live, stale): (Vec<&grant::Model>, Vec<&grant::Model>) =
            grants.iter().partition(|row| held.contains_key(&row.user_role_id));
        let granted: BTreeSet<(Uuid, Uuid)> =
            live.iter().map(|row| (row.org_id, row.role_id)).collect();
        let held_pairs: BTreeSet<(Uuid, Uuid)> = held.values().copied().collect();
        // Thêm: (nút, role) muốn mà chưa có membership nào. Đã có membership gán tay thì không
        // ghi nhận là của SCIM — để lần gỡ sau không lấy mất membership của người.
        let add: Vec<(Uuid, Uuid)> = wanted
            .iter()
            .filter(|pair| !granted.contains(pair) && !held_pairs.contains(pair))
            .copied()
            .collect();
        let remove: Vec<&grant::Model> = live
            .iter()
            .copied()
            .filter(|row| !wanted.contains(&(row.org_id, row.role_id)))
            .collect();
        if add.is_empty() && remove.is_empty() && stale.is_empty() {
            transaction.rollback().await?;
            return Ok(false);
        }
        let admins_before = count_active_admins(&transaction, tenant).await?;
        let before = memberships_json(&memberships_of(&transaction, tenant, user_id).await?);
        let now = now();
        for row in &stale {
            grant::Entity::delete_many()
                .filter(grant::Column::Id.eq(row.id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        for (org_id, role_id) in &add {
            let membership = Uuid::now_v7();
            user_role::ActiveModel {
                id: Set(membership),
                tenant_id: Set(tenant.id().as_uuid()),
                user_id: Set(user_id),
                role_id: Set(*role_id),
                org_id: Set(Some(*org_id)),
                created_at: Set(now),
            }
            .insert(&transaction)
            .await?;
            grant::ActiveModel {
                tenant_id: Set(tenant.id().as_uuid()),
                id: Set(Uuid::now_v7()),
                user_role_id: Set(membership),
                user_id: Set(user_id),
                org_id: Set(*org_id),
                role_id: Set(*role_id),
                created_at: Set(now),
            }
            .insert(&transaction)
            .await?;
        }
        for row in &remove {
            user_role::Entity::delete_many()
                .filter(user_role::Column::Id.eq(row.user_role_id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
            grant::Entity::delete_many()
                .filter(grant::Column::Id.eq(row.id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        if admins_before > 0 && count_active_admins(&transaction, tenant).await? == 0 {
            transaction.rollback().await?;
            return Err(last_admin());
        }
        let after = memberships_json(&memberships_of(&transaction, tenant, user_id).await?);
        let changed = before != after;
        if changed {
            let event = events::MembershipsChanged { before: Some(before), after: Some(after) };
            self.outbox
                .record(&transaction, tenant, actor, events::user_subject(user_id), &event)
                .await?;
        }
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, changed, "SCIM memberships synced");
        Ok(changed)
    }
}

/// Xoá dấu "do SCIM cấp" của người `user_id` (xoá tài khoản; membership xoá theo foreign key).
pub(crate) async fn forget_scim_grants<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<(), Error> {
    grant::Entity::delete_many()
        .filter(grant::Column::UserId.eq(user_id))
        .scoped(tenant)
        .exec(connection)
        .await?;
    Ok(())
}

/// Xoá luật ánh xạ nhóm SCIM trỏ tới role `role_id` và dấu "do SCIM cấp" của nó (xoá role).
pub(super) async fn forget_role<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    role_id: Uuid,
) -> Result<(), Error> {
    use crate::scim::entity::mapping;
    mapping::Entity::delete_many()
        .filter(mapping::Column::RoleId.eq(role_id))
        .scoped(tenant)
        .exec(connection)
        .await?;
    grant::Entity::delete_many()
        .filter(grant::Column::RoleId.eq(role_id))
        .scoped(tenant)
        .exec(connection)
        .await?;
    Ok(())
}
