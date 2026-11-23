//! Per-tenant request log (step 52 item 1, D-202): a technical desk for developers and operators to see who
//! (a person, an API key, an agent) called what, and with what result.
//!
//! - Table `kernel_request_log`, tenant level. Each row: time, user, API key, `actor_kind`, method, **route
//!   template** (`/api/v1/content/entries/{id}`, with no real ID), status, error `code`, processing time,
//!   `Idempotency-Key`, `approval_id`, `request_id`, `trace_id`. It stores no request body, authentication
//!   header or query string (they may contain personal data).
//! - MCP commands: one row per tool call, path `mcp:<tool name>`, method `MCP`, with no parameters logged. The
//!   enclosing `POST /mcp` request is not logged separately (the tool's row is enough).
//! - Only authenticated requests are logged: public requests (storefront pages) are not the technical desk's
//!   business, and logging them would make the table grow with page views.
//! - Writing is **asynchronous**: the HTTP layer pushes rows into a bounded in-memory queue ([`QUEUE`] rows),
//!   and a background task gathers and writes them in batches. When the queue is full the row is dropped
//!   (counted by `kernel_request_log_dropped_total`) rather than slowing the request. When the server stops,
//!   rows still in the queue may be lost; this is a log, not business data.
//! - Retention `AUTOXIDE_REQUEST_LOG_DAYS` (default 14, `0` turns it off): each row carries `expires_at`; the
//!   writer task deletes expired rows of each database at most once an hour (D-370).

use crate::time::Timestamp;
use jiff::SignedDuration;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use axum::extract::{Query, State};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::access::{ActorKind, Permission, Principal};
use crate::error::{Error, FieldError};
use crate::mcp::Tool;
use crate::migration::prelude::*;
use crate::pagination::{Page, PageQuery, PageRequest, fetch_page};
use crate::response::Problem;
use crate::tenancy::{DatabaseRegistry, ScopeExt, TenantContext, TenantId, TenantScoped};

/// Reading the technical desk: the request log, events by record.
pub const DEVTOOLS_READ: Permission = Permission::new(
    "kernel.devtools.read",
    "See the developer tools of the tenant: the log of API and MCP calls, the events of a record.",
);

/// Actions on the technical desk: resending a webhook delivery, rerunning a failed job.
pub const DEVTOOLS_MANAGE: Permission = Permission::new(
    "kernel.devtools.manage",
    "Act from the developer tools: redeliver webhooks and run failed jobs again.",
);

/// Maximum rows waiting to be written in memory.
const QUEUE: usize = 10_000;
/// Maximum rows per write.
const BATCH: usize = 200;
/// Each database cleans up expired rows at most once in this interval.
const PRUNE_EVERY: StdDuration = StdDuration::from_secs(3600);
/// Method of the row that records an MCP command.
pub(crate) const MCP_METHOD: &str = "MCP";

mod entity {
    //! Table `kernel_request_log`.
    use crate::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_request_log")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub at: Timestamp,
        pub expires_at: Timestamp,
        pub actor_kind: String,
        pub user_id: Option<Uuid>,
        pub api_key_id: Option<Uuid>,
        pub method: String,
        pub path: String,
        pub status: i32,
        pub code: Option<String>,
        pub duration_ms: i64,
        pub idempotency_key: Option<String>,
        pub approval_id: Option<Uuid>,
        pub request_id: Option<String>,
        pub trace_id: Option<String>,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

use entity::{Column, Entity};

impl TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}

// --- Ghi ------------------------------------------------------------------------------------

/// The caller of a request, filled in by the `Principal` / `TenantContext` extractor when it resolves.
#[derive(Default)]
struct Caller {
    tenant: Option<TenantContext>,
    user_id: Option<Uuid>,
    tenant_of_user: Option<TenantId>,
    api_key_id: Option<Uuid>,
    actor_kind: Option<ActorKind>,
}

/// A cell shared between the logging layer (placed into the request) and the extractor (fills it in). `Arc`
/// because axum copies the request's extensions into the part the handler receives: the copy still points to
/// the same cell.
#[derive(Clone, Default)]
pub(crate) struct CallerSlot(Arc<Mutex<Caller>>);

impl CallerSlot {
    /// Records the authenticated caller.
    pub(crate) fn note_principal(&self, principal: &Principal) {
        if let Ok(mut caller) = self.0.lock() {
            caller.user_id = Some(principal.user_id());
            caller.tenant_of_user = Some(principal.tenant_id());
            caller.api_key_id = principal.api_key_id();
            caller.actor_kind = Some(principal.actor_kind());
        }
    }

    /// Records the request's tenant.
    pub(crate) fn note_tenant(&self, tenant: &TenantContext) {
        if let Ok(mut caller) = self.0.lock() {
            caller.tenant = Some(tenant.clone());
        }
    }
}

/// A finished request waiting to be written.
pub(crate) struct Entry {
    tenant: Option<TenantContext>,
    tenant_id: TenantId,
    at: Timestamp,
    actor_kind: ActorKind,
    user_id: Uuid,
    api_key_id: Option<Uuid>,
    method: String,
    path: String,
    status: u16,
    code: Option<String>,
    duration: StdDuration,
    idempotency_key: Option<String>,
    approval_id: Option<Uuid>,
    request_id: Option<String>,
    trace_id: Option<String>,
}

/// The part of the request that the HTTP layer knows (apart from the caller).
pub(crate) struct Finished {
    pub method: String,
    pub path: String,
    pub status: u16,
    pub code: Option<String>,
    pub duration: StdDuration,
    pub idempotency_key: Option<String>,
    pub approval_id: Option<Uuid>,
    pub request_id: Option<String>,
    pub trace_id: Option<String>,
}

/// Queue that writes the log; cheap to clone (only increments the channel's reference count).
#[derive(Clone)]
pub struct RequestLog {
    sender: mpsc::Sender<Entry>,
}

impl RequestLog {
    /// Turns the log on: `None` when `days == 0`. The writer task runs until every clone is dropped.
    pub fn start(registry: DatabaseRegistry, days: u16) -> Option<Self> {
        if days == 0 {
            return None;
        }
        let (sender, receiver) = mpsc::channel(QUEUE);
        tokio::spawn(write_loop(
            registry,
            receiver,
            SignedDuration::from_hours(24 * (i64::from(days))),
        ));
        Some(RequestLog { sender })
    }

    /// Pushes a finished request into the queue; with no authenticated caller it is skipped.
    pub(crate) fn record(&self, slot: &CallerSlot, finished: Finished) {
        let Some(entry) = slot.0.lock().ok().and_then(|caller| {
            Some(Entry {
                tenant: caller.tenant.clone(),
                tenant_id: caller
                    .tenant
                    .as_ref()
                    .map(TenantContext::id)
                    .or(caller.tenant_of_user)?,
                at: Timestamp::now(),
                actor_kind: caller.actor_kind?,
                user_id: caller.user_id?,
                api_key_id: caller.api_key_id,
                method: finished.method,
                path: finished.path,
                status: finished.status,
                code: finished.code,
                duration: finished.duration,
                idempotency_key: finished.idempotency_key,
                approval_id: finished.approval_id,
                request_id: finished.request_id,
                trace_id: finished.trace_id,
            })
        }) else {
            return;
        };
        if self.sender.try_send(entry).is_err() {
            metrics::counter!("kernel_request_log_dropped_total").increment(1);
        }
    }

    /// Records an MCP command (`mcp:<tool>`), in the same queue as REST.
    pub(crate) fn record_tool(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        tool: &str,
        code: Option<String>,
        duration: StdDuration,
        request_id: Option<String>,
    ) {
        let slot = CallerSlot::default();
        slot.note_principal(principal);
        slot.note_tenant(tenant);
        self.record(
            &slot,
            Finished {
                method: MCP_METHOD.to_owned(),
                path: format!("mcp:{tool}"),
                status: if code.is_some() { 400 } else { 200 },
                code,
                duration,
                idempotency_key: None,
                approval_id: None,
                request_id,
                trace_id: None,
            },
        );
    }
}

/// A trustworthy header value to record: short, visible characters only.
pub(crate) fn header_text(value: Option<&axum::http::HeaderValue>, max: usize) -> Option<String> {
    let text = value?.to_str().ok()?.trim();
    (!text.is_empty() && text.len() <= max && text.bytes().all(|b| b.is_ascii_graphic()))
        .then(|| text.to_owned())
}

async fn write_loop(
    registry: DatabaseRegistry,
    mut receiver: mpsc::Receiver<Entry>,
    keep: SignedDuration,
) {
    let mut pruned: HashMap<TenantId, Instant> = HashMap::new();
    let mut batch = Vec::with_capacity(BATCH);
    while receiver.recv_many(&mut batch, BATCH).await > 0 {
        // Gom theo khách: mỗi khách một câu `INSERT` nhiều dòng trên database của nó.
        let mut by_tenant: HashMap<TenantId, Vec<Entry>> = HashMap::new();
        for entry in batch.drain(..) {
            by_tenant.entry(entry.tenant_id).or_default().push(entry);
        }
        for (tenant_id, entries) in by_tenant {
            let tenant = match entries.iter().find_map(|entry| entry.tenant.clone()) {
                Some(tenant) => tenant,
                None => match registry.tenant(tenant_id).await {
                    Ok(tenant) => tenant,
                    Err(error) => {
                        tracing::warn!(tenant_id = %tenant_id, error = %error, "request log: tenant not found");
                        continue;
                    }
                },
            };
            if let Err(error) = write(&tenant, entries, keep).await {
                tracing::warn!(tenant_id = %tenant_id, error = %error, "request log not written");
            }
            let due = pruned.get(&tenant_id).is_none_or(|last| last.elapsed() >= PRUNE_EVERY);
            if due {
                pruned.insert(tenant_id, Instant::now());
                if let Err(error) = prune(&tenant, Timestamp::now()).await {
                    tracing::warn!(tenant_id = %tenant_id, error = %error, "request log not pruned");
                }
            }
        }
    }
}

async fn write(
    tenant: &TenantContext,
    entries: Vec<Entry>,
    keep: SignedDuration,
) -> Result<(), Error> {
    let rows = entries.into_iter().map(|entry| entity::ActiveModel {
        id: sea_orm::Set(Uuid::now_v7()),
        tenant_id: sea_orm::Set(tenant.id().as_uuid()),
        at: sea_orm::Set(entry.at),
        expires_at: sea_orm::Set(entry.at + keep),
        actor_kind: sea_orm::Set(entry.actor_kind.as_str().to_owned()),
        user_id: sea_orm::Set(Some(entry.user_id)),
        api_key_id: sea_orm::Set(entry.api_key_id),
        method: sea_orm::Set(entry.method),
        path: sea_orm::Set(entry.path.chars().take(300).collect()),
        status: sea_orm::Set(i32::from(entry.status)),
        code: sea_orm::Set(entry.code.map(|code| code.chars().take(200).collect())),
        duration_ms: sea_orm::Set(
            <i64 as TryFrom<u128>>::try_from(entry.duration.as_millis()).unwrap_or(i64::MAX),
        ),
        idempotency_key: sea_orm::Set(entry.idempotency_key),
        approval_id: sea_orm::Set(entry.approval_id),
        request_id: sea_orm::Set(entry.request_id),
        trace_id: sea_orm::Set(entry.trace_id),
    });
    Entity::insert_many(rows).exec_without_returning(tenant.database().write()).await?;
    Ok(())
}

/// Deletes expired rows of the database of `tenant` (every tenant on that database).
pub(crate) async fn prune(tenant: &TenantContext, now: Timestamp) -> Result<u64, Error> {
    let deleted = Entity::delete_many()
        .filter(Column::ExpiresAt.lt(now))
        .exec(tenant.database().write())
        .await?;
    Ok(deleted.rows_affected)
}

// --- Đọc ------------------------------------------------------------------------------------

/// One call in the log of the tenant.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct LoggedRequest {
    pub id: Uuid,
    pub at: Timestamp,
    /// `user`, `agent`, `service`, `system`, `support`, `app` or `scim`.
    #[schema(example = "agent")]
    pub actor_kind: String,
    pub user_id: Option<Uuid>,
    /// The API key used, when the call came with one.
    pub api_key_id: Option<Uuid>,
    /// HTTP method, or `MCP` for a tool call.
    #[schema(example = "POST")]
    pub method: String,
    /// The route template (no real ids), e.g. `/api/v1/content/entries/{id}`, or
    /// `mcp:<tool>` for a tool call, e.g. `mcp:content.publish_entry`.
    #[schema(example = "/api/v1/content/entries/{id}")]
    pub path: String,
    /// HTTP status; for a tool call `200`, or `400` when the tool returned an error.
    pub status: u16,
    /// The error `code`, when the call failed.
    pub code: Option<String>,
    pub duration_ms: i64,
    pub idempotency_key: Option<String>,
    /// The approval request the call created (`202`) or used (`Autoxide-Approval`).
    pub approval_id: Option<Uuid>,
    /// `x-request-id` of the call.
    pub request_id: Option<String>,
    /// OpenTelemetry trace id, when traces are exported.
    pub trace_id: Option<String>,
}

impl From<entity::Model> for LoggedRequest {
    fn from(row: entity::Model) -> Self {
        LoggedRequest {
            id: row.id,
            at: row.at,
            actor_kind: row.actor_kind,
            user_id: row.user_id,
            api_key_id: row.api_key_id,
            method: row.method,
            path: row.path,
            status: <u16 as TryFrom<i32>>::try_from(row.status).unwrap_or_default(),
            code: row.code,
            duration_ms: row.duration_ms,
            idempotency_key: row.idempotency_key,
            approval_id: row.approval_id,
            request_id: row.request_id,
            trace_id: row.trace_id,
        }
    }
}

/// Which calls to list.
#[derive(Debug, Default, Clone, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct RequestFilter {
    /// Only calls of these kinds of actor, comma-separated, e.g. `agent,service`.
    #[param(example = "agent")]
    pub actor_kind: Option<String>,
    /// Only calls made with this API key.
    pub api_key_id: Option<Uuid>,
    /// Only calls to exactly this route template or tool, e.g. `mcp:content.publish_entry`.
    pub path: Option<String>,
    /// Only calls that ended with this HTTP status.
    pub status: Option<u16>,
    /// Only calls whose HTTP status is in this class: `2xx`, `3xx`, `4xx` or `5xx` (e.g. `5xx` for
    /// server errors).
    #[param(example = "5xx")]
    pub status_class: Option<String>,
    /// Only calls at or after this time (RFC 3339).
    pub from: Option<Timestamp>,
    /// Only calls before this time (RFC 3339).
    pub to: Option<Timestamp>,
}

/// List by filter, newest first (or by `order`).
pub(crate) async fn list(
    tenant: &TenantContext,
    filter: &RequestFilter,
    page: &PageRequest,
) -> Result<Page<LoggedRequest>, Error> {
    let mut select = Entity::find().scoped(tenant);
    if let Some(kinds) = filter.actor_kind.as_deref().filter(|kinds| !kinds.is_empty()) {
        let mut parsed = Vec::new();
        for raw in kinds.split(',') {
            match ActorKind::parse(raw.trim()) {
                Some(kind) => parsed.push(kind.as_str()),
                None => {
                    return Err(Error::validation(vec![FieldError::new(
                        "actor_kind",
                        "invalid",
                        "must be `user`, `agent`, `service`, `system`, `support`, `app` or `scim`, comma-separated",
                    )]));
                }
            }
        }
        select = select.filter(Column::ActorKind.is_in(parsed));
    }
    if let Some(key) = filter.api_key_id {
        select = select.filter(Column::ApiKeyId.eq(key));
    }
    if let Some(path) = &filter.path {
        select = select.filter(Column::Path.eq(path.as_str()));
    }
    if let Some(status) = filter.status {
        select = select.filter(Column::Status.eq(i32::from(status)));
    }
    if let Some(class) = filter.status_class.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        let hundreds = match class {
            "2xx" => 200,
            "3xx" => 300,
            "4xx" => 400,
            "5xx" => 500,
            _ => {
                return Err(Error::validation(vec![FieldError::new(
                    "status_class",
                    "invalid",
                    "must be `2xx`, `3xx`, `4xx` or `5xx`",
                )]));
            }
        };
        select = select.filter(Column::Status.between(hundreds, hundreds + 99));
    }
    if let Some(from) = filter.from {
        select = select.filter(Column::At.gte(from));
    }
    if let Some(to) = filter.to {
        select = select.filter(Column::At.lt(to));
    }
    let rows = fetch_page(select, Column::Id, |row| row.id, page, tenant.database().read()).await?;
    Ok(rows.map(LoggedRequest::from))
}

/// List the calls made in this tenant.
///
/// Requires `kernel.devtools.read`. Signed-in REST calls and MCP tool calls (path `mcp:<tool>`),
/// newest first, kept `AUTOXIDE_REQUEST_LOG_DAYS` days (14 by default). Paths are route templates
/// without real ids; bodies, tokens and query strings are never logged. Calls are written a
/// moment after they finish.
#[utoipa::path(
    get,
    path = "/devtools/requests",
    extensions(("x-permissions" = json!(["kernel.devtools.read"]))),
    operation_id = "list_logged_requests",
    security(("bearer" = [])),
    params(PageQuery, RequestFilter),
    responses(
        (status = 200, description = "A page of calls.", body = Page<LoggedRequest>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `kernel.devtools.read`.", body = Problem),
        (status = 422, description = "A filter or the page parameters are invalid; see `errors`.", body = Problem),
    )
)]
async fn list_requests(
    State(_state): State<Arc<DevtoolsState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
    Query(filter): Query<RequestFilter>,
) -> Result<Page<LoggedRequest>, Error> {
    principal.require(&DEVTOOLS_READ)?;
    list(&tenant, &filter, &PageRequest::try_from(page)?).await
}

#[derive(Clone, Default)]
pub(crate) struct DevtoolsState;

pub(crate) fn routes() -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(list_requests)).with_state(Arc::new(DevtoolsState))
}

/// Which calls to list (the same filters as the REST endpoint).
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestLogInput {
    /// Kinds of actor, comma-separated, e.g. `agent`.
    actor_kind: Option<String>,
    /// Only calls made with this API key.
    api_key_id: Option<Uuid>,
    /// Exactly this route template or `mcp:<tool>`.
    path: Option<String>,
    /// Only calls that ended with this HTTP status.
    status: Option<u16>,
    /// Only calls whose status is in this class: `2xx`, `3xx`, `4xx` or `5xx`.
    status_class: Option<String>,
    /// At or after this time (RFC 3339).
    from: Option<Timestamp>,
    /// Before this time (RFC 3339).
    to: Option<Timestamp>,
    /// 1 to 200; 50 by default.
    limit: Option<u32>,
    /// `next_cursor` of the previous page.
    cursor: Option<String>,
}

pub(crate) fn tools() -> Vec<Tool> {
    vec![
        Tool::new(
            "request_log",
            "List the recent API and MCP calls made in this tenant (who, which route or tool, \
             status, error code), newest first. Filter by `actor_kind` (e.g. `agent`), \
             `api_key_id`, exact `path` (e.g. `mcp:content.publish_entry`), `status` or \
             `status_class` (e.g. `5xx`).",
            |tenant: TenantContext, principal: Principal, input: RequestLogInput| async move {
                principal.require(&DEVTOOLS_READ)?;
                let filter = RequestFilter {
                    actor_kind: input.actor_kind,
                    api_key_id: input.api_key_id,
                    path: input.path,
                    status: input.status,
                    status_class: input.status_class,
                    from: input.from,
                    to: input.to,
                };
                let page = PageRequest::try_from(PageQuery::new(
                    input.limit.map(|limit| limit.to_string()),
                    input.cursor,
                    None,
                ))?;
                list(&tenant, &filter, &page).await
            },
        )
        .requires(DEVTOOLS_READ)
        .read_only(),
    ]
}

// --- Migration ------------------------------------------------------------------------------

/// Table `kernel_request_log`.
pub(crate) struct CreateRequestLog;

impl MigrationName for CreateRequestLog {
    fn name(&self) -> &str {
        "m0031_create_request_log"
    }
}

const TABLE: &str = "kernel_request_log";

#[async_trait::async_trait]
impl MigrationTrait for CreateRequestLog {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TABLE))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::timestamp_with_time_zone("at"))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::string_len("actor_kind", 16))
                    .col(schema::uuid_null("user_id"))
                    .col(schema::uuid_null("api_key_id"))
                    .col(schema::string_len("method", 8))
                    .col(schema::string_len("path", 300))
                    .col(schema::integer("status"))
                    .col(schema::string_len_null("code", 200))
                    .col(schema::big_integer("duration_ms"))
                    .col(schema::string_len_null("idempotency_key", 255))
                    .col(schema::uuid_null("approval_id"))
                    .col(schema::string_len_null("request_id", 128))
                    .col(schema::string_len_null("trace_id", 32))
                    .to_owned(),
            )
            .await?;
        for (name, columns) in [
            ("kernel_request_log_tenant_idx", vec!["tenant_id", "id"]),
            ("kernel_request_log_actor_idx", vec!["tenant_id", "actor_kind", "id"]),
            ("kernel_request_log_key_idx", vec!["tenant_id", "api_key_id", "id"]),
            ("kernel_request_log_expires_sweep_idx", vec!["expires_at"]),
        ] {
            let mut index = Index::create();
            index.name(name).table(Alias::new(TABLE));
            for column in columns {
                index.col(Alias::new(column));
            }
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new(TABLE)).to_owned()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ConfigReader, ConfigSource};
    use crate::database::Database;
    use crate::migration::{Migrator, ModuleMigrations};
    use autoxide_testkit::TestDatabase;
    use sea_orm::PaginatorTrait;

    fn entry(tenant: &TenantContext) -> Entry {
        Entry {
            tenant: Some(tenant.clone()),
            tenant_id: tenant.id(),
            at: Timestamp::now(),
            actor_kind: ActorKind::Agent,
            user_id: Uuid::now_v7(),
            api_key_id: None,
            method: "GET".to_owned(),
            path: "/api/v1/me/flags".to_owned(),
            status: 200,
            code: None,
            duration: StdDuration::from_millis(3),
            idempotency_key: None,
            approval_id: None,
            request_id: None,
            trace_id: None,
        }
    }

    #[tokio::test]
    async fn request_log_het_han_bi_xoa() {
        TestDatabase::run(async |test| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
            let mut reader = ConfigReader::new(&source);
            let config = Config::read(&mut reader);
            let (config, _) = reader.finish(config).unwrap();
            let database = Database::connect(&config.database).await.unwrap();
            let migrations = vec![
                Box::new(crate::tenancy::schema::CreateTenancy) as Box<dyn MigrationTrait>,
                Box::new(CreateRequestLog),
            ];
            Migrator::new(vec![ModuleMigrations::new("kernel", migrations)])
                .unwrap()
                .up(&database)
                .await
                .unwrap();
            let tenant = TenantContext::default_on(database);
            // Một dòng đã quá hạn (hạn lưu âm), một dòng còn hạn.
            write(&tenant, vec![entry(&tenant)], SignedDuration::from_hours(-24)).await.unwrap();
            write(&tenant, vec![entry(&tenant)], SignedDuration::from_hours(24 * 14))
                .await
                .unwrap();
            assert_eq!(prune(&tenant, Timestamp::now()).await.unwrap(), 1);
            let left =
                Entity::find().scoped(&tenant).count(tenant.database().read()).await.unwrap();
            assert_eq!(left, 1);
        })
        .await
        .unwrap();
    }
}
