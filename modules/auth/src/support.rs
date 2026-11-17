//! Nhân sự nền tảng vào dữ liệu của khách để hỗ trợ (bước 34 mục 13, D-190, D-211, D-313).
//!
//! Chỉ khi có **lần cấp có thời hạn** (`auth_support_grant`): người có `auth.support.grant` ở khách
//! nền tảng cấp cho một nhân sự nền tảng, một role của khách nền tảng, ở một khách, trong tối đa
//! `AUTOXIDE_AUTH_SUPPORT_ACCESS_MAX_HOURS` giờ, kèm lý do (và số văn bản khi là yêu cầu của cơ quan
//! có thẩm quyền). Nhân sự đó gọi API bằng credential của mình kèm `Autoxide-Tenant`; kernel hỏi
//! [`SupportService::enter`] mỗi request. Khách tắt cài đặt `auth.support_access` thì chỉ lần cấp
//! `legal_request` còn hiệu lực; khách tự thu hồi lần cấp `support` còn hạn. Mọi việc (cấp, lần
//! vào đầu, thu hồi) ghi event vào outbox CỦA KHÁCH để audit của khách thấy.
//!
//! Dòng nằm ở database của khách nền tảng (`tenant_id` = khách nền tảng, `target_tenant_id` = khách
//! được vào): một chỗ tra cho mọi request vào, mọi danh sách.

use std::sync::Arc;

use autoxide_kernel::access::{AccessPath, Grant, Principal, Realm};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::{Actor, Event, Outbox};
use autoxide_kernel::tenancy::{DatabaseRegistry, ScopeExt, TenantContext, TenantId, root_of};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde_json::json;
use uuid::Uuid;

use crate::events;

pub(crate) mod entity {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_support_grant")]
    pub struct Model {
        /// Khách nền tảng (bên cấp, giữ dòng).
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// Khách được vào.
        pub target_tenant_id: Uuid,
        /// Nhân sự nền tảng được vào.
        pub staff_user_id: Uuid,
        /// Role của khách nền tảng: quyền khi vào.
        pub role_id: Uuid,
        /// `support` hay `legal_request`.
        pub basis: String,
        pub reason: String,
        pub legal_reference: Option<String>,
        pub granted_by: Option<Uuid>,
        pub starts_at: DateTimeUtc,
        pub expires_at: DateTimeUtc,
        pub first_used_at: Option<DateTimeUtc>,
        pub revoked_at: Option<DateTimeUtc>,
        pub revoked_by: Option<Uuid>,
        /// Khách tự thu hồi (D-211).
        pub revoked_by_tenant: bool,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

pub(crate) type SupportGrant = entity::Model;

/// Lần cấp mới.
pub(crate) struct NewSupportGrant {
    pub tenant_id: Uuid,
    pub staff_user_id: Uuid,
    pub role_id: Uuid,
    pub basis: String,
    pub reason: String,
    pub legal_reference: Option<String>,
    pub hours: u32,
}

pub(crate) const SUPPORT: &str = "support";
pub(crate) const LEGAL_REQUEST: &str = "legal_request";

pub(crate) struct SupportService {
    pub databases: DatabaseRegistry,
    pub outbox: Outbox,
    pub orgs: Option<Arc<dyn autoxide_organizations::api::OrgDirectory>>,
    /// `AUTOXIDE_AUTH_SUPPORT_ACCESS_MAX_HOURS`.
    pub max_hours: u32,
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

impl SupportService {
    fn platform(&self) -> TenantContext {
        self.databases.default_tenant()
    }

    /// Khách `target` có cho nhân sự nền tảng vào để hỗ trợ không (cài đặt `auth.support_access`).
    async fn allows_support(&self, target: &TenantContext) -> Result<bool, Error> {
        crate::settings::enabled(self.orgs.as_ref(), target, &crate::settings::SUPPORT_ACCESS).await
    }

    /// Ghi event vào outbox của khách được vào (audit của khách thấy). Database của khách có thể
    /// khác database nền tảng: transaction riêng, sau khi việc chính đã commit.
    async fn record_at_target<E: Event + Sync>(
        &self,
        target: &TenantContext,
        actor: Actor,
        id: Uuid,
        event: &E,
    ) -> Result<(), Error> {
        let transaction = target.database().begin_write().await?;
        self.outbox.record(&transaction, target, actor, subject(id), event).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Việc của nhân sự nền tảng: người gọi phải ở chính khách nền tảng (không qua header "vào"
    /// khách khác). Chỉ kiểm permission là chưa đủ — role `admin` của khách nào cũng có mọi
    /// permission, kể cả `auth.support.grant`.
    pub(crate) fn require_platform(&self, principal: &Principal) -> Result<(), Error> {
        if principal.tenant_id() != self.platform().id() || principal.operator_from().is_some() {
            return Err(Error::forbidden(
                "auth.support_platform_only",
                "Only platform staff, in the platform tenant, can grant support access.",
            ));
        }
        Ok(())
    }

    /// Khách `target` có cho nhân sự nền tảng vào để hỗ trợ không — để màn cấp quyền biết trước,
    /// thay vì chỉ biết khi bị từ chối `auth.support_access_disabled`.
    pub(crate) async fn allowed(&self, target: TenantId) -> Result<bool, Error> {
        let target = self.databases.tenant(target).await?;
        self.allows_support(&target).await
    }

    /// Cấp quyền vào (người gọi ở khách nền tảng, đã có `auth.support.grant`).
    pub(crate) async fn grant(
        &self,
        principal: &Principal,
        input: NewSupportGrant,
    ) -> Result<SupportGrant, Error> {
        let platform = self.platform();
        self.require_platform(principal)?;
        let reason = input.reason.trim().to_owned();
        let mut problems = Vec::new();
        if !(10..=500).contains(&reason.chars().count()) {
            problems.push(
                FieldError::new("reason", "length", "must be {min} to {max} characters")
                    .with_param("min", 10)
                    .with_param("max", 500),
            );
        }
        let legal = input.basis == LEGAL_REQUEST;
        if !legal && input.basis != SUPPORT {
            problems.push(FieldError::new(
                "basis",
                "invalid",
                "must be `support` or `legal_request`",
            ));
        }
        let reference = input
            .legal_reference
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty());
        if legal && reference.is_none() {
            problems.push(FieldError::new(
                "legal_reference",
                "required",
                "is required for a request of a competent authority (number of the document)",
            ));
        }
        if input.hours == 0 {
            problems.push(FieldError::new("hours", "range", "must be at least 1"));
        }
        let staff = self.users_staff(&platform, input.staff_user_id).await?;
        if !staff {
            problems.push(FieldError::new(
                "staff_user_id",
                "unknown",
                "is not a staff account of the platform",
            ));
        }
        if crate::role::role_grants(&platform, &[input.role_id]).await?.is_empty() {
            problems.push(FieldError::new("role_id", "not_found", "no such role in this tenant"));
        }
        let target_id = TenantId::from_uuid(input.tenant_id);
        if target_id == platform.id() {
            problems.push(FieldError::new(
                "tenant_id",
                "invalid",
                "must be another tenant than the platform",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        if input.hours > self.max_hours {
            return Err(Error::new(
                ErrorKind::Validation,
                "auth.support_duration_exceeded",
                "Support access lasts at most {max} hours.",
            )
            .with_param("max", self.max_hours));
        }
        let target = self.databases.tenant(target_id).await?;
        if !legal && !self.allows_support(&target).await? {
            return Err(Error::forbidden(
                "auth.support_access_disabled",
                "This tenant turned support access off; only a request of a competent authority \\
                 can be granted.",
            ));
        }
        let now = now();
        let row = entity::ActiveModel {
            tenant_id: Set(platform.id().as_uuid()),
            id: Set(Uuid::now_v7()),
            target_tenant_id: Set(input.tenant_id),
            staff_user_id: Set(input.staff_user_id),
            role_id: Set(input.role_id),
            basis: Set(input.basis),
            reason: Set(reason),
            legal_reference: Set(reference),
            granted_by: Set(Some(principal.user_id())),
            starts_at: Set(now),
            expires_at: Set(now + chrono::Duration::hours(i64::from(input.hours))),
            first_used_at: Set(None),
            revoked_at: Set(None),
            revoked_by: Set(None),
            revoked_by_tenant: Set(false),
            created_at: Set(now),
        };
        let transaction = platform.database().begin_write().await?;
        let row = row.insert(&transaction).await?;
        let event = events::SupportAccessGranted { before: None, after: Some(describe(&row)) };
        self.outbox
            .record(&transaction, &platform, Actor::from(principal), subject(row.id), &event)
            .await?;
        transaction.commit().await?;
        self.record_at_target(&target, Actor::from(principal), row.id, &event).await?;
        tracing::info!(grant_id = %row.id, tenant_id = %row.target_tenant_id, "support access granted");
        Ok(row)
    }

    async fn users_staff(&self, platform: &TenantContext, user_id: Uuid) -> Result<bool, Error> {
        let row = crate::user::entity::Entity::find_by_id(user_id)
            .scoped(platform)
            .one(platform.database().read())
            .await?;
        Ok(row.is_some_and(|row| row.realm == "staff" && row.disabled_at.is_none()))
    }

    /// Tên hiển thị của các tài khoản nhân sự `ids` (ở khách nền tảng) — khách thấy ai được vào dữ
    /// liệu của mình. Chỉ tên, không email.
    pub(crate) async fn staff_names(
        &self,
        ids: impl IntoIterator<Item = Uuid>,
    ) -> Result<std::collections::HashMap<Uuid, String>, Error> {
        let ids: std::collections::BTreeSet<Uuid> = ids.into_iter().collect();
        if ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let platform = self.platform();
        use sea_orm::QuerySelect as _;
        let rows: Vec<(Uuid, String)> = crate::user::entity::Entity::find()
            .select_only()
            .column(crate::user::entity::Column::Id)
            .column(crate::user::entity::Column::Name)
            .filter(crate::user::entity::Column::Id.is_in(ids))
            .scoped(&platform)
            .into_tuple()
            .all(platform.database().read())
            .await?;
        Ok(rows.into_iter().collect())
    }

    /// Mọi lần cấp cho khách `target` (mới nhất trước) — cho quản trị của khách và của nền tảng.
    pub(crate) async fn for_tenant(&self, target: TenantId) -> Result<Vec<SupportGrant>, Error> {
        self.grants(Some(target), None).await
    }

    /// Lần cấp mới nhất trước, của khách `target` (`None`: mọi khách), tối đa `limit`.
    pub(crate) async fn grants(
        &self,
        target: Option<TenantId>,
        limit: Option<u64>,
    ) -> Result<Vec<SupportGrant>, Error> {
        use sea_orm::QuerySelect as _;
        let platform = self.platform();
        let mut select = entity::Entity::find().scoped(&platform);
        if let Some(target) = target {
            select = select.filter(entity::Column::TargetTenantId.eq(target.as_uuid()));
        }
        Ok(select
            .order_by_desc(entity::Column::Id)
            .limit(limit)
            .all(platform.database().read())
            .await?)
    }

    /// Thu hồi lần cấp `id` của khách `target`. `by_tenant`: khách tự thu hồi — lần cấp
    /// `legal_request` thì không được (D-211). Đã thu hồi: giữ nguyên.
    pub(crate) async fn revoke(
        &self,
        target: TenantId,
        principal: &Principal,
        id: Uuid,
        by_tenant: bool,
    ) -> Result<SupportGrant, Error> {
        let platform = self.platform();
        let current = entity::Entity::find_by_id((platform.id().as_uuid(), id))
            .filter(entity::Column::TargetTenantId.eq(target.as_uuid()))
            .scoped(&platform)
            .one(platform.database().read())
            .await?
            .ok_or_else(|| not_found(id))?;
        if current.revoked_at.is_some() {
            return Ok(current);
        }
        if by_tenant && current.basis == LEGAL_REQUEST {
            return Err(Error::forbidden(
                "auth.support_grant_legal_request",
                "Access granted for a request of a competent authority cannot be revoked by the \\
                 tenant.",
            ));
        }
        let transaction = platform.database().begin_write().await?;
        entity::Entity::update_many()
            .col_expr(entity::Column::RevokedAt, Expr::value(Some(now())))
            .col_expr(entity::Column::RevokedBy, Expr::value(Some(principal.user_id())))
            .col_expr(entity::Column::RevokedByTenant, Expr::value(by_tenant))
            .filter(entity::Column::Id.eq(id))
            .filter(entity::Column::RevokedAt.is_null())
            .scoped(&platform)
            .exec(&transaction)
            .await?;
        let after = entity::Entity::find_by_id((platform.id().as_uuid(), id))
            .scoped(&platform)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(id))?;
        let event = events::SupportAccessRevoked {
            before: Some(describe(&current)),
            after: Some(describe(&after)),
        };
        self.outbox
            .record(&transaction, &platform, Actor::from(principal), subject(id), &event)
            .await?;
        transaction.commit().await?;
        let target = self.databases.tenant(target).await?;
        self.record_at_target(&target, Actor::from(principal), id, &event).await?;
        Ok(after)
    }

    /// Lần cấp còn hiệu lực cho nhân sự `user_id` (mọi khách, hay chỉ `target`).
    pub(crate) async fn live_for(
        &self,
        user_id: Uuid,
        target: Option<TenantId>,
    ) -> Result<Vec<SupportGrant>, Error> {
        let platform = self.platform();
        let now = now();
        let mut select = entity::Entity::find()
            .filter(entity::Column::StaffUserId.eq(user_id))
            .filter(entity::Column::RevokedAt.is_null())
            .filter(entity::Column::StartsAt.lte(now))
            .filter(entity::Column::ExpiresAt.gt(now))
            .scoped(&platform);
        if let Some(target) = target {
            select = select.filter(entity::Column::TargetTenantId.eq(target.as_uuid()));
        }
        Ok(select.order_by_asc(entity::Column::Id).all(platform.database().read()).await?)
    }

    /// Nhân sự nền tảng `principal` vào khách `target` bằng lần cấp còn hạn: quyền của role được
    /// cấp, ở nút gốc của khách. Không có: `None`. Lần vào đầu ghi `auth.support_access_started`.
    pub(crate) async fn enter(
        &self,
        principal: &Principal,
        target: &TenantContext,
    ) -> Result<Option<Principal>, Error> {
        let platform = self.platform();
        if principal.tenant_id() != platform.id()
            || principal.realm() == Realm::Customer
            || principal.api_key_id().is_some()
            || principal.operator_from().is_some()
        {
            return Ok(None);
        }
        let grants = self.live_for(principal.user_id(), Some(target.id())).await?;
        if grants.is_empty() {
            return Ok(None);
        }
        let support_allowed = self.allows_support(target).await?;
        let Some(grant) =
            grants.into_iter().find(|grant| grant.basis == LEGAL_REQUEST || support_allowed)
        else {
            return Ok(None);
        };
        if grant.first_used_at.is_none() {
            let claimed = entity::Entity::update_many()
                .col_expr(entity::Column::FirstUsedAt, Expr::value(Some(now())))
                .filter(entity::Column::Id.eq(grant.id))
                .filter(entity::Column::FirstUsedAt.is_null())
                .scoped(&platform)
                .exec(platform.database().write())
                .await?;
            if claimed.rows_affected == 1 {
                let event =
                    events::SupportAccessStarted { before: None, after: Some(describe(&grant)) };
                let actor =
                    Actor::from(&principal.clone().with_access(AccessPath::Support(grant.id)));
                self.record_at_target(target, actor, grant.id, &event).await?;
            }
        }
        let access = AccessPath::Support(grant.id);
        let granted = crate::role::role_grants(&platform, &[grant.role_id]).await?;
        let Some((all, permissions)) = granted.get(&grant.role_id) else { return Ok(None) };
        let entered = if *all {
            Principal::with_all_permissions(principal.user_id())
        } else {
            let root = root_of(target.id());
            let grants = permissions
                .iter()
                .map(|(permission, scope)| (permission.clone(), Grant::new(root, *scope)));
            Principal::with_grants(principal.user_id(), grants)
        };
        Ok(Some(entered.in_tenant(platform.id()).operating(target.id()).with_access(access)))
    }
}

fn subject(id: Uuid) -> Option<String> {
    Some(format!("auth.support_grant:{id}"))
}

fn describe(row: &SupportGrant) -> serde_json::Value {
    json!({
        "tenant_id": row.target_tenant_id,
        "staff_user_id": row.staff_user_id,
        "role_id": row.role_id,
        "basis": row.basis,
        "reason": row.reason,
        "legal_reference": row.legal_reference,
        "starts_at": row.starts_at,
        "expires_at": row.expires_at,
        "revoked_at": row.revoked_at,
        "revoked_by_tenant": row.revoked_by_tenant,
    })
}

pub(crate) fn not_found(id: Uuid) -> Error {
    Error::not_found("auth.support_grant_not_found", "There is no support access grant `{id}`.")
        .with_param("id", id)
}
