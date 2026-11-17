//! Bước 37 (D-362): nhãn "viết từ gợi ý của AI" (`ai_assisted`, Luật Trí tuệ nhân tạo 134/2025/QH15)
//! trả ở API quản trị và công khai; bỏ nhãn cần `content.publish`. Chạy trên SQLite và Postgres.

use std::sync::Arc;

use autoxide_kernel::ai::{Embedder, TextModel};
use autoxide_media::api::Media;
use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};
use autoxide_testkit::{TestApp, TestDatabase, TestResponse};
use axum::http::{Method, StatusCode, header};
use serde_json::{Value, json};

use crate::testing::FakeMedia;

const EDITOR: [&str; 3] = ["content.read", "content.write", "content.publish"];
const WRITER: [&str; 2] = ["content.read", "content.write"];

async fn app(test: &TestDatabase) -> TestApp {
    let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
    let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
    TestApp::builder(test)
        .user("editor", 1, &EDITOR)
        .user("writer", 2, &WRITER)
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
    method: Method,
    uri: &str,
    who: Option<&str>,
    etag: Option<&str>,
    body: Option<Value>,
) -> TestResponse {
    let mut request = app.request(method, &format!("/api/v1/content{uri}"));
    if let Some(who) = who {
        request = request.token(who);
    }
    if let Some(etag) = etag {
        request = request.header("if-match", etag);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    request.send().await.unwrap()
}

async fn version(app: &TestApp, id: &str) -> String {
    let got = send(app, Method::GET, &format!("/entries/{id}"), Some("editor"), None, None).await;
    got.header(header::ETAG).to_owned()
}

#[tokio::test]
async fn ai_assisted_tra_ve_cong_khai() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let created = send(
            &app,
            Method::POST,
            "/entries",
            Some("writer"),
            None,
            Some(json!({"kind": "post", "title": "Gợi ý SEO", "ai_assisted": true})),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.text());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(created.json()["data"]["ai_assisted"], true);
        assert_eq!(created.json()["data"]["ai_generated"], false);
        let etag = version(&app, &id).await;
        let published = send(
            &app,
            Method::POST,
            &format!("/entries/{id}/publish"),
            Some("editor"),
            Some(&etag),
            None,
        )
        .await;
        assert_eq!(published.status(), StatusCode::OK, "{}", published.text());
        let path = "/public/resolve?locale=en&path=/blog/goi-y-seo";
        let found = send(&app, Method::GET, path, None, None, None).await;
        assert_eq!(found.status(), StatusCode::OK, "{}", found.text());
        assert_eq!(found.json()["data"]["entry"]["ai_assisted"], true);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bo_ai_assisted_can_content_publish() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let created = send(
            &app,
            Method::POST,
            "/entries",
            Some("writer"),
            None,
            Some(json!({"kind": "post", "title": "Bản nháp", "ai_assisted": true})),
        )
        .await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let etag = version(&app, &id).await;
        let uri = format!("/entries/{id}");
        // Người chỉ có `content.write` đặt được nhãn, nhưng không bỏ được.
        let refused = send(
            &app,
            Method::PATCH,
            &uri,
            Some("writer"),
            Some(&etag),
            Some(json!({"ai_assisted": false})),
        )
        .await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{}", refused.text());
        assert_eq!(refused.json()["code"], "content.ai_label_requires_publish");
        // Gửi lại `true` (không đổi) thì được.
        let same = send(
            &app,
            Method::PATCH,
            &uri,
            Some("writer"),
            Some(&etag),
            Some(json!({"ai_assisted": true, "excerpt": "Tóm tắt"})),
        )
        .await;
        assert_eq!(same.status(), StatusCode::OK, "{}", same.text());
        let etag = same.header(header::ETAG).to_owned();
        let cleared = send(
            &app,
            Method::PATCH,
            &uri,
            Some("editor"),
            Some(&etag),
            Some(json!({"ai_assisted": false})),
        )
        .await;
        assert_eq!(cleared.status(), StatusCode::OK, "{}", cleared.text());
        assert_eq!(cleared.json()["data"]["ai_assisted"], false);
    })
    .await
    .unwrap();
}

/// Event ẩn / hiện của `compliance` (hợp đồng `autoxide_kernel::moderation`), giao thẳng cho bên nghe.
async fn deliver(app: &TestApp, name: &'static str, entry: &str) {
    let tenant = app.tenant("default").await.unwrap();
    let notice = autoxide_kernel::moderation::Restriction::new(
        uuid::Uuid::now_v7(),
        format!("content.entry:{entry}"),
        "counterfeit",
    );
    let event = autoxide_kernel::event::Delivered::new(
        tenant,
        "compliance",
        name,
        serde_json::to_value(notice).unwrap(),
    );
    use autoxide_kernel::event::EventHandler;
    crate::restriction::Listener.handle(&event).await.unwrap();
}

/// Bước 37 (D-363): mục bị ẩn vì vụ việc trả 451 và vắng khỏi danh sách, sitemap; hiện lại thì như
/// cũ; nhận lại cùng event không hỏng gì.
#[tokio::test]
async fn nhan_lai_event_an_khong_hong() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let created = send(
            &app,
            Method::POST,
            "/entries",
            Some("editor"),
            None,
            Some(json!({"kind": "post", "title": "Hang gia"})),
        )
        .await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let etag = version(&app, &id).await;
        let published = send(
            &app,
            Method::POST,
            &format!("/entries/{id}/publish"),
            Some("editor"),
            Some(&etag),
            None,
        )
        .await;
        assert_eq!(published.status(), StatusCode::OK, "{}", published.text());
        let path = "/public/resolve?locale=en&path=/blog/hang-gia";
        assert_eq!(send(&app, Method::GET, path, None, None, None).await.status(), StatusCode::OK);

        let restricted = autoxide_kernel::moderation::TARGET_RESTRICTED;
        deliver(&app, restricted, &id).await;
        deliver(&app, restricted, &id).await;
        let hidden = send(&app, Method::GET, path, None, None, None).await;
        assert_eq!(hidden.status(), StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS, "{}", hidden.text());
        assert_eq!(hidden.json()["code"], "content.restricted");
        let posts = send(&app, Method::GET, "/public/posts?locale=en", None, None, None).await;
        assert_eq!(posts.json()["data"], json!([]));
        let sitemap = send(&app, Method::GET, "/public/sitemap", None, None, None).await;
        assert_eq!(sitemap.json()["data"], json!([]));

        let restored = autoxide_kernel::moderation::TARGET_RESTORED;
        deliver(&app, restored, &id).await;
        deliver(&app, restored, &id).await;
        assert_eq!(send(&app, Method::GET, path, None, None, None).await.status(), StatusCode::OK);
        let posts = send(&app, Method::GET, "/public/posts?locale=en", None, None, None).await;
        assert_eq!(posts.json()["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}
