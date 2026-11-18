//! Rate limit theo API key đếm chung qua database (bước 40 P6, D-373): hai máy chủ dùng chung một
//! database chia nhau một hạn mức, mỗi khách đếm riêng. Chạy trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use autoxide_kernel::access::{Authenticator, Principal};
use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig, HttpConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{self, MCP_PATH};
use autoxide_kernel::module::Modules;
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantId};
use autoxide_testkit::TestDatabase;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt as _;
use uuid::Uuid;

/// API key dùng chung trong mọi bài.
const KEY: u128 = 42;

/// Token `a` và `b`: cùng API key `KEY`, khác khách.
struct Keys;

#[async_trait::async_trait]
impl Authenticator for Keys {
    async fn authenticate(
        &self,
        authorization: Option<&str>,
        _: &autoxide_kernel::tenancy::TenantHint,
    ) -> Result<Principal, Error> {
        let tenant = match authorization {
            Some("Bearer a") => 1,
            Some("Bearer b") => 2,
            _ => return Err(Error::unauthenticated("invalid_token", "The token is not valid.")),
        };
        Ok(Principal::new(Uuid::from_u128(7), Vec::<String>::new())
            .via_api_key(Uuid::from_u128(KEY))
            .in_tenant(TenantId::from_uuid(Uuid::from_u128(tenant))))
    }
}

async fn database(test: &TestDatabase) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    Modules::new(Vec::new()).unwrap().migrator().unwrap().up(&database).await.unwrap();
    database
}

/// Một "máy chủ": router đầy đủ, 10 request mỗi phút cho mỗi API key, burst 4 (lô 1).
fn server(database: &Database, store: &str) -> axum::Router {
    let source = ConfigSource::from_pairs([
        ("AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_PER_MINUTE", "10"),
        ("AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_BURST", "4"),
        ("AUTOXIDE_HTTP_RATE_LIMIT_STORE", store),
        // Giới hạn theo địa chỉ không là thứ được đo ở đây.
        ("AUTOXIDE_HTTP_RATE_LIMIT_PER_MINUTE", "0"),
    ]);
    let mut reader = ConfigReader::new(&source);
    let config = HttpConfig::read(&mut reader);
    let config = reader.finish(config).unwrap().0;
    let extras = http::Extras::default()
        .authenticator(Some(Arc::new(Keys)))
        .tenancy(DatabaseRegistry::single(database.clone()));
    http::build(&config, axum::Router::new().into(), Vec::new(), extras).router
}

/// Một request có xác thực (lớp xác thực của `/mcp` đếm như REST); `true` khi không bị 429.
async fn allowed(router: &axum::Router, token: &str) -> bool {
    let request = Request::post(MCP_PATH)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let status = router.clone().oneshot(request).await.unwrap().status();
    assert_ne!(status, StatusCode::UNAUTHORIZED);
    status != StatusCode::TOO_MANY_REQUESTS
}

#[tokio::test]
async fn hai_may_chu_chung_han_muc_api_key() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let (one, two) = (server(&database, "database"), server(&database, "database"));
        let mut passed = 0;
        for round in 0..30 {
            let router = if round % 2 == 0 { &one } else { &two };
            if allowed(router, "a").await {
                passed += 1;
            }
        }
        // Hạn mức 10 mỗi phút, lô 1, hai máy chủ: sai số tối đa 1 × 2.
        assert!((10..=12).contains(&passed), "{passed} requests got through");

        // Đếm trong bộ nhớ (mặc định): mỗi máy chủ đếm riêng — đúng điều `database` sửa.
        let (one, two) = (server(&database, "memory"), server(&database, "memory"));
        let mut passed = 0;
        for round in 0..30 {
            let router = if round % 2 == 0 { &one } else { &two };
            if allowed(router, "a").await {
                passed += 1;
            }
        }
        assert!(passed >= 8, "{passed}: each process lets its own burst through");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khoa_co_khach_khong_tinh_chung_giua_khach() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let router = server(&database, "database");
        let mut passed = 0;
        for _ in 0..12 {
            if allowed(&router, "a").await {
                passed += 1;
            }
        }
        assert_eq!(passed, 10);
        // Cùng ID API key nhưng ở khách khác: còn đủ hạn mức của mình.
        for _ in 0..10 {
            assert!(allowed(&router, "b").await);
        }
        assert!(!allowed(&router, "b").await);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lay_lo_dong_thoi_tren_mot_may_chu_khong_vuot_limit() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let router = server(&database, "database");
        let tasks: Vec<_> = (0..30)
            .map(|_| {
                let router = router.clone();
                tokio::spawn(async move { allowed(&router, "a").await })
            })
            .collect();
        let mut passed = 0;
        for task in tasks {
            if task.await.unwrap() {
                passed += 1;
            }
        }
        assert_eq!(passed, 10);
    })
    .await
    .unwrap();
}
