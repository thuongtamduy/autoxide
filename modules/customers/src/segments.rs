//! Nhóm khách (bước 47 mục 2): nhóm `manual` (nhân viên thêm) hay `rule` (quy tắc là dữ liệu JSON có
//! kiểm). Nhóm dùng cho bảng giá của `catalog` (mã `price_group`) và khuyến mại của `promotions` (ID
//! nhóm) qua `BuyerGroups` cắm vào `orders` — ID nhóm là UUID mờ với hai module đó (D-222).
//!
//! Quy tắc dịch thẳng thành điều kiện SQL trên bảng hồ sơ ([`GroupRule::condition`]): xem trước,
//! đánh giá một hồ sơ và đánh giá cả nhóm dùng cùng một điều kiện, chạy như nhau trên hai database.

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext, root_of};
use chrono::Duration;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, PaginatorTrait,
    QueryFilter, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{group, group_member, profile};
use crate::errors;
use crate::events::{Change, GroupChanged};
use crate::permissions::SEGMENTS_MANAGE;
use crate::service::{CUSTOMER, CustomersService, now};

pub(crate) const MANUAL: &str = "manual";
pub(crate) const RULE: &str = "rule";

/// Rule of a rule group: a customer belongs when ALL given conditions hold.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = CustomerGroupRule)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroupRule {
    /// Total spent at least this much, in the smallest unit of the shop's currency (e.g. `5000000`
    /// for 5,000,000 VND).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_spent_min: Option<i64>,
    /// At least this many orders (cancelled orders do not count).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orders_count_min: Option<i64>,
    /// Ordered within the last N days (1 to 3650).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_order_within_days: Option<i64>,
    /// Membership tier code is one of these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tier_in: Vec<String>,
    /// Has at least one of these tags (ids from `/api/v1/kernel/tags`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags_any: Vec<Uuid>,
    /// Registered or first bought at one of these units or below them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub org_in: Vec<Uuid>,
}

impl GroupRule {
    /// Kiểm quy tắc; lỗi thêm vào `problems` (trường `rule.<…>`).
    pub(crate) fn check(&self, problems: &mut Vec<FieldError>) {
        let empty = self.total_spent_min.is_none()
            && self.orders_count_min.is_none()
            && self.last_order_within_days.is_none()
            && self.tier_in.is_empty()
            && self.tags_any.is_empty()
            && self.org_in.is_empty();
        if empty {
            problems.push(FieldError::new("rule", "required", "must have at least one condition"));
        }
        if self.total_spent_min.is_some_and(|value| value < 0) {
            problems.push(FieldError::new("rule.total_spent_min", "range", "must not be negative"));
        }
        if self.orders_count_min.is_some_and(|value| value < 0) {
            problems.push(FieldError::new(
                "rule.orders_count_min",
                "range",
                "must not be negative",
            ));
        }
        if self.last_order_within_days.is_some_and(|days| !(1..=3650).contains(&days)) {
            problems.push(FieldError::new(
                "rule.last_order_within_days",
                "range",
                "must be from 1 to 3650",
            ));
        }
        if self.tier_in.len() > 50 || self.tags_any.len() > 50 || self.org_in.len() > 50 {
            problems.push(FieldError::new("rule", "length", "lists hold at most 50 values"));
        }
    }

    /// Điều kiện trên bảng hồ sơ.
    pub(crate) fn condition(&self, tenant: &TenantContext) -> Condition {
        let mut condition = Condition::all();
        if let Some(min) = self.total_spent_min {
            condition = condition.add(profile::Column::TotalSpentMinor.gte(min));
        }
        if let Some(min) = self.orders_count_min {
            condition = condition.add(profile::Column::OrdersCount.gte(min));
        }
        if let Some(days) = self.last_order_within_days {
            condition =
                condition.add(profile::Column::LastOrderAt.gte(now() - Duration::days(days)));
        }
        if !self.tier_in.is_empty() {
            condition = condition.add(profile::Column::TierCode.is_in(self.tier_in.clone()));
        }
        if !self.tags_any.is_empty() {
            condition = condition.add(autoxide_kernel::tags::TagIndex.condition(
                tenant,
                CUSTOMER,
                profile::Column::Id,
                &self.tags_any,
                &[],
            ));
        }
        if !self.org_in.is_empty() {
            let roots: Vec<OrgId> = self.org_in.iter().copied().map(OrgId::from_uuid).collect();
            condition = condition.add(OrgTree::subtree_condition(
                tenant.id(),
                &roots,
                profile::Column::OrgId,
            ));
        }
        condition
    }
}

/// Nhóm mới hay thay đổi của nhóm (`None`: giữ nguyên khi sửa).
#[derive(Debug, Clone, Default)]
pub(crate) struct GroupInput {
    pub name: Option<String>,
    pub description: Option<String>,
    pub kind: Option<String>,
    pub rule: Option<GroupRule>,
    /// `Some("")` xoá mã.
    pub price_group: Option<String>,
    pub org_id: Option<Uuid>,
}

/// Một nhóm và số thành viên.
#[derive(Debug, Clone)]
pub(crate) struct GroupView {
    pub row: group::Model,
    pub members: u64,
}

/// Điều kiện của nhóm `row` trên bảng hồ sơ: quy tắc và nhánh của nhóm. `None`: nhóm tay.
pub(crate) fn group_condition(tenant: &TenantContext, row: &group::Model) -> Option<Condition> {
    let rule: GroupRule = serde_json::from_str(row.rule.as_deref()?).ok()?;
    let mut condition = rule.condition(tenant);
    if row.org_id != root_of(tenant.id()).as_uuid() {
        condition = condition.add(OrgTree::subtree_condition(
            tenant.id(),
            &[OrgId::from_uuid(row.org_id)],
            profile::Column::OrgId,
        ));
    }
    Some(condition)
}

fn check_input(input: &GroupInput, creating: bool, kind: &str, problems: &mut Vec<FieldError>) {
    match &input.name {
        Some(name) if !name.trim().is_empty() && name.trim().chars().count() <= 100 => {}
        Some(_) => problems.push(FieldError::new("name", "length", "must be 1 to 100 characters")),
        None if creating => problems.push(FieldError::new("name", "required", "is required")),
        None => {}
    }
    if input.description.as_ref().is_some_and(|text| text.chars().count() > 500) {
        problems.push(FieldError::new("description", "length", "must be at most 500 characters"));
    }
    if ![MANUAL, RULE].contains(&kind) {
        problems.push(FieldError::new("kind", "invalid", "must be `manual` or `rule`"));
    }
    match (&input.rule, kind) {
        (Some(rule), RULE) => rule.check(problems),
        (None, RULE) if creating => {
            problems.push(FieldError::new("rule", "required", "a rule group needs a rule"));
        }
        (Some(_), MANUAL) => {
            problems.push(FieldError::new("rule", "invalid", "a manual group has no rule"));
        }
        _ => {}
    }
    if let Some(code) = input.price_group.as_deref().filter(|code| !code.is_empty()) {
        let ok = code.len() <= 64
            && code.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !ok {
            problems.push(FieldError::new(
                "price_group",
                "invalid",
                "must be up to 64 characters of `a-z`, `0-9` and `_`",
            ));
        }
    }
}

impl CustomersService {
    async fn group_in_scope(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<group::Model, Error> {
        self.group_visible(tenant, principal, &SEGMENTS_MANAGE, id).await
    }

    /// Nhóm `id` trong phạm vi `permission` của người gọi; ngoài phạm vi: 404.
    async fn group_visible(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        permission: &autoxide_kernel::access::Permission,
        id: Uuid,
    ) -> Result<group::Model, Error> {
        let read = tenant.database().read();
        let row = group::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(errors::group_not_found)?;
        principal
            .require_at(read, permission, OrgTarget::new(OrgId::from_uuid(row.org_id), None))
            .await
            .map_err(|error| {
                if error.code() == "org_out_of_scope" { errors::group_not_found() } else { error }
            })?;
        Ok(row)
    }

    async fn name_taken<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        name: &str,
        except: Option<Uuid>,
    ) -> Result<bool, Error> {
        let mut select = group::Entity::find().filter(group::Column::Name.eq(name)).scoped(tenant);
        if let Some(id) = except {
            select = select.filter(group::Column::Id.ne(id));
        }
        Ok(select.count(connection).await? > 0)
    }

    /// Tạo nhóm ở nút `org_id` (mặc định nút gốc) — `customers.segments.manage` ở đó.
    pub(crate) async fn create_group(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: GroupInput,
    ) -> Result<group::Model, Error> {
        let kind = input.kind.clone().unwrap_or_else(|| MANUAL.to_owned());
        let mut problems = Vec::new();
        check_input(&input, true, &kind, &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let name = input.name.clone().unwrap_or_default().trim().to_owned();
        let org = OrgId::from_uuid(input.org_id.unwrap_or_else(|| root_of(tenant.id()).as_uuid()));
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(&transaction, &SEGMENTS_MANAGE, OrgTarget::new(org, None))
            .await?;
        if Self::name_taken(&transaction, tenant, &name, None).await? {
            return Err(errors::group_name_taken(&name));
        }
        let at = now();
        let row = group::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org.as_uuid()),
            name: Set(name),
            description: Set(input.description.filter(|text| !text.trim().is_empty())),
            kind: Set(kind),
            rule: Set(input.rule.map(|rule| json!(rule).to_string())),
            price_group: Set(input.price_group.filter(|code| !code.is_empty())),
            created_at: Set(at),
            updated_at: Set(at),
        }
        .insert(&transaction)
        .await?;
        self.group_event(&transaction, tenant, principal, &row, "created").await?;
        if row.kind == RULE {
            self.schedule_group_evaluation(&transaction, tenant, row.id).await?;
        }
        transaction.commit().await?;
        Ok(row)
    }

    /// Sửa nhóm `id`. Đổi quy tắc → đánh giá lại thành viên (job).
    pub(crate) async fn update_group(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        input: GroupInput,
    ) -> Result<group::Model, Error> {
        let current = self.group_in_scope(tenant, principal, id).await?;
        let mut problems = Vec::new();
        if input.kind.as_deref().is_some_and(|kind| kind != current.kind) {
            problems.push(FieldError::new(
                "kind",
                "immutable",
                "cannot change; create a new group instead",
            ));
        }
        check_input(&input, false, &current.kind, &mut problems);
        if input.org_id.is_some_and(|org| org != current.org_id) {
            problems.push(FieldError::new("org_id", "immutable", "cannot change"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let transaction = tenant.database().begin_write().await?;
        let current = autoxide_kernel::rows::lock_one(
            &transaction,
            group::Entity::find_by_id(id).scoped(tenant),
        )
        .await?
        .ok_or_else(errors::group_not_found)?;
        let mut row: group::ActiveModel = current.clone().into();
        if let Some(name) = &input.name {
            let name = name.trim().to_owned();
            if Self::name_taken(&transaction, tenant, &name, Some(id)).await? {
                return Err(errors::group_name_taken(&name));
            }
            row.name = Set(name);
        }
        if let Some(description) = input.description {
            row.description = Set((!description.trim().is_empty()).then_some(description));
        }
        let rule_changed = input.rule.is_some();
        if let Some(rule) = input.rule {
            row.rule = Set(Some(json!(rule).to_string()));
        }
        if let Some(code) = input.price_group {
            row.price_group = Set((!code.is_empty()).then_some(code));
        }
        row.updated_at = Set(now());
        let updated = row.update(&transaction).await?;
        self.group_event(&transaction, tenant, principal, &updated, "updated").await?;
        if rule_changed {
            self.schedule_group_evaluation(&transaction, tenant, id).await?;
        }
        transaction.commit().await?;
        Ok(updated)
    }

    /// Xoá nhóm `id` và mọi thành viên của nó (bảng giá / khuyến mại còn trỏ tới ID nhóm thì không
    /// còn khớp ai nữa).
    pub(crate) async fn delete_group(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let current = self.group_in_scope(tenant, principal, id).await?;
        let transaction = tenant.database().begin_write().await?;
        group_member::Entity::delete_many()
            .filter(group_member::Column::GroupId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        group::Entity::delete_many()
            .filter(group::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        self.group_event(&transaction, tenant, principal, &current, "deleted").await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn group_event<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        principal: &Principal,
        row: &group::Model,
        action: &str,
    ) -> Result<(), Error> {
        self.outbox
            .record(
                connection,
                tenant,
                Actor::user(principal.user_id()),
                Some(format!("customers.group:{}", row.id)),
                &GroupChanged(Change::after(json!({
                    "group_id": row.id, "name": row.name, "kind": row.kind, "action": action,
                }))),
            )
            .await
    }

    /// Một nhóm trong phạm vi, kèm số thành viên.
    pub(crate) async fn group(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<GroupView, Error> {
        let row = self.group_in_scope(tenant, principal, id).await?;
        let members = group_member::Entity::find()
            .filter(group_member::Column::GroupId.eq(id))
            .scoped(tenant)
            .count(tenant.database().read())
            .await?;
        Ok(GroupView { row, members })
    }

    /// Một trang nhóm trong phạm vi `customers.segments.manage` (hay `customers.read`: nhân viên
    /// xem hồ sơ cần biết tên nhóm).
    pub(crate) async fn list_groups(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<group::Model>, Error> {
        let permission = if principal.has(&SEGMENTS_MANAGE) {
            &SEGMENTS_MANAGE
        } else {
            &crate::permissions::READ
        };
        let scope = principal.scope_condition(permission, group::Column::OrgId, None)?;
        let select = group::Entity::find().filter(scope).scoped(tenant);
        fetch_page(select, group::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Số hồ sơ khớp quy tắc của nhóm `id` lúc này (xem trước, không ghi gì) — với
    /// `customers.segments.manage` hay `customers.read` ở nút của nhóm.
    pub(crate) async fn preview_group(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<u64, Error> {
        let permission = if principal.has(&SEGMENTS_MANAGE) {
            &SEGMENTS_MANAGE
        } else {
            &crate::permissions::READ
        };
        let row = self.group_visible(tenant, principal, permission, id).await?;
        self.count_matching(tenant, &row).await
    }

    /// Số hồ sơ khớp nhóm `row` (nhóm tay: số thành viên).
    pub(crate) async fn count_matching(
        &self,
        tenant: &TenantContext,
        row: &group::Model,
    ) -> Result<u64, Error> {
        let read = tenant.database().read();
        match group_condition(tenant, row) {
            Some(condition) => {
                Ok(profile::Entity::find().filter(condition).scoped(tenant).count(read).await?)
            }
            None => Ok(group_member::Entity::find()
                .filter(group_member::Column::GroupId.eq(row.id))
                .scoped(tenant)
                .count(read)
                .await?),
        }
    }
}
