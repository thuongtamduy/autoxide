//! Luật ánh xạ nhóm SCIM → role ở nút (quản trị, envelope chung của Autoxide) và nguồn việc "nhóm
//! chưa có luật". Người tạo luật phải gán được role đó ở nút đó (cùng kiểm như gán membership:
//! nút trong phạm vi, có mọi permission của role, `admin` chỉ ở gốc) — luật là cách gán role cho
//! mọi người trong nhóm, nên không được dùng để tự nâng quyền.

use std::collections::{BTreeSet, HashMap};

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt as _, TenantContext};
use autoxide_kernel::work::{Severity, WorkItem, WorkSource};
use chrono::{SubsecRound, Utc};
use sea_orm::sea_query::{Expr, ExprTrait as _, Query};
use sea_orm::{
    ActiveModelTrait as _, ColumnTrait as _, EntityTrait as _, PaginatorTrait as _,
    QueryFilter as _, QueryOrder as _, SqlErr,
};
use serde_json::json;
use uuid::Uuid;

use super::ScimService;
use super::entity::{group, mapping, member};
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

/// Thành viên của nhóm `group_id`.
async fn members(tenant: &TenantContext, group_id: Uuid) -> Result<BTreeSet<Uuid>, Error> {
    Ok(member::Entity::find()
        .filter(member::Column::GroupId.eq(group_id))
        .scoped(tenant)
        .all(tenant.database().read())
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

    /// Mọi luật ánh xạ của khách (một khách có ít luật), cũ nhất trước.
    pub(crate) async fn mappings(
        &self,
        tenant: &TenantContext,
    ) -> Result<Vec<GroupMapping>, Error> {
        Ok(mapping::Entity::find()
            .scoped(tenant)
            .order_by_asc(mapping::Column::Id)
            .all(tenant.database().read())
            .await?)
    }

    /// Thêm luật "nhóm `group_id` → role `role_id` ở nút `org_id`", rồi cấp role cho mọi thành
    /// viên hiện có của nhóm.
    pub(crate) async fn create_mapping(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        group_id: Uuid,
        org_id: Uuid,
        role_id: Uuid,
    ) -> Result<GroupMapping, Error> {
        let transaction = tenant.database().begin_write().await?;
        let Some(found) =
            group::Entity::find_by_id((tenant.id().as_uuid(), group_id)).one(&transaction).await?
        else {
            transaction.rollback().await?;
            return Err(group_not_found(group_id));
        };
        let roles = BTreeSet::from([role_id]);
        let problems =
            crate::role::grant_problems(&transaction, tenant, principal, "", org_id, &roles)
                .await?;
        if !problems.is_empty() {
            transaction.rollback().await?;
            return Err(Error::validation(problems));
        }
        let row = mapping::Model {
            tenant_id: tenant.id().as_uuid(),
            id: Uuid::now_v7(),
            group_id,
            org_id,
            role_id,
            created_by: principal.user_id(),
            created_at: Utc::now().trunc_subsecs(6),
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
        self.sync_in(tenant, Actor::from(principal), &members(tenant, group_id).await?).await?;
        Ok(row)
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
        self.sync_in(tenant, Actor::from(principal), &members(tenant, row.group_id).await?).await
    }
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
