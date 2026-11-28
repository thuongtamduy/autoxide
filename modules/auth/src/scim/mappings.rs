//! Luật ánh xạ nhóm SCIM → role ở nút (quản trị, envelope chung của Autoxide) và nguồn việc "nhóm
//! chưa có luật". Người tạo luật phải gán được role đó ở nút đó (cùng kiểm như gán membership:
//! nút trong phạm vi, có mọi permission của role, `admin` chỉ ở gốc) — luật là cách gán role cho
//! mọi người trong nhóm, nên không được dùng để tự nâng quyền. Bỏ luật cũng vậy: chỉ người cấp được
//! role đó ở nút đó mới bỏ được (D-20261125-scim-grant-cover).

use autoxide_kernel::time::Timestamp;
use std::collections::{BTreeSet, HashMap};

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt as _, TenantContext};
use autoxide_kernel::work::{Severity, WorkItem, WorkSource};
use sea_orm::sea_query::{Expr, ExprTrait as _, Query};
use sea_orm::{
    ActiveModelTrait as _, ColumnTrait as _, EntityTrait as _, PaginatorTrait as _,
    QueryFilter as _, SqlErr,
};
use serde_json::json;
use uuid::Uuid;

use super::ScimService;
use super::entity::{group, mapping, member};
use super::group_sync::SyncReport;
use super::groups::group_subject;
use crate::events;
use crate::permissions::SCIM_MANAGE;

/// Một luật ánh xạ.
pub(crate) type GroupMapping = mapping::Model;

/// Một nhóm SCIM cho trang quản trị: số thành viên và số luật.
pub(crate) struct GroupSummary {
    pub group: group::Model,
    pub members: u64,
    pub mappings: u64,
}

pub(crate) fn group_not_found(id: Uuid) -> Error {
    Error::not_found("auth.scim_group_not_found", "There is no SCIM group with id `{id}`.")
        .with_param("id", id)
}

pub(crate) fn mapping_not_found(id: Uuid) -> Error {
    Error::not_found(
        "auth.scim_mapping_not_found",
        "There is no SCIM group mapping with id `{id}`.",
    )
    .with_param("id", id)
}

/// Thành viên của nhóm `group_id`. Kết nối GHI: đọc ngay sau commit luật để đồng bộ (replica có thể
/// trễ).
async fn members(tenant: &TenantContext, group_id: Uuid) -> Result<BTreeSet<Uuid>, Error> {
    Ok(member::Entity::find()
        .filter(member::Column::GroupId.eq(group_id))
        .scoped(tenant)
        .all(tenant.database().write())
        .await?
        .into_iter()
        .map(|row| row.user_id)
        .collect())
}

impl ScimService {
    /// Một trang nhóm SCIM của khách, kèm số thành viên và số luật (hai câu cho cả trang).
    pub(crate) async fn group_summaries(
        &self,
        tenant: &TenantContext,
        page: &PageRequest,
    ) -> Result<Page<GroupSummary>, Error> {
        let read = tenant.database().read();
        let page = fetch_page(
            group::Entity::find().scoped(tenant),
            group::Column::Id,
            |row| row.id,
            page,
            read,
        )
        .await?;
        let ids: Vec<Uuid> = page.data.iter().map(|row| row.id).collect();
        let mut members: HashMap<Uuid, u64> = HashMap::new();
        for row in member::Entity::find()
            .filter(member::Column::GroupId.is_in(ids.iter().copied()))
            .scoped(tenant)
            .all(read)
            .await?
        {
            *members.entry(row.group_id).or_default() += 1;
        }
        let mut mappings: HashMap<Uuid, u64> = HashMap::new();
        for row in mapping::Entity::find()
            .filter(mapping::Column::GroupId.is_in(ids.iter().copied()))
            .scoped(tenant)
            .all(read)
            .await?
        {
            *mappings.entry(row.group_id).or_default() += 1;
        }
        Ok(page.map(|group| GroupSummary {
            members: members.get(&group.id).copied().unwrap_or_default(),
            mappings: mappings.get(&group.id).copied().unwrap_or_default(),
            group,
        }))
    }

    /// Một trang luật ánh xạ của khách, mới nhất trước trừ `order=asc` (keyset trên khoá chính).
    pub(crate) async fn mappings_page(
        &self,
        tenant: &TenantContext,
        page: &autoxide_kernel::pagination::PageRequest,
    ) -> Result<autoxide_kernel::pagination::Page<GroupMapping>, Error> {
        autoxide_kernel::pagination::fetch_page(
            mapping::Entity::find().scoped(tenant),
            mapping::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await
    }

    /// Thêm luật "nhóm `group_id` → role `role_id` ở nút `org_id`", rồi cấp role cho mọi thành
    /// viên hiện có của nhóm. Trả luật và kết quả đồng bộ từng thành viên (D-20261125-scim-grant-cover,
    /// D-20261125-scim-sync-report: thành viên của nhóm chưa có luật có thể do token quyền thấp đưa vào
    /// — người đặt luật chịu trách nhiệm cho danh sách này, nên nó được trả về để xem lại).
    pub(crate) async fn create_mapping(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        group_id: Uuid,
        org_id: Uuid,
        role_id: Uuid,
    ) -> Result<(GroupMapping, SyncReport), Error> {
        let transaction = tenant.database().begin_write().await?;
        // Khoá dòng nhóm như khi ghi thành viên (`update_group`): luật mới và một lần thêm thành viên
        // nối đuôi nhau, nên lần thêm thành viên luôn thấy luật đã commit và kiểm theo nó — không còn
        // khe để một người lọt vào nhóm đã có luật mà người đưa vào không cấp được role (Postgres;
        // SQLite vốn một bên ghi).
        let Some(found) = autoxide_kernel::rows::lock_one(
            &transaction,
            group::Entity::find_by_id((tenant.id().as_uuid(), group_id)),
        )
        .await?
        else {
            transaction.rollback().await?;
            return Err(group_not_found(group_id));
        };
        let roles = BTreeSet::from([role_id]);
        let problems =
            crate::role::grant_problems(&transaction, tenant, principal, "", org_id, &roles)
                .await?;
        if let Err(error) = crate::role::refuse_grants(problems) {
            transaction.rollback().await?;
            return Err(error);
        }
        let row = mapping::Model {
            tenant_id: tenant.id().as_uuid(),
            id: Uuid::now_v7(),
            group_id,
            org_id,
            role_id,
            created_by: principal.user_id(),
            created_at: Timestamp::now(),
        };
        mapping::ActiveModel::from(row.clone()).insert(&transaction).await.map_err(|error| {
            match error.sql_err() {
                Some(SqlErr::UniqueConstraintViolation(_)) => Error::conflict(
                    "auth.scim_mapping_taken",
                    "This group already gives this role at this unit.",
                ),
                _ => Error::from(error),
            }
        })?;
        let event = events::ScimGroupMapped {
            before: None,
            after: Some(
                json!({"id": row.id, "group": found.display_name, "org_id": org_id, "role_id": role_id}),
            ),
        };
        self.outbox
            .record(&transaction, tenant, Actor::from(principal), group_subject(group_id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(mapping_id = %row.id, "SCIM group mapping created");
        // Luật đã commit: đồng bộ mọi thành viên, người lỗi không chặn người sau; ai đã nhận role và
        // ai lỗi đều trả về (và ghi log) — không im lặng để lại thành viên chưa đồng bộ.
        let rows = member::Entity::find()
            .filter(member::Column::GroupId.eq(group_id))
            .scoped(tenant)
            .all(tenant.database().write())
            .await?;
        let members: BTreeSet<Uuid> = rows.iter().map(|row| row.user_id).collect();
        let mut report = self.sync_in(tenant, Actor::from(principal), principal, &members).await;
        // Chỉ người thật sự giữ role ở nút sau khi đồng bộ (không gồm tài khoản vừa bị xoá hay không
        // phải nhân sự mà lần đồng bộ bỏ qua). Đọc qua kết nối ghi: ngay sau commit, replica có thể trễ.
        let holders = holders_of(tenant, org_id, role_id, &report.synced).await?;
        report.synced.retain(|user_id| holders.contains(user_id));
        // Thành viên mà người đưa vào không cấp được role này: chờ duyệt (D-20261128-scim-owners) — trả về để người
        // đặt luật xem và duyệt (`POST …/scim/groups/{id}/members/{user_id}/approve`).
        let mut owners = super::owners::Owners::new(self, tenant);
        for entry in &rows {
            if !holders.contains(&entry.user_id)
                && !owners.can_give(entry.added_by, (org_id, role_id)).await?
            {
                report.waiting.push(entry.user_id);
                tracing::info!(user_id = %entry.user_id, %group_id, "SCIM group member waits for approval");
            }
        }
        Ok((row, report))
    }

    /// Bỏ luật `id`; thành viên của nhóm mất role đó (trừ khi nhóm khác hay tay cũng cấp).
    pub(crate) async fn delete_mapping(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let Some(row) =
            mapping::Entity::find_by_id((tenant.id().as_uuid(), id)).one(&transaction).await?
        else {
            transaction.rollback().await?;
            return Err(mapping_not_found(id));
        };
        // Khoá dòng nhóm như `create_mapping` / `update_group`: mọi lần đổi luật hay thành viên của
        // một nhóm nối đuôi nhau (cùng thứ tự khoá: nhóm trước).
        autoxide_kernel::rows::lock_one(
            &transaction,
            group::Entity::find_by_id((tenant.id().as_uuid(), row.group_id)),
        )
        .await?;
        // Bỏ luật là gỡ role đó khỏi mọi thành viên nhóm: chỉ người cấp được role đó ở nút đó mới
        // bỏ được (cùng luật như khi đặt luật, D-20261125-scim-grant-cover).
        let taken = [(row.org_id, row.role_id)];
        if let Err(error) =
            crate::role::require_grantable(&transaction, tenant, principal, &[], &taken).await
        {
            transaction.rollback().await?;
            return Err(error);
        }
        let deleted = mapping::Entity::delete_many()
            .filter(mapping::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if deleted.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(mapping_not_found(id));
        }
        let event = events::ScimGroupUnmapped {
            before: Some(
                json!({"id": id, "group_id": row.group_id, "org_id": row.org_id, "role_id": row.role_id}),
            ),
            after: None,
        };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                group_subject(row.group_id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(mapping_id = %id, "SCIM group mapping deleted");
        let members = members(tenant, row.group_id).await?;
        let report = self.sync_in(tenant, Actor::from(principal), principal, &members).await;
        match report.failed.into_iter().next() {
            Some((_, error)) => Err(error),
            None => Ok(()),
        }
    }
}

/// Những người trong `users` đang giữ role `role_id` ở nút `org_id` (dòng cũ `org_id` NULL là gốc).
async fn holders_of(
    tenant: &TenantContext,
    org_id: Uuid,
    role_id: Uuid,
    users: &[Uuid],
) -> Result<BTreeSet<Uuid>, Error> {
    use crate::role::entity::user_role;
    if users.is_empty() {
        return Ok(BTreeSet::new());
    }
    let root = autoxide_kernel::tenancy::root_of(tenant.id()).as_uuid();
    let at_org = if org_id == root {
        Expr::col(user_role::Column::OrgId)
            .eq(org_id)
            .or(Expr::col(user_role::Column::OrgId).is_null())
    } else {
        Expr::col(user_role::Column::OrgId).eq(org_id)
    };
    Ok(user_role::Entity::find()
        .filter(user_role::Column::RoleId.eq(role_id))
        .filter(user_role::Column::UserId.is_in(users.iter().copied()))
        .filter(at_org)
        .scoped(tenant)
        .all(tenant.database().write())
        .await?
        .into_iter()
        .map(|row| row.user_id)
        .collect())
}

/// Nhóm chưa có luật ánh xạ nào: thành viên của nó không nhận role gì từ nhóm.
fn unmapped(tenant: &TenantContext) -> sea_orm::Select<group::Entity> {
    let mapped = Query::select()
        .column(mapping::Column::GroupId)
        .from(mapping::Entity)
        .and_where(Expr::col(mapping::Column::TenantId).eq(tenant.id().as_uuid()))
        .to_owned();
    group::Entity::find()
        .scoped(tenant)
        .filter(Expr::col((group::Entity, group::Column::Id)).not_in_subquery(mapped))
}

/// Nguồn việc `auth.scim_unmapped_groups` (hộp việc, bước 51): nhóm SCIM chưa có luật ánh xạ.
pub(crate) struct UnmappedGroups;

#[async_trait::async_trait]
impl WorkSource for UnmappedGroups {
    fn id(&self) -> &'static str {
        "auth.scim_unmapped_groups"
    }

    fn permission(&self) -> Permission {
        SCIM_MANAGE
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        _: &Principal,
        _: OrgTarget,
    ) -> Result<u64, Error> {
        Ok(unmapped(tenant).count(tenant.database().read()).await?)
    }

    async fn items(
        &self,
        tenant: &TenantContext,
        _: &Principal,
        _: OrgTarget,
        limit: u16,
    ) -> Result<Vec<WorkItem>, Error> {
        let page = PageRequest::first(u32::from(limit), autoxide_kernel::pagination::Order::Asc);
        let rows = fetch_page(
            unmapped(tenant),
            group::Column::Id,
            |row| row.id,
            &page,
            tenant.database().read(),
        )
        .await?
        .data;
        Ok(rows
            .into_iter()
            .map(|row| {
                WorkItem::new(
                    row.id.to_string(),
                    Severity::Attention,
                    "Map the SCIM group {group} to roles; its members get no roles from it yet",
                    "organization.access",
                )
                .param("group", row.display_name)
            })
            .collect())
    }
}
