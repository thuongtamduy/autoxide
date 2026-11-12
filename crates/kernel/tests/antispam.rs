//! Chống spam hai pha: bẫy, token form (thiếu, sai, khác form, hết hạn, quá nhanh), giới hạn
//! theo IP băm và theo người dùng, trùng; nonce chèn trong transaction là idempotency cho người
//! lạ (gửi lại → kết quả cũ, đồng thời → đúng một bản); endpoint lấy token. Chạy trên cả SQLite
//! và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::Principal;
use autoxide_kernel::antispam::{
    AntispamConfig, Claim, Limits, SpamGuard, Submission, Verdict, antispam_migration, content_hash,
};
use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, Modules};
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::throttle::{Throttle, throttle_migration};
use autoxide_testkit::TestDatabase;
use uuid::Uuid;

struct Demo;

impl Module for Demo {
    fn name(&self) -> &'static str {
        "demo"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            throttle_migration("m0001_create_throttle", "demo"),
            antispam_migration("m0002_create_nonces", "demo"),
        ]
    }
}

async fn setup(test: &TestDatabase) -> (Database, Modules) {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    let mut modules = Modules::new(vec![Box::new(Demo)]).unwrap();
    modules.migrator().unwrap().up(&database).await.unwrap();
    modules.start(&database).unwrap();
    (database, modules)
}

/// Bộ chống spam với cấu hình riêng (không chờ 3 giây trong test).
fn make_guard(database: &Database, config: AntispamConfig) -> SpamGuard {
    SpamGuard::new(SecretKey::new(database.clone(), None), config)
}

fn quick() -> AntispamConfig {
    AntispamConfig { min_fill: Duration::ZERO, ..AntispamConfig::default() }
}

fn client(ip: &str) -> ClientInfo {
    ClientInfo { ip: Some(ip.parse().unwrap()), ..ClientInfo::default() }
}

fn submission<'a>(client: &'a ClientInfo, token: Option<&'a str>) -> Submission<'a> {
    Submission {
        scope: "demo:contact",
        principal: None,
        client,
        honeypot: None,
        form_token: token,
        challenge: None,
        email: None,
        content: Some("Hello"),
        duplicate: false,
    }
}

#[tokio::test]
async fn bay_token_va_qua_nhanh() {
    TestDatabase::run(async |test| {
        let (database, modules) = setup(test).await;
        let visitor = client("203.0.113.9");
        let limits = Limits::default();
        // Bộ công bố bởi module nội bộ `antispam`: mặc định giữ lại bài gửi trong 3 giây đầu.
        let default = modules.api::<SpamGuard>().unwrap();
        let issued = default.issue("demo:contact").await.unwrap();
        assert!(issued.submit_after > chrono::Utc::now());
        let checked =
            default.check(&submission(&visitor, Some(&issued.token)), &limits).await.unwrap();
        assert_eq!(checked.verdict, Verdict::Quarantine { reasons: vec!["too_fast"] });

        let guard = make_guard(&database, quick());
        let token = guard.issue("demo:contact").await.unwrap().token;
        let mut trapped = submission(&visitor, Some(&token));
        trapped.honeypot = Some("http://spam.example");
        assert_eq!(
            guard.check(&trapped, &limits).await.unwrap().verdict,
            Verdict::Discard { reason: "honeypot" }
        );
        let missing = guard.check(&submission(&visitor, None), &limits).await.unwrap_err();
        assert_eq!(missing.code(), "antispam.token_missing");
        let other = guard.issue("demo:other").await.unwrap().token;
        assert_eq!(
            guard.check(&submission(&visitor, Some(&other)), &limits).await.unwrap_err().code(),
            "antispam.token_invalid"
        );
        assert_eq!(
            guard.check(&submission(&visitor, Some("x.y")), &limits).await.unwrap_err().code(),
            "antispam.token_invalid"
        );
        let mut duplicate = submission(&visitor, Some(&token));
        duplicate.duplicate = true;
        assert_eq!(
            guard.check(&duplicate, &limits).await.unwrap().verdict,
            Verdict::Quarantine { reasons: vec!["duplicate"] }
        );
        assert_eq!(
            guard.check(&submission(&visitor, Some(&token)), &limits).await.unwrap().verdict,
            Verdict::Accept
        );
        assert_eq!(content_hash("  Hello\n WORLD "), content_hash("hello world"));

        // Token sống 1 giây: sau 2 giây là hết hạn.
        let short = make_guard(
            &database,
            AntispamConfig { token_max_age: Duration::from_secs(1), ..quick() },
        );
        let token = short.issue("demo:contact").await.unwrap().token;
        tokio::time::sleep(Duration::from_millis(2100)).await;
        assert_eq!(
            short.check(&submission(&visitor, Some(&token)), &limits).await.unwrap_err().code(),
            "antispam.token_expired"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gioi_han_theo_ip_va_nguoi_dung() {
    TestDatabase::run(async |test| {
        let (database, _) = setup(test).await;
        let guard = make_guard(&database, quick());
        let throttle = |limit| {
            Throttle::new(database.clone(), "demo", limit, Duration::from_secs(3600)).unwrap()
        };
        let limits = Limits { ip: Some(throttle(2)), email: None, user: Some(throttle(1)) };
        // Cùng khối /24: chung bộ đếm.
        for ip in ["203.0.113.9", "203.0.113.10"] {
            let token = guard.issue("demo:contact").await.unwrap().token;
            let visitor = client(ip);
            assert_eq!(
                guard.check(&submission(&visitor, Some(&token)), &limits).await.unwrap().verdict,
                Verdict::Accept
            );
        }
        let token = guard.issue("demo:contact").await.unwrap().token;
        let visitor = client("203.0.113.11");
        let third = guard.check(&submission(&visitor, Some(&token)), &limits).await.unwrap();
        assert_eq!(
            third.verdict,
            Verdict::Discard { reason: "rate_limited" },
            "no error a bot could read"
        );
        let elsewhere = client("198.51.100.1");
        assert_eq!(
            guard.check(&submission(&elsewhere, Some(&token)), &limits).await.unwrap().verdict,
            Verdict::Accept
        );

        // Người gọi đã đăng nhập: không cần token, nhưng bị giới hạn và nhận 429 rõ ràng.
        let website = Principal::new(Uuid::now_v7(), Vec::<String>::new());
        let mut signed = submission(&visitor, None);
        signed.principal = Some(&website);
        assert_eq!(guard.check(&signed, &limits).await.unwrap().verdict, Verdict::Accept);
        let error = guard.check(&signed, &limits).await.unwrap_err();
        assert_eq!((error.code(), error.retry_after().is_some()), ("antispam.rate_limited", true));
    })
    .await
    .unwrap();
}

/// Gửi lại cùng token: lần sau nhận kết quả lần đầu; gửi đồng thời: đúng một bản ghi.
#[tokio::test]
async fn nonce_la_idempotency_cho_nguoi_la() {
    TestDatabase::run(async |test| {
        let (database, _) = setup(test).await;
        let guard = Arc::new(make_guard(&database, quick()));
        let visitor = client("203.0.113.9");
        let token = guard.issue("demo:contact").await.unwrap().token;
        let first =
            guard.check(&submission(&visitor, Some(&token)), &Limits::default()).await.unwrap();
        let transaction = database.begin_write().await.unwrap();
        assert_eq!(first.claim(&transaction, "demo", Some("record-1")).await.unwrap(), Claim::New);
        transaction.commit().await.unwrap();
        let again =
            guard.check(&submission(&visitor, Some(&token)), &Limits::default()).await.unwrap();
        let replay = again.claim(database.write(), "demo", Some("record-2")).await.unwrap();
        assert_eq!(replay, Claim::Replay { result: Some("record-1".to_owned()) });

        let token = guard.issue("demo:contact").await.unwrap().token;
        let tasks: Vec<_> = (0..10)
            .map(|index| {
                let (guard, database, token, visitor) =
                    (guard.clone(), database.clone(), token.clone(), visitor.clone());
                tokio::spawn(async move {
                    let checked = guard
                        .check(&submission(&visitor, Some(&token)), &Limits::default())
                        .await
                        .unwrap();
                    let transaction = database.begin_write().await.unwrap();
                    let claim = checked
                        .claim(&transaction, "demo", Some(&format!("r{index}")))
                        .await
                        .unwrap();
                    transaction.commit().await.unwrap();
                    claim
                })
            })
            .collect();
        let mut new = 0;
        for task in tasks {
            if task.await.unwrap() == Claim::New {
                new += 1;
            }
        }
        assert_eq!(new, 1);
        assert_eq!(
            guard.purge_nonces(database.write(), "demo").await.unwrap(),
            0,
            "nothing expired yet"
        );
    })
    .await
    .unwrap();
}

/// Endpoint công khai lấy token: không cần đăng nhập, không cache, phạm vi sai thì 422.
#[tokio::test]
async fn endpoint_lay_token() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use tower::ServiceExt as _;
    TestDatabase::run(async |test| {
        let (_, modules) = setup(test).await;
        let source = ConfigSource::default();
        let mut reader = ConfigReader::new(&source);
        let config = autoxide_kernel::config::HttpConfig::read(&mut reader);
        let http = reader.finish(config).unwrap().0;
        let api = autoxide_kernel::http::build(
            &http,
            utoipa_axum::router::OpenApiRouter::new(),
            modules.routes(),
            Default::default(),
        );
        let get = |uri: &str| Request::builder().uri(uri).body(Body::empty()).unwrap();
        let ok = api
            .router
            .clone()
            .oneshot(get("/api/v1/antispam/token?scope=demo:contact"))
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(ok.headers()[header::CACHE_CONTROL], "no-store");
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(ok.into_body(), usize::MAX).await.unwrap(),
        )
        .unwrap();
        assert!(body["data"]["token"].as_str().unwrap().contains('.'));
        assert!(body["data"]["challenge"].is_null());
        let bad = api
            .router
            .clone()
            .oneshot(get("/api/v1/antispam/token?scope=Bad%20Scope"))
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}
