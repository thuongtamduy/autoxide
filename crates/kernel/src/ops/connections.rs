//! Settings that connect to external services (step 51, A-c10, A-c11): a tenant's own mail server, AI provider,
//! payment gateway, ... - the `connection` settings group, named `<module>.connection.<kind>.<field>`.
//!
//! - A module reports each use of a connection with [`ConnectionStatus::record`] (last use, last error);
//!   `GET /api/v1/kernel/settings` returns `last_used_at` and `last_error` for each setting of a connection.
//! - A module declares how to test with [`StartContext::connection_probe`](crate::module::StartContext);
//!   `POST /api/v1/kernel/settings/{name}/test` tries it right away (not inside a write transaction, with a
//!   timeout) and records the result like a use.
//!
//! Table `kernel_connection_status` (tenant level): one row per (tenant, connection).

use crate::time::Timestamp;
use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use axum::extract::{Path, State};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, OnConflict, Query as SqlQuery};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::access::Principal;
use crate::error::Error;
use crate::migration::prelude::*;
use crate::response::{Data, Problem};
use crate::tenancy::TenantContext;

const TABLE: &str = "kernel_connection_status";
/// A connection test waits at most this long.
const TEST_TIMEOUT: Duration = Duration::from_secs(15);
/// The last error is kept to at most this many characters.
const ERROR_MAX: usize = 500;

/// How to test one of a module's connections.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait ConnectionProbe: Send + Sync {
    /// Connection, `<module>.connection.<kind>`, e.g. `mail.connection.smtp`.
    fn connection(&self) -> &'static str;
    /// Tests the connection of tenant `tenant` (the tenant's settings, or the server's when the tenant has
    /// not set them). Error: a sentence for the administrator to read, containing no secrets.
    async fn test(&self, tenant: &TenantContext) -> Result<(), Error>;
}

/// Registry of every module's connection tests. Cheap to clone.
#[derive(Clone, Default)]
pub struct ConnectionProbes {
    entries: Arc<RwLock<Vec<Arc<dyn ConnectionProbe>>>>,
}

impl ConnectionProbes {
    pub(crate) fn register(
        &self,
        module: &'static str,
        probe: Arc<dyn ConnectionProbe>,
    ) -> Result<(), Error> {
        let connection = probe.connection();
        if !connection.starts_with(&format!("{module}.connection.")) {
            return Err(Error::internal(format!(
                "connection probe `{connection}` of module `{module}` must be named \
                 `{module}.connection.<kind>`"
            )));
        }
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        if entries.iter().any(|known| known.connection() == connection) {
            return Err(Error::internal(format!(
                "connection probe `{connection}` is registered twice"
            )));
        }
        entries.push(probe);
        Ok(())
    }

    fn find(&self, connection: &str) -> Option<Arc<dyn ConnectionProbe>> {
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|probe| probe.connection() == connection)
            .cloned()
    }
}

/// Connection of a setting: `<module>.connection.<kind>` from `<module>.connection.<kind>.<field>`.
pub(crate) fn connection_of(setting: &str) -> Option<&str> {
    let mut dots = setting.match_indices('.').map(|(index, _)| index);
    let (_, second, third) = (dots.next()?, dots.next()?, dots.next()?);
    let connection = &setting[..third];
    (setting[..second].ends_with(".connection")).then_some(connection)
}

/// Last use / error of a connection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Status {
    pub last_used_at: Option<Timestamp>,
    pub last_error: Option<String>,
}

/// Records the use of a connection (modules call it after each use). Cheap to clone.
#[derive(Clone, Copy, Default)]
pub struct ConnectionStatus;

impl ConnectionStatus {
    /// Connection `connection` of tenant `tenant` was just used; `error`: the error (containing no secrets),
    /// `None` on success (clears the old error). Never breaks the work in progress: if recording fails it
    /// only logs.
    pub async fn record(&self, tenant: &TenantContext, connection: &str, error: Option<&str>) {
        if let Err(failure) = write(tenant, connection, error).await {
            tracing::warn!(connection, error = %failure, "cannot record a connection's status");
        }
    }
}

async fn write(tenant: &TenantContext, connection: &str, error: Option<&str>) -> Result<(), Error> {
    let now = Timestamp::now();
    let error: Option<String> = error.map(|error| error.chars().take(ERROR_MAX).collect());
    let insert = SqlQuery::insert()
        .into_table(Alias::new(TABLE))
        .columns(["id", "tenant_id", "connection", "last_used_at", "last_error", "updated_at"])
        .values_panic([
            uuid::Uuid::now_v7().into(),
            tenant.id().as_uuid().into(),
            connection.into(),
            now.into(),
            error.into(),
            now.into(),
        ])
        .on_conflict(
            OnConflict::columns([Alias::new("tenant_id"), Alias::new("connection")])
                .update_columns([
                    Alias::new("last_used_at"),
                    Alias::new("last_error"),
                    Alias::new("updated_at"),
                ])
                .to_owned(),
        )
        .to_owned();
    tenant.database().write().execute(&insert).await?;
    Ok(())
}

/// Status of every connection of tenant `tenant`.
pub(crate) async fn statuses(tenant: &TenantContext) -> Result<HashMap<String, Status>, Error> {
    let select = SqlQuery::select()
        .columns(["connection", "last_used_at", "last_error"])
        .from(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .to_owned();
    let mut found = HashMap::new();
    for row in tenant.database().read().query_all(&select).await? {
        found.insert(
            row.try_get("", "connection")?,
            Status {
                last_used_at: row.try_get("", "last_used_at")?,
                last_error: row.try_get("", "last_error")?,
            },
        );
    }
    Ok(found)
}

/// The result of testing a connection.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct ConnectionTest {
    /// The connection tested, `<module>.connection.<kind>`.
    #[schema(example = "mail.connection.smtp")]
    pub connection: String,
    pub ok: bool,
    /// Why it failed, for the administrator.
    pub error: Option<String>,
    pub tested_at: Timestamp,
}

#[derive(Clone)]
pub(crate) struct ConnectionsState {
    pub probes: ConnectionProbes,
}

/// Test a connection.
///
/// Requires `kernel.settings.manage`. Tries the partner connection the setting `name` belongs to
/// (e.g. `mail.connection.smtp.url` tests the mail server) with this tenant's settings — or the
/// server's, for the ones not set — and records the result as `last_used_at` / `last_error`.
/// Gives up after 15 seconds.
#[utoipa::path(
    post,
    path = "/settings/{name}/test",
    extensions(("x-permissions" = json!(["kernel.settings.manage"]))),
    operation_id = "test_tenant_connection",
    security(("bearer" = [])),
    params(("name" = String, Path, description = "A setting of the connection, e.g. `mail.connection.smtp.url`.")),
    responses(
        (status = 200, description = "The result, whether the connection works or not.", body = Data<ConnectionTest>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.settings.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "The setting belongs to no connection that can be tested; code `kernel.connection_untestable`.", body = Problem),
    )
)]
async fn test_tenant_connection(
    State(state): State<Arc<ConnectionsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(name): Path<String>,
) -> Result<Data<ConnectionTest>, Error> {
    principal.require(&crate::tenancy::SETTINGS_MANAGE)?;
    Ok(Data { data: test(&state.probes, &tenant, &name).await? })
}

/// Tests the connection that setting `name` belongs to, with the settings of tenant `tenant`; records the
/// result like a use. REST and the MCP tool share it.
async fn test(
    probes: &ConnectionProbes,
    tenant: &TenantContext,
    name: &str,
) -> Result<ConnectionTest, Error> {
    let probe =
        connection_of(name).and_then(|connection| probes.find(connection)).ok_or_else(|| {
            Error::not_found(
                "kernel.connection_untestable",
                "Setting `{name}` belongs to no partner connection that can be tested.",
            )
            .with_param("name", name)
        })?;
    let error = match tokio::time::timeout(TEST_TIMEOUT, probe.test(tenant)).await {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(error.message()),
        Err(_) => Some("the partner did not answer within 15 seconds".to_owned()),
    };
    ConnectionStatus.record(tenant, probe.connection(), error.as_deref()).await;
    Ok(ConnectionTest {
        connection: probe.connection().to_owned(),
        ok: error.is_none(),
        error,
        tested_at: Timestamp::now(),
    })
}

/// Input of `kernel.test_connection`.
#[derive(Debug, Deserialize, validator::Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TestConnection {
    /// A setting of the connection, e.g. `mail.connection.smtp.url`.
    #[schema(example = "mail.connection.smtp.url")]
    name: String,
}

pub(crate) fn tools(probes: ConnectionProbes) -> Vec<crate::mcp::Tool> {
    vec![
        crate::mcp::Tool::new(
            "test_connection",
            "Test the partner connection a setting belongs to (e.g. `mail.connection.smtp.url` \
             tests the mail server) with the current tenant's settings, and record the result. \
             Answers `ok` and, when it failed, why. Gives up after 15 seconds.",
            move |tenant: TenantContext, _principal: Principal, input: TestConnection| {
                let probes = probes.clone();
                async move { test(&probes, &tenant, &input.name).await }
            },
        )
        .requires(crate::tenancy::SETTINGS_MANAGE),
    ]
}

pub(crate) fn routes(state: ConnectionsState) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(test_tenant_connection)).with_state(Arc::new(state))
}

/// Migration that creates the table `kernel_connection_status` (tenant level, every data database).
pub(crate) struct CreateConnectionStatus;

impl MigrationName for CreateConnectionStatus {
    fn name(&self) -> &str {
        "m0026_create_connection_status"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateConnectionStatus {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TABLE))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("connection", 128))
                    .col(schema::timestamp_with_time_zone_null("last_used_at"))
                    .col(schema::string_len_null("last_error", 2000))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_connection_status_connection")
                    .table(Alias::new(TABLE))
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("connection"))
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new(TABLE)).to_owned()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ket_noi_cua_cai_dat() {
        assert_eq!(connection_of("mail.connection.smtp.url"), Some("mail.connection.smtp"));
        assert_eq!(connection_of("ai.connection.text.api_key"), Some("ai.connection.text"));
        assert_eq!(connection_of("kernel.public_url"), None);
        assert_eq!(connection_of("mail.connection.smtp"), None);
    }
}
