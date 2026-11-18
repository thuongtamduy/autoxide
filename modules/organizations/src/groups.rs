//! Nhóm nút ngoài cây cho báo cáo (bước 38, D-232): tập nút tự do trong MỘT khách (vd "cửa hàng
//! miền Bắc"), không theo cây, một nút ở nhiều nhóm được.
//!
//! **Nhóm không cấp quyền.** Quản lý nhóm cần `organizations.groups.manage` (chỉ cấp ở độ sâu
//! `tenant`, vì nhóm vắt qua nhiều nhánh); đọc nhóm cần `organizations.units.read`, và số thành viên
//! / danh sách thành viên luôn là phần GIAO với phạm vi đọc của người gọi — nút ngoài phạm vi chỉ lộ
//! thành số đếm (`excluded_org_count`), không lộ ID hay tên.

use std::collections::{BTreeMap, BTreeSet};

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::search::fold;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, QuerySelect, Set, SqlErr,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::unit;
use crate::events::{self, Change};
use crate::permissions::{GROUPS_MANAGE, UNITS_READ};
use crate::service::OrgService;

/// Số nút tối đa của một nhóm.
pub(crate) const MAX_MEMBERS: usize = 1000;
/// Số nút tối đa của một lệnh thêm.
pub(crate) const MAX_ADD: usize = 200;
const NAME_MAX: usize = 200;
const DESCRIPTION_MAX: usize = 1000;

pub(crate) mod entity {
    pub(crate) mod group {
        use sea_orm::entity::prelude::*;

        #[sea_orm::model]
        #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
        #[sea_orm(table_name = "organizations_group")]
        pub struct Model {
            #[sea_orm(primary_key, auto_increment = false)]
            pub tenant_id: Uuid,
            #[sea_orm(primary_key, auto_increment = false)]
            pub id: Uuid,
            pub name: String,
            /// `search::fold(name)`: tên duy nhất trong khách, không phân biệt dấu và hoa thường.
            pub name_folded: String,
            pub description: Option<String>,
            pub created_by: Option<Uuid>,
            pub created_at: DateTimeUtc,
            pub updated_at: DateTimeUtc,
        }

        impl ActiveModelBehavior for ActiveModel {}

        impl autoxide_kernel::tenancy::TenantScoped for Entity {
            fn tenant_column() -> Self::Column {
                Column::TenantId
            }
        }
    }

    pub(crate) mod member {
        use sea_orm::entity::prelude::*;

        #[sea_orm::model]
        #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
        #[sea_orm(table_name = "organizations_group_member")]
        pub struct Model {
            #[sea_orm(primary_key, auto_increment = false)]
            pub tenant_id: Uuid,
            #[sea_orm(primary_key, auto_increment = false)]
            pub group_id: Uuid,
            /// Nút bất kỳ của khách; không foreign key (nút lưu trữ vẫn ở lại trong nhóm).
            #[sea_orm(primary_key, auto_increment = false)]
            pub org_id: Uuid,
            pub added_by: Option<Uuid>,
            pub added_at: DateTimeUtc,
        }

        impl ActiveModelBehavior for ActiveModel {}

        impl autoxide_kernel::tenancy::TenantScoped for Entity {
            fn tenant_column() -> Self::Column {
                Column::TenantId
            }
        }
    }
}

use entity::{group, member};

pub(crate) type GroupRow = group::Model;

/// Một nhóm kèm số thành viên trong phạm vi người gọi.
#[derive(Debug, Clone)]
pub(crate) struct GroupView {
    pub row: GroupRow,
    pub member_count: u64,
    /// Thành viên ngoài phạm vi người gọi (chỉ lộ số đếm).
    pub excluded_org_count: u64,
}

/// Thay đổi một nhóm; `None` giữ nguyên. `description: Some(None)` bỏ mô tả.
#[derive(Debug, Clone, Default)]
pub(crate) struct GroupChanges {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
}

/// Kết quả thêm thành viên.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Added {
    /// Nút mới vào nhóm (nút đã có không tính).
    pub added: u64,
    pub member_count: u64,
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

pub(crate) fn group_not_found(id: Uuid) -> Error {
    Error::not_found("organizations.group_not_found", "There is no unit group `{id}`.")
        .with_param("id", id)
}

fn name_taken(name: &str) -> Error {
    Error::conflict(
        "organizations.group_name_taken",
        "The group name `{name}` is already used by another group.",
    )
    .with_param("name", name)
}

fn snapshot(row: &GroupRow) -> Value {
    json!({"id": row.id, "name": row.name, "description": row.description})
}

fn subject(id: Uuid) -> Option<String> {
    Some(format!("organizations.group:{id}"))
}

/// Tên (1–200 ký tự) và mô tả (tối đa 1000) đã bỏ khoảng trắng hai đầu; báo mọi lỗi một lần.
fn check(name: Option<&str>, description: Option<Option<&str>>) -> Result<GroupChanges, Error> {
    let mut problems = Vec::new();
    let name = name.map(str::trim);
    if name.is_some_and(|name| !(1..=NAME_MAX).contains(&name.chars().count())) {
        problems.push(
            FieldError::new("name", "length", "must be 1 to {max} characters, not only spaces")
                .with_param("max", NAME_MAX),
        );
    }
    let description = description.map(|text| text.map(str::trim).filter(|text| !text.is_empty()));
    if let Some(Some(text)) = description
        && text.chars().count() > DESCRIPTION_MAX
    {
        problems.push(
            FieldError::new("description", "length", "must be at most {max} characters long")
                .with_param("max", DESCRIPTION_MAX),
        );
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    Ok(GroupChanges {
        name: name.map(str::to_owned),
        description: description.map(|text| text.map(str::to_owned)),
    })
}

async fn find<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<GroupRow, Error> {
    group::Entity::find()
        .filter(group::Column::Id.eq(id))
        .scoped(tenant)
        .one(connection)
        .await?
        .ok_or_else(|| group_not_found(id))
}

fn unique_name(error: sea_orm::DbErr, name: &str) -> Error {
    match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => name_taken(name),
        _ => Error::from(error),
    }
}

impl OrgService {
    /// Một trang nhóm của khách (tìm theo tên không dấu); `member_count` chỉ đếm nút trong phạm vi
    /// `organizations.units.read` của người gọi.
    pub(crate) async fn list_groups(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        q: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<GroupView>, Error> {
        principal.require(&UNITS_READ)?;
        let read = tenant.database().read();
        let mut select = group::Entity::find().scoped(tenant);
        if let Some(q) = q.map(fold).filter(|q| !q.is_empty()) {
            select = select.filter(group::Column::NameFolded.contains(q));
        }
        let page = fetch_page(select, group::Column::Id, |row| row.id, page, read).await?;
        let ids: Vec<Uuid> = page.data.iter().map(|row| row.id).collect();
        let counts = self.member_counts(tenant, principal, &ids).await?;
        Ok(page.map(|row| {
            let (member_count, total) = counts.get(&row.id).copied().unwrap_or_default();
            GroupView { row, member_count, excluded_org_count: total.saturating_sub(member_count) }
        }))
    }

    /// `(trong phạm vi người gọi, tổng)` số thành viên của từng nhóm `ids` — hai câu `GROUP BY`.
    async fn member_counts(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        ids: &[Uuid],
    ) -> Result<BTreeMap<Uuid, (u64, u64)>, Error> {
        if ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        let scope = principal.scope_condition(&UNITS_READ, member::Column::OrgId, None)?;
        let count = async |scope: Option<sea_orm::Condition>| -> Result<Vec<(Uuid, i64)>, Error> {
            let mut select = member::Entity::find()
                .select_only()
                .column(member::Column::GroupId)
                .column_as(member::Column::OrgId.count(), "members")
                .filter(member::Column::GroupId.is_in(ids.to_vec()));
            if let Some(scope) = scope {
                select = select.filter(scope);
            }
            Ok(select
                .scoped(tenant)
                .group_by(member::Column::GroupId)
                .into_tuple()
                .all(tenant.database().read())
                .await?)
        };
        let mut counts: BTreeMap<Uuid, (u64, u64)> = BTreeMap::new();
        for (id, total) in count(None).await? {
            counts.entry(id).or_default().1 = u64::try_from(total).unwrap_or(0);
        }
        for (id, visible) in count(Some(scope)).await? {
            counts.entry(id).or_default().0 = u64::try_from(visible).unwrap_or(0);
        }
        Ok(counts)
    }

    /// Nhóm `id` kèm số thành viên trong phạm vi người gọi.
    pub(crate) async fn get_group(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<GroupView, Error> {
        principal.require(&UNITS_READ)?;
        let row = find(tenant.database().read(), tenant, id).await?;
        let (member_count, total) = self
            .member_counts(tenant, principal, &[id])
            .await?
            .get(&id)
            .copied()
            .unwrap_or_default();
        Ok(GroupView { row, member_count, excluded_org_count: total.saturating_sub(member_count) })
    }

    pub(crate) async fn create_group(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        name: &str,
        description: Option<&str>,
    ) -> Result<GroupRow, Error> {
        principal.require(&GROUPS_MANAGE)?;
        let checked = check(Some(name), Some(description))?;
        let name = checked.name.unwrap_or_default();
        let now = now();
        let transaction = tenant.database().begin_write().await?;
        let row = group::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            id: Set(Uuid::now_v7()),
            name_folded: Set(fold(&name)),
            name: Set(name.clone()),
            description: Set(checked.description.flatten()),
            created_by: Set(Some(principal.user_id())),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await
        .map_err(|error| unique_name(error, &name))?;
        let change = Change { before: None, after: Some(snapshot(&row)) };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                subject(row.id),
                &events::GroupCreated(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Đổi tên / mô tả. `expected`: `updated_at` người gọi đã đọc (`If-Match`).
    pub(crate) async fn update_group(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        changes: GroupChanges,
        expected: Option<DateTime<Utc>>,
    ) -> Result<GroupRow, Error> {
        principal.require(&GROUPS_MANAGE)?;
        let checked = check(
            changes.name.as_deref(),
            changes.description.as_ref().map(|text| text.as_deref()),
        )?;
        let transaction = tenant.database().begin_write().await?;
        let before = find(&transaction, tenant, id).await?;
        let mut row = before.clone().into_active_model();
        if let Some(name) = &checked.name {
            row.name_folded = Set(fold(name));
            row.name = Set(name.clone());
        }
        if let Some(description) = checked.description {
            row.description = Set(description);
        }
        row.updated_at = Set(now());
        let mut update = group::Entity::update(row).validate()?.scoped(tenant);
        if let Some(expected) = expected {
            update = update.filter(group::Column::UpdatedAt.eq(expected));
        }
        let name = checked.name.clone().unwrap_or_else(|| before.name.clone());
        let after = update.exec(&transaction).await.map_err(|error| match error {
            sea_orm::DbErr::RecordNotUpdated => autoxide_kernel::version::changed(),
            error => unique_name(error, &name),
        })?;
        let change = Change { before: Some(snapshot(&before)), after: Some(snapshot(&after)) };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                subject(id),
                &events::GroupUpdated(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Xoá nhóm cùng danh sách thành viên; số liệu báo cáo không bị xoá.
    pub(crate) async fn delete_group(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&GROUPS_MANAGE)?;
        let transaction = tenant.database().begin_write().await?;
        let before = find(&transaction, tenant, id).await?;
        member::Entity::delete_many()
            .filter(member::Column::GroupId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        group::Entity::delete_many()
            .filter(group::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let change = Change { before: Some(snapshot(&before)), after: None };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                subject(id),
                &events::GroupDeleted(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Một trang thành viên trong phạm vi đọc của người gọi (số nút ngoài phạm vi: `get_group`).
    pub(crate) async fn list_members(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<member::Model>, Error> {
        principal.require(&UNITS_READ)?;
        let read = tenant.database().read();
        find(read, tenant, id).await?;
        let scope = principal.scope_condition(&UNITS_READ, member::Column::OrgId, None)?;
        let select = member::Entity::find()
            .filter(member::Column::GroupId.eq(id))
            .filter(scope)
            .scoped(tenant);
        fetch_page(select, member::Column::OrgId, |row| row.org_id, page, read).await
    }

    /// Thêm nút vào nhóm; nút đã có bỏ qua (gọi lại không trùng, kể cả hai lời gọi cùng lúc). Nút
    /// không thuộc khách → `organizations.group_member_invalid`; quá 1 000 nút →
    /// `organizations.group_too_large`.
    pub(crate) async fn add_members(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        org_ids: &[Uuid],
    ) -> Result<Added, Error> {
        principal.require(&GROUPS_MANAGE)?;
        let wanted: BTreeSet<Uuid> = org_ids.iter().copied().collect();
        if wanted.is_empty() || wanted.len() > MAX_ADD {
            return Err(Error::validation(vec![
                FieldError::new("org_ids", "length", "must list 1 to {max} unit ids")
                    .with_param("max", MAX_ADD),
            ]));
        }
        let transaction = tenant.database().begin_write().await?;
        let before = find(&transaction, tenant, id).await?;
        // Chạm dòng nhóm trước: hai lệnh thêm cùng nhóm nối đuôi nhau (khoá dòng trên Postgres,
        // single-writer trên SQLite), nên trần 1 000 nút đếm đúng.
        group::Entity::update_many()
            .col_expr(group::Column::UpdatedAt, sea_orm::sea_query::Expr::value(now()))
            .filter(group::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let found: BTreeSet<Uuid> = unit::Entity::find()
            .select_only()
            .column(unit::Column::Id)
            .filter(unit::Column::Id.is_in(wanted.iter().copied()))
            .scoped(tenant)
            .into_tuple::<Uuid>()
            .all(&transaction)
            .await?
            .into_iter()
            .collect();
        let missing: Vec<String> = wanted.difference(&found).map(ToString::to_string).collect();
        if !missing.is_empty() {
            return Err(Error::new(
                ErrorKind::Validation,
                "organizations.group_member_invalid",
                "These units do not belong to this tenant: {ids}.",
            )
            .with_param("ids", missing.join(", ")));
        }
        let now = now();
        let rows = wanted.iter().map(|org| member::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            group_id: Set(id),
            org_id: Set(*org),
            added_by: Set(Some(principal.user_id())),
            added_at: Set(now),
        });
        let inserted = member::Entity::insert_many(rows)
            .on_conflict(
                OnConflict::columns([
                    member::Column::TenantId,
                    member::Column::GroupId,
                    member::Column::OrgId,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
        let member_count = member::Entity::find()
            .filter(member::Column::GroupId.eq(id))
            .scoped(tenant)
            .count(&transaction)
            .await?;
        if member_count > MAX_MEMBERS as u64 {
            return Err(too_large());
        }
        if inserted > 0 {
            let change = Change {
                before: Some(snapshot(&before)),
                after: Some(json!({"id": id, "added": inserted, "removed": 0})),
            };
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::from(principal),
                    subject(id),
                    &events::GroupMembersChanged(change),
                )
                .await?;
        }
        transaction.commit().await?;
        Ok(Added { added: inserted, member_count })
    }

    /// Bỏ nút `org_id` khỏi nhóm; nút không có trong nhóm thì không làm gì.
    pub(crate) async fn remove_member(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        org_id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&GROUPS_MANAGE)?;
        let transaction = tenant.database().begin_write().await?;
        let before = find(&transaction, tenant, id).await?;
        let removed = member::Entity::delete_many()
            .filter(member::Column::GroupId.eq(id))
            .filter(member::Column::OrgId.eq(org_id))
            .scoped(tenant)
            .exec(&transaction)
            .await?
            .rows_affected;
        if removed > 0 {
            let change = Change {
                before: Some(snapshot(&before)),
                after: Some(json!({"id": id, "added": 0, "removed": removed})),
            };
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::from(principal),
                    subject(id),
                    &events::GroupMembersChanged(change),
                )
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}

fn too_large() -> Error {
    Error::new(
        ErrorKind::Validation,
        "organizations.group_too_large",
        "A group can have at most {max} units.",
    )
    .with_param("max", MAX_MEMBERS)
}

#[async_trait::async_trait]
impl crate::api::OrgGroups for OrgService {
    async fn members(
        &self,
        tenant: &TenantContext,
        group: Uuid,
    ) -> Result<Option<Vec<OrgId>>, Error> {
        let read = tenant.database().read();
        let exists = group::Entity::find()
            .filter(group::Column::Id.eq(group))
            .scoped(tenant)
            .one(read)
            .await?
            .is_some();
        if !exists {
            return Ok(None);
        }
        let ids: Vec<Uuid> = member::Entity::find()
            .select_only()
            .column(member::Column::OrgId)
            .filter(member::Column::GroupId.eq(group))
            .scoped(tenant)
            .into_tuple()
            .all(read)
            .await?;
        Ok(Some(ids.into_iter().map(OrgId::from_uuid).collect()))
    }
}
