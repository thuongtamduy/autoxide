//! Cross-module search (step 51 P3, the UI's Ctrl K box): a module declares **search sources** for its record
//! kinds; `GET /api/v1/kernel/find?q=` queries the sources the caller has permission for, each source with a
//! time limit like the work inbox, and groups results by kind. Matching ignores diacritics
//! (`autoxide_kernel::search`): a source searches a precomputed search-key column, like every module's search
//! box.
//!
//! Commands ("create new", "go to screen…") are built by the UI from the navigation registry; the core has no
//! command list.

use std::sync::{Arc, PoisonError, RwLock};

use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::access::{OrgTarget, Permission, Principal};
use crate::entitlement::EntitlementsHandle;
use crate::error::{Error, FieldError};
use crate::response::{Data, Problem};
use crate::tenancy::{OrgId, TenantContext, root_of};

/// At least this many characters (after trimming whitespace at both ends).
const MIN_QUERY: usize = 2;

/// A record found.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Found {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    /// The screen in the navigation registry that opens the record (`NavItem::id`).
    pub screen: &'static str,
}

impl Found {
    pub fn new(id: impl Into<String>, title: impl Into<String>, screen: &'static str) -> Self {
        Found { id: id.into(), title: title.into(), subtitle: None, screen }
    }

    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        let subtitle = subtitle.into();
        self.subtitle = (!subtitle.is_empty()).then_some(subtitle);
        self
    }
}

/// The comparison operator of a filter in the search box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchOp {
    /// `field:value`
    Eq(String),
    /// `field>value`
    Gt(String),
    /// `field>=value`
    Ge(String),
    /// `field<value`
    Lt(String),
    /// `field<=value`
    Le(String),
    /// `field:from..to` (inclusive of both ends; an empty end means no limit on that side).
    Range(String, String),
}

/// A filter in the search box, e.g. `status:draft`, `-locale:en`, `updated>2026-01-01`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchFilter {
    /// Field name, lowercase.
    pub field: String,
    pub op: SearchOp,
    /// Preceded by `-`: excludes matching records.
    pub negated: bool,
    /// Verbatim text from the search box (to report back in `ignored`).
    pub raw: String,
}

/// Search scope over the organization tree (step 51 P9).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SearchScope {
    /// Every node the caller has permission for (default; permission scope is applied by the source itself).
    #[default]
    Accessible,
    /// Only records of exactly this node.
    Node(OrgId),
    /// Records of this node and all its child nodes.
    Subtree(OrgId),
}

/// A parsed search-box query (step 51 P9): remaining keywords, kinds (`is:`), filters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchQuery {
    /// Remaining keywords, separated by a single space.
    pub text: String,
    /// `is:<kind>`: only these kinds (full name `content.entry` or the part after the dot `entry`).
    pub kinds: Vec<String>,
    /// `-is:<kind>`: excludes these kinds.
    pub excluded_kinds: Vec<String>,
    pub filters: Vec<SearchFilter>,
    /// The parts not understood (negated keywords `-word`, comparisons missing a value).
    pub ignored: Vec<String>,
    /// Scope over the organization tree (the endpoint's `scope=`).
    pub scope: SearchScope,
}

impl SearchQuery {
    /// Parses the search box. A phrase in double quotes is one keyword (whitespace is kept).
    pub fn parse(input: &str) -> Self {
        let mut query = SearchQuery::default();
        let mut words = Vec::new();
        for token in tokens(input) {
            if token.quoted {
                words.push(token.text);
                continue;
            }
            let raw = token.text;
            let (negated, body) = match raw.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (true, rest),
                _ => (false, raw.as_str()),
            };
            match operator(body) {
                Some((field, _)) if field == "is" => {
                    let kind = body[3..].to_lowercase();
                    if kind.is_empty() {
                        query.ignored.push(raw.clone());
                    } else if negated {
                        query.excluded_kinds.push(kind);
                    } else {
                        query.kinds.push(kind);
                    }
                }
                Some((field, op)) => {
                    query.filters.push(SearchFilter { field, op, negated, raw: raw.clone() })
                }
                None if negated => query.ignored.push(raw.clone()),
                None => words.push(raw.clone()),
            }
        }
        query.text = words.join(" ");
        query
    }

    /// Scope condition for the node column `column` of the source table; `None` for `Accessible`.
    pub fn scope_condition<Col: sea_orm::ColumnTrait>(
        &self,
        tenant: &TenantContext,
        column: Col,
    ) -> Option<sea_orm::Condition> {
        match self.scope {
            SearchScope::Accessible => None,
            SearchScope::Node(org) => Some(sea_orm::Condition::all().add(column.eq(org.as_uuid()))),
            SearchScope::Subtree(org) => {
                Some(crate::tenancy::OrgTree::subtree_condition(tenant.id(), &[org], column))
            }
        }
    }

    /// Whether kind `kind` (`<module>.<kind>`) is matched by `is:` / `-is:`.
    pub fn wants(&self, kind: &str) -> bool {
        let short = kind.rsplit('.').next().unwrap_or(kind);
        let named =
            |list: &Vec<String>| list.iter().any(|wanted| wanted == kind || wanted == short);
        (self.kinds.is_empty() || named(&self.kinds)) && !named(&self.excluded_kinds)
    }
}

struct Token {
    text: String,
    quoted: bool,
}

fn tokens(input: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut was_quoted = false;
    for c in input.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                was_quoted = true;
            }
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    out.push(Token { text: std::mem::take(&mut current), quoted: was_quoted });
                }
                was_quoted = false;
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(Token { text: current, quoted: was_quoted });
    }
    out
}

/// `field:value`, `field>value`, `field>=value`, `field<value`, `field<=value`, `field:a..b`; the field
/// name is lowercase letters, digits, `_`.
fn operator(body: &str) -> Option<(String, SearchOp)> {
    let split = body.find([':', '>', '<'])?;
    let field = &body[..split];
    if field.is_empty() || !field.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return None;
    }
    let rest = &body[split..];
    let (op, value): (fn(String) -> SearchOp, &str) = if let Some(value) = rest.strip_prefix(">=") {
        (SearchOp::Ge, value)
    } else if let Some(value) = rest.strip_prefix("<=") {
        (SearchOp::Le, value)
    } else if let Some(value) = rest.strip_prefix('>') {
        (SearchOp::Gt, value)
    } else if let Some(value) = rest.strip_prefix('<') {
        (SearchOp::Lt, value)
    } else {
        let value = rest.strip_prefix(':').unwrap_or(rest);
        if let Some((from, to)) = value.split_once("..") {
            return Some((field.to_lowercase(), SearchOp::Range(from.to_owned(), to.to_owned())));
        }
        (SearchOp::Eq, value)
    };
    (!value.is_empty()).then(|| (field.to_lowercase(), op(value.to_owned())))
}

/// A search source a module declares with [`StartContext::finder_source`](crate::module::StartContext).
/// The source applies the caller's node scope itself and returns only records that person may view.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait FinderSource: Send + Sync {
    /// Record kind, `<module>.<kind>` (like the entity registry), e.g. `content.entry`.
    fn kind(&self) -> &'static str;
    fn permission(&self) -> Permission;
    async fn find(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: OrgTarget,
        query: &str,
        limit: u16,
    ) -> Result<Vec<Found>, Error>;

    /// The source can apply `scope=node|subtree` (records have a node column). Default: no.
    fn scoped(&self) -> bool {
        false
    }

    /// Filter fields the source understands (step 51 P9), e.g. `status`, `updated`. Default: none.
    fn fields(&self) -> &'static [&'static str] {
        &[]
    }

    /// Searches by the parsed query; only accepts the filters of [`fields`](Self::fields). Default: searches
    /// by keywords like [`find`](Self::find) (a source that does not understand filters yet).
    async fn find_query(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: OrgTarget,
        query: &SearchQuery,
        limit: u16,
    ) -> Result<Vec<Found>, Error> {
        self.find(tenant, principal, target, &query.text, limit).await
    }
}

type Entry = (&'static str, Arc<dyn FinderSource>);

/// Registry of every module's search sources. Cheap to clone.
#[derive(Clone, Default)]
pub struct FinderSources {
    entries: Arc<RwLock<Vec<Entry>>>,
}

impl FinderSources {
    pub(crate) fn register(
        &self,
        module: &'static str,
        source: Arc<dyn FinderSource>,
    ) -> Result<(), Error> {
        let kind = source.kind();
        if !kind.starts_with(&format!("{module}.")) {
            return Err(Error::internal(format!(
                "finder source `{kind}` of module `{module}` must be named `{module}.<type>`"
            )));
        }
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        if entries.iter().any(|(_, known)| known.kind() == kind) {
            return Err(Error::internal(format!("finder source `{kind}` is registered twice")));
        }
        entries.push((module, source));
        Ok(())
    }

    fn all(&self) -> Vec<Entry> {
        self.entries.read().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

/// Records found, by type.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct FindView {
    /// One group per record type that found something, in the order modules start.
    pub groups: Vec<FoundGroup>,
    /// Sources that did not answer in time or failed.
    pub incomplete: Vec<String>,
    /// Parts of `q` no searched type understands (unknown fields, `-word`), left out of the search.
    pub ignored: Vec<String>,
}

/// Records of one type.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct FoundGroup {
    #[schema(example = "content.entry")]
    pub kind: String,
    pub items: Vec<FoundView>,
}

/// One record found.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct FoundView {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    /// Screen id of the navigation that opens the record.
    #[schema(example = "content.entries")]
    pub screen: String,
}

/// What to find.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct FindQuery {
    /// Words (at least 2 characters; any case, with or without accents) and operators:
    /// `is:<type>` / `-is:<type>` (e.g. `is:entry`), `field:value`, `field>value`, `field<value`,
    /// `field>=value`, `field<=value`, `field:from..to`, and `-` before a filter to exclude. Quote a
    /// phrase to search it as written.
    q: Option<String>,
    /// The organization unit chosen; omitted: the whole tenant.
    org_id: Option<Uuid>,
    /// `accessible` (the default: every unit you may see), `node` (only records of `org_id`) or
    /// `subtree` (records of `org_id` and the units below it); the last two need `org_id`.
    scope: Option<String>,
    /// At most this many records per type, 1 to 20. Defaults to 5.
    limit: Option<u16>,
}

#[derive(Clone)]
pub(crate) struct FindState {
    pub sources: FinderSources,
    pub entitlements: EntitlementsHandle,
}

impl FindState {
    pub(crate) async fn find(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org_id: Option<Uuid>,
        query: &SearchQuery,
        limit: u16,
    ) -> Result<FindView, Error> {
        let target = OrgTarget::new(org_id.map_or(root_of(tenant.id()), OrgId::from_uuid), None);
        let read = tenant.database().read();
        let mut allowed = Vec::new();
        for (module, source) in self.sources.all() {
            // Không từ khoá và nguồn không hiểu bộ lọc nào thì nguồn sẽ trả mọi thứ: bỏ qua.
            let understood =
                query.filters.iter().any(|filter| source.fields().contains(&filter.field.as_str()));
            if !query.wants(source.kind())
                || (query.text.chars().count() < MIN_QUERY && !understood)
            {
                continue;
            }
            if self.entitlements.module_enabled(tenant.id(), module).await?
                && principal.require_at(read, &source.permission(), target).await.is_ok()
            {
                allowed.push(source);
            }
        }
        let answers = futures_util::future::join_all(allowed.iter().map(|source| async move {
            let found = tokio::time::timeout(
                crate::ops::work::SOURCE_TIMEOUT,
                source.find_query(tenant, principal, target, query, limit),
            )
            .await;
            (source.kind(), found)
        }))
        .await;
        let mut groups = Vec::new();
        let mut incomplete = Vec::new();
        for (kind, answer) in answers {
            match answer {
                Ok(Ok(found)) if !found.is_empty() => groups.push(FoundGroup {
                    kind: kind.to_owned(),
                    items: found
                        .into_iter()
                        .take(usize::from(limit))
                        .map(|found| FoundView {
                            id: found.id,
                            title: found.title,
                            subtitle: found.subtitle,
                            screen: found.screen.to_owned(),
                        })
                        .collect(),
                }),
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    tracing::warn!(source = kind, error = %error, "finder source failed");
                    incomplete.push(kind.to_owned());
                }
                Err(_) => {
                    tracing::warn!(source = kind, "finder source timed out");
                    incomplete.push(kind.to_owned());
                }
            }
        }
        let mut ignored = query.ignored.clone();
        for filter in &query.filters {
            if !allowed.iter().any(|source| source.fields().contains(&filter.field.as_str())) {
                ignored.push(filter.raw.clone());
            }
        }
        if query.scope != SearchScope::Accessible && allowed.iter().any(|source| !source.scoped()) {
            ignored.push("scope".to_owned());
        }
        Ok(FindView { groups, incomplete, ignored })
    }
}

pub(crate) fn routes(state: FindState) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(find)).with_state(Arc::new(state))
}

/// Find records across modules.
///
/// Any signed-in user. Asks every module's finder for records whose title (or name, email…)
/// contains `q`, in any case and with or without accents, among the types the caller has
/// permission for at `org_id`; results are grouped by type, each record with the screen that
/// opens it. A slow or failing source is skipped and named in `incomplete`.
#[utoipa::path(
    get,
    path = "/find",
    extensions(("x-permissions" = json!([]))),
    operation_id = "find_records",
    security(("bearer" = [])),
    params(FindQuery),
    responses(
        (status = 200, description = "The records found.", body = Data<FindView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "`q` is shorter than 2 characters, or `limit` is not between 1 and 20; see `errors`.", body = Problem),
    )
)]
async fn find(
    State(state): State<Arc<FindState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<FindQuery>,
) -> Result<Data<FindView>, Error> {
    let text = query.q.as_deref().map(str::trim).unwrap_or_default();
    let mut parsed = SearchQuery::parse(text);
    let limit = query.limit.unwrap_or(5);
    let mut problems = Vec::new();
    // Chỉ có toán tử (vd `is:entry status:draft`) cũng được; chỉ từ khoá thì cần đủ dài.
    if parsed.text.chars().count() < MIN_QUERY && parsed.filters.is_empty() {
        problems.push(
            FieldError::new("q", "too_short", "must be at least {min} characters")
                .with_param("min", MIN_QUERY),
        );
    }
    if !(1..=20).contains(&limit) {
        problems.push(FieldError::new("limit", "invalid", "must be between 1 and 20"));
    }
    let unit = query.org_id.map(OrgId::from_uuid);
    parsed.scope = match (query.scope.as_deref(), unit) {
        (None | Some("accessible"), _) => SearchScope::Accessible,
        (Some("node"), Some(org)) => SearchScope::Node(org),
        (Some("subtree"), Some(org)) => SearchScope::Subtree(org),
        (Some("node" | "subtree"), None) => {
            problems.push(FieldError::new("org_id", "required", "is required"));
            SearchScope::Accessible
        }
        _ => {
            problems.push(FieldError::new(
                "scope",
                "invalid",
                "must be `accessible`, `node` or `subtree`",
            ));
            SearchScope::Accessible
        }
    };
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    Ok(Data { data: state.find(&tenant, &principal, query.org_id, &parsed, limit).await? })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phan_tich_o_tim_co_toan_tu() {
        let query = SearchQuery::parse(
            r#"is:entry -is:media "Đà Lạt" status:draft -locale:en updated>2026-01-01 created:2026-01-01..2026-02-01 -spam x:"#,
        );
        assert_eq!(query.text, "Đà Lạt x:");
        assert_eq!(
            (query.kinds, query.excluded_kinds),
            (vec!["entry".to_owned()], vec!["media".to_owned()])
        );
        assert_eq!(query.ignored, ["-spam"]);
        let ops: Vec<(&str, &SearchOp, bool)> =
            query.filters.iter().map(|f| (f.field.as_str(), &f.op, f.negated)).collect();
        assert_eq!(
            ops,
            [
                ("status", &SearchOp::Eq("draft".to_owned()), false),
                ("locale", &SearchOp::Eq("en".to_owned()), true),
                ("updated", &SearchOp::Gt("2026-01-01".to_owned()), false),
                (
                    "created",
                    &SearchOp::Range("2026-01-01".to_owned(), "2026-02-01".to_owned()),
                    false
                ),
            ]
        );
        let wants = SearchQuery::parse("is:entry");
        assert!(wants.wants("content.entry") && !wants.wants("media.item"));
        assert!(SearchQuery::parse("-is:item").wants("content.entry"));
    }
}
