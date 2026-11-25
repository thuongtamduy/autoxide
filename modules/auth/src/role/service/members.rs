//! Gán role cho người dùng và membership theo nút; đếm người theo role và theo nút.

use super::*;

impl RoleService {
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
        let will_hold = !role_ids.is_empty();
        // Lần gán này luôn ghi (kể cả khi trùng role cũ): coi là "có đổi" cho hạn mức.
        let change = async {
            self.write_user_roles(tenant, actor, user_id, role_ids).await.map(|roles| (roles, true))
        };
        self.with_member_limit(tenant, user_id, will_hold, change).await
    }

    pub(super) async fn write_user_roles(
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
        let will_hold = memberships.iter().any(|(_, roles)| !roles.is_empty());
        let change = async {
            let written = self.write_memberships(tenant, actor, user_id, memberships).await;
            written.map(|memberships| (memberships, true))
        };
        self.with_member_limit(tenant, user_id, will_hold, change).await
    }

    pub(super) async fn write_memberships(
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

    /// Số người giữ mỗi role của khách (ở bất kỳ nút nào), mỗi người một lần; mọi role, kể cả role
    /// không ai giữ (0). Hai câu: các role, và các cặp (role, người) khác nhau.
    pub(crate) async fn role_member_counts(
        &self,
        tenant: &TenantContext,
    ) -> Result<Vec<(Uuid, usize)>, Error> {
        use std::collections::{BTreeMap, BTreeSet};
        let connection = tenant.database().read();
        let roles: Vec<Uuid> = role::Entity::find()
            .select_only()
            .column(role::Column::Id)
            .scoped(tenant)
            .order_by_asc(role::Column::Id)
            .into_tuple()
            .all(connection)
            .await?;
        let pairs: Vec<(Uuid, Uuid)> = user_role::Entity::find()
            .select_only()
            .column(user_role::Column::RoleId)
            .column(user_role::Column::UserId)
            .distinct()
            .scoped(tenant)
            .into_tuple()
            .all(connection)
            .await?;
        let mut holders: BTreeMap<Uuid, BTreeSet<Uuid>> = BTreeMap::new();
        for (role, user) in pairs {
            holders.entry(role).or_default().insert(user);
        }
        Ok(roles
            .into_iter()
            .map(|role| (role, holders.get(&role).map_or(0, BTreeSet::len)))
            .collect())
    }

    /// Số người ở mỗi nút của cây: `direct` = người có membership ở chính nút, `total` = người có
    /// membership ở nút hay ở một nút trên nó (cùng nghĩa với `inherited=true` của danh sách). Mỗi
    /// người tính một lần dù có nhiều role. Mọi nút của cây (kể cả nút không có ai), nút gốc trước.
    /// Hai câu: các cặp (nút, người) khác nhau, và các cặp tổ tiên – con cháu của cây.
    pub(crate) async fn member_counts(
        &self,
        tenant: &TenantContext,
    ) -> Result<Vec<MemberCount>, Error> {
        use std::collections::{BTreeMap, BTreeSet};
        let connection = tenant.database().read();
        let root = root_of(tenant.id()).as_uuid();
        let rows: Vec<(Option<Uuid>, Uuid)> = user_role::Entity::find()
            .select_only()
            .column(user_role::Column::OrgId)
            .column(user_role::Column::UserId)
            .distinct()
            .scoped(tenant)
            .into_tuple()
            .all(connection)
            .await?;
        let mut people: BTreeMap<Uuid, BTreeSet<Uuid>> = BTreeMap::new();
        for (org, user) in rows {
            people.entry(org.unwrap_or(root)).or_default().insert(user);
        }
        let pairs = OrgTree::pairs(connection, tenant.id()).await?;
        // Tổ tiên (gồm chính nó) của từng nút; nút gốc luôn có, kể cả khi cây chưa được tạo.
        let mut ancestors: BTreeMap<Uuid, Vec<Uuid>> = BTreeMap::from([(root, vec![root])]);
        for (ancestor, descendant) in pairs {
            let list = ancestors.entry(descendant.as_uuid()).or_default();
            if !list.contains(&ancestor.as_uuid()) {
                list.push(ancestor.as_uuid());
            }
        }
        let mut counts: Vec<MemberCount> = ancestors
            .iter()
            .map(|(org, above)| {
                let reach: BTreeSet<&Uuid> =
                    above.iter().filter_map(|id| people.get(id)).flatten().collect();
                MemberCount {
                    org_id: *org,
                    direct: people.get(org).map_or(0, BTreeSet::len),
                    total: reach.len(),
                }
            })
            .collect();
        counts.sort_by_key(|count| (count.org_id != root, count.org_id));
        Ok(counts)
    }
}

pub(super) async fn roles_of_user<C: ConnectionTrait>(
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

/// Membership của người `user_id`, gom theo nút (nút theo ID; role theo tên).
pub(super) async fn memberships_of<C: ConnectionTrait>(
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
pub(super) fn memberships_json(memberships: &[Membership]) -> serde_json::Value {
    let map: serde_json::Map<String, serde_json::Value> = memberships
        .iter()
        .map(|membership| (membership.org_id.to_string(), role_names(&membership.roles)))
        .collect();
    serde_json::Value::Object(map)
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
