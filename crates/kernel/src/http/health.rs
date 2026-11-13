//! Health check: hai endpoint cho hệ thống vận hành (Docker, Kubernetes, bộ cân
//! bằng tải) hỏi tình trạng của tiến trình.
//!
//! - `/health/live` — tiến trình còn sống không. Không đụng tới thứ gì khác: nếu
//!   trả lời được là còn sống. Hỏng thì hệ thống vận hành KHỞI ĐỘNG LẠI tiến trình.
//! - `/health/ready` — có phục vụ được request không: database phải trả lời. Hỏng
//!   thì hệ thống vận hành NGỪNG GỬI request tới (nhưng không khởi động lại).
//!
//! Lúc tắt có một khoảng "rút" (`AUTOXIDE_HTTP_DRAIN_SECONDS`, [`Draining`]): tiến trình vẫn
//! phục vụ bình thường nhưng `/health/ready` trả 503, để bộ cân bằng tải kịp thôi gửi request
//! mới tới trước khi tiến trình ngừng nhận kết nối — deploy không làm rơi request nào.
//!
//! Hai endpoint nằm ngoài `/api/v1` vì chúng không thuộc hợp đồng nghiệp vụ và
//! không đổi theo phiên bản API.

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

/// Bộ kiểm tra gọi liên tục, vài giây một lần: không để một lần hỏi treo lâu hơn mức này.
const READY_TIMEOUT: Duration = Duration::from_secs(2);
/// Kết quả hỏi database dùng lại trong chừng này: endpoint công khai, không bị rate limit, mà
/// mỗi lần hỏi chiếm kết nối ghi duy nhất của SQLite — một người gọi dồn dập không được làm
/// chậm các lệnh ghi thật (D-157).
const READY_CACHE: Duration = Duration::from_secs(1);

/// Cờ "tiến trình đang tắt": bật một lần, không tắt lại. Bản sao dùng chung một cờ (`Arc`), nên
/// nơi nhận tín hiệu dừng bật cờ và route health check thấy ngay.
#[derive(Debug, Clone, Default)]
pub struct Draining(Arc<AtomicBool>);

impl Draining {
    /// Bắt đầu rút: từ giờ `/health/ready` trả 503 `shutting_down`.
    pub fn start(&self) {
        // `Relaxed` đủ: cờ đứng một mình, không kèm dữ liệu nào khác phải thấy cùng lúc.
        self.0.store(true, Ordering::Relaxed);
    }

    /// Đã bắt đầu rút chưa.
    pub fn is_draining(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Database, kèm lần hỏi gần nhất (lúc hỏi, có trả lời không).
#[derive(Clone)]
struct Readiness {
    database: Database,
    draining: Draining,
    // Mutex của tokio: giữ qua `.await` của lần hỏi, nên nhiều request cùng lúc chỉ hỏi database
    // một lần, các request kia chờ rồi dùng lại kết quả.
    last: Arc<tokio::sync::Mutex<Option<Checked>>>,
}

/// Một lần hỏi database: lúc hỏi, và lỗi (nếu có).
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

/// Hai route health check, gắn ở gốc (ngoài `/api/v1`). `draining` là cờ do nơi chạy máy chủ bật
/// khi nhận tín hiệu dừng; test và lệnh không phục vụ HTTP truyền `Draining::default()`.
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
