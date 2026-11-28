//! Cài đặt hỗ trợ theo nút: nhóm nhân viên, thành viên, quy tắc tự phân công
//! (`support.settings.manage`).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::permissions::{SETTINGS_MANAGE, TICKETS_REPLY};
use crate::service::SupportService;
use crate::status::Channel;
use crate::teams::{Member, NewRule, Rule, Team};

/// A support team of an organization unit; it covers tickets of the unit and the units below.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportTeam)]
pub(crate) struct TeamResponse {
    id: Uuid,
    org_id: Uuid,
    name: String,
    created_at: Timestamp,
}

impl From<Team> for TeamResponse {
    fn from(team: Team) -> Self {
        TeamResponse {
            id: team.id,
            org_id: team.org_id,
            name: team.name,
            created_at: team.created_at,
        }
    }
}

/// A member of a support team.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportTeamMember)]
pub(crate) struct MemberResponse {
    user_id: Uuid,
    /// Takes new tickets in round-robin assignment.
    accepting: bool,
    last_assigned_at: Option<Timestamp>,
}

impl From<Member> for MemberResponse {
    fn from(member: Member) -> Self {
        MemberResponse {
            user_id: member.user_id,
            accepting: member.accepting,
            last_assigned_at: member.last_assigned_at,
        }
    }
}

/// A team with its members.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportTeamDetail)]
pub(crate) struct TeamDetail {
    #[serde(flatten)]
    team: TeamResponse,
    members: Vec<MemberResponse>,
}

/// List support teams in your scope.
///
/// Requires `support.settings.manage` or `support.tickets.reply`; you see the teams of the units in
/// your scope.
#[utoipa::path(
    get,
    path = "/teams",
    extensions(("x-permissions" = json!(["support.tickets.reply", "support.settings.manage"]))),
    operation_id = "list_support_teams",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of teams.", body = Page<TeamResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.tickets.reply`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_teams(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<TeamResponse>, Error> {
    if !principal.has(&SETTINGS_MANAGE) {
        principal.require(&TICKETS_REPLY)?;
    }
    let page = service.list_teams(&tenant, &principal, &PageRequest::try_from(page)?).await?;
    Ok(page.map(TeamResponse::from))
}

/// A new team.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = CreateSupportTeamRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateTeamRequest {
    /// Unit of the team.
    org_id: Uuid,
    /// 1 to 100 characters; unique in the unit.
    #[schema(example = "Orders")]
    name: String,
}

/// Create a support team.
///
/// Requires `support.settings.manage` at the unit.
#[utoipa::path(
    post,
    path = "/teams",
    extensions(("x-permissions" = json!(["support.settings.manage"]))),
    operation_id = "create_support_team",
    security(("bearer" = [])),
    request_body = CreateTeamRequest,
    responses(
        (status = 201, description = "Created.", body = Data<TeamResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.settings.manage` (`permission_denied`), the unit is outside your scope (`org_out_of_scope`) or not active (`org_frozen`).", body = Problem),
        (status = 409, description = "The unit has a team with this name; code `support.team_name_taken`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_team(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateTeamRequest>,
) -> Result<Created<TeamResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let team = service.create_team(&tenant, &principal, input.org_id, &input.name).await?;
    Ok(Created::new(format!("/support/teams/{}", team.id), team.into()))
}

/// Get a support team with its members.
///
/// Requires `support.settings.manage` or `support.tickets.reply` covering the team's unit.
#[utoipa::path(
    get,
    path = "/teams/{id}",
    extensions(("x-permissions" = json!(["support.tickets.reply", "support.settings.manage"]))),
    operation_id = "get_support_team",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the team.")),
    responses(
        (status = 200, description = "The team.", body = Data<TeamDetail>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.tickets.reply`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No team with this id in your scope; code `support.team_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_team(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<TeamDetail>, Error> {
    if !principal.has(&SETTINGS_MANAGE) {
        principal.require(&TICKETS_REPLY)?;
    }
    let (team, members) = service.team(&tenant, &principal, id).await?;
    let data = TeamDetail {
        team: team.into(),
        members: members.into_iter().map(MemberResponse::from).collect(),
    };
    Ok(Data { data })
}

/// Delete a support team.
///
/// Requires `support.settings.manage` at the team's unit. Its members and routing rules go too;
/// tickets of the team keep their assignee but lose the team.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    delete,
    path = "/teams/{id}",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["support.settings.manage"]))),
    operation_id = "delete_support_team",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the team.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.settings.manage` (`permission_denied`), or it does not cover the team (`org_out_of_scope`, `org_frozen`).", body = Problem),
        (status = 404, description = "No team with this id; code `support.team_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_team(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    service.delete_team(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A member's setting.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = SetSupportTeamMemberRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemberRequest {
    /// Takes new tickets in round-robin assignment.
    accepting: bool,
}

/// Add a member to a support team, or change whether they take new tickets.
///
/// Requires `support.settings.manage` at the team's unit; a member may also switch `accepting` for
/// themselves. A new member must be able to answer tickets of the unit
/// (`support.assignee_out_of_scope` otherwise).
#[utoipa::path(
    put,
    path = "/teams/{id}/members/{user_id}",
    extensions(("x-permissions" = json!(["support.settings.manage", "support.tickets.reply"]))),
    operation_id = "set_support_team_member",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the team."), ("user_id" = Uuid, Path, description = "The member.")),
    request_body = MemberRequest,
    responses(
        (status = 200, description = "The member.", body = Data<MemberResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.settings.manage` (`permission_denied`; a member with `support.tickets.reply` may only switch their own `accepting`), or it does not cover the team (`org_out_of_scope`).", body = Problem),
        (status = 404, description = "No team with this id in your scope; code `support.team_not_found`.", body = Problem),
        (status = 422, description = "The user cannot answer tickets of this unit; code `support.assignee_out_of_scope`.", body = Problem),
    )
)]
pub(crate) async fn set_member(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, user)): Path<(Uuid, Uuid)>,
    ValidJson(input): ValidJson<MemberRequest>,
) -> Result<Data<MemberResponse>, Error> {
    if !principal.has(&SETTINGS_MANAGE) {
        principal.require(&TICKETS_REPLY)?;
    }
    let member = service.set_member(&tenant, &principal, id, user, input.accepting).await?;
    Ok(Data { data: member.into() })
}

/// Remove a member from a support team.
///
/// Requires `support.settings.manage` at the team's unit.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    delete,
    path = "/teams/{id}/members/{user_id}",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["support.settings.manage"]))),
    operation_id = "remove_support_team_member",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the team."), ("user_id" = Uuid, Path, description = "The member.")),
    responses(
        (status = 204, description = "Removed (or was not a member)."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.settings.manage` (`permission_denied`), or it does not cover the team (`org_out_of_scope`, `org_frozen`).", body = Problem),
        (status = 404, description = "No team with this id in your scope; code `support.team_not_found`.", body = Problem),
    )
)]
pub(crate) async fn remove_member(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, user)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    service.remove_member(&tenant, &principal, id, user).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A routing rule: new tickets of the unit (and the units below) that match go to the team.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportRoutingRule)]
pub(crate) struct RuleResponse {
    id: Uuid,
    org_id: Uuid,
    /// Only tickets of this category; empty: any.
    category: Option<String>,
    /// Only tickets from this channel; empty: any.
    channel: Option<Channel>,
    team_id: Uuid,
    /// Rules of one unit are tried in this order; rules of nearer units first.
    position: i32,
}

impl From<Rule> for RuleResponse {
    fn from(rule: Rule) -> Self {
        RuleResponse {
            id: rule.id,
            org_id: rule.org_id,
            category: rule.category,
            channel: rule.channel.as_deref().and_then(Channel::parse),
            team_id: rule.team_id,
            position: rule.position,
        }
    }
}

/// The unit whose rules to list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct RuleQuery {
    /// The unit (organization node) whose routing rules to list.
    org_id: Uuid,
}

/// List the routing rules of a unit.
///
/// Requires `support.settings.manage` at the unit. Rules of the units above apply too when none of
/// the unit's own rules match. In the order they are tried (`position`, then creation) unless
/// `order=desc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/routing-rules",
    extensions(("x-permissions" = json!(["support.settings.manage"]))),
    operation_id = "list_support_routing_rules",
    security(("bearer" = [])),
    params(RuleQuery, NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the rules, in the order they are tried.", body = Page<RuleResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.settings.manage` (`permission_denied`), or it does not cover the unit (`org_out_of_scope`).", body = Problem),
        (status = 422, description = "`org_id` is missing or a paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_rules(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<RuleQuery>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<RuleResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let page = PageRequest::try_from(page)?;
    let rules = service.rules(&tenant, &principal, query.org_id, &page).await?;
    Ok(rules.map(RuleResponse::from))
}

/// A new routing rule.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = CreateSupportRoutingRuleRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateRuleRequest {
    org_id: Uuid,
    /// Only tickets of this category; omitted: any.
    category: Option<String>,
    /// Only tickets from this channel; omitted: any.
    channel: Option<Channel>,
    /// A team of the unit or of a unit above it.
    team_id: Uuid,
    /// Order among the unit's rules (lower first).
    #[serde(default)]
    position: i32,
}

/// Add a routing rule.
///
/// Requires `support.settings.manage` at the unit. A new ticket goes to the team of the first
/// matching rule, and round-robin to a team member who takes tickets and can answer them.
#[utoipa::path(
    post,
    path = "/routing-rules",
    extensions(("x-permissions" = json!(["support.settings.manage"]))),
    operation_id = "create_support_routing_rule",
    security(("bearer" = [])),
    request_body = CreateRuleRequest,
    responses(
        (status = 201, description = "Created.", body = Data<RuleResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.settings.manage` (`permission_denied`), the unit is outside your scope (`org_out_of_scope`) or not active (`org_frozen`).", body = Problem),
        (status = 422, description = "Unknown category or team; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_rule(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateRuleRequest>,
) -> Result<Created<RuleResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let rule = NewRule {
        org_id: input.org_id,
        category: input.category,
        channel: input.channel,
        team_id: input.team_id,
        position: input.position,
    };
    let rule = service.create_rule(&tenant, &principal, rule).await?;
    Ok(Created::new(format!("/support/routing-rules/{}", rule.id), rule.into()))
}

/// Delete a routing rule.
///
/// Requires `support.settings.manage` at the rule's unit.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    delete,
    path = "/routing-rules/{id}",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["support.settings.manage"]))),
    operation_id = "delete_support_routing_rule",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the rule.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.settings.manage` (`permission_denied`), or it does not cover the rule (`org_out_of_scope`, `org_frozen`).", body = Problem),
        (status = 404, description = "No rule with this id; code `support.rule_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_rule(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    service.delete_rule(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
