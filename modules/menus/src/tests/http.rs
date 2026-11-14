//! Test qua HTTP (router thật của kernel) và MCP (client thật): 201 + `Location`, `If-Match` bắt
//! buộc, `ETag` + 304 + `Cache-Control` ở menu công khai, permission, tool dùng chung service.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_media::api::Media;
use autoxide_testkit::{TestApp, TestDatabase, tool};
use axum::http::{Method, StatusCode, header};
use serde_json::{Value, json};

use super::{Categories, FakeMedia, Items};

/// Module giả cung cấp `shop.item` và `shop.category` cho sổ đối tượng.
struct Shop;

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.resource_type("shop.item", Arc::new(Items))?;
        context.resource_type("shop.category", Arc::new(Categories))
    }
}

async fn app(test: &TestDatabase) -> TestApp {
    let media: Arc<dyn Media> = Arc::new(FakeMedia);
    TestApp::builder(test)
        .user("admin", 1, &["menus.read", "menus.manage"])
        .user("reader", 2, &["menus.read"])
        .provide::<dyn Media>(media)
        .module(Shop)
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

/// Gửi request tới `/api/v1/menus{uri}`; `who`: token (`None`: không đăng nhập).
async fn send(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> autoxide_testkit::TestResponse {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/menus{uri}"));
    if let Some(who) = who {
        request = request.token(who);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    for (name, value) in headers {
        request = request.header(name, value);
    }
    request.send().await.unwrap()
}

#[tokio::test]
async fn vong_doi_qua_rest() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let body = json!({
            "key": "main",
            "locale": "vi",
            "items": [
                {"ref": {"kind": "shop.category", "id": "c"}, "auto_children": true, "auto_children_limit": 2, "icon": "category"},
                {"label": "Khuyến mại", "url": "/sale", "highlight": "sale", "badge": {"text": "-50%", "tone": "sale"}},
                {"ref": {"kind": "shop.item", "id": "h1"}}
            ]
        });
        let denied = send(&app, "POST", "", Some("reader"), &[], Some(body.clone())).await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let created = send(&app, "POST", "", Some("admin"), &[], Some(body.clone())).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(created.header(header::LOCATION), format!("/api/v1/menus/{id}"));
        let again = send(&app, "POST", "", Some("admin"), &[], Some(body)).await;
        assert_eq!((again.status(), again.json()["code"].as_str()), (StatusCode::CONFLICT, Some("menus.menu_exists")));

        let bad = json!({"key": "x", "items": [{"ref": {"kind": "shop.item", "id": "x1"}}]});
        let bad = send(&app, "POST", "", Some("admin"), &[("accept-language", "vi")], Some(bad)).await;
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(bad.json()["code"], "menus.ref_not_found");
        assert_eq!(bad.json()["errors"][0]["field"], "items[0].ref.id");

        let read = send(&app, "GET", &format!("/{id}"), Some("reader"), &[], None).await;
        assert_eq!(read.status(), StatusCode::OK);
        assert_eq!(read.json()["data"]["items"][0]["ref"]["kind"], "shop.category");
        let etag = read.header(header::ETAG).to_owned();
        let listed = send(&app, "GET", "", Some("reader"), &[], None).await;
        assert_eq!(listed.json()["data"][0]["key"], "main");

        let public = send(&app, "GET", "/public/main?locale=vi", None, &[], None).await;
        assert_eq!(public.status(), StatusCode::OK, "{}", public.json());
        let shown = public.json();
        assert_eq!(shown["data"]["items"].as_array().unwrap().len(), 2, "hidden item left out");
        assert_eq!(shown["data"]["items"][0]["label"], "Category c");
        assert_eq!(shown["data"]["items"][0]["children"].as_array().unwrap().len(), 2);
        assert_eq!(shown["data"]["items"][1]["badge"]["text"], "-50%");
        assert_eq!(public.header(header::CACHE_CONTROL), "public, max-age=60");
        let tag = public.header(header::ETAG).to_owned();
        let cached = send(&app, "GET", "/public/main?locale=vi", None, &[("if-none-match", &tag)], None).await;
        assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);

        let items = json!({"items": [{"label": "Trang chủ", "url": "/"}]});
        let missing = send(&app, "PUT", &format!("/{id}"), Some("admin"), &[], Some(items.clone())).await;
        assert_eq!(missing.status(), StatusCode::PRECONDITION_REQUIRED);
        let replaced = send(&app, "PUT", &format!("/{id}"), Some("admin"), &[("if-match", &etag)], Some(items.clone())).await;
        assert_eq!(replaced.status(), StatusCode::OK, "{}", replaced.json());
        let stale = send(&app, "PUT", &format!("/{id}"), Some("admin"), &[("if-match", &etag)], Some(items)).await;
        assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);
        let fresh = send(&app, "GET", "/public/main?locale=vi", None, &[("if-none-match", &tag)], None).await;
        assert_eq!(fresh.status(), StatusCode::OK, "a change gives a new ETag");
        assert_eq!(fresh.json()["data"]["items"][0]["label"], "Trang chủ");

        let gone = send(&app, "DELETE", &format!("/{id}"), Some("admin"), &[], None).await;
        assert_eq!(gone.status(), StatusCode::NO_CONTENT);
        let missing = send(&app, "GET", "/public/main", None, &[], None).await;
        assert_eq!((missing.status(), missing.json()["code"].as_str()), (StatusCode::NOT_FOUND, Some("menus.public_menu_not_found")));
    })
    .await
    .unwrap();
}

/// Tool MCP gọi cùng service với REST: tạo qua MCP đọc được qua REST; `version` cũ bị từ chối.
#[tokio::test]
async fn tool_mcp_dung_chung_service() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let admin = app.mcp("admin").await.unwrap();
        let made = admin
            .call_tool(tool(
                "menus.create_menu",
                json!({"key": "footer", "items": [{"label": "Liên hệ", "url": "mailto:a@b.vn"}]}),
            ))
            .await
            .unwrap();
        let made = made.structured_content.unwrap();
        let id = made["data"]["id"].as_str().unwrap().to_owned();
        let version = made["data"]["version"].as_str().unwrap().to_owned();
        let read = send(&app, "GET", &format!("/{id}"), Some("admin"), &[], None).await;
        assert_eq!(read.json()["data"]["items"][0]["url"], "mailto:a@b.vn");
        assert_eq!(read.header(header::ETAG), version);

        let input = json!({"id": id, "version": version, "items": [{"label": "Home", "url": "/"}]});
        let replaced =
            admin.call_tool(tool("menus.replace_menu_items", input.clone())).await.unwrap();
        assert_ne!(replaced.is_error, Some(true), "{replaced:?}");
        let stale = admin.call_tool(tool("menus.replace_menu_items", input)).await.unwrap();
        assert_eq!(stale.is_error, Some(true));
        assert!(format!("{stale:?}").contains("precondition_failed"), "{stale:?}");

        let reader = app.mcp("reader").await.unwrap();
        let listed = reader.call_tool(tool("menus.list_menus", json!({}))).await.unwrap();
        assert_eq!(listed.structured_content.unwrap()["data"][0]["id"], id.as_str());
        let tools = reader.list_all_tools().await.unwrap();
        assert!(tools.iter().all(|tool| !tool.name.ends_with("delete_menu")), "needs menus.manage");
    })
    .await
    .unwrap();
}
