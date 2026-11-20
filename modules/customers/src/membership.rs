//! Thành viên của nhóm khách (bước 47 mục 2): thêm / bớt tay ở nhóm `manual`; đánh giá quy tắc ở
//! nhóm `rule` — theo hồ sơ ngay trong transaction làm đổi hồ sơ, theo nhóm bằng job khi quy tắc
//! đổi, và toàn bộ hằng đêm (quy tắc theo thời gian đổi kết quả dù không có event).
//!
//! Cắm vào `orders` qua `BuyerGroups`: nhóm, hạng thành viên và mã nhóm giá của người mua (D-222).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{
    EnqueueOptions, Job, JobContext, JobHandler, JobRegistration, Schedule,
};
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, TenantId};
use autoxide_orders::api::{BuyerGroups, BuyerProfile};
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::api::{CustomerSummary, Customers};
use crate::entity::{group, group_member, profile};
use crate::errors;
use crate::events::{Change, GroupMembershipChanged};
use crate::permissions::SEGMENTS_MANAGE;
use crate::profiles::subject;
use crate::segments::{MANUAL, RULE, group_condition};
use crate::service::{CustomersService, mask_email, mask_phone, now};

pub(crate) const EVALUATE_GROUP: &str = "customers.evaluate_group";
pub(crate) const EVALUATE_GROUPS: &str = "customers.evaluate_groups";

/// Số hồ sơ mỗi lượt khi đánh giá cả nhóm.
const CHUNK: u64 = 500;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct EvaluateGroup {
    pub group_id: Uuid,
}

impl Job for EvaluateGroup {
    const KIND: &'static str = EVALUATE_GROUP;
}

impl CustomersService {
    /// Đánh giá lại nhóm quy tắc của hồ sơ NGAY trong transaction của thay đổi (vài câu đọc theo
    /// index, không gọi mạng): thay đổi và thành viên nhóm luôn khớp nhau, không có job nào bị bỏ vì
    /// trùng khoá.
    pub(crate) async fn schedule_evaluation<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        profile_id: Uuid,
    ) -> Result<(), Error> {
        self.evaluate_profile(connection, tenant, profile_id).await
    }

    pub(crate) async fn schedule_group_evaluation<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        group_id: Uuid,
    ) -> Result<(), Error> {
        // Không khoá trùng: khoá của job còn sống tới khi job xong bị dọn, hai lần đổi quy tắc liền
        // nhau sẽ mất lần sau. Đánh giá hai lần vô hại (chèn / xoá có điều kiện).
        self.jobs
            .enqueue(connection, tenant, &EvaluateGroup { group_id }, EnqueueOptions::default())
            .await?;
        Ok(())
    }

    /// Thêm hồ sơ `profile_id` vào nhóm tay `group_id` (gọi lại vô hại).
    pub(crate) async fn add_member(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        group_id: Uuid,
        profile_id: Uuid,
    ) -> Result<(), Error> {
        let (row, _) = self.manual_target(tenant, principal, group_id, profile_id).await?;
        let transaction = tenant.database().begin_write().await?;
        let inserted = insert_member(&transaction, tenant, row.id, profile_id, MANUAL).await?;
        if inserted {
            self.membership_event(
                &transaction,
                tenant,
                principal_actor(principal),
                profile_id,
                &[row.id],
                &[],
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Bỏ hồ sơ `profile_id` khỏi nhóm tay `group_id` (không có thì thôi).
    pub(crate) async fn remove_member(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        group_id: Uuid,
        profile_id: Uuid,
    ) -> Result<(), Error> {
        let (row, _) = self.manual_target(tenant, principal, group_id, profile_id).await?;
        let transaction = tenant.database().begin_write().await?;
        let removed = group_member::Entity::delete_many()
            .filter(group_member::Column::GroupId.eq(row.id))
            .filter(group_member::Column::ProfileId.eq(profile_id))
            .scoped(tenant)
            .exec(&transaction)
            .await?
            .rows_affected;
        if removed > 0 {
            self.membership_event(
                &transaction,
                tenant,
                principal_actor(principal),
                profile_id,
                &[],
                &[row.id],
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn manual_target(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        group_id: Uuid,
        profile_id: Uuid,
    ) -> Result<(group::Model, profile::Model), Error> {
        let read = tenant.database().read();
        let row = group::Entity::find_by_id(group_id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(errors::group_not_found)?;
        principal
            .require_at(read, &SEGMENTS_MANAGE, OrgTarget::new(OrgId::from_uuid(row.org_id), None))
            .await
            .map_err(|error| {
                if error.code() == "org_out_of_scope" { errors::group_not_found() } else { error }
            })?;
        if row.kind != MANUAL {
            return Err(errors::group_not_manual());
        }
        let found = self.get(tenant, principal, profile_id).await?;
        Ok((row, found))
    }

    async fn membership_event<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        actor: Actor,
        profile_id: Uuid,
        joined: &[Uuid],
        left: &[Uuid],
    ) -> Result<(), Error> {
        self.outbox
            .record(
                connection,
                tenant,
                actor,
                Some(subject(profile_id)),
                &GroupMembershipChanged(Change::after(json!({
                    "profile_id": profile_id, "joined": joined, "left": left,
                }))),
            )
            .await
    }

    /// Đánh giá mọi nhóm quy tắc cho một hồ sơ.
    async fn evaluate_profile<C: ConnectionTrait>(
        &self,
        transaction: &C,
        tenant: &TenantContext,
        profile_id: Uuid,
    ) -> Result<(), Error> {
        let rules = group::Entity::find()
            .filter(group::Column::Kind.eq(RULE))
            .scoped(tenant)
            .order_by_asc(group::Column::Id)
            .all(transaction)
            .await?;
        let mut matching = BTreeSet::new();
        for row in &rules {
            let Some(condition) = group_condition(tenant, row) else { continue };
            let found = profile::Entity::find()
                .filter(condition)
                .filter(profile::Column::Id.eq(profile_id))
                .scoped(tenant)
                .select_only()
                .column(profile::Column::Id)
                .into_tuple::<Uuid>()
                .one(transaction)
                .await?;
            if found.is_some() {
                matching.insert(row.id);
            }
        }
        let current: BTreeSet<Uuid> = group_member::Entity::find()
            .filter(group_member::Column::ProfileId.eq(profile_id))
            .filter(group_member::Column::Source.eq(RULE))
            .scoped(tenant)
            .select_only()
            .column(group_member::Column::GroupId)
            .into_tuple::<Uuid>()
            .all(transaction)
            .await?
            .into_iter()
            .collect();
        let joined: Vec<Uuid> = matching.difference(&current).copied().collect();
        let left: Vec<Uuid> = current.difference(&matching).copied().collect();
        for group_id in &joined {
            insert_member(transaction, tenant, *group_id, profile_id, RULE).await?;
        }
        if !left.is_empty() {
            group_member::Entity::delete_many()
                .filter(group_member::Column::ProfileId.eq(profile_id))
                .filter(group_member::Column::GroupId.is_in(left.clone()))
                .filter(group_member::Column::Source.eq(RULE))
                .scoped(tenant)
                .exec(transaction)
                .await?;
        }
        if !joined.is_empty() || !left.is_empty() {
            self.membership_event(transaction, tenant, Actor::system(), profile_id, &joined, &left)
                .await?;
        }
        Ok(())
    }

    /// Đánh giá lại cả nhóm quy tắc `group_id`: thêm hồ sơ khớp, bỏ hồ sơ không còn khớp. Theo lô
    /// `CHUNK` hồ sơ, mỗi lô một transaction ngắn.
    pub(crate) async fn evaluate_group(
        &self,
        tenant: &TenantContext,
        group_id: Uuid,
    ) -> Result<(), Error> {
        let read = tenant.database().read();
        let Some(row) = group::Entity::find_by_id(group_id).scoped(tenant).one(read).await? else {
            return Ok(());
        };
        let Some(condition) = group_condition(tenant, &row) else { return Ok(()) };
        // 1. Hồ sơ khớp mà chưa là thành viên.
        let mut cursor = Uuid::nil();
        loop {
            let ids: Vec<Uuid> = profile::Entity::find()
                .filter(condition.clone())
                .filter(profile::Column::Id.gt(cursor))
                .scoped(tenant)
                .select_only()
                .column(profile::Column::Id)
                .order_by_asc(profile::Column::Id)
                .limit(CHUNK)
                .into_tuple()
                .all(read)
                .await?;
            let Some(last) = ids.last().copied() else { break };
            let transaction = tenant.database().begin_write().await?;
            for id in ids {
                if insert_member(&transaction, tenant, group_id, id, RULE).await? {
                    self.membership_event(
                        &transaction,
                        tenant,
                        Actor::system(),
                        id,
                        &[group_id],
                        &[],
                    )
                    .await?;
                }
            }
            transaction.commit().await?;
            cursor = last;
        }
        // 2. Thành viên không còn khớp.
        let mut cursor = Uuid::nil();
        loop {
            let members: Vec<Uuid> = group_member::Entity::find()
                .filter(group_member::Column::GroupId.eq(group_id))
                .filter(group_member::Column::ProfileId.gt(cursor))
                .scoped(tenant)
                .select_only()
                .column(group_member::Column::ProfileId)
                .order_by_asc(group_member::Column::ProfileId)
                .limit(CHUNK)
                .into_tuple()
                .all(read)
                .await?;
            let Some(last) = members.last().copied() else { break };
            let still: BTreeSet<Uuid> = profile::Entity::find()
                .filter(condition.clone())
                .filter(profile::Column::Id.is_in(members.clone()))
                .scoped(tenant)
                .select_only()
                .column(profile::Column::Id)
                .into_tuple::<Uuid>()
                .all(read)
                .await?
                .into_iter()
                .collect();
            let gone: Vec<Uuid> = members.into_iter().filter(|id| !still.contains(id)).collect();
            if !gone.is_empty() {
                let transaction = tenant.database().begin_write().await?;
                for id in gone {
                    let removed = group_member::Entity::delete_many()
                        .filter(group_member::Column::GroupId.eq(group_id))
                        .filter(group_member::Column::ProfileId.eq(id))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?
                        .rows_affected;
                    if removed > 0 {
                        self.membership_event(
                            &transaction,
                            tenant,
                            Actor::system(),
                            id,
                            &[],
                            &[group_id],
                        )
                        .await?;
                    }
                }
                transaction.commit().await?;
            }
            cursor = last;
        }
        Ok(())
    }

    /// ID các nhóm của hồ sơ `profile_id`.
    pub(crate) async fn groups_of_profile(
        &self,
        tenant: &TenantContext,
        profile_id: Uuid,
    ) -> Result<Vec<Uuid>, Error> {
        Ok(group_member::Entity::find()
            .filter(group_member::Column::ProfileId.eq(profile_id))
            .scoped(tenant)
            .select_only()
            .column(group_member::Column::GroupId)
            .order_by_asc(group_member::Column::GroupId)
            .into_tuple()
            .all(tenant.database().read())
            .await?)
    }

    async fn summary(
        &self,
        tenant: &TenantContext,
        row: profile::Model,
    ) -> Result<CustomerSummary, Error> {
        let group_ids = self.groups_of_profile(tenant, row.id).await?;
        let total_spent = match &row.currency {
            Some(currency) => Some(Money::from_columns(row.total_spent_minor, currency)?),
            None => None,
        };
        Ok(CustomerSummary {
            id: row.id,
            user_id: row.user_id,
            org_id: OrgId::from_uuid(row.org_id),
            full_name: row.full_name,
            phone_masked: row.phone.as_deref().map(mask_phone),
            email_masked: row.email.as_deref().map(mask_email),
            tier: row.tier_code,
            group_ids,
            total_spent,
            orders_count: row.orders_count,
        })
    }
}

fn principal_actor(principal: &Principal) -> Actor {
    Actor::user(principal.user_id())
}

/// Chèn thành viên; trả `true` khi là dòng mới.
async fn insert_member<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    group_id: Uuid,
    profile_id: Uuid,
    source: &str,
) -> Result<bool, Error> {
    let inserted = group_member::Entity::insert(group_member::ActiveModel {
        tenant_id: Set(tenant.id().as_uuid()),
        group_id: Set(group_id),
        profile_id: Set(profile_id),
        source: Set(source.to_owned()),
        added_at: Set(now()),
    })
    .on_conflict(
        OnConflict::columns([
            group_member::Column::TenantId,
            group_member::Column::GroupId,
            group_member::Column::ProfileId,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec_without_returning(connection)
    .await?;
    Ok(inserted == 1)
}

/// Cài `Customers` cho module khác và `BuyerGroups` cho `orders`.
pub(crate) struct Provider(pub Arc<CustomersService>);

#[async_trait::async_trait]
impl Customers for Provider {
    async fn find_by_user(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Option<CustomerSummary>, Error> {
        let row = profile::Entity::find()
            .filter(profile::Column::UserId.eq(user_id))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        match row {
            Some(row) => Ok(Some(self.0.summary(tenant, row).await?)),
            None => Ok(None),
        }
    }

    async fn groups_of_user(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Vec<Uuid>, Error> {
        Ok(self
            .find_by_user(tenant, user_id)
            .await?
            .map(|found| found.group_ids)
            .unwrap_or_default())
    }
}

#[async_trait::async_trait]
impl BuyerGroups for Provider {
    async fn profile_of(&self, tenant: &TenantContext, buyer: Uuid) -> Result<BuyerProfile, Error> {
        let read = tenant.database().read();
        let Some(row) = profile::Entity::find()
            .filter(profile::Column::UserId.eq(buyer))
            .scoped(tenant)
            .one(read)
            .await?
        else {
            return Ok(BuyerProfile::default());
        };
        let groups = self.0.groups_of_profile(tenant, row.id).await?;
        // Mã nhóm giá: của nhóm đầu tiên theo tên có mã — cố định, không phụ thuộc thứ tự chèn.
        let price_group = if groups.is_empty() {
            None
        } else {
            group::Entity::find()
                .filter(group::Column::Id.is_in(groups.clone()))
                .filter(group::Column::PriceGroup.is_not_null())
                .scoped(tenant)
                .order_by_asc(group::Column::Name)
                .one(read)
                .await?
                .and_then(|row| row.price_group)
        };
        Ok(BuyerProfile::new(groups, row.tier_code, price_group))
    }
}

pub(crate) fn registrations(service: &Arc<CustomersService>) -> Vec<JobRegistration> {
    vec![
        JobRegistration::new(EVALUATE_GROUP, Arc::new(GroupJob(service.clone()))),
        JobRegistration::new(EVALUATE_GROUPS, Arc::new(NightlyJob(service.clone()))),
    ]
}

pub(crate) fn schedules() -> Vec<Schedule> {
    // Mỗi ngày một lần: quy tắc theo thời gian ("mua trong 90 ngày") đổi kết quả dù hồ sơ không đổi.
    vec![Schedule::new(EVALUATE_GROUPS, EVALUATE_GROUPS, Duration::from_secs(24 * 3600))]
}

fn tenant_of(job: &JobContext) -> Result<&TenantContext, Error> {
    job.tenant.as_ref().ok_or_else(|| Error::internal("customers job without a tenant"))
}

struct GroupJob(Arc<CustomersService>);

#[async_trait::async_trait]
impl JobHandler for GroupJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let input: EvaluateGroup =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.evaluate_group(tenant_of(job)?, input.group_id).await
    }
}

/// Lịch hằng đêm (theo database): mọi nhóm quy tắc của mọi khách trong database.
struct NightlyJob(Arc<CustomersService>);

#[async_trait::async_trait]
impl JobHandler for NightlyJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let rows: Vec<(Uuid, Uuid)> = group::Entity::find()
            .filter(group::Column::Kind.eq(RULE))
            .select_only()
            .column(group::Column::TenantId)
            .column(group::Column::Id)
            .order_by_asc(group::Column::TenantId)
            .into_tuple()
            .all(job.database.read())
            .await?;
        for (tenant_id, group_id) in rows {
            let tenant = self.0.databases.tenant(TenantId::from_uuid(tenant_id)).await?;
            if let Err(error) = self.0.evaluate_group(&tenant, group_id).await {
                tracing::warn!(tenant_id = %tenant_id, group_id = %group_id, error = %error, "customer group evaluation failed");
            }
        }
        Ok(())
    }
}
