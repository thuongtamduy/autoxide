//! Health check: hai endpoint cho hệ thống vận hành (Docker, Kubernetes, bộ cân
//! bằng tải) hỏi tình trạng của tiến trình.
//!
//! - `/health/live` — tiến trình còn sống không. Không đụng tới thứ gì khác: nếu
//!   trả lời được là còn sống. Hỏng thì hệ thống vận hành KHỞI ĐỘNG LẠI tiến trình.
//! - `/health/ready` — có phục vụ được request không: database phải trả lời. Hỏng
//!   thì hệ thống vận hành NGỪNG GỬI request tới (nhưng không khởi động lại).
//!
//! Hai endpoint nằm ngoài `/api/v1` vì chúng không thuộc hợp đồng nghiệp vụ và
//! không đổi theo phiên bản API.

use std::time::Duration;

use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::database::Database;
use crate::error::Error;
use crate::response::{Data, Problem};

/// Bộ kiểm tra gọi liên tục, vài giây một lần: không để một lần hỏi treo lâu hơn mức này.
const READY_TIMEOUT: Duration = Duration::from_secs(2);

/// Health of the service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Health {
    /// Always `ok`; a service that is not healthy answers with an error instead.
    #[schema(example = "ok")]
    pub status: String,
}

impl Health {
    fn ok() -> Data<Health> {
        Data { data: Health { status: "ok".to_owned() } }
    }
}

/// Hai route health check, gắn ở gốc (ngoài `/api/v1`).
pub fn health_routes(database: Database) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(live)).routes(routes!(ready)).with_state(database)
}

/// Liveness probe: is the process running?
///
/// Never touches the database. Restart the process when this fails.
#[utoipa::path(
    get,
    path = "/health/live",
    tag = "health",
    operation_id = "health_live",
    responses((status = 200, description = "The process is running.", body = Data<Health>))
)]
async fn live() -> Data<Health> {
    Health::ok()
}

/// Readiness probe: can the service handle requests?
///
/// Checks that the database answers. Stop sending traffic when this fails.
#[utoipa::path(
    get,
    path = "/health/ready",
    tag = "health",
    operation_id = "health_ready",
    responses(
        (status = 200, description = "Ready to handle requests.", body = Data<Health>),
        (status = 503, description = "A dependency is unavailable; code `not_ready`.", body = Problem),
    )
)]
async fn ready(State(database): State<Database>) -> Result<Data<Health>, Error> {
    let not_ready = || {
        Error::unavailable(
            "not_ready",
            "The service cannot handle requests right now: the database is not answering.",
        )
    };
    match tokio::time::timeout(READY_TIMEOUT, database.ping()).await {
        Ok(Ok(())) => Ok(Health::ok()),
        // Nguyên nhân gốc gắn vào lỗi để vào log; client chỉ thấy câu chung ở trên.
        Ok(Err(error)) => Err(not_ready().with_source(error)),
        Err(_) => Err(not_ready().with_source("database ping timed out")),
    }
}
