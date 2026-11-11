//! Health check trên database đang chọn (gate chạy cả SQLite và Postgres).

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
// Clippy chỉ tự miễn cho hàm `#[test]`, không miễn cho hàm phụ, nên khai ở đây.
// Trong `src/` thì KHÔNG được khai dòng này (`cargo xtask check` chặn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
use autoxide_kernel::database::Database;
use autoxide_kernel::http::{self, Api};
use autoxide_testkit::TestDatabase;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

async fn start(database: &TestDatabase) -> (Api, Database) {
    let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = Config::read(&mut reader);
    let config = reader.finish(config).unwrap().0;
    let db = Database::connect(&config.database).await.unwrap();
    let api = http::build(&config.http, http::health_routes(db.clone()), Vec::new());
    (api, db)
}

async fn get(api: &Api, path: &str) -> (StatusCode, String, serde_json::Value) {
    let request = Request::builder().uri(path).body(Body::empty()).unwrap();
    let response = api.router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let content_type = response.headers()[header::CONTENT_TYPE].to_str().unwrap().to_owned();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, content_type, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn song_va_san_sang_khi_database_tra_loi() {
    TestDatabase::run(async |database| {
        let (api, _db) = start(database).await;
        let ok = serde_json::json!({"data": {"status": "ok"}});

        assert_eq!(
            get(&api, "/health/live").await,
            (StatusCode::OK, "application/json".to_owned(), ok.clone())
        );
        assert_eq!(
            get(&api, "/health/ready").await,
            (StatusCode::OK, "application/json".to_owned(), ok)
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn database_khong_tra_loi_thi_chua_san_sang_nhung_van_song() {
    TestDatabase::run(async |database| {
        let (api, db) = start(database).await;
        // Đóng pool: mọi bản sao của `Database` dùng chung pool nên route health
        // check cũng mất kết nối, như khi database thật ngừng trả lời.
        db.close().await.unwrap();

        let (status, content_type, body) = get(&api, "/health/ready").await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(content_type, "application/problem+json");
        assert_eq!(body["code"], "not_ready");
        assert_eq!(body["status"], 503);
        // Client không thấy chi tiết lỗi của database.
        let detail = body["detail"].as_str().unwrap();
        assert_eq!(
            detail,
            "The service cannot handle requests right now: the database is not answering."
        );

        // Tiến trình vẫn sống: không được khởi động lại chỉ vì database tạm mất.
        assert_eq!(get(&api, "/health/live").await.0, StatusCode::OK);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn health_check_co_trong_tai_lieu_openapi() {
    TestDatabase::run(async |database| {
        let (api, _db) = start(database).await;

        let document = serde_json::to_value(&api.openapi).unwrap();

        let live = &document["paths"]["/health/live"]["get"];
        assert_eq!(live["tags"], serde_json::json!(["health"]));
        assert_eq!(live["operationId"], "health_live");
        assert_eq!(live["summary"], "Liveness probe: is the process running?");
        assert!(document["paths"]["/health/ready"]["get"]["responses"]["503"].is_object());
        assert_eq!(http::lint(&api.openapi), Vec::<String>::new());
    })
    .await
    .unwrap();
}
