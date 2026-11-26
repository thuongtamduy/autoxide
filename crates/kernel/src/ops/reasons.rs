//! Per-tenant reason lists (step 51 P18, D-223): reasons for cancelling an order, refunding, hiding a review…
//!
//! A module declares a list and its default codes with [`Module::reason_lists`](crate::module::Module) — name
//! `<module>.<action>`, `snake_case` code, English label (translated through the module's `text:`; `xtask`
//! checks that translations exist). The table `kernel_reason_codes` (tenant level) holds only **what the
//! tenant changed**: its own labels, enabled / disabled, order of default codes, and codes the tenant added.
//! Reading merges the default codes with the tenant's part — new default codes from an upgrade appear
//! immediately, no sync needed, and no code is ever deleted.
//!
//! - `GET /api/v1/kernel/reasons/{list}?include_inactive=` — whoever can read the module that owns the
//!   list (`<module>.read`) or has `kernel.reasons.manage`;
//! - `PUT /api/v1/kernel/reasons/{list}/{code}` (`kernel.reasons.manage`) — label, enabled / disabled,
//!   order; a code that does not exist yet is a tenant's own code. The `legal` code cannot be disabled
//!   (`kernel.reason_legal`).
//!
//! A module's service validates the code a user submits with [`ReasonCodes::ensure`]. Data rows store the code,
//! not the label: changing a label does not rewrite history; a code in use cannot be deleted, only disabled.

use crate::time::Timestamp;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use axum::extract::{Path, Query, State};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, OnConflict, Query as SqlQuery};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::access::{Permission, PermissionCatalog, Principal};
use crate::error::{Error, ErrorKind, FieldError};
use crate::http::ValidJson;
use crate::i18n::{Language, Translations};
use crate::migration::prelude::*;
use crate::response::{Data, Problem};
use crate::tenancy::TenantContext;

const TABLE: &str = "kernel_reason_codes";

/// Edits a tenant's reason list.
pub const REASONS_MANAGE: Permission = Permission::new(
    "kernel.reasons.manage",
    "Change the tenant's lists of reasons: labels, order, turn codes off, add codes.",
);

/// A default code of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReasonDefault {
    pub code: &'static str,
    /// English label (translated through the module's `text:`).
    pub label: &'static str,
    /// A reason the law requires to exist: cannot be disabled.
    pub legal: bool,
}

impl ReasonDefault {
    pub const fn new(code: &'static str, label: &'static str) -> Self {
        ReasonDefault { code, label, legal: false }
    }

    pub const fn legal(mut self) -> Self {
        self.legal = true;
        self
    }
}

/// A reason list declared by a module.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReasonList {
    /// `<module>.<action>`, vd `orders.cancel`.
    pub name: &'static str,
    pub defaults: Vec<ReasonDefault>,
}

impl ReasonList {
    pub fn new(name: &'static str, defaults: Vec<ReasonDefault>) -> Self {
        ReasonList { name, defaults }
    }
}

fn is_code(code: &str) -> bool {
    (1..=64).contains(&code.len())
        && code.starts_with(|c: char| c.is_ascii_lowercase())
        && code.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Validates the declarations of every module: name `<module>.<action>`, well-formed codes, no duplicates.
pub(crate) fn check(module: &str, lists: &[ReasonList], problems: &mut Vec<String>) {
    for list in lists {
        let own = list
            .name
            .strip_prefix(module)
            .and_then(|rest| rest.strip_prefix('.'))
            .is_some_and(is_code);
        if !own {
            problems.push(format!(
                "reason list `{}` of module `{module}` must be named `{module}.<lower_case_name>`",
                list.name
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for default in &list.defaults {
            if !is_code(default.code) || !seen.insert(default.code) {
                problems.push(format!(
                    "reason `{}` of list `{}` must be a unique lower_case code",
                    default.code, list.name
                ));
            }
        }
    }
}

/// Registry of every module's reason lists, filled once at startup. Cheap to clone.
#[derive(Clone, Default)]
pub struct ReasonCodes {
    lists: Arc<OnceLock<BTreeMap<&'static str, ReasonList>>>,
}

/// A code after merging the defaults with the tenant's part.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Merged {
    code: String,
    english: Option<&'static str>,
    labels: BTreeMap<String, String>,
    active: bool,
    position: i32,
    builtin: bool,
    legal: bool,
}

impl ReasonCodes {
    pub(crate) fn fill(&self, lists: Vec<ReasonList>) {
        let _ = self.lists.set(lists.into_iter().map(|list| (list.name, list)).collect());
    }

    fn list(&self, name: &str) -> Option<&ReasonList> {
        self.lists.get().and_then(|lists| lists.get(name))
    }

    /// Whether code `code` is in list `list` and enabled for tenant `tenant`; if not, error 422
    /// `kernel.reason_unknown` (a service calls this before saving a code a user submitted).
    pub async fn ensure(
        &self,
        tenant: &TenantContext,
        list: &str,
        code: &str,
    ) -> Result<(), Error> {
        let merged = self.merged(tenant, list).await?;
        if merged.iter().any(|reason| reason.code == code && reason.active) {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::Validation,
                "kernel.reason_unknown",
                "`{code}` is not an active reason of `{list}`; see `GET /api/v1/kernel/reasons/{list}`.",
            )
            .with_param("code", code.to_owned())
            .with_param("list", list.to_owned()))
        }
    }

    async fn merged(&self, tenant: &TenantContext, name: &str) -> Result<Vec<Merged>, Error> {
        let list = self.list(name).ok_or_else(|| list_not_found(name))?;
        let mut merged: Vec<Merged> = list
            .defaults
            .iter()
            .zip(0..)
            .map(|(default, index)| Merged {
                code: default.code.to_owned(),
                english: Some(default.label),
                labels: BTreeMap::new(),
                active: true,
                position: index * 10,
                builtin: true,
                legal: default.legal,
            })
            .collect();
        let select = SqlQuery::select()
            .columns(["code", "labels", "active", "position"])
            .from(Alias::new(TABLE))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("list")).eq(name))
            .to_owned();
        for row in tenant.database().read().query_all(&select).await? {
            let code: String = row.try_get("", "code")?;
            let labels: String = row.try_get("", "labels")?;
            let labels: BTreeMap<String, String> =
                serde_json::from_str(&labels).unwrap_or_default();
            let active: bool = row.try_get("", "active")?;
            let position: i32 = row.try_get("", "position")?;
            match merged.iter_mut().find(|reason| reason.code == code) {
                Some(reason) => {
                    reason.labels = labels;
                    // Mã pháp luật đòi luôn bật, dù dòng của khách ghi gì.
                    reason.active = active || reason.legal;
                    reason.position = position;
                }
                None => merged.push(Merged {
                    code,
                    english: None,
                    labels,
                    active,
                    position,
                    builtin: false,
                    legal: false,
                }),
            }
        }
        merged.sort_by(|a, b| (a.position, &a.code).cmp(&(b.position, &b.code)));
        Ok(merged)
    }
}

fn list_not_found(name: &str) -> Error {
    Error::not_found("kernel.reason_list_not_found", "There is no list of reasons `{list}`.")
        .with_param("list", name.to_owned())
}

/// One reason of a list.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct ReasonView {
    #[schema(example = "customer_request")]
    pub code: String,
    /// In the language of `Accept-Language`: the tenant's own label, else the default.
    pub label: String,
    /// The tenant's own labels, by language.
    pub labels: BTreeMap<String, String>,
    pub active: bool,
    pub position: i32,
    /// Declared by the module (else added by the tenant).
    pub builtin: bool,
    /// Required by law: cannot be turned off.
    pub legal: bool,
}

/// Which reasons.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ReasonsQuery {
    /// `true`: also the reasons turned off.
    #[serde(default)]
    include_inactive: bool,
}

/// Changes to a reason; omitted fields stay. A code the list does not have yet is added.
#[derive(Debug, Clone, Deserialize, ToSchema, validator::Validate)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReasonChange {
    /// The tenant's labels by language (`vi`, `en`…), up to 200 characters each; an empty label
    /// goes back to the default. A new code needs at least one.
    #[serde(default)]
    pub labels: Option<BTreeMap<String, String>>,
    pub active: Option<bool>,
    pub position: Option<i32>,
}

#[derive(Clone)]
pub(crate) struct ReasonsState {
    pub codes: ReasonCodes,
    pub permissions: PermissionCatalog,
    pub translations: Translations,
}

impl ReasonsState {
    fn may_read(&self, principal: &Principal, list: &str) -> bool {
        let module = list.split('.').next().unwrap_or_default();
        let reader = format!("{module}.read");
        principal.require(&REASONS_MANAGE).is_ok()
            || self.permissions.iter().any(|permission| {
                permission.name() == reader && principal.require(permission).is_ok()
            })
    }

    fn view(&self, reason: Merged, language: Language) -> ReasonView {
        let own = reason.labels.get(language.tag()).filter(|label| !label.is_empty());
        let label = match (own, reason.english) {
            (Some(own), _) => own.clone(),
            (None, Some(english)) => self.translations.describe(english, language),
            (None, None) => reason
                .labels
                .get("en")
                .or_else(|| reason.labels.values().next())
                .cloned()
                .unwrap_or_else(|| reason.code.clone()),
        };
        ReasonView {
            code: reason.code,
            label,
            labels: reason.labels,
            active: reason.active,
            position: reason.position,
            builtin: reason.builtin,
            legal: reason.legal,
        }
    }

    async fn reasons(
        &self,
        tenant: &TenantContext,
        list: &str,
        include_inactive: bool,
        language: Language,
    ) -> Result<Vec<ReasonView>, Error> {
        Ok(self
            .codes
            .merged(tenant, list)
            .await?
            .into_iter()
            .filter(|reason| include_inactive || reason.active)
            .map(|reason| self.view(reason, language))
            .collect())
    }
}

/// List the reasons of a list.
///
/// Anyone who reads the module that owns the list (`<module>.read`), or holds
/// `kernel.reasons.manage`. The module's reasons and the tenant's own, by `position` then code
/// (`order=desc` reverses it; paged with `limit` / `cursor`), with labels in the language of
/// `Accept-Language`.
#[utoipa::path(
    get,
    path = "/reasons/{list}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_reasons",
    security(("bearer" = [])),
    params(("list" = String, Path, description = "The list, e.g. `orders.cancel`."), ReasonsQuery, crate::pagination::NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of reasons, by position.", body = crate::pagination::Page<ReasonView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such list for you; code `kernel.reason_list_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_reasons(
    State(state): State<Arc<ReasonsState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
    Path(list): Path<String>,
    Query(query): Query<ReasonsQuery>,
    Query(page): Query<crate::pagination::NaturalOrderPageQuery>,
) -> Result<crate::pagination::Page<ReasonView>, Error> {
    if !state.may_read(&principal, &list) {
        return Err(list_not_found(&list));
    }
    let page = crate::pagination::PageRequest::try_from(page)?;
    let reasons = state.reasons(&tenant, &list, query.include_inactive, language).await?;
    // Lý do mặc định của module (trong code) gộp với dòng của khách theo mã — thứ tự `position` có thể do
    // khách đổi, nên chỉ biết sau khi gộp: phân trang trong bộ nhớ (D-20261125-list-pages mục 4). Khoá giữ
    // đúng thứ tự (vị trí, mã) khi so theo byte: vị trí dời về số không âm, đủ 10 chữ số.
    crate::pagination::page_of(
        reasons,
        |reason| {
            let position = i64::from(reason.position) - i64::from(i32::MIN);
            format!("{position:010}\u{1f}{}", reason.code)
        },
        &page,
    )
}

/// Change or add a reason.
///
/// Requires `kernel.reasons.manage`. Changes the labels, order or on / off of a reason; a code the
/// list does not have yet becomes the tenant's own reason. Reasons required by law stay on
/// (`kernel.reason_legal`). Codes are lower-case letters, digits and `_` (`kernel.reason_invalid`).
#[utoipa::path(
    put,
    path = "/reasons/{list}/{code}",
    extensions(("x-permissions" = json!(["kernel.reasons.manage"]))),
    operation_id = "set_reason",
    security(("bearer" = [])),
    params(
        ("list" = String, Path, description = "The list, e.g. `orders.cancel`."),
        ("code" = String, Path, description = "The reason's code, e.g. `customer_request`."),
    ),
    request_body = ReasonChange,
    responses(
        (status = 200, description = "The list after the change.", body = Data<Vec<ReasonView>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.reasons.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such list; code `kernel.reason_list_not_found`.", body = Problem),
        (status = 409, description = "Turning off a reason required by law; code `kernel.reason_legal`.", body = Problem),
        (status = 422, description = "The code is malformed (`kernel.reason_invalid`), or a label is invalid; see `errors`.", body = Problem),
    )
)]
async fn set_reason(
    State(state): State<Arc<ReasonsState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
    Path((list, code)): Path<(String, String)>,
    ValidJson(change): ValidJson<ReasonChange>,
) -> Result<Data<Vec<ReasonView>>, Error> {
    principal.require(&REASONS_MANAGE)?;
    let merged = state.codes.merged(&tenant, &list).await?;
    if !is_code(&code) {
        return Err(Error::new(
            ErrorKind::Validation,
            "kernel.reason_invalid",
            "`{code}` is not a valid code: use 1 to 64 lower-case letters, digits and `_`, starting with a letter.",
        )
        .with_param("code", code));
    }
    let current = merged.iter().find(|reason| reason.code == code);
    if current.is_some_and(|reason| reason.legal) && change.active == Some(false) {
        return Err(Error::conflict(
            "kernel.reason_legal",
            "Reason `{code}` is required by law and cannot be turned off.",
        )
        .with_param("code", code));
    }
    let mut labels = current.map(|reason| reason.labels.clone()).unwrap_or_default();
    let mut problems = Vec::new();
    if let Some(changed) = change.labels {
        for (locale, label) in changed {
            let valid_locale = (2..=10).contains(&locale.len())
                && locale.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
            if !valid_locale || label.chars().count() > 200 {
                problems.push(FieldError::new(
                    format!("labels.{locale}"),
                    "invalid",
                    "must be a language code with a label of at most 200 characters",
                ));
                continue;
            }
            let label = label.trim().to_owned();
            if label.is_empty() {
                labels.remove(&locale);
            } else {
                labels.insert(locale, label);
            }
        }
    }
    if current.is_none() && labels.is_empty() {
        problems.push(FieldError::new("labels", "required", "a new reason needs a label"));
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    let active = change.active.or(current.map(|reason| reason.active)).unwrap_or(true);
    let position = change
        .position
        .or(current.map(|reason| reason.position))
        .unwrap_or_else(|| merged.iter().map(|reason| reason.position).max().unwrap_or(0) + 10);
    let insert = SqlQuery::insert()
        .into_table(Alias::new(TABLE))
        .columns([
            "id",
            "tenant_id",
            "list",
            "code",
            "labels",
            "active",
            "position",
            "builtin",
            "updated_at",
        ])
        .values_panic([
            uuid::Uuid::now_v7().into(),
            tenant.id().as_uuid().into(),
            list.clone().into(),
            code.clone().into(),
            serde_json::to_string(&labels).map_err(Error::internal)?.into(),
            active.into(),
            position.into(),
            current.is_some_and(|reason| reason.builtin).into(),
            now().into(),
        ])
        .on_conflict(
            OnConflict::columns([Alias::new("tenant_id"), Alias::new("list"), Alias::new("code")])
                .update_columns([
                    Alias::new("labels"),
                    Alias::new("active"),
                    Alias::new("position"),
                    Alias::new("updated_at"),
                ])
                .to_owned(),
        )
        .to_owned();
    tenant.database().write().execute(&insert).await?;
    Ok(Data { data: state.reasons(&tenant, &list, true, language).await? })
}

fn now() -> Timestamp {
    Timestamp::now()
}

pub(crate) fn routes(state: ReasonsState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_reasons))
        .routes(routes!(set_reason))
        .with_state(Arc::new(state))
}

/// Migration that creates the table `kernel_reason_codes` (tenant level, every data database).
pub(crate) struct CreateReasonCodes;

impl MigrationName for CreateReasonCodes {
    fn name(&self) -> &str {
        "m0025_create_reason_codes"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateReasonCodes {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TABLE))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("list", 128))
                    .col(schema::string_len("code", 64))
                    .col(schema::text("labels"))
                    .col(schema::boolean("active"))
                    .col(schema::integer("position"))
                    .col(schema::boolean("builtin"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_reason_codes_code")
                    .table(Alias::new(TABLE))
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("list"))
                    .col(Alias::new("code"))
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new(TABLE)).to_owned()).await
    }
}
