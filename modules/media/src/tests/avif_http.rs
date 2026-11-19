//! Bản AVIF của ảnh thu nhỏ (bước 39 B8, D-20261119-step39-avif) qua đường thật: router của kernel,
//! worker chạy job. Gate chạy trên cả SQLite và Postgres.

use std::sync::Arc;

use autoxide_kernel::ai::TextModel;
use autoxide_kernel::storage::Storage;
use autoxide_testkit::ai::FakeTextModel;
use autoxide_testkit::{MemoryStorage, TestApp, TestDatabase, TestResponse};
use axum::http::{Method, StatusCode, header};
use image::ImageFormat;
use sea_orm::{ConnectionTrait, Statement};
use serde_json::Value;

use crate::images::variants::samples::encoded;

async fn app(test: &TestDatabase, avif: bool) -> TestApp {
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::default());
    let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
    TestApp::builder(test)
        .setting("AUTOXIDE_MEDIA_IMAGE_WIDTHS", "100")
        .setting("AUTOXIDE_MEDIA_AVIF", if avif { "true" } else { "false" })
        .user("admin", 1, &["media.read", "media.write", "media.purge"])
        .provide::<dyn Storage>(storage)
        .provide::<dyn TextModel>(text)
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

async fn send(app: &TestApp, method: Method, uri: &str, body: Vec<u8>) -> TestResponse {
    let mut request = app.request(method, &format!("/api/v1/media{uri}"));
    request = request.header("authorization", "Bearer admin");
    if !body.is_empty() {
        request = request.header("content-type", "image/png");
    }
    request.body(axum::body::Body::from(body)).send().await.unwrap()
}

async fn upload(app: &TestApp, name: &str, bytes: &[u8]) -> Value {
    let answer = send(app, Method::POST, &format!("?name={name}"), bytes.to_vec()).await;
    assert_eq!(answer.status(), StatusCode::CREATED, "{}", answer.json());
    answer.json()["data"].clone()
}

/// Worker chạy mọi job đang chờ.
async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap();
    while worker.run_once().await.unwrap() > 0 {}
}

async fn variants(app: &TestApp, id: &str) -> Value {
    send(app, Method::GET, &format!("/{id}"), Vec::new()).await.json()["data"]["variants"].clone()
}

async fn avif_rows(app: &TestApp) -> usize {
    let database = app.database();
    let backend = database.read().get_database_backend();
    let sql = "SELECT id FROM media_variant WHERE content_type = 'image/avif'";
    database.read().query_all_raw(Statement::from_string(backend, sql)).await.unwrap().len()
}

#[tokio::test]
async fn avif_tat_thi_khong_sinh() {
    TestDatabase::run(async |test| {
        let app = app(test, false).await;
        let png = encoded(240, 120, ImageFormat::Png, false);
        let item = upload(&app, "wide.png", &png).await;
        drain(&app).await;
        let variants = variants(&app, item["id"].as_str().unwrap()).await;
        assert_eq!(variants.as_array().unwrap().len(), 1, "{variants}");
        assert!(variants[0]["webp_url"].is_string());
        assert!(variants[0]["avif_url"].is_null());
        assert_eq!(avif_rows(&app).await, 0);
    })
    .await
    .unwrap();
}

/// Bật AVIF: ngay sau khi tải lên chưa có bản AVIF (không làm trên đường request); worker làm
/// ảnh thu nhỏ rồi bản AVIF; `format=avif` trả đúng bản AVIF với ETag riêng. Chạy lại job, ảnh
/// hỏng, hai worker cùng lúc: `crate::tests::avif`.
#[tokio::test]
async fn avif_bat_thi_job_sinh_dung_mot_lan() {
    TestDatabase::run(async |test| {
        let app = app(test, true).await;
        let png = encoded(240, 120, ImageFormat::Png, false);
        let item = upload(&app, "wide.png", &png).await;
        // `avif_khong_sinh_tren_request_tai_len`: chưa có gì ngay sau khi tải lên.
        assert_eq!(avif_rows(&app).await, 0);
        drain(&app).await;
        let id = item["id"].as_str().unwrap();
        let variants = variants(&app, id).await;
        let url = variants[0]["avif_url"].as_str().unwrap().trim_start_matches("/api/v1/media");
        assert!(url.ends_with("&w=100&format=avif"), "{url}");
        assert!(variants[0]["avif_size"].as_u64().unwrap() > 0);
        let got = send(&app, Method::GET, url, Vec::new()).await;
        assert_eq!(got.status(), StatusCode::OK);
        assert_eq!(got.header(header::CONTENT_TYPE), "image/avif");
        assert!(got.header(header::ETAG).ends_with("-avif\""), "{}", got.header(header::ETAG));
        assert_eq!(got.header(header::CACHE_CONTROL), "private, max-age=31536000, immutable");
        assert_eq!(&got.body()[4..8], b"ftyp", "an ISO-BMFF (AVIF) file");
        assert_eq!(avif_rows(&app).await, 1);
    })
    .await
    .unwrap();
}

/// Xin `format=avif` khi chưa có bản AVIF (máy chủ không bật): trả bản WebP cùng cỡ, cache ngắn.
#[tokio::test]
async fn format_avif_chua_co_tra_ban_webp() {
    TestDatabase::run(async |test| {
        let app = app(test, false).await;
        let png = encoded(240, 120, ImageFormat::Png, false);
        let item = upload(&app, "wide.png", &png).await;
        drain(&app).await;
        let variants = variants(&app, item["id"].as_str().unwrap()).await;
        let url = variants[0]["url"].as_str().unwrap().trim_start_matches("/api/v1/media");
        let got = send(&app, Method::GET, &format!("{url}&format=avif"), Vec::new()).await;
        assert_eq!(got.status(), StatusCode::OK);
        assert_eq!(got.header(header::CONTENT_TYPE), "image/webp");
        assert_eq!(got.header(header::CACHE_CONTROL), "private, no-cache");
        let wrong = send(&app, Method::GET, &format!("{url}&format=heic"), Vec::new()).await;
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}
