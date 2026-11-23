//! Uỷ quyền theo cây khách hàng (bước 34 mục 10, D-310).
//!
//! Khách cha P cấp: "người giữ role R (ở nút gốc của P) được làm việc ở khách con C, tại nút O của
//! C, với các permission của R". Người đó gọi API như thường bằng credential của P, kèm header
//! `Autoxide-Tenant: C`; kernel hỏi [`DelegationService::enter`] mỗi request — thu hồi, hết hạn
//! có hiệu lực ngay request kế. Mọi thay đổi họ làm ở C ghi event với `Actor.access` = uỷ quyền
//! (audit của C thấy); việc cấp, thu hồi ghi ở P.

use autoxide_kernel::access::{AccessPath, Grant, Principal, Realm};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::tenancy::{
    DatabaseRegistry, OrgId, OrgTree, ScopeExt, TenantContext, TenantId, root_of,
};
use autoxide_kernel::time::Timestamp;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::json;
use uuid::Uuid;

use crate::events;

pub(crate) mod entity {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_delegation")]
    pub struct Model {
        /// Khách cha: bên cấp, giữ dòng.
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub child_tenant_id: Uuid,
        /// Role của khách cha.
        pub role_id: Uuid,
        /// Nút của khách con mà quyền gắn vào.
        pub org_id: Uuid,
        pub reason: String,
        pub granted_by: Option<Uuid>,
        pub expires_at: Option<Timestamp>,
        pub revoked_at: Option<Timestamp>,
        pub revoked_by: Option<Uuid>,
        pub created_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

pub(crate) type Delegation = entity::Model;

/// Uỷ quyền mới.
pub(crate) struct NewDelegation {
    pub child_tenant_id: Uuid,
    pub role_id: Uuid,
    pub org_id: Option<Uuid>,
    pub expires_at: Option<Timestamp>,
    pub reason: String,
}

const REASON_MAX: usize = 500;

pub(crate) struct DelegationService {
    pub databases: DatabaseRegistry,
    pub outbox: Outbox,
    /// Mọi permission của chương trình: mở role toàn quyền khi có role cấm (bước 54).
    pub catalog: autoxide_kernel::access::PermissionCatalog,
}

fn now() -> Timestamp {
    Timestamp::now()
}

/// Uỷ quyền còn hiệu lực lúc `at`.
fn live(at: Timestamp) -> Condition {
    Condition::all().add(entity::Column::RevokedAt.is_null()).add(
        Condition::any()
            .add(entity::Column::ExpiresAt.is_null())
            .add(entity::Column::ExpiresAt.gt(at)),
    )
}

impl DelegationService {
    /// Khách cha `parent` cấp uỷ quyền (người gọi đã có `auth.delegations.manage`).
    pub(crate) async fn grant(
        &self,
        parent: &TenantContext,
        principal: &Principal,
        input: NewDelegation,
    ) -> Result<Delegation, Error> {
        let now = now();
        let reason = input.reason.trim().to_owned();
        let mut problems = Vec::new();
        if reason.is_empty() || reason.chars().count() > REASON_MAX {
            problems.push(
                FieldError::new("reason", "length", "must be 1 to {max} characters")
                    .with_param("max", REASON_MAX),
            );
        }
        if input.expires_at.is_some_and(|at| at <= now) {
            problems.push(FieldError::new("expires_at", "past", "must be in the future"));
        }
        let child_id = TenantId::from_uuid(input.child_tenant_id);
        let ancestors = self.databases.tenant_ancestors(child_id).await?;
        if !ancestors.contains(&parent.id()) {
            problems.push(
                FieldError::new(
                    "child_tenant_id",
                    "auth.delegation_not_descendant",
                    "tenant `{tenant}` is not a child of this tenant",
                )
                .with_param("tenant", input.child_tenant_id),
            );
        }
        let org = input.org_id.map(OrgId::from_uuid).unwrap_or(root_of(child_id));
        match crate::role::role_grants(parent, &[input.role_id]).await?.get(&input.role_id) {
            None => problems.push(FieldError::new(
                "role_id",
                "not_found",
                "no such role in this tenant",
            )),
            // Toàn quyền ở một nút con nghĩa là cả permission cấp khách: chỉ cho ở nút gốc.
            Some((true, _)) if org != root_of(child_id) => problems.push(FieldError::new(
                "org_id",
                "invalid",
                "a role with every permission can only be delegated at the root unit",
            )),
            Some(_) => {}
        }
        if problems.is_empty() {
            let child = self.databases.tenant(child_id).await?;
            let lineage = OrgTree::ancestors(child.database().read(), child_id, org).await?;
            if lineage.is_empty() && org != root_of(child_id) {
                problems.push(FieldError::new(
                    "org_id",
                    "auth.membership_org_invalid",
                    "no such unit in the child tenant",
                ));
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let row = entity::ActiveModel {
            tenant_id: Set(parent.id().as_uuid()),
            id: Set(Uuid::now_v7()),
            child_tenant_id: Set(input.child_tenant_id),
            role_id: Set(input.role_id),
            org_id: Set(org.as_uuid()),
            reason: Set(reason),
            granted_by: Set(Some(principal.user_id())),
            expires_at: Set(input.expires_at),
            revoked_at: Set(None),
            revoked_by: Set(None),
            created_at: Set(now),
        };
        let transaction = parent.database().begin_write().await?;
        let row = row.insert(&transaction).await?;
        let event = events::DelegationGranted { before: None, after: Some(describe(&row)) };
        self.outbox
            .record(&transaction, parent, Actor::from(principal), subject(row.id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(delegation_id = %row.id, child = %row.child_tenant_id, "delegation granted");
        Ok(row)
    }

    /// Khách cha `parent` thu hồi uỷ quyền `id`. Đã thu hồi: giữ nguyên (gọi lại an toàn).
    pub(crate) async fn revoke(
        &self,
        parent: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Delegation, Error> {
        let transaction = parent.database().begin_write().await?;
        let current = entity::Entity::find_by_id((parent.id().as_uuid(), id))
            .scoped(parent)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(id))?;
        if current.revoked_at.is_some() {
            transaction.rollback().await?;
            return Ok(current);
        }
        entity::Entity::update_many()
            .col_expr(entity::Column::RevokedAt, Expr::value(Some(now())))
            .col_expr(entity::Column::RevokedBy, Expr::value(Some(principal.user_id())))
            .filter(entity::Column::Id.eq(id))
            .scoped(parent)
            .exec(&transaction)
            .await?;
        let after = entity::Entity::find_by_id((parent.id().as_uuid(), id))
            .scoped(parent)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(id))?;
        let event = events::DelegationRevoked {
            before: Some(describe(&current)),
            after: Some(describe(&after)),
        };
        self.outbox
            .record(&transaction, parent, Actor::from(principal), subject(id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(delegation_id = %id, "delegation revoked");
        Ok(after)
    }

    /// Uỷ quyền khách `tenant` đã cấp cho các khách con (`outgoing`), mới nhất trước.
    pub(crate) async fn outgoing(&self, tenant: &TenantContext) -> Result<Vec<Delegation>, Error> {
        Ok(entity::Entity::find()
            .scoped(tenant)
            .order_by_desc(entity::Column::Id)
            .all(tenant.database().read())
            .await?)
    }

    /// Uỷ quyền các khách cấp trên đã cấp cho khách `tenant` (`incoming`), mới nhất trước.
    pub(crate) async fn incoming(&self, tenant: &TenantContext) -> Result<Vec<Delegation>, Error> {
        let mut found = Vec::new();
        for parent in self.databases.tenant_ancestors(tenant.id()).await? {
            let parent = self.databases.tenant(parent).await?;
            found.extend(
                entity::Entity::find()
                    .filter(entity::Column::ChildTenantId.eq(tenant.id().as_uuid()))
                    .scoped(&parent)
                    .all(parent.database().read())
                    .await?,
            );
        }
        found.sort_by_key(|row| std::cmp::Reverse(row.id));
        Ok(found)
    }

    /// Uỷ quyền còn hiệu lực cho người dùng `user_id` của khách `home`: những uỷ quyền cho role
    /// họ giữ ở nút gốc của `home`. `child`: chỉ cho khách con đó.
    pub(crate) async fn usable(
        &self,
        home: &TenantContext,
        user_id: Uuid,
        child: Option<TenantId>,
    ) -> Result<Vec<Delegation>, Error> {
        let roles = crate::role::root_roles(home, user_id).await?;
        if roles.is_empty() {
            return Ok(Vec::new());
        }
        let mut select = entity::Entity::find()
            .filter(entity::Column::RoleId.is_in(roles))
            .filter(live(now()))
            .scoped(home);
        if let Some(child) = child {
            select = select.filter(entity::Column::ChildTenantId.eq(child.as_uuid()));
        }
        Ok(select.order_by_asc(entity::Column::Id).all(home.database().read()).await?)
    }

    /// Người `principal` (đã xác thực ở khách nhà) vào khách `target` bằng uỷ quyền: quyền là
    /// permission của các role được uỷ quyền, gắn ở nút của uỷ quyền trong `target`. Không có uỷ
    /// quyền nào còn hiệu lực: `None`. API key và người mua không vào bằng uỷ quyền.
    pub(crate) async fn enter(
        &self,
        principal: &Principal,
        target: &TenantContext,
    ) -> Result<Option<Principal>, Error> {
        if principal.realm() == Realm::Customer
            || principal.api_key_id().is_some()
            || principal.operator_from().is_some()
        {
            return Ok(None);
        }
        let home = self.databases.tenant(principal.tenant_id()).await?;
        let delegations = self.usable(&home, principal.user_id(), Some(target.id())).await?;
        let Some(first) = delegations.first() else { return Ok(None) };
        let access = AccessPath::Delegation(first.id);
        let roles: Vec<Uuid> = delegations.iter().map(|row| row.role_id).collect();
        let granted = crate::role::role_grants(&home, &roles).await?;
        // Role đòi MFA mà người này chưa bật MFA: không cấp ở khách con, như ở khách nhà (cấm vẫn
        // giữ).
        let blocked = crate::role::roles_blocked_by_mfa(&home, principal.user_id(), &roles).await?;
        let mut grants = Vec::new();
        let mut denied = Vec::new();
        let mut everything = false;
        for delegation in &delegations {
            let org = OrgId::from_uuid(delegation.org_id);
            let Some((all, rules)) = granted.get(&delegation.role_id) else { continue };
            denied.extend(rules.denies.iter().cloned());
            if blocked.contains(&delegation.role_id) {
                continue;
            }
            // Role toàn quyền của khách cha chỉ uỷ quyền được ở nút gốc của khách con (`grant`).
            everything |= *all;
            for (permission, scope) in &rules.grants {
                grants.push((permission.clone(), Grant::new(org, *scope)));
            }
        }
        let entered = if everything {
            Principal::with_all_permissions(principal.user_id())
        } else {
            Principal::with_grants(principal.user_id(), grants)
        };
        // Role cấm thắng mọi role cấp, như ở khách nhà (bước 54).
        let entered = entered
            .in_tenant(home.id())
            .operating(target.id())
            .without(denied, &self.catalog)
            .with_access(access);
        Ok(Some(entered))
    }
}

/// Chủ thể của event về uỷ quyền `id`.
fn subject(id: Uuid) -> Option<String> {
    Some(format!("auth.delegation:{id}"))
}

fn describe(row: &Delegation) -> serde_json::Value {
    json!({
        "child_tenant_id": row.child_tenant_id,
        "role_id": row.role_id,
        "org_id": row.org_id,
        "expires_at": row.expires_at,
        "revoked_at": row.revoked_at,
        "reason": row.reason,
    })
}

pub(crate) fn not_found(id: Uuid) -> Error {
    Error::not_found("auth.delegation_not_found", "There is no delegation `{id}`.")
        .with_param("id", id)
}
