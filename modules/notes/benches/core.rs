//! Bộ đo hiệu năng của core (D-071), qua toàn bộ đường đi của một request thật: các
//! lớp HTTP của kernel → handler → service → database, dùng module mẫu `notes`.
//!
//! ```text
//! cargo bench -p autoxide-notes                                   # SQLite
//! TEST_DATABASE_URL=postgres://… cargo bench -p autoxide-notes    # Postgres
//! ```
//!
//! Đo bốn việc: đọc một ghi chú, list một trang 50 dòng, tạo ghi chú (kèm event trong
//! cùng transaction), và tạo ghi chú từ 16 task cùng lúc. In số việc mỗi giây và thời
//! gian p50 / p95 / p99. Số gốc để so: `docs/BENCHMARKS.md`.

// Bộ đo là code công cụ: hỏng thì dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use autoxide_kernel::access::{Authenticator, Principal};
use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::module::Modules;
use autoxide_testkit::TestDatabase;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt as _;
use uuid::Uuid;

/// Số lần đo mỗi việc (sau khi chạy nóng).
const ITERATIONS: usize = 2000;
/// Số task ghi cùng lúc.
const CONCURRENCY: usize = 16;

struct Writer;

#[async_trait::async_trait]
impl Authenticator for Writer {
    async fn authenticate(&self, _: Option<&str>) -> Result<Principal, Error> {
        let permissions = ["notes.read".to_owned(), "notes.write".to_owned()];
        Ok(Principal::new(Uuid::from_u128(1), permissions))
    }
}

async fn app(test: &TestDatabase) -> (Router, Modules, Database) {
    let source = ConfigSource::from_pairs([
        ("DATABASE_URL", test.url()),
        // Đo database và code, không đo bộ giới hạn request.
        ("AUTOXIDE_HTTP_RATE_LIMIT_PER_MINUTE", "0"),
    ]);
    let mut reader = ConfigReader::new(&source);
    let config = Config::read(&mut reader);
    let (config, _) = reader.finish(config).unwrap();
    let database = Database::connect(&config.database).await.unwrap();
    let mut modules = Modules::new(vec![Box::new(autoxide_notes::module())]).unwrap();
    modules.migrator().unwrap().up(&database).await.unwrap();
    modules.start(&database).unwrap();
    let extras = autoxide_kernel::http::Extras::default()
        .authenticator(Some(Arc::new(Writer)))
        .translations(modules.translations().clone());
    let routes = modules.routes();
    let router = autoxide_kernel::http::build(
        &config.http,
        utoipa_axum::router::OpenApiRouter::new(),
        routes,
        extras,
    )
    .router;
    (router, modules, database)
}

async fn send(router: &Router, method: &str, uri: &str, body: Option<String>) -> serde_json::Value {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, "Bearer bench")
        .header(header::CONTENT_TYPE, "application/json")
        .body(body.map_or_else(Body::empty, Body::from))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(status == StatusCode::OK || status == StatusCode::CREATED, "{status}: {bytes:?}");
    serde_json::from_slice(&bytes).unwrap()
}

fn create(index: usize) -> Option<String> {
    Some(format!(r#"{{"title": "note {index} {}", "body": "benchmark"}}"#, Uuid::now_v7()))
}

/// In một dòng kết quả từ các thời gian đo được.
fn report(name: &str, mut samples: Vec<Duration>, elapsed: Duration) {
    samples.sort();
    let at = |fraction: f64| {
        let index = ((samples.len() as f64 - 1.0) * fraction).round() as usize;
        samples[index].as_secs_f64() * 1000.0
    };
    let per_second = samples.len() as f64 / elapsed.as_secs_f64();
    println!(
        "{name:<28} {per_second:>9.0}/s   p50 {:>7.3} ms   p95 {:>7.3} ms   p99 {:>7.3} ms",
        at(0.50),
        at(0.95),
        at(0.99)
    );
}

async fn measure<F, Fut>(name: &str, mut operation: F)
where
    F: FnMut(usize) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    for index in 0..ITERATIONS / 10 {
        operation(index).await;
    }
    let mut samples = Vec::with_capacity(ITERATIONS);
    let started = Instant::now();
    for index in 0..ITERATIONS {
        let start = Instant::now();
        operation(index).await;
        samples.push(start.elapsed());
    }
    report(name, samples, started.elapsed());
}

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        TestDatabase::run(async |test| {
            let (router, _modules, _database) = app(test).await;
            let backend = if test.url().starts_with("postgres") { "PostgreSQL" } else { "SQLite" };
            println!("Autoxide core benchmark — {backend}, {ITERATIONS} operations each");

            measure("write (create note)", |index| {
                let router = router.clone();
                async move {
                    send(&router, "POST", "/api/v1/notes", create(index)).await;
                }
            })
            .await;

            let created = send(&router, "POST", "/api/v1/notes", create(usize::MAX)).await;
            let id = created["data"]["id"].as_str().unwrap().to_owned();
            measure("read (get note by id)", |_| {
                let (router, uri) = (router.clone(), format!("/api/v1/notes/{id}"));
                async move {
                    send(&router, "GET", &uri, None).await;
                }
            })
            .await;

            measure("list (page of 50)", |_| {
                let router = router.clone();
                async move {
                    let page = send(&router, "GET", "/api/v1/notes?limit=50", None).await;
                    assert_eq!(page["data"].as_array().unwrap().len(), 50);
                }
            })
            .await;

            // Ghi đồng thời: CONCURRENCY task, mỗi task một phần việc.
            let mut samples = Vec::with_capacity(ITERATIONS);
            let started = Instant::now();
            let tasks: Vec<_> = (0..CONCURRENCY)
                .map(|worker| {
                    let router = router.clone();
                    tokio::spawn(async move {
                        let mut times = Vec::new();
                        for index in 0..ITERATIONS / CONCURRENCY {
                            let start = Instant::now();
                            send(
                                &router,
                                "POST",
                                "/api/v1/notes",
                                create(worker * 100_000 + index),
                            )
                            .await;
                            times.push(start.elapsed());
                        }
                        times
                    })
                })
                .collect();
            for task in tasks {
                samples.extend(task.await.unwrap());
            }
            report(&format!("concurrent write ({CONCURRENCY} tasks)"), samples, started.elapsed());
        })
        .await
        .unwrap();
    });
}
