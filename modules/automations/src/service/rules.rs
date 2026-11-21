//! Luật: tạo, đọc, sửa, xoá, bật, tắt. Luật thuộc một nút của cây tổ chức (bước 34): danh sách lọc
//! theo phạm vi của `automations.read`, ghi kiểm `automations.manage` ở nút của luật TRONG
//! transaction ghi.
//!
//! Luật chạy với quyền HIỆN TẠI của `run_as`: người tạo, hay người bật luật (người bật chịu trách
//! nhiệm — luật do AI agent tạo nằm ở `pending_approval` tới khi một người có quyền bật nó, và từ đó
//! chạy với quyền của người đó).

use autoxide_kernel::access::{ActorKind, OrgTarget, Principal, Scope};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, root_of};
use autoxide_kernel::time::{self, Timestamp};
use jiff::tz::TimeZone;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use serde_json::{Value, json};
use uuid::Uuid;

use super::validate::RawBody;
use super::{Automations, human_required, now, rule_not_found};
use crate::definition::RuleBody;
use crate::entity::rule::{self, Column, Entity};
use crate::events::{self, Change};
use crate::permissions::{AUTOMATIONS_MANAGE, AUTOMATIONS_READ};

pub(crate) type Rule = rule::Model;

/// Trạng thái của luật.
pub(crate) const ACTIVE: &str = "active";
pub(crate) const DISABLED: &str = "disabled";
pub(crate) const PENDING_APPROVAL: &str = "pending_approval";
pub(crate) const PAUSED: &str = "paused";

/// Dữ liệu để tạo luật (đã qua kiểm độ dài ở REST / MCP).
#[derive(Debug, Clone)]
pub(crate) struct NewRule {
    pub name: String,
    pub description: Option<String>,
    /// Nút của luật; `None`: nút duy nhất người tạo được quản lý luật (gốc khi có quyền toàn khách).
    pub org_id: Option<Uuid>,
    pub body: RawBody,
    /// `false`: lưu ở trạng thái tắt.
    pub enabled: bool,
}

/// Thay đổi cho một luật; trường `None` giữ nguyên.
#[derive(Debug, Clone, Default)]
pub(crate) struct RuleChanges {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub trigger: Option<Value>,
    pub conditions: Option<Value>,
    pub actions: Option<Value>,
}

/// Bộ lọc danh sách luật.
#[derive(Debug, Clone, Default)]
pub(crate) struct RuleFilter {
    pub status: Option<String>,
    pub trigger_type: Option<String>,
}

impl Automations {
    /// Tạo luật. Người gọi là AI agent thì luật chờ một người bật (`pending_approval`).
    pub(crate) async fn create_rule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewRule,
    ) -> Result<Rule, Error> {
        principal.require(&AUTOMATIONS_MANAGE)?;
        check_name(&input.name)?;
        let org = match input.org_id {
            Some(org) => OrgId::from_uuid(org),
            None => default_org(tenant, principal)?,
        };
        let body = self.validate(tenant, principal, &input.body).await?;
        let zone = self.zone(tenant, org, &body).await;
        let now = now();
        let agent = principal.actor_kind() == ActorKind::Agent;
        let status = match (agent, input.enabled) {
            (true, _) => PENDING_APPROVAL,
            (false, true) => ACTIVE,
            (false, false) => DISABLED,
        };
        let id = Uuid::now_v7();
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(&transaction, &AUTOMATIONS_MANAGE, OrgTarget::new(org, None))
            .await?;
        let (key, limit) = super::credential(principal)?;
        let rule = rule::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org.as_uuid()),
            name: Set(input.name.trim().to_owned()),
            description: Set(input.description.filter(|text| !text.trim().is_empty())),
            trigger_type: Set(body.trigger.kind().to_owned()),
            trigger_ref: Set(body.trigger.reference().map(str::to_owned)),
            trigger: Set(input.body.trigger.to_string()),
            conditions: Set(input.body.conditions.to_string()),
            actions: Set(input.body.actions.to_string()),
            status: Set(status.to_owned()),
            status_reason: Set(None),
            timezone: Set(zone_name(&zone)),
            run_as: Set(principal.user_id()),
            run_as_api_key: Set(key),
            run_as_limit: Set(limit),
            created_by: Set(principal.user_id()),
            created_by_kind: Set(principal.actor_kind().as_str().to_owned()),
            updated_by: Set(principal.user_id()),
            version: Set(1),
            next_run_at: Set(next_run(&body, status, &zone, now)),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await?;
        let event = events::RuleChanged(Change { before: None, after: Some(snapshot(&rule)) });
        self.outbox.record(&transaction, tenant, actor, events::rule_subject(id), &event).await?;
        transaction.commit().await?;
        tracing::info!(rule_id = %id, status, "automation rule created");
        Ok(rule)
    }

    /// Một trang luật trong phạm vi `automations.read` của người gọi, mới tạo trước (mặc định).
    pub(crate) async fn list_rules(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &RuleFilter,
        page: &PageRequest,
    ) -> Result<Page<Rule>, Error> {
        let scope = principal.scope_condition(&AUTOMATIONS_READ, Column::OrgId, None)?;
        let mut select = Entity::find().filter(scope).scoped(tenant);
        if let Some(status) = &filter.status {
            select = select.filter(Column::Status.eq(status.as_str()));
        }
        if let Some(kind) = &filter.trigger_type {
            select = select.filter(Column::TriggerType.eq(kind.as_str()));
        }
        fetch_page(select, Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Luật `id`. Ngoài phạm vi `automations.read` (hay của khách khác) báo như không có.
    pub(crate) async fn get_rule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Rule, Error> {
        let read = tenant.database().read();
        let rule = Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| rule_not_found(id))?;
        hidden(principal.require_at(read, &AUTOMATIONS_READ, target(&rule)).await, id)?;
        Ok(rule)
    }

    /// Sửa luật `id`. Thân luật đổi thì kiểm lại cả luật với quyền của người sửa — người sửa thành
    /// `run_as` (họ chịu trách nhiệm cho hành động mới). AI agent sửa thì luật lại chờ duyệt.
    pub(crate) async fn update_rule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        changes: RuleChanges,
        expected: Option<Timestamp>,
    ) -> Result<Rule, Error> {
        if let Some(name) = &changes.name {
            check_name(name)?;
        }
        let current = self.get_rule(tenant, principal, id).await?;
        let body_changed =
            changes.trigger.is_some() || changes.conditions.is_some() || changes.actions.is_some();
        let raw = RawBody {
            trigger: changes.trigger.unwrap_or_else(|| parse_json(&current.trigger)),
            conditions: changes.conditions.unwrap_or_else(|| parse_json(&current.conditions)),
            actions: changes.actions.unwrap_or_else(|| parse_json(&current.actions)),
        };
        let body = self.validate(tenant, principal, &raw).await?;
        let org = OrgId::from_uuid(current.org_id);
        let zone = self.zone(tenant, org, &body).await;
        let agent = principal.actor_kind() == ActorKind::Agent;
        let now = now();
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        // Khoá luật (D-181): hai lần sửa / bật / tắt cùng lúc nối đuôi nhau — bên sau thấy phiên
        // bản bên trước vừa ghi, không cùng ghi `version + 1` từ bản cũ.
        let before =
            autoxide_kernel::rows::lock_one(&transaction, Entity::find_by_id(id).scoped(tenant))
                .await?
                .ok_or_else(|| rule_not_found(id))?;
        principal.require_write_at(&transaction, &AUTOMATIONS_MANAGE, target(&before)).await?;
        let mut row = before.clone().into_active_model();
        if let Some(name) = changes.name {
            row.name = Set(name.trim().to_owned());
        }
        if let Some(description) = changes.description {
            row.description = Set(description.filter(|text| !text.trim().is_empty()));
        }
        let mut status = before.status.clone();
        if body_changed {
            row.trigger_type = Set(body.trigger.kind().to_owned());
            row.trigger_ref = Set(body.trigger.reference().map(str::to_owned));
            row.trigger = Set(raw.trigger.to_string());
            row.conditions = Set(raw.conditions.to_string());
            row.actions = Set(raw.actions.to_string());
            row.run_as = Set(principal.user_id());
            let (key, limit) = super::credential(principal)?;
            row.run_as_api_key = Set(key);
            row.run_as_limit = Set(limit);
            if agent {
                status = PENDING_APPROVAL.to_owned();
                row.status = Set(status.clone());
                row.status_reason = Set(None);
            }
        }
        row.timezone = Set(zone_name(&zone));
        row.next_run_at = Set(next_run(&body, &status, &zone, now));
        row.version = Set(before.version + 1);
        row.updated_by = Set(principal.user_id());
        row.updated_at = Set(now);
        let after = self.save(&transaction, tenant, row, expected).await?;
        let change = Change { before: Some(snapshot(&before)), after: Some(snapshot(&after)) };
        self.outbox
            .record(
                &transaction,
                tenant,
                actor,
                events::rule_subject(id),
                &events::RuleChanged(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Bật luật: kiểm lại cả luật với quyền của người bật, người bật thành `run_as`. AI agent không
    /// bật được (luật agent tạo phải có người duyệt).
    pub(crate) async fn enable_rule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        expected: Option<Timestamp>,
    ) -> Result<Rule, Error> {
        if principal.actor_kind() == ActorKind::Agent {
            return Err(human_required());
        }
        let current = self.get_rule(tenant, principal, id).await?;
        let raw = RawBody {
            trigger: parse_json(&current.trigger),
            conditions: parse_json(&current.conditions),
            actions: parse_json(&current.actions),
        };
        let body = self.validate(tenant, principal, &raw).await?;
        let zone = self.zone(tenant, OrgId::from_uuid(current.org_id), &body).await;
        let checked = Some((&body, zone, current.version));
        self.set_status(tenant, principal, id, (ACTIVE, checked), expected).await
    }

    /// Tắt luật (lần chạy đang dở vẫn chạy nốt).
    pub(crate) async fn disable_rule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        expected: Option<Timestamp>,
    ) -> Result<Rule, Error> {
        self.get_rule(tenant, principal, id).await?;
        self.set_status(tenant, principal, id, (DISABLED, None), expected).await
    }

    /// Đổi trạng thái của luật `id` bằng tay; `body` (khi bật): luật đã kiểm lại, múi giờ của nó và
    /// phiên bản đã kiểm.
    async fn set_status(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        (status, body): (&str, Option<(&RuleBody, TimeZone, i32)>),
        expected: Option<Timestamp>,
    ) -> Result<Rule, Error> {
        let now = now();
        let transaction = tenant.database().begin_write().await?;
        // Khoá luật (D-181): hai lần sửa / bật / tắt cùng lúc nối đuôi nhau — bên sau thấy phiên
        // bản bên trước vừa ghi, không cùng ghi `version + 1` từ bản cũ.
        let before =
            autoxide_kernel::rows::lock_one(&transaction, Entity::find_by_id(id).scoped(tenant))
                .await?
                .ok_or_else(|| rule_not_found(id))?;
        principal.require_write_at(&transaction, &AUTOMATIONS_MANAGE, target(&before)).await?;
        // Bật luật đúng bản đã kiểm lại với quyền người bật: luật bị sửa chen giữa (vd agent sửa,
        // luật chuyển sang chờ duyệt) thì từ chối — không bật bản chưa ai kiểm dưới quyền người này.
        if let Some((_, _, checked)) = body
            && before.version != checked
        {
            return Err(autoxide_kernel::version::changed());
        }
        let mut row = before.clone().into_active_model();
        row.status = Set(status.to_owned());
        row.status_reason = Set(None);
        if let Some((body, zone, _)) = body {
            row.run_as = Set(principal.user_id());
            let (key, limit) = super::credential(principal)?;
            row.run_as_api_key = Set(key);
            row.run_as_limit = Set(limit);
            row.timezone = Set(zone_name(&zone));
            row.next_run_at = Set(next_run(body, status, &zone, now));
        } else {
            row.next_run_at = Set(None);
        }
        row.updated_by = Set(principal.user_id());
        row.updated_at = Set(now);
        let after = self.save(&transaction, tenant, row, expected).await?;
        let change = Change { before: Some(snapshot(&before)), after: Some(snapshot(&after)) };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::rule_subject(id),
                &events::RuleChanged(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Xoá luật. Nhật ký các lần chạy giữ tới hết hạn lưu.
    pub(crate) async fn delete_rule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        expected: Option<Timestamp>,
    ) -> Result<(), Error> {
        self.get_rule(tenant, principal, id).await?;
        let transaction = tenant.database().begin_write().await?;
        // Khoá luật (D-181): hai lần sửa / bật / tắt cùng lúc nối đuôi nhau — bên sau thấy phiên
        // bản bên trước vừa ghi, không cùng ghi `version + 1` từ bản cũ.
        let before =
            autoxide_kernel::rows::lock_one(&transaction, Entity::find_by_id(id).scoped(tenant))
                .await?
                .ok_or_else(|| rule_not_found(id))?;
        principal.require_write_at(&transaction, &AUTOMATIONS_MANAGE, target(&before)).await?;
        let mut delete = Entity::delete_many().filter(Column::Id.eq(id)).scoped(tenant);
        if let Some(expected) = expected {
            delete = delete.filter(Column::UpdatedAt.eq(expected));
        }
        if delete.exec(&transaction).await?.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(autoxide_kernel::version::changed());
        }
        let change = Change { before: Some(snapshot(&before)), after: None };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::rule_subject(id),
                &events::RuleChanged(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Ghi `row`; `expected`: chỉ ghi khi luật còn đúng phiên bản đó (D-084).
    async fn save<C: sea_orm::ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        row: rule::ActiveModel,
        expected: Option<Timestamp>,
    ) -> Result<Rule, Error> {
        let mut update = Entity::update(row).validate()?.scoped(tenant);
        if let Some(expected) = expected {
            update = update.filter(Column::UpdatedAt.eq(expected));
        }
        update.exec(connection).await.map_err(|error| match error {
            sea_orm::DbErr::RecordNotUpdated => autoxide_kernel::version::changed(),
            error => error.into(),
        })
    }

    /// Múi giờ của lịch: khai trong kích hoạt, không thì của nút (`organizations.timezone`, kế
    /// thừa theo cây), không đọc được thì UTC.
    pub(crate) async fn zone(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        body: &RuleBody,
    ) -> TimeZone {
        if let Some(zone) = &body.timezone {
            return zone.clone();
        }
        let Some(orgs) = &self.orgs else { return TimeZone::UTC };
        match orgs.setting(tenant, org, "organizations.timezone").await {
            Ok(found) => found
                .and_then(|setting| setting.value)
                .and_then(|value| value.as_str().and_then(time::zone))
                .unwrap_or(TimeZone::UTC),
            Err(error) => {
                tracing::warn!(%error, "cannot read the time zone of a unit; using UTC");
                TimeZone::UTC
            }
        }
    }
}

/// Lần chạy theo lịch kế tiếp của luật đang bật.
/// Tên IANA của múi giờ để lưu (`Asia/Ho_Chi_Minh`); múi nào ở đây cũng lấy theo tên nên luôn có tên.
fn zone_name(zone: &TimeZone) -> String {
    zone.iana_name().unwrap_or("UTC").to_owned()
}

fn next_run(body: &RuleBody, status: &str, zone: &TimeZone, now: Timestamp) -> Option<Timestamp> {
    if status != ACTIVE {
        return None;
    }
    body.trigger.next_run(now, zone)
}

pub(crate) fn parse_json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

fn check_name(name: &str) -> Result<(), Error> {
    let length = name.trim().chars().count();
    if (1..=200).contains(&length) {
        return Ok(());
    }
    Err(Error::validation(vec![
        FieldError::new("name", "length", "must have from {min} to {max} characters")
            .with_param("min", 1)
            .with_param("max", 200),
    ]))
}

/// Đích kiểm quyền của luật: nút của nó.
pub(crate) fn target(rule: &Rule) -> OrgTarget {
    OrgTarget::new(OrgId::from_uuid(rule.org_id), None)
}

/// Luật ngoài phạm vi đọc báo như không có.
pub(crate) fn hidden(result: Result<(), Error>, id: Uuid) -> Result<(), Error> {
    result.map_err(
        |error| if error.code() == "org_out_of_scope" { rule_not_found(id) } else { error },
    )
}

/// Nút của luật mới khi người tạo không nêu: gốc nếu họ có `automations.manage` cho cả khách; nút
/// duy nhất nếu mọi grant ở một nút; nhiều nút thì phải chọn (`automations.org_required`).
fn default_org(tenant: &TenantContext, principal: &Principal) -> Result<OrgId, Error> {
    let grants = principal.grants(&AUTOMATIONS_MANAGE);
    if grants.is_empty() || grants.iter().any(|grant| grant.scope == Scope::Tenant) {
        return Ok(root_of(tenant.id()));
    }
    principal.home_org(tenant.id(), &AUTOMATIONS_MANAGE).ok_or_else(|| {
        Error::validation(vec![FieldError::new(
            "org_id",
            "automations.org_required",
            "is required: you manage rules in several units, so name one",
        )])
    })
}

/// Bản chụp của luật cho event (audit "trước / sau").
pub(crate) fn snapshot(rule: &Rule) -> Value {
    json!({
        "id": rule.id,
        "org_id": rule.org_id,
        "name": rule.name,
        "status": rule.status,
        "status_reason": rule.status_reason,
        "trigger": parse_json(&rule.trigger),
        "conditions": parse_json(&rule.conditions),
        "actions": parse_json(&rule.actions),
        "run_as": rule.run_as,
        "version": rule.version,
    })
}
