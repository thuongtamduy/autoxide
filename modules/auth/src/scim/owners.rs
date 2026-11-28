//! Người chịu trách nhiệm cho dữ liệu SCIM (P16 của bước 70, D-20261128-scim-owners).
//!
//! - **Thành viên nhóm** (`auth_scim_group_members.added_by`): người tạo token SCIM đã đưa người vào nhóm, hay người
//!   đã duyệt. Luật ánh xạ của nhóm chỉ cấp role cho thành viên mà người này cấp được role đó (quyền LÚC NÀY); còn
//!   lại là "chờ duyệt" — token quyền thấp đưa người vào nhóm chưa có luật thì luật admin đặt sau không biến họ thành
//!   admin. Thành viên chờ duyệt không bị gỡ role họ đã có (không đổi gì cho người đang giữ).
//! - **Liên kết tài khoản** (`auth_scim_users.created_by`): người tạo token đã tạo hay nhận về liên kết. Đồng bộ sắp
//!   đưa tài khoản lên role người này không cấp được thì mọi cách vào cũ của tài khoản bị thu hồi TRƯỚC (như đổi
//!   email, D-20261125-scim-sync-report mục 7), rồi người chịu trách nhiệm cho lần nâng thành người tạo liên kết.
//!
//! NULL (dòng trước m0049): không biết ai — không cấp được gì.

use std::collections::{BTreeSet, HashMap};

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{ScopeExt as _, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _};
use serde_json::json;
use uuid::Uuid;

use super::ScimService;
use super::entity::{group, mapping, member, scim_user};
use super::group_sync::SyncReport;
use super::groups::group_subject;
use super::mappings::group_not_found;
use crate::events;

/// (Nút, role).
pub(super) type Pair = (Uuid, Uuid);

/// Hỏi "người `owner` cấp được (nút, role) này không" trong một lần đồng bộ, nhớ kết quả: một nhóm thường có ít người
/// đưa vào và ít luật.
pub(super) struct Owners<'a> {
    service: &'a ScimService,
    tenant: &'a TenantContext,
    principals: HashMap<Option<Uuid>, Principal>,
    answers: HashMap<(Option<Uuid>, Pair), bool>,
}

impl<'a> Owners<'a> {
    pub(super) fn new(service: &'a ScimService, tenant: &'a TenantContext) -> Self {
        Owners { service, tenant, principals: HashMap::new(), answers: HashMap::new() }
    }

    /// `owner` (người chịu trách nhiệm, tính quyền lúc này như token SCIM — `authority_of`) cấp được `pair` không.
    pub(super) async fn can_give(
        &mut self,
        owner: Option<Uuid>,
        pair: Pair,
    ) -> Result<bool, Error> {
        if let Some(answer) = self.answers.get(&(owner, pair)) {
            return Ok(*answer);
        }
        let principal = match self.principals.get(&owner) {
            Some(principal) => principal,
            None => {
                let principal = self.service.authority_of(self.tenant, owner).await?;
                self.principals.entry(owner).or_insert(principal)
            }
        };
        // Kết nối GHI: chạy ngay sau commit nhóm / luật (read replica có thể trễ).
        let connection = self.tenant.database().write();
        let answer = crate::role::grantable(connection, self.tenant, principal, pair, true).await?;
        self.answers.insert((owner, pair), answer);
        Ok(answer)
    }
}

/// (Nút, role) một người phải giữ theo nhóm và luật ánh xạ: `give` — có ít nhất một nhóm mà người đưa họ vào cấp
/// được; `waiting` — chỉ do nhóm mà người đưa vào không cấp được (chờ duyệt: không cấp, cũng không gỡ).
#[derive(Debug, Default)]
pub(super) struct Wanted {
    pub give: BTreeSet<Pair>,
    pub waiting: BTreeSet<Pair>,
}

impl ScimService {
    /// [`Wanted`] của người `user_id`. Kết nối GHI: đọc ngay sau commit nhóm / luật, read replica có thể trễ — thấy
    /// người đã bị gỡ khỏi nhóm vẫn còn thì không gỡ role (D-20261125-scim-sync-report).
    pub(super) async fn wanted_grants(
        &self,
        owners: &mut Owners<'_>,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Wanted, Error> {
        let connection = tenant.database().write();
        let rows = member::Entity::find()
            .filter(member::Column::UserId.eq(user_id))
            .scoped(tenant)
            .all(connection)
            .await?;
        if rows.is_empty() {
            return Ok(Wanted::default());
        }
        let rules = mapping::Entity::find()
            .filter(mapping::Column::GroupId.is_in(rows.iter().map(|row| row.group_id)))
            .scoped(tenant)
            .all(connection)
            .await?;
        let mut wanted = Wanted::default();
        for rule in rules {
            let pair = (rule.org_id, rule.role_id);
            for row in rows.iter().filter(|row| row.group_id == rule.group_id) {
                if owners.can_give(row.added_by, pair).await? {
                    wanted.give.insert(pair);
                } else {
                    wanted.waiting.insert(pair);
                }
            }
        }
        let give = &wanted.give;
        wanted.waiting.retain(|pair| !give.contains(pair));
        Ok(wanted)
    }

    /// Trước khi đồng bộ cấp `give` cho người `user_id` (do `authority` chịu trách nhiệm): (nút, role) MỚI mà
    /// `authority` cấp được (phần lần đồng bộ sẽ thêm) nhưng người tạo liên kết SCIM của tài khoản không cấp được thì
    /// thu hồi mọi cách vào cũ của tài khoản (`lock_out_in`) và ghi `authority` là người tạo liên kết, trong một
    /// transaction riêng TRƯỚC lần cấp — không có lúc nào tài khoản vừa có role mới vừa giữ cửa vào cũ (vd kẻ cắm sẵn
    /// tài khoản bằng token quyền thấp). Thu hồi rồi mà lần cấp lỗi thì vẫn an toàn (chỉ mất cửa vào cũ).
    pub(super) async fn guard_link_raise(
        &self,
        owners: &mut Owners<'_>,
        tenant: &TenantContext,
        actor: Actor,
        authority: &Principal,
        user_id: Uuid,
        give: &BTreeSet<Pair>,
    ) -> Result<(), Error> {
        use crate::role::entity::user_role;
        let connection = tenant.database().write();
        let Some(link) =
            scim_user::Entity::find_by_id((tenant.id().as_uuid(), user_id)).one(connection).await?
        else {
            return Ok(());
        };
        let root = autoxide_kernel::tenancy::root_of(tenant.id()).as_uuid();
        let held: BTreeSet<Pair> = user_role::Entity::find()
            .filter(user_role::Column::UserId.eq(user_id))
            .scoped(tenant)
            .all(connection)
            .await?
            .into_iter()
            .map(|row| (row.org_id.unwrap_or(root), row.role_id))
            .collect();
        let mut raises = false;
        for pair in give.difference(&held) {
            if link.created_by == Some(authority.user_id())
                || !crate::role::grantable(connection, tenant, authority, *pair, true).await?
            {
                // Người tạo liên kết tự cấp, hay lần đồng bộ sẽ không cấp (vượt quyền `authority`): không nâng gì.
                continue;
            }
            if !owners.can_give(link.created_by, *pair).await? {
                raises = true;
                break;
            }
        }
        if !raises {
            return Ok(());
        }
        let transaction = tenant.database().begin_write().await?;
        self.users.lock_out_in(tenant, &transaction, actor, user_id).await?;
        scim_user::Entity::update_many()
            .col_expr(scim_user::Column::CreatedBy, Expr::value(Some(authority.user_id())))
            .filter(scim_user::Column::Id.eq(user_id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        tracing::warn!(
            user_id = %user_id,
            "SCIM sync raised an account above its link creator: sign-in credentials revoked"
        );
        Ok(())
    }

    /// Duyệt thành viên `user_id` của nhóm `group_id`: `principal` thành người chịu trách nhiệm cho thành viên đó
    /// (`added_by`), nên phải cấp được MỌI luật của nhóm (403 `auth.role_beyond_you`), rồi đồng bộ người đó. Gọi lại
    /// được (duyệt lại chỉ ghi lại người duyệt và đồng bộ lại).
    pub(crate) async fn approve_member(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        group_id: Uuid,
        user_id: Uuid,
    ) -> Result<SyncReport, Error> {
        let transaction = tenant.database().begin_write().await?;
        // Khoá dòng nhóm như khi ghi thành viên hay luật: mọi lần đổi của một nhóm nối đuôi nhau.
        let found = autoxide_kernel::rows::lock_one(
            &transaction,
            group::Entity::find_by_id((tenant.id().as_uuid(), group_id)),
        )
        .await?;
        if found.is_none() {
            transaction.rollback().await?;
            return Err(group_not_found(group_id));
        }
        let Some(row) = member::Entity::find()
            .filter(member::Column::GroupId.eq(group_id))
            .filter(member::Column::UserId.eq(user_id))
            .scoped(tenant)
            .one(&transaction)
            .await?
        else {
            transaction.rollback().await?;
            return Err(member_not_found(group_id, user_id));
        };
        let pairs: Vec<Pair> = mapping::Entity::find()
            .filter(mapping::Column::GroupId.eq(group_id))
            .scoped(tenant)
            .all(&transaction)
            .await?
            .into_iter()
            .map(|rule| (rule.org_id, rule.role_id))
            .collect();
        if let Err(error) =
            crate::role::require_grantable(&transaction, tenant, principal, &pairs, &[]).await
        {
            transaction.rollback().await?;
            return Err(error);
        }
        member::Entity::update_many()
            .col_expr(member::Column::AddedBy, Expr::value(Some(principal.user_id())))
            .filter(member::Column::Id.eq(row.id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let event = events::ScimGroupUpdated {
            before: Some(json!({"member": user_id, "added_by": row.added_by})),
            after: Some(
                json!({"member": user_id, "added_by": principal.user_id(), "approved": true}),
            ),
        };
        self.outbox
            .record(&transaction, tenant, Actor::from(principal), group_subject(group_id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(%group_id, %user_id, "SCIM group member approved");
        let users = BTreeSet::from([user_id]);
        Ok(self.sync_in(tenant, Actor::from(principal), principal, &users).await)
    }
}

pub(crate) fn member_not_found(group_id: Uuid, user_id: Uuid) -> Error {
    Error::not_found(
        "auth.scim_member_not_found",
        "User `{user_id}` is not a member of SCIM group `{group_id}`.",
    )
    .with_param("group_id", group_id)
    .with_param("user_id", user_id)
}
