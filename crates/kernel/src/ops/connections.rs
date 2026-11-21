//! Cài đặt có nối ra ngoài (bước 51, A-c10, A-c11): máy chủ mail, nhà cung cấp AI, cổng thanh toán…
//! của riêng khách — nhóm cấu hình `connection`, tên `<module>.connection.<loại>.<trường>`.
//!
//! - Module báo mỗi lần dùng kết nối bằng [`ConnectionStatus::record`] (lần dùng cuối, lỗi cuối);
//!   `GET /api/v1/kernel/settings` trả `last_used_at`, `last_error` cho từng cài đặt của kết nối.
//! - Module khai cách thử bằng [`StartContext::connection_probe`](crate::module::StartContext);
//!   `POST /api/v1/kernel/settings/{name}/test` gọi thử ngay (không trong transaction ghi, có hạn
//!   giờ) và ghi kết quả như một lần dùng.
//!
//! Bảng `kernel_connection_status` (cấp khách hàng): một dòng mỗi (khách, kết nối).

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
/// Thử kết nối chờ tối đa chừng này.
const TEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Lỗi cuối giữ tối đa chừng này ký tự.
const ERROR_MAX: usize = 500;

/// Cách thử một kết nối của module.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait ConnectionProbe: Send + Sync {
    /// Kết nối, `<module>.connection.<loại>`, vd `mail.connection.smtp`.
    fn connection(&self) -> &'static str;
    /// Thử kết nối của khách `tenant` (cài đặt của khách, hay của máy chủ khi khách chưa đặt).
    /// Lỗi: câu cho người quản trị đọc, không chứa secret.
    async fn test(&self, tenant: &TenantContext) -> Result<(), Error>;
}

/// Sổ cách thử kết nối của mọi module. Clone rẻ.
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

/// Kết nối của một cài đặt: `<module>.connection.<loại>` từ `<module>.connection.<loại>.<trường>`.
pub(crate) fn connection_of(setting: &str) -> Option<&str> {
    let mut dots = setting.match_indices('.').map(|(index, _)| index);
    let (_, second, third) = (dots.next()?, dots.next()?, dots.next()?);
    let connection = &setting[..third];
    (setting[..second].ends_with(".connection")).then_some(connection)
}

/// Lần dùng / lỗi cuối của một kết nối.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Status {
    pub last_used_at: Option<Timestamp>,
    pub last_error: Option<String>,
}

/// Ghi lần dùng của kết nối (module gọi sau mỗi lần dùng). Clone rẻ.
#[derive(Clone, Copy, Default)]
pub struct ConnectionStatus;

impl ConnectionStatus {
    /// Kết nối `connection` của khách `tenant` vừa được dùng; `error`: lỗi (không chứa secret),
    /// `None` khi thành công (xoá lỗi cũ). Không bao giờ làm hỏng việc đang làm: ghi lỗi thì chỉ log.
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

/// Trạng thái mọi kết nối của khách `tenant`.
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
    let probe = connection_of(&name)
        .and_then(|connection| state.probes.find(connection))
        .ok_or_else(|| {
            Error::not_found(
                "kernel.connection_untestable",
                "Setting `{name}` belongs to no partner connection that can be tested.",
            )
            .with_param("name", name.clone())
        })?;
    let error = match tokio::time::timeout(TEST_TIMEOUT, probe.test(&tenant)).await {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(error.message()),
        Err(_) => Some("the partner did not answer within 15 seconds".to_owned()),
    };
    ConnectionStatus.record(&tenant, probe.connection(), error.as_deref()).await;
    Ok(Data {
        data: ConnectionTest {
            connection: probe.connection().to_owned(),
            ok: error.is_none(),
            error,
            tested_at: Timestamp::now(),
        },
    })
}

pub(crate) fn routes(state: ConnectionsState) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(test_tenant_connection)).with_state(Arc::new(state))
}

/// Migration tạo bảng `kernel_connection_status` (cấp khách hàng, mọi database dữ liệu).
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
