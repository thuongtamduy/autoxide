//! Đồng bộ membership sau khi ghi nhóm hay luật ánh xạ (tách từ [`super::groups`], chỉ di chuyển
//! code): người có thành viên đổi, người còn "chờ", kết quả từng người (D-20261125-scim-sync-report).

use std::collections::BTreeSet;

use autoxide_kernel::access::Principal;
use autoxide_kernel::tenancy::{ScopeExt as _, TenantContext};
use sea_orm::sea_query::{Expr, ExprTrait as _, Query};
use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _};
use serde_json::Value;
use uuid::Uuid;

use super::entity::{mapping, member};
use super::error::ScimError;
use super::{ScimCaller, ScimService};

/// Nhiều thành viên "chờ" nhất được đồng bộ lại sau một lần ghi nhóm: nhóm có hàng nghìn người chờ
/// (vd gói hết chỗ) không làm mỗi request chậm theo số người đó.
const PENDING_MAX: usize = 100;

/// Mọi ID người dùng (chuỗi là UUID) xuất hiện trong thân request: thành viên request nêu tên.
pub(super) fn named_users(body: &Value) -> BTreeSet<Uuid> {
    let mut found = BTreeSet::new();
    let mut stack = vec![body];
    while let Some(value) = stack.pop() {
        match value {
            Value::String(text) => {
                if let Ok(id) = Uuid::try_parse(text.trim()) {
                    found.insert(id);
                }
            }
            Value::Array(items) => stack.extend(items),
            Value::Object(fields) => stack.extend(fields.values()),
            _ => {}
        }
    }
    found
}

impl ScimService {
    /// Sau một lần ghi nhóm `group_id`: đồng bộ người có thành viên đổi (`changed`), cùng tối đa
    /// [`PENDING_MAX`] thành viên còn "chờ" (thiếu role mà luật của nhóm cấp — lần đồng bộ trước lỗi),
    /// ưu tiên người request nêu tên (`named`). Hệ thống danh tính gửi lại đúng request cũ thì không
    /// còn ai đổi, nhưng người chờ được nêu tên vẫn được đồng bộ lại. Trả lỗi chỉ khi người đổi hay
    /// người được nêu tên lỗi; người chờ cũ khác lỗi (vd vẫn hết chỗ) chỉ ghi log — không làm hỏng
    /// request không liên quan tới họ (D-20261125-scim-sync-report).
    pub(super) async fn sync_group_members(
        &self,
        caller: &ScimCaller,
        group_id: Uuid,
        changed: BTreeSet<Uuid>,
        named: &BTreeSet<Uuid>,
    ) -> Result<(), ScimError> {
        let authority = self.authority(caller).await?;
        let pending = pending_members(&caller.tenant, group_id, &authority).await?;
        let (first, rest): (Vec<Uuid>, Vec<Uuid>) =
            pending.into_iter().partition(|user_id| named.contains(user_id));
        let pending: BTreeSet<Uuid> = first.into_iter().chain(rest).take(PENDING_MAX).collect();
        let mut must = changed.clone();
        must.extend(pending.intersection(named).copied());
        let mut users = changed;
        users.extend(pending);
        if users.is_empty() {
            return Ok(());
        }
        let report = self.sync_in(&caller.tenant, caller.actor(), &authority, &users).await;
        match report.failed.into_iter().find(|(user_id, _)| must.contains(user_id)) {
            Some((_, error)) => Err(ScimError::from(error)),
            None => Ok(()),
        }
    }

    /// Đồng bộ membership do SCIM cấp của những người `users` theo nhóm hiện tại của họ.
    pub(crate) async fn sync_users(
        &self,
        caller: &ScimCaller,
        users: &BTreeSet<Uuid>,
    ) -> Result<(), ScimError> {
        if users.is_empty() {
            return Ok(());
        }
        let authority = self.authority(caller).await?;
        let report = self.sync_in(&caller.tenant, caller.actor(), &authority, users).await;
        // Đã đồng bộ mọi người được; còn lỗi thì báo lỗi đầu để hệ thống danh tính gửi lại. Gửi lại
        // đồng bộ lại người còn chờ (`pending_members`) dù thành viên nhóm không đổi thêm.
        match report.failed.into_iter().next() {
            Some((_, error)) => Err(ScimError::from(error)),
            None => Ok(()),
        }
    }

    /// Như [`sync_users`](Self::sync_users), cho bên gọi ngoài SCIM (đổi luật ánh xạ).
    /// `authority`: người chịu trách nhiệm — chỉ role người đó cấp / gỡ được mới được thêm / gỡ.
    /// Đồng bộ MỌI người dù có người lỗi (vd vượt hạn mức số nhân sự): một người lỗi không để những
    /// người sau thành "chờ"; trả ai đã đồng bộ xong và ai lỗi (D-20261125-scim-sync-report).
    pub(crate) async fn sync_in(
        &self,
        tenant: &TenantContext,
        actor: autoxide_kernel::event::Actor,
        authority: &autoxide_kernel::access::Principal,
        users: &BTreeSet<Uuid>,
    ) -> SyncReport {
        let mut report = SyncReport::default();
        for user_id in users {
            let synced = async {
                let wanted = wanted_grants(tenant, *user_id).await?;
                self.roles.sync_scim_grants(tenant, actor, authority, *user_id, wanted).await
            };
            match synced.await {
                Ok(_) => report.synced.push(*user_id),
                Err(error) => {
                    // `error` đầy đủ chỉ vào log (kể cả lỗi nội bộ); ra ngoài chỉ có mã và câu chung.
                    tracing::warn!(user_id = %user_id, code = error.code(), %error, "SCIM membership sync failed for a user");
                    report.failed.push((*user_id, error));
                }
            }
        }
        report
    }
}

/// Kết quả đồng bộ membership của một nhóm người: ai xong, ai lỗi (kèm lỗi).
#[derive(Debug, Default)]
pub(crate) struct SyncReport {
    pub synced: Vec<Uuid>,
    pub failed: Vec<(Uuid, autoxide_kernel::error::Error)>,
}

/// Thành viên của nhóm `group_id` còn thiếu một (nút, role) mà luật của nhóm cấp VÀ `authority`
/// cấp được — lần đồng bộ trước của họ lỗi (vượt hạn mức…). Role `authority` không cấp được không
/// làm ai "chờ" đối với nó: không thì người chỉ thiếu role đó bị chọn lại mãi, chiếm hết
/// [`PENDING_MAX`] chỗ, người sau không bao giờ được đồng bộ. Mỗi luật một câu; đọc qua kết nối ghi
/// (ngay sau commit, read replica có thể trễ).
async fn pending_members(
    tenant: &TenantContext,
    group_id: Uuid,
    authority: &Principal,
) -> Result<BTreeSet<Uuid>, ScimError> {
    use crate::role::entity::user_role;
    let connection = tenant.database().write();
    let root = autoxide_kernel::tenancy::root_of(tenant.id()).as_uuid();
    let rules = mapping::Entity::find()
        .filter(mapping::Column::GroupId.eq(group_id))
        .scoped(tenant)
        .all(connection)
        .await?;
    let mut pending = BTreeSet::new();
    for rule in rules {
        let pair = (rule.org_id, rule.role_id);
        if !crate::role::grantable(connection, tenant, authority, pair, true).await? {
            continue;
        }
        let holders = Query::select()
            .column(user_role::Column::UserId)
            .from(user_role::Entity)
            .and_where(Expr::col(user_role::Column::TenantId).eq(tenant.id().as_uuid()))
            // Dòng cũ (trước bước 34) để `org_id` NULL nghĩa là nút gốc.
            .and_where(if rule.org_id == root {
                Expr::col(user_role::Column::OrgId)
                    .eq(rule.org_id)
                    .or(Expr::col(user_role::Column::OrgId).is_null())
            } else {
                Expr::col(user_role::Column::OrgId).eq(rule.org_id)
            })
            .and_where(Expr::col(user_role::Column::RoleId).eq(rule.role_id))
            .to_owned();
        let missing = member::Entity::find()
            .filter(member::Column::GroupId.eq(group_id))
            .filter(Expr::col((member::Entity, member::Column::UserId)).not_in_subquery(holders))
            .scoped(tenant)
            .all(connection)
            .await?;
        pending.extend(missing.into_iter().map(|row| row.user_id));
    }
    Ok(pending)
}

/// (Nút, role) người `user_id` phải giữ theo các nhóm của họ và luật ánh xạ.
async fn wanted_grants(
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<BTreeSet<(Uuid, Uuid)>, autoxide_kernel::error::Error> {
    // Kết nối GHI: đọc ngay sau commit nhóm / luật, read replica có thể trễ — thấy người đã bị gỡ
    // khỏi nhóm vẫn còn thì không gỡ role (D-20261125-scim-sync-report).
    let read = tenant.database().write();
    let groups = Query::select()
        .column(member::Column::GroupId)
        .from(member::Entity)
        .and_where(Expr::col(member::Column::TenantId).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(member::Column::UserId).eq(user_id))
        .to_owned();
    Ok(mapping::Entity::find()
        .filter(Expr::col((mapping::Entity, mapping::Column::GroupId)).in_subquery(groups))
        .scoped(tenant)
        .all(read)
        .await?
        .into_iter()
        .map(|row| (row.org_id, row.role_id))
        .collect())
}
