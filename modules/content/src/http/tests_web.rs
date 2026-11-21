//! Đề bài giao diện B4 (D-266): bộ lọc mới của danh sách bài, `published_at` / `path` /
//! `version`, số bài của danh mục và tác giả, tìm theo chữ, lịch đăng của mọi bài, câu kiểm SEO
//! theo ngôn ngữ, ảnh trong bản xem trước, lịch đăng mang `ai_reviewed`.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::ai::{Embedder, TextModel};
use autoxide_media::api::Media;
use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};
use autoxide_testkit::{TestApp, TestDatabase, TestResponse};
use axum::http::{Method, StatusCode, header};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::testing::FakeMedia;

const SECRET: &str = "a-test-secret-that-is-long-enough-0123456789";
const ALL: [&str; 5] =
    ["content.read", "content.write", "content.publish", "content.delete", "content.manage"];

async fn app(test: &TestDatabase, media: Arc<FakeMedia>) -> TestApp {
    let media: Arc<dyn Media> = media;
    let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
    TestApp::builder(test)
        .setting("AUTOXIDE_CONTENT_PREVIEW_SECRET", SECRET)
        .user("admin", 1, &ALL)
        .user("bob", 2, &ALL)
        .principal(
            "agent",
            Principal::new(Uuid::from_u128(3), ALL.map(str::to_owned))
                .via_api_key(Uuid::from_u128(4)),
        )
        .provide::<dyn Media>(media)
        .provide::<dyn TextModel>(text)
        .provide::<dyn Embedder>(embedder)
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

async fn send(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> TestResponse {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/content{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    request.send().await.unwrap()
}

async fn create(app: &TestApp, who: &str, body: Value) -> Value {
    let created = send(app, "POST", "/entries", who, Some(body)).await;
    assert_eq!(created.status(), StatusCode::CREATED, "{}", created.text());
    created.json()["data"].clone()
}

fn ids(page: &Value) -> Vec<String> {
    page["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn bo_loc_moi_va_truong_moi_cua_danh_sach_bai() {
    TestDatabase::run(async |test| {
        let app = app(test, Arc::new(FakeMedia::default())).await;
        let mine = create(&app, "admin", json!({"kind": "post", "title": "Của tôi"})).await;
        let ai =
            create(&app, "bob", json!({"kind": "post", "title": "AI viết", "ai_generated": true}))
                .await;
        let id = |entry: &Value| entry["id"].as_str().unwrap().to_owned();

        let page = send(&app, "GET", "/entries?created_by=me", "admin", None).await.json();
        assert_eq!(ids(&page), [id(&mine)]);
        let page = send(&app, "GET", "/entries?ai_generated=true", "admin", None).await.json();
        assert_eq!(ids(&page), [id(&ai)]);
        let bad = send(&app, "GET", "/entries?created_by=someone", "admin", None).await;
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);

        // Chờ duyệt.
        let entry = send(&app, "GET", &format!("/entries/{}", id(&ai)), "bob", None).await;
        let etag = entry.header(header::ETAG).to_owned();
        let asked = app
            .post(&format!("/api/v1/content/entries/{}/request-review", id(&ai)))
            .token("bob")
            .header("if-match", &etag)
            .json(json!({}))
            .send()
            .await
            .unwrap();
        assert!(asked.status().is_success(), "{}", asked.text());
        let page = send(&app, "GET", "/entries?review_requested=true", "admin", None).await.json();
        assert_eq!(ids(&page), [id(&ai)]);

        // Thiếu bản dịch tiếng Việt: cả hai; dịch bài của tôi thì chỉ còn bài AI.
        let page =
            send(&app, "GET", "/entries?missing_locale=vi&locale=en", "admin", None).await.json();
        assert_eq!(ids(&page).len(), 2);
        let path = format!("/entries/{}/translations", id(&mine));
        let made = send(
            &app,
            "POST",
            &path,
            "admin",
            Some(json!({"locale": "vi", "title": "Của tôi bản Việt"})),
        )
        .await;
        assert_eq!(made.status(), StatusCode::CREATED, "{}", made.text());
        let page =
            send(&app, "GET", "/entries?missing_locale=vi&locale=en", "admin", None).await.json();
        assert_eq!(ids(&page), [id(&ai)]);

        // Đăng: danh sách có `published_at`, `path`, `version` (dùng thẳng làm `If-Match`).
        let entry = send(&app, "GET", &format!("/entries/{}", id(&mine)), "admin", None).await;
        let etag = entry.header(header::ETAG).to_owned();
        let published = app
            .post(&format!("/api/v1/content/entries/{}/publish", id(&mine)))
            .token("admin")
            .header("if-match", &etag)
            .json(json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(published.status(), StatusCode::OK, "{}", published.text());
        let page =
            send(&app, "GET", "/entries?created_by=me&locale=en", "admin", None).await.json();
        let row = &page["data"][0];
        assert_eq!(row["path"], "/blog/cua-toi");
        assert!(row["published_at"].is_string());
        let fresh = send(&app, "GET", &format!("/entries/{}", id(&mine)), "admin", None).await;
        assert_eq!(row["version"], fresh.header(header::ETAG));
        let page = send(&app, "GET", "/entries?ai_generated=true", "admin", None).await.json();
        assert!(page["data"][0]["path"].is_null());

        // Lịch đăng: lọc `scheduled`, danh sách lịch của mọi bài, `ai_reviewed` chỉ của người.
        let entry = send(&app, "GET", &format!("/entries/{}", id(&ai)), "admin", None).await;
        let etag = entry.header(header::ETAG).to_owned();
        let at = (Timestamp::now() + SignedDuration::from_hours(24)).to_rfc3339();
        let schedule = json!({"action": "publish", "run_at": at, "ai_reviewed": true});
        let by_key = app
            .post(&format!("/api/v1/content/entries/{}/schedules", id(&ai)))
            .token("agent")
            .header("if-match", &etag)
            .json(schedule.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(by_key.json()["code"], "content.ai_review_requires_human");
        let made = app
            .post(&format!("/api/v1/content/entries/{}/schedules", id(&ai)))
            .token("admin")
            .header("if-match", &etag)
            .json(schedule)
            .send()
            .await
            .unwrap();
        assert_eq!(made.status(), StatusCode::CREATED, "{}", made.text());
        assert_eq!(made.json()["data"]["ai_reviewed"], true);
        let page = send(&app, "GET", "/entries?scheduled=true", "admin", None).await.json();
        assert_eq!(ids(&page), [id(&ai)]);
        let all = send(&app, "GET", "/schedules?status=scheduled&order=asc", "admin", None).await;
        assert_eq!(all.status(), StatusCode::OK, "{}", all.text());
        assert_eq!(all.json()["data"][0]["entry_id"], id(&ai));
        let bad = send(&app, "GET", "/schedules?status=later", "admin", None).await;
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn so_bai_va_tim_danh_muc_tac_gia_seo_va_xem_truoc() {
    TestDatabase::run(async |test| {
        let media = Arc::new(FakeMedia::default());
        let image = media.add(true);
        let app = app(test, media.clone()).await;
        let term = send(
            &app,
            "POST",
            "/terms",
            "admin",
            Some(json!({"taxonomy": "category", "locale": "vi", "name": "Du lịch"})),
        )
        .await;
        assert_eq!(term.status(), StatusCode::CREATED, "{}", term.text());
        let term_id = term.json()["data"]["id"].as_str().unwrap().to_owned();
        let author = send(
            &app,
            "POST",
            "/authors",
            "admin",
            Some(json!({"display_name": "Trần Thị Ánh", "slug": "anh"})),
        )
        .await;
        assert_eq!(author.status(), StatusCode::CREATED, "{}", author.text());
        let author_id = author.json()["data"]["id"].as_str().unwrap().to_owned();
        let entry = create(
            &app,
            "admin",
            json!({"kind": "post", "title": "Đà Lạt", "featured_media_id": image}),
        )
        .await;
        let id = entry["id"].as_str().unwrap();
        let put = |path: String, body: Value| {
            let app = &app;
            async move { send(app, "PUT", &path, "admin", Some(body)).await }
        };
        let terms = put(format!("/entries/{id}/terms"), json!({"term_ids": [term_id]})).await;
        assert!(terms.status().is_success(), "{}", terms.text());
        let authors =
            put(format!("/entries/{id}/authors"), json!({"author_ids": [author_id]})).await;
        assert!(authors.status().is_success(), "{}", authors.text());

        // Tìm không dấu, không phân biệt hoa thường; kèm số bài.
        let found = send(&app, "GET", "/terms?q=DU%20LICH", "admin", None).await.json();
        assert_eq!(found["data"][0]["id"], term_id.as_str());
        assert_eq!(found["data"][0]["entry_count"], 1);
        let none = send(&app, "GET", "/terms?q=bien", "admin", None).await.json();
        assert_eq!(none["data"], json!([]));
        let found = send(&app, "GET", "/authors?q=anh", "admin", None).await.json();
        assert_eq!(found["data"][0]["entry_count"], 1);

        // Kiểm SEO bằng tiếng Việt.
        let report = app
            .get(&format!("/api/v1/content/entries/{id}/seo-check"))
            .token("admin")
            .header("accept-language", "vi")
            .send()
            .await
            .unwrap();
        assert_eq!(report.status(), StatusCode::OK, "{}", report.text());
        let messages: Vec<String> = report.json()["data"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|finding| finding["message"].as_str().unwrap().to_owned())
            .collect();
        assert!(messages.iter().any(|m| m.starts_with("chưa có tóm tắt")), "{messages:?}");

        // Bản xem trước có địa chỉ ảnh chính.
        let token =
            send(&app, "POST", &format!("/entries/{id}/preview-token"), "admin", Some(json!({})))
                .await;
        assert!(token.status().is_success(), "{}", token.text());
        let token = token.json()["data"]["token"].as_str().unwrap().to_owned();
        let preview =
            app.get(&format!("/api/v1/content/public/preview?token={token}")).send().await.unwrap();
        assert_eq!(preview.status(), StatusCode::OK, "{}", preview.text());
        assert_eq!(preview.json()["data"]["featured_media"]["id"], json!(image));
        assert!(preview.json()["data"]["featured_media"]["url"].is_string());
    })
    .await
    .unwrap();
}
