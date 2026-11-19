//! Banner qua REST và MCP (bước 55): vị trí, banner, sắp thứ tự với `If-Match`, đọc công khai có
//! `Cache-Control` / `ETag` / 304, tool MCP chỉ tạo bản nháp.

use std::sync::Arc;

use autoxide_kernel::ai::{Embedder, TextModel};
use autoxide_media::api::Media;
use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};
use autoxide_testkit::{TestApp, TestDatabase, TestResponse};
use axum::http::{Method, StatusCode, header};
use serde_json::{Value, json};

use crate::testing::FakeMedia;

async fn app(test: &TestDatabase) -> TestApp {
    let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
    let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
    TestApp::builder(test)
        .user("admin", 1, &["content.manage", "content.banners"])
        .user("reader", 2, &["content.read"])
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
    if_match: Option<&str>,
    body: Option<Value>,
) -> TestResponse {
    let mut request = app.request(method, &format!("/api/v1/content{uri}"));
    if let Some(who) = who {
        request = request.token(who);
    }
    if let Some(version) = if_match {
        request = request.header("if-match", version);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    request.send().await.unwrap()
}

#[tokio::test]
async fn vi_tri_va_banner_qua_rest() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let admin = Some("admin");
        let presets = send(&app, Method::POST, "/placements/presets", admin, None, None).await;
        assert_eq!(presets.status(), StatusCode::OK, "{}", presets.text());
        assert_eq!(presets.json()["data"].as_array().unwrap().len(), 9);
        let refused = send(&app, Method::GET, "/placements", Some("reader"), None, None).await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        let bad = json!({"key": "x", "name": "X", "display": "carousel", "display_settings": {"autoplay_seconds": 1}});
        let bad = send(&app, Method::POST, "/placements", admin, None, Some(bad)).await;
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(bad.json()["code"], "content.placement_settings_invalid");

        let mut ids = Vec::new();
        for title in ["Free shipping", "New season"] {
            let body = json!({"placement": "top_strip", "kind": "text", "title": title, "status": "active", "link": {"url": "/sale"}});
            let created = send(&app, Method::POST, "/banners", admin, None, Some(body)).await;
            assert_eq!(created.status(), StatusCode::CREATED, "{}", created.text());
            assert!(created.header(header::LOCATION).starts_with("/api/v1/content/banners/"));
            assert_eq!(created.json()["data"]["placement"], "top_strip");
            ids.push(created.json()["data"]["id"].as_str().unwrap().to_owned());
        }
        let no_version = send(&app, Method::PATCH, &format!("/banners/{}", ids[0]), admin, None, Some(json!({"title": "x"}))).await;
        assert_eq!(no_version.status(), StatusCode::PRECONDITION_REQUIRED);
        let patched = send(&app, Method::PATCH, &format!("/banners/{}", ids[0]), admin, Some("*"), Some(json!({"subtitle": "today", "link": null}))).await;
        assert_eq!(patched.status(), StatusCode::OK, "{}", patched.text());
        assert_eq!(patched.json()["data"]["link"], Value::Null);

        let public = |query: &'static str| {
            let app = &app;
            async move { send(app, Method::GET, &format!("/public/placements?{query}"), None, None, None).await }
        };
        let read = public("keys=top_strip,home_hero").await;
        assert_eq!(read.status(), StatusCode::OK, "{}", read.text());
        let data = read.json()["data"].clone();
        let keys: Vec<&str> = data["placements"].as_array().unwrap().iter().map(|p| p["key"].as_str().unwrap()).collect();
        assert_eq!(keys, ["top_strip", "home_hero"]);
        let titles: Vec<&str> = data["placements"][0]["banners"].as_array().unwrap().iter().map(|b| b["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Free shipping", "New season"]);
        assert_eq!(data["placements"][0]["banners"][0]["href"], Value::Null);
        assert_eq!(data["placements"][0]["banners"][1]["href"], "/sale");
        assert!(read.header(header::CACHE_CONTROL).starts_with("public, max-age="));
        let etag = read.header(header::ETAG).to_owned();
        let again = app
            .request(Method::GET, "/api/v1/content/public/placements?keys=top_strip,home_hero")
            .header("if-none-match", &etag)
            .send()
            .await
            .unwrap();
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);

        // Đặt lại thứ tự cả vị trí, `If-Match` là phiên bản của vị trí.
        let placement = send(&app, Method::GET, "/placements/top_strip", admin, None, None).await;
        let version = placement.header(header::ETAG).to_owned();
        let body = json!({"banner_ids": [ids[1]]});
        let ordered = send(&app, Method::PUT, "/placements/top_strip/order", admin, Some(&version), Some(body.clone())).await;
        assert_eq!(ordered.status(), StatusCode::OK, "{}", ordered.text());
        let stale = send(&app, Method::PUT, "/placements/top_strip/order", admin, Some(&version), Some(body)).await;
        assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);
        let one = send(&app, Method::GET, "/public/placements/top_strip", None, None, None).await;
        let one = one.json();
        let titles: Vec<&str> = one["data"]["banners"].as_array().unwrap().iter().map(|b| b["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["New season", "Free shipping"]);
        let missing = send(&app, Method::GET, "/public/placements/nope", None, None, None).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(missing.json()["code"], "content.placement_not_found");

        let in_use = send(&app, Method::DELETE, "/placements/top_strip", admin, None, None).await;
        assert_eq!(in_use.status(), StatusCode::CONFLICT);
        for id in &ids {
            let deleted = send(&app, Method::DELETE, &format!("/banners/{id}"), admin, None, None).await;
            assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
        }
        let deleted = send(&app, Method::DELETE, "/placements/top_strip", admin, None, None).await;
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    })
    .await
    .unwrap();
}
