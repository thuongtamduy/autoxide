//! Health checks: two endpoints for operations systems (Docker, Kubernetes, load
//! balancers) to ask about the process's condition.
//!
//! - `/health/live` - whether the process is alive. Touches nothing else: if
//!   it can answer, it is alive. On failure the operations system RESTARTS the process.
//! - `/health/ready` - whether it can serve requests: the database must answer. On failure
//!   the operations system STOPS SENDING requests to it (but does not restart it).
//!
//! On shutdown there is a "drain" period (`AUTOXIDE_HTTP_DRAIN_SECONDS`, [`Draining`]): the process keeps
//! serving normally but `/health/ready` returns 503, so the load balancer has time to stop sending new
//! requests before the process stops accepting connections - a deploy drops no requests.
//!
//! The two endpoints are outside `/api/v1` because they are not part of the business contract and
//! do not change with the API version.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::database::Database;
use crate::error::Error;
use crate::response::{Data, Problem};

/// The checker is called continuously, every few seconds: one query must not hang longer than this.
const READY_TIMEOUT: Duration = Duration::from_secs(2);
/// The result of the database query is reused for this long: a public endpoint, not rate limited, and
/// each query takes SQLite's single write connection - a caller hammering it must not slow down
/// real writes (D-157).
const READY_CACHE: Duration = Duration::from_secs(1);

/// The "process is shutting down" flag: set once, never cleared. Copies share one flag (`Arc`), so
/// where the stop signal is received sets the flag and the health check route sees it immediately.
#[derive(Debug, Clone, Default)]
pub struct Draining(Arc<AtomicBool>);

impl Draining {
    /// Starts draining: from now on `/health/ready` returns 503 `shutting_down`.
    pub fn start(&self) {
        // `Relaxed` đủ: cờ đứng một mình, không kèm dữ liệu nào khác phải thấy cùng lúc.
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether draining has started.
    pub fn is_draining(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// The database, with its most recent query (time of query, whether it answered).
#[derive(Clone)]
struct Readiness {
    database: Database,
    draining: Draining,
    // Mutex của tokio: giữ qua `.await` của lần hỏi, nên nhiều request cùng lúc chỉ hỏi database
    // một lần, các request kia chờ rồi dùng lại kết quả.
    last: Arc<tokio::sync::Mutex<Option<Checked>>>,
}

/// One database query: the time of the query, and the error (if any).
type Checked = (Instant, Result<(), String>);

/// Health of the service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
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

/// The two health check routes, mounted at the root (outside `/api/v1`). `draining` is the flag set by
/// whatever runs the server on receiving the stop signal; tests and commands that do not serve HTTP pass
/// `Draining::default()`.
pub fn health_routes(database: Database, draining: Draining) -> OpenApiRouter {
    let readiness = Readiness { database, draining, last: Arc::default() };
    OpenApiRouter::new().routes(routes!(live)).routes(routes!(ready)).with_state(readiness)
}

/// Liveness probe: is the process running?
///
/// Never touches the database. Restart the process when this fails.
#[utoipa::path(
    get,
    path = "/health/live",
    extensions(("x-public" = json!(true))),
    tag = "health",
    operation_id = "health_live",
    responses((status = 200, description = "The process is running.", body = Data<Health>))
)]
async fn live() -> Data<Health> {
    Health::ok()
}

/// Readiness probe: can the service handle requests?
///
/// Checks that the database answers. Stop sending traffic when this fails. While the process
/// is shutting down (`AUTOXIDE_HTTP_DRAIN_SECONDS`), it fails even though requests are still
/// handled, so load balancers move traffic away before the server stops accepting connections.
#[utoipa::path(
    get,
    path = "/health/ready",
    extensions(("x-public" = json!(true))),
    tag = "health",
    operation_id = "health_ready",
    responses(
        (status = 200, description = "Ready to handle requests.", body = Data<Health>),
        (status = 503, description = "Not ready: code `not_ready` when the database does not answer, `shutting_down` while the process is shutting down.", body = Problem),
    )
)]
async fn ready(State(readiness): State<Readiness>) -> Result<Data<Health>, Error> {
    // Đang tắt thì trả lời ngay, không hỏi database: kết quả không đổi được nữa.
    if readiness.draining.is_draining() {
        return Err(Error::unavailable(
            "shutting_down",
            "The service is shutting down: send requests to another instance.",
        ));
    }
    let mut last = readiness.last.lock().await;
    let fresh = last.as_ref().filter(|(at, _)| at.elapsed() < READY_CACHE).map(|(_, ok)| ok);
    let outcome = match fresh {
        Some(outcome) => outcome.clone(),
        None => {
            let outcome = match tokio::time::timeout(READY_TIMEOUT, readiness.database.ping()).await
            {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(error.to_string()),
                Err(_) => Err("database ping timed out".to_owned()),
            };
            *last = Some((Instant::now(), outcome.clone()));
            outcome
        }
    };
    drop(last);
    match outcome {
        Ok(()) => Ok(Health::ok()),
        // Nguyên nhân gốc gắn vào lỗi để vào log; client chỉ thấy câu chung.
        Err(cause) => Err(Error::unavailable(
            "not_ready",
            "The service cannot handle requests right now: the database is not answering.",
        )
        .with_source(cause)),
    }
}
