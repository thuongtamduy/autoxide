//! Theo khách qua HTTP (bước 33): endpoint công khai trả bài của đúng khách theo header
//! `autoxide-tenant` hay tên miền riêng đã xác minh (`Host`), `ETag` khác nhau giữa khách;
//! link xem trước mang khách của nó; người của khách B không đọc được gì của khách A qua mọi `GET`
//! và mọi tool MCP chỉ đọc. Khách `a` và `c` dùng chung database chính, khách `b` có database
//! riêng.

use std::sync::Arc;

use autoxide_kernel::ai::{Embedder, TextModel};
use autoxide_media::api::Media;
use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};
use autoxide_testkit::{TestApp, TestDatabase, TestResponse};
use axum::http::{Method, StatusCode, header};
use serde_json::{Value, json};

use crate::testing::FakeMedia;

const SECRET: &str = "a-test-secret-that-is-long-enough-0123456789";

const ALL: [&str; 5] =
    ["content.read", "content.write", "content.publish", "content.delete", "content.manage"];

async fn app(test: &TestDatabase) -> TestApp {
    let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
    let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
    TestApp::builder(test)
        .setting("AUTOXIDE_CONTENT_PREVIEW_SECRET", SECRET)
        .tenant("a")
        .tenant_dedicated("b")
        .tenant("c")
        // Tên miền riêng đã xác minh của khách `b` (database riêng).
        .domain("b", "shop-b.test")
        .user_in("a", "admin-a", 1, &ALL)
        .user_in("b", "admin-b", 2, &ALL)
        .user_in("c", "admin-c", 3, &ALL)
        .provide::<dyn Media>(media)
        .provide::<dyn TextModel>(text)
        .provide::<dyn Embedder>(embedder)
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

/// Gửi request tới `/api/v1/content{uri}`; `who`: token, `headers`: header thêm vào.
async fn send(
    app: &TestApp,
    method: Method,
    uri: &str,
    who: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> TestResponse {
    let mut request = app.request(method, &format!("/api/v1/content{uri}"));
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

/// Tạo (201) và trả `data`.
async fn create(app: &TestApp, who: &str, uri: &str, body: Value) -> Value {
    let answer = send(app, Method::POST, uri, Some(who), &[], Some(body)).await;
    assert_eq!(answer.status(), StatusCode::CREATED, "{uri}: {}", answer.json());
    answer.json()["data"].clone()
}

/// Tạo và đăng bài `title` có tóm tắt `excerpt`; trả ID.
async fn publish_post(app: &TestApp, who: &str, title: &str, excerpt: &str) -> String {
    let body = json!({"kind": "post", "title": title, "slug": "hello", "excerpt": excerpt});
    let id = create(app, who, "/entries", body).await["id"].as_str().unwrap().to_owned();
    let read = send(app, Method::GET, &format!("/entries/{id}"), Some(who), &[], None).await;
    let etag = read.header(header::ETAG).to_owned();
    let uri = format!("/entries/{id}/publish");
    let published = send(app, Method::POST, &uri, Some(who), &[("if-match", &etag)], None).await;
    assert_eq!(published.status(), StatusCode::OK, "{}", published.json());
    id
}

/// Cùng đường dẫn `/blog/hello` đăng ở hai khách, khác nội dung: endpoint công khai trả bản của
/// khách trong header, `ETag` khác nhau, `ETag` của khách này không cho 304 ở khách kia. Link xem
/// trước của khách `b` mở được không cần header (token mang khách).
#[tokio::test]
async fn trang_cong_khai_theo_host() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let a = publish_post(&app, "admin-a", "Hello", "Body A").await;
        let b = publish_post(&app, "admin-b", "Hello", "Body B").await;
        let path = "/public/resolve?locale=en&path=/blog/hello";
        let mut etags = Vec::new();
        for (slug, id, excerpt) in [("a", &a, "Body A"), ("b", &b, "Body B")] {
            let found =
                send(&app, Method::GET, path, None, &[("autoxide-tenant", slug)], None).await;
            assert_eq!(found.status(), StatusCode::OK, "{}", found.json());
            let entry = &found.json()["data"]["entry"];
            assert_eq!(
                (entry["id"].as_str(), entry["excerpt"].as_str()),
                (Some(id.as_str()), Some(excerpt))
            );
            etags.push(found.header(header::ETAG).to_owned());
            let posts = send(
                &app,
                Method::GET,
                "/public/posts?locale=en",
                None,
                &[("autoxide-tenant", slug)],
                None,
            )
            .await;
            assert_eq!(posts.json()["data"].as_array().unwrap().len(), 1);
            assert_eq!(posts.json()["data"][0]["id"].as_str(), Some(id.as_str()));
            let sitemap = send(
                &app,
                Method::GET,
                "/public/sitemap",
                None,
                &[("autoxide-tenant", slug)],
                None,
            )
            .await;
            assert_eq!(sitemap.json()["data"].as_array().unwrap().len(), 1);
        }
        assert_ne!(etags[0], etags[1]);
        let cross = send(
            &app,
            Method::GET,
            path,
            None,
            &[("autoxide-tenant", "b"), ("if-none-match", &etags[0])],
            None,
        )
        .await;
        assert_eq!(cross.status(), StatusCode::OK, "the ETag of `a` is no 304 in `b`");
        let same = send(
            &app,
            Method::GET,
            path,
            None,
            &[("autoxide-tenant", "b"), ("if-none-match", &etags[1])],
            None,
        )
        .await;
        assert_eq!(same.status(), StatusCode::NOT_MODIFIED);
        // Tên miền riêng của `b`, không header: cùng bài của `b`, cùng `ETag`.
        let by_host = send(&app, Method::GET, path, None, &[("host", "shop-b.test")], None).await;
        assert_eq!(
            (by_host.status(), by_host.json()["data"]["entry"]["excerpt"].as_str()),
            (StatusCode::OK, Some("Body B")),
            "{}",
            by_host.json()
        );
        assert_eq!(by_host.header(header::ETAG), etags[1]);
        // Người của `a` gửi tới tên miền của `b`: từ chối, không lẫn khách.
        let mismatch =
            send(&app, Method::GET, path, Some("admin-a"), &[("host", "shop-b.test")], None).await;
        assert_eq!(mismatch.json()["code"], "tenant_mismatch");

        // Link xem trước của `b`: mở được trên tên miền chính không header; ở khách `a` thì không.
        let uri = format!("/entries/{b}/preview-token");
        let token = send(&app, Method::POST, &uri, Some("admin-b"), &[], None).await.json();
        let token = token["data"]["token"].as_str().unwrap().to_owned();
        let preview = format!("/public/preview?token={token}");
        let opened = send(&app, Method::GET, &preview, None, &[], None).await;
        assert_eq!(
            (opened.status(), opened.json()["data"]["excerpt"].as_str()),
            (StatusCode::OK, Some("Body B")),
            "{}",
            opened.json()
        );
        let wrong = send(
            &app,
            Method::GET,
            &format!("/public/preview?token={}", token.replace('.', "x.")),
            None,
            &[],
            None,
        )
        .await;
        assert_eq!(wrong.json()["code"], "content.preview_token_invalid");
        // Token của `a` mở ở khách `b`: như token sai; ở khách `a` thì được.
        let uri = format!("/entries/{a}/preview-token");
        let token = send(&app, Method::POST, &uri, Some("admin-a"), &[], None).await.json();
        let token = token["data"]["token"].as_str().unwrap().to_owned();
        let preview = format!("/public/preview?token={token}");
        let elsewhere =
            send(&app, Method::GET, &preview, None, &[("autoxide-tenant", "b")], None).await;
        assert_eq!(elsewhere.json()["code"], "content.preview_token_invalid");
        let opened = send(
            &app,
            Method::GET,
            &format!("/public/preview?token={token}"),
            None,
            &[("autoxide-tenant", "a")],
            None,
        )
        .await;
        assert_eq!(opened.json()["data"]["excerpt"], "Body A");
    })
    .await
    .unwrap();
}

/// Người của khách `b` (mọi permission của `content`) không thấy bài, tác giả, danh mục, chuyển
/// hướng hay menu của khách `a` qua bất kỳ `GET` hay tool MCP chỉ đọc nào.
#[tokio::test]
async fn khong_ro_ri_giua_khach() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let marker = "marker-content-a";
        let body = json!({"kind": "page", "title": marker, "excerpt": marker});
        let entry =
            create(&app, "admin-a", "/entries", body).await["id"].as_str().unwrap().to_owned();
        let read =
            send(&app, Method::GET, &format!("/entries/{entry}"), Some("admin-a"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();
        let uri = format!("/entries/{entry}/publish");
        let published =
            send(&app, Method::POST, &uri, Some("admin-a"), &[("if-match", &etag)], None).await;
        assert_eq!(published.status(), StatusCode::OK, "{}", published.json());
        let author =
            create(&app, "admin-a", "/authors", json!({"display_name": marker, "slug": marker}))
                .await;
        let term = create(
            &app,
            "admin-a",
            "/terms",
            json!({"taxonomy": "tag", "locale": "en", "name": marker, "slug": marker}),
        )
        .await;
        let body =
            json!({"from_path": format!("/old-{marker}"), "to_entry_id": entry, "status_code": 301});
        let redirect = create(&app, "admin-a", "/redirects", body).await;
        let menu = json!({"items": [{"label": marker, "entry_id": entry}]});
        let put = send(
            &app,
            Method::PUT,
            &format!("/menus/{marker}?locale=en"),
            Some("admin-a"),
            &[("if-match", "*")],
            Some(menu),
        )
        .await;
        assert_eq!(put.status(), StatusCode::OK, "{}", put.json());
        // Khách `a` thấy dữ liệu của mình (bài rò rỉ không đạt vì lý do khác).
        let own = send(&app, Method::GET, "/public/sitemap", Some("admin-a"), &[], None).await;
        assert!(own.text().contains(&entry), "{}", own.text());

        let id = |value: &Value| value["id"].as_str().unwrap().to_owned();
        let term_id = term["term"]["id"].as_str().or(term["id"].as_str()).unwrap().to_owned();
        let seeded = [
            ("id", entry.clone()),
            ("entry_id", entry.clone()),
            ("id", id(&author)),
            ("id", term_id.clone()),
            ("term_id", term_id),
            ("id", id(&redirect)),
            ("slug", marker.to_owned()),
            ("key", marker.to_owned()),
        ];
        // `b`: database riêng; `c`: cùng database với `a` (chỉ bộ lọc theo khách ngăn rò rỉ).
        for reader in ["admin-b", "admin-c"] {
            let violations =
                autoxide_testkit::leak_check(&app, reader, marker, &seeded).await.unwrap();
            assert_eq!(violations, Vec::<String>::new(), "{reader}");
        }
    })
    .await
    .unwrap();
}
