//! Nhóm nhân viên, quy tắc tự phân công và chia vòng (bước 53 mục 2).
//!
//! - Nhóm thuộc một nút; phủ phiếu của nút đó và nhánh dưới.
//! - Quy tắc (theo loại / kênh của phiếu → nhóm) là dữ liệu của nút, kế thừa theo cây: phiếu mới khớp
//!   quy tắc của nút gần nhất trước, trong một nút theo `position`.
//! - Chia vòng: trong nhóm, người đang bật "nhận việc" lâu chưa được giao nhất nhận phiếu. Không bao
//!   giờ giao cho người không có `support.tickets.reply` ở nút của phiếu (hỏi bộ xác thực
//!   `principal_of` — quyền có thể đã đổi từ lúc thêm vào nhóm).

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, Set, SqlErr,
};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{routing_rule, team, team_member, ticket};
use crate::events::{self, Change};
use crate::permissions::{SETTINGS_MANAGE, TICKETS_REPLY};
use crate::service::{SupportService, Ticket, finish, now, org_exists, required_text};
use crate::status::Channel;

pub(crate) type Team = team::Model;
pub(crate) type Member = team_member::Model;
pub(crate) type Rule = routing_rule::Model;

/// Độ dài tối đa của tên nhóm.
pub(crate) const TEAM_NAME_MAX: usize = 100;

/// Một quy tắc cần tạo.
#[derive(Debug, Clone)]
pub(crate) struct NewRule {
    pub org_id: Uuid,
    pub category: Option<String>,
    pub channel: Option<Channel>,
    pub team_id: Uuid,
    pub position: i32,
}

impl SupportService {
    /// Người dùng `user` làm được phiếu ở nút `org` không: còn hoạt động và có
    /// `support.tickets.reply` phủ nút đó. Không có bộ xác thực (cài thiếu `auth`) → không.
    pub(crate) async fn can_work(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        user: Uuid,
    ) -> Result<bool, Error> {
        let Some(authenticator) = &self.parts.authenticator else {
            return Ok(false);
        };
        let Some(them) = authenticator.principal_of(tenant, user).await? else {
            return Ok(false);
        };
        let target = OrgTarget::new(org, Some(user));
        Ok(them.require_at(tenant.database().read(), &TICKETS_REPLY, target).await.is_ok())
    }

    /// Người được giao bằng tay: chính người gọi, hay người làm được phiếu ở nút của phiếu.
    pub(crate) async fn check_assignee(
        &self,
        tenant: &TenantContext,
        ticket: &Ticket,
        principal: &Principal,
        assignee: Uuid,
    ) -> Result<(), Error> {
        if assignee == principal.user_id()
            || self.can_work(tenant, OrgId::from_uuid(ticket.org_id), assignee).await?
        {
            return Ok(());
        }
        Err(crate::tickets::assignee_out_of_scope(assignee))
    }

    /// Nhóm phải là nhóm của nút của phiếu hay một tổ tiên (nhóm phủ nhánh của nút nó).
    pub(crate) async fn check_team(
        &self,
        tenant: &TenantContext,
        ticket: &Ticket,
        team: Uuid,
    ) -> Result<(), Error> {
        let read = tenant.database().read();
        let row = team::Entity::find_by_id(team).scoped(tenant).one(read).await?;
        let covers = match row {
            Some(row) => {
                OrgTree::is_within(
                    read,
                    tenant.id(),
                    OrgId::from_uuid(ticket.org_id),
                    &[OrgId::from_uuid(row.org_id)],
                )
                .await?
                    || row.org_id == ticket.org_id
            }
            None => false,
        };
        if covers {
            Ok(())
        } else {
            Err(Error::validation(vec![FieldError::new(
                "team_id",
                "unknown",
                "is not a support team of this unit",
            )]))
        }
    }

    /// Phiếu mới chưa giao: tìm quy tắc khớp, giao cho nhóm và (chia vòng) một thành viên làm được
    /// phiếu. Không có quy tắc khớp → để nguyên. Gọi sau khi phiếu đã ghi (ngoài transaction tạo).
    pub(crate) async fn auto_assign(
        &self,
        tenant: &TenantContext,
        ticket: &Ticket,
    ) -> Result<(), Error> {
        let read = tenant.database().read();
        let org = OrgId::from_uuid(ticket.org_id);
        let mut lineage = OrgTree::ancestors(read, tenant.id(), org).await?;
        if lineage.is_empty() {
            lineage.push(org);
        }
        let rules = routing_rule::Entity::find()
            .filter(routing_rule::Column::OrgId.is_in(lineage.iter().map(OrgId::as_uuid)))
            .scoped(tenant)
            .order_by_asc(routing_rule::Column::Position)
            .order_by_asc(routing_rule::Column::Id)
            .all(read)
            .await?;
        // Nút gần nhất trước (thứ tự của `lineage`), rồi theo `position`.
        let rule = lineage.iter().find_map(|node| {
            rules.iter().find(|rule| {
                rule.org_id == node.as_uuid()
                    && rule.category.as_deref().is_none_or(|category| category == ticket.category)
                    && rule.channel.as_deref().is_none_or(|channel| channel == ticket.channel)
            })
        });
        let Some(rule) = rule else {
            return Ok(());
        };
        // Ứng viên: thành viên đang nhận việc, lâu chưa được giao nhất trước; chỉ người làm được
        // phiếu ở nút của phiếu (hỏi ngoài transaction ghi).
        let members = team_member::Entity::find()
            .filter(team_member::Column::TeamId.eq(rule.team_id))
            .filter(team_member::Column::Accepting.eq(true))
            .scoped(tenant)
            .all(read)
            .await?;
        let mut eligible = Vec::new();
        for member in &members {
            if self.can_work(tenant, org, member.user_id).await? {
                eligible.push(member.user_id);
            }
        }
        let transaction = tenant.database().begin_write().await?;
        // Khoá dòng nhóm: hai phiếu cùng lúc không cùng chọn một người (Postgres xếp hàng ở đây;
        // SQLite vốn một bên ghi).
        team::Entity::update_many()
            .col_expr(team::Column::UpdatedAt, Expr::col(team::Column::UpdatedAt))
            .filter(team::Column::Id.eq(rule.team_id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let before = ticket::Entity::find_by_id(ticket.id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .filter(|row| row.assignee_user_id.is_none() && row.team_id.is_none());
        let Some(before) = before else {
            transaction.rollback().await?;
            return Ok(());
        };
        let chosen = pick(&transaction, tenant, rule.team_id, &eligible).await?;
        let at = now();
        let mut row = before.clone().into_active_model();
        row.team_id = Set(Some(rule.team_id));
        row.assignee_user_id = Set(chosen);
        row.updated_at = Set(at);
        let after =
            ticket::Entity::update(row).validate()?.scoped(tenant).exec(&transaction).await?;
        if let Some(user) = chosen {
            team_member::Entity::update_many()
                .col_expr(team_member::Column::LastAssignedAt, Expr::value(at))
                .filter(team_member::Column::TeamId.eq(rule.team_id))
                .filter(team_member::Column::UserId.eq(user))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        self.system_line(&transaction, tenant, ticket.id, "assigned", at).await?;
        self.record_changes(&transaction, tenant, Actor::system(), &before, &after, true).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Một trang nhóm trong phạm vi `support.settings.manage` (hay `support.tickets.reply`: người
    /// làm phiếu cần biết nhóm để giao).
    pub(crate) async fn list_teams(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<Team>, Error> {
        let permission =
            if principal.has(&SETTINGS_MANAGE) { &SETTINGS_MANAGE } else { &TICKETS_REPLY };
        let scope = principal.scope_condition(permission, team::Column::OrgId, None)?;
        let select = team::Entity::find().filter(scope).scoped(tenant);
        fetch_page(select, team::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Nhóm `id` và thành viên.
    pub(crate) async fn team(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(Team, Vec<Member>), Error> {
        let read = tenant.database().read();
        let row = team::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| team_not_found(id))?;
        let permission =
            if principal.has(&SETTINGS_MANAGE) { &SETTINGS_MANAGE } else { &TICKETS_REPLY };
        let target = OrgTarget::new(OrgId::from_uuid(row.org_id), None);
        principal.require_at(read, permission, target).await.map_err(|error| {
            if error.code() == "org_out_of_scope" { team_not_found(id) } else { error }
        })?;
        let members = team_member::Entity::find()
            .filter(team_member::Column::TeamId.eq(id))
            .scoped(tenant)
            .order_by_asc(team_member::Column::UserId)
            .all(read)
            .await?;
        Ok((row, members))
    }

    /// Tạo nhóm `name` ở nút `org_id` (`support.settings.manage` ở nút đó).
    pub(crate) async fn create_team(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org_id: Uuid,
        name: &str,
    ) -> Result<Team, Error> {
        let mut problems = Vec::new();
        let name = required_text("name", name, TEAM_NAME_MAX, &mut problems);
        let org = OrgId::from_uuid(org_id);
        if !org_exists(tenant, org).await? {
            problems.push(FieldError::new("org_id", "unknown", "is not a unit of this shop"));
        }
        finish(problems)?;
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(&transaction, &SETTINGS_MANAGE, OrgTarget::new(org, None))
            .await?;
        let at = now();
        let row = team::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org_id),
            name: Set(name.clone()),
            created_at: Set(at),
            updated_at: Set(at),
        }
        .insert(&transaction)
        .await
        .map_err(|error| name_taken(error, &name))?;
        self.settings_event(
            &transaction,
            tenant,
            principal,
            json!({"team": row.id, "name": row.name}),
        )
        .await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Xoá nhóm (cùng thành viên và quy tắc trỏ tới nó); phiếu đang thuộc nhóm bỏ nhóm.
    pub(crate) async fn delete_team(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let row = team::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| team_not_found(id))?;
        let target = OrgTarget::new(OrgId::from_uuid(row.org_id), None);
        principal.require_write_at(&transaction, &SETTINGS_MANAGE, target).await?;
        team_member::Entity::delete_many()
            .filter(team_member::Column::TeamId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        routing_rule::Entity::delete_many()
            .filter(routing_rule::Column::TeamId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        ticket::Entity::update_many()
            .col_expr(ticket::Column::TeamId, Expr::value(Option::<Uuid>::None))
            .filter(ticket::Column::TeamId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        team::Entity::delete_by_id(id).scoped(tenant).exec(&transaction).await?;
        self.settings_event(&transaction, tenant, principal, json!({"team": id, "deleted": true}))
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Thêm / đổi thành viên `user` của nhóm. Người quản lý cài đặt (`support.settings.manage`) thêm
    /// được người làm được phiếu ở nút của nhóm; chính thành viên tự bật / tắt "nhận việc".
    pub(crate) async fn set_member(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        team_id: Uuid,
        user: Uuid,
        accepting: bool,
    ) -> Result<Member, Error> {
        let (team, members) = self.team(tenant, principal, team_id).await?;
        let org = OrgId::from_uuid(team.org_id);
        let existing = members.iter().find(|member| member.user_id == user).cloned();
        let manager = principal
            .require_at(tenant.database().read(), &SETTINGS_MANAGE, OrgTarget::new(org, None))
            .await;
        // Chính thành viên tự bật / tắt "nhận việc" không cần quyền cài đặt.
        let self_toggle = user == principal.user_id() && existing.is_some();
        if !self_toggle {
            manager?;
        }
        if existing.is_none() && !self.can_work(tenant, org, user).await? {
            return Err(crate::tickets::assignee_out_of_scope(user));
        }
        let transaction = tenant.database().begin_write().await?;
        let at = now();
        let row = match existing {
            Some(existing) => {
                let mut row = existing.into_active_model();
                row.accepting = Set(accepting);
                team_member::Entity::update(row)
                    .validate()?
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?
            }
            None => {
                team_member::ActiveModel {
                    tenant_id: Set(tenant.id().as_uuid()),
                    team_id: Set(team_id),
                    user_id: Set(user),
                    accepting: Set(accepting),
                    last_assigned_at: Set(None),
                    created_at: Set(at),
                }
                .insert(&transaction)
                .await?
            }
        };
        self.settings_event(
            &transaction,
            tenant,
            principal,
            json!({"team": team_id, "member": user, "accepting": accepting}),
        )
        .await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Bỏ thành viên `user` khỏi nhóm.
    pub(crate) async fn remove_member(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        team_id: Uuid,
        user: Uuid,
    ) -> Result<(), Error> {
        let (team, _) = self.team(tenant, principal, team_id).await?;
        let transaction = tenant.database().begin_write().await?;
        let target = OrgTarget::new(OrgId::from_uuid(team.org_id), None);
        principal.require_write_at(&transaction, &SETTINGS_MANAGE, target).await?;
        team_member::Entity::delete_many()
            .filter(team_member::Column::TeamId.eq(team_id))
            .filter(team_member::Column::UserId.eq(user))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        self.settings_event(
            &transaction,
            tenant,
            principal,
            json!({"team": team_id, "removed": user}),
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Quy tắc của nút `org_id` (không gồm quy tắc kế thừa từ nút trên), theo `position`.
    pub(crate) async fn rules(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org_id: Uuid,
    ) -> Result<Vec<Rule>, Error> {
        let read = tenant.database().read();
        let target = OrgTarget::new(OrgId::from_uuid(org_id), None);
        principal.require_at(read, &SETTINGS_MANAGE, target).await?;
        Ok(routing_rule::Entity::find()
            .filter(routing_rule::Column::OrgId.eq(org_id))
            .scoped(tenant)
            .order_by_asc(routing_rule::Column::Position)
            .order_by_asc(routing_rule::Column::Id)
            .all(read)
            .await?)
    }

    /// Thêm quy tắc; nhóm phải phủ nút của quy tắc, loại phải có.
    pub(crate) async fn create_rule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewRule,
    ) -> Result<Rule, Error> {
        let mut problems = Vec::new();
        let category = input.category.map(|category| category.trim().to_owned());
        if let Some(category) = &category
            && !self.category_known(tenant, category).await?
        {
            problems.push(crate::tickets::category_unknown());
        }
        let read = tenant.database().read();
        let team = team::Entity::find_by_id(input.team_id).scoped(tenant).one(read).await?;
        let org = OrgId::from_uuid(input.org_id);
        let covers = match &team {
            Some(team) => {
                team.org_id == input.org_id
                    || OrgTree::is_within(read, tenant.id(), org, &[OrgId::from_uuid(team.org_id)])
                        .await?
            }
            None => false,
        };
        if !covers {
            problems.push(FieldError::new(
                "team_id",
                "unknown",
                "is not a support team of this unit",
            ));
        }
        finish(problems)?;
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(&transaction, &SETTINGS_MANAGE, OrgTarget::new(org, None))
            .await?;
        let row = routing_rule::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(input.org_id),
            category: Set(category),
            channel: Set(input.channel.map(|channel| channel.as_str().to_owned())),
            team_id: Set(input.team_id),
            position: Set(input.position),
            created_at: Set(now()),
        }
        .insert(&transaction)
        .await?;
        self.settings_event(
            &transaction,
            tenant,
            principal,
            json!({"rule": row.id, "team": row.team_id}),
        )
        .await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Xoá quy tắc `id`.
    pub(crate) async fn delete_rule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let row = routing_rule::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| rule_not_found(id))?;
        let target = OrgTarget::new(OrgId::from_uuid(row.org_id), None);
        principal.require_write_at(&transaction, &SETTINGS_MANAGE, target).await?;
        routing_rule::Entity::delete_by_id(id).scoped(tenant).exec(&transaction).await?;
        self.settings_event(&transaction, tenant, principal, json!({"rule": id, "deleted": true}))
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Event `support.settings_changed` cho audit.
    pub(crate) async fn settings_event<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        principal: &Principal,
        after: serde_json::Value,
    ) -> Result<(), Error> {
        let event = events::SettingsChanged(Change { before: None, after: Some(after) });
        self.parts.outbox.record(connection, tenant, Actor::from(principal), None, &event).await
    }
}

/// Người nhận phiếu tiếp theo của nhóm `team`: trong `eligible`, người lâu chưa được giao nhất
/// (chưa từng được giao đứng đầu), hoà thì theo ID.
async fn pick<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    team: Uuid,
    eligible: &[Uuid],
) -> Result<Option<Uuid>, Error> {
    if eligible.is_empty() {
        return Ok(None);
    }
    let mut members = team_member::Entity::find()
        .filter(team_member::Column::TeamId.eq(team))
        .filter(team_member::Column::Accepting.eq(true))
        .filter(team_member::Column::UserId.is_in(eligible.to_vec()))
        .scoped(tenant)
        .all(connection)
        .await?;
    // Sắp trong Rust: `NULLS FIRST` viết khác nhau giữa SQLite và Postgres.
    members.sort_by_key(|member| (member.last_assigned_at, member.user_id));
    Ok(members.first().map(|member| member.user_id))
}

pub(crate) fn team_not_found(id: Uuid) -> Error {
    Error::not_found("support.team_not_found", "There is no support team `{id}` you can see.")
        .with_param("id", id)
}

fn rule_not_found(id: Uuid) -> Error {
    Error::not_found("support.rule_not_found", "There is no routing rule `{id}` you can see.")
        .with_param("id", id)
}

fn name_taken(error: sea_orm::DbErr, name: &str) -> Error {
    match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => Error::conflict(
            "support.team_name_taken",
            "This unit already has a support team named `{name}`.",
        )
        .with_param("name", name),
        _ => Error::from(error),
    }
}
