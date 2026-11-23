//! Test qua HTTP (router thật của kernel) và MCP (client thật): endpoint công khai không lộ bản
//! nháp, cache (`Cache-Control`, `ETag`, 304), `If-Match` bắt buộc, link xem trước, tool MCP
//! với tham số `version`. Gate chạy trên cả SQLite và Postgres.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::ai::{Embedder, TextModel};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_media::api::Media;
use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};
use autoxide_testkit::{TestApp, TestDatabase, TestResponse, tool};
use axum::http::{Method, StatusCode, header};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::testing::FakeMedia;

mod agent;
mod ai_pages;
mod blocks_base;
mod bulk;
mod campaign_drafts;
mod edit_lock;
mod embed;
mod health;
mod layout;
mod layout_public;
mod ops;
mod page_templates;
mod public_site;
mod shared_sections;
mod theme_variants;
mod themes;
mod types;
mod versions;

// Tìm nâng cao qua `/kernel/find`: tách riêng để file này không dài thêm (D-20261118-file-length).
mod find;

const SECRET: &str = "a-test-secret-that-is-long-enough-0123456789";

/// Token → permission: `admin` mọi quyền của module; `reader` chỉ đọc; `agent` gọi bằng API key,
/// chỉ viết bài của mình. Thư viện media giả thay cho module `media`; module `ai` giả (đã tắt):
/// `content` cần `dyn TextModel` và `dyn Embedder`.
async fn app(test: &TestDatabase) -> TestApp {
    let media: Arc<dyn Media> = Arc::new(FakeMedia::new().tagging_locale());
    let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
    let agent = ["content.read", "content.write_own"].map(str::to_owned);
    TestApp::builder(test)
        .setting("AUTOXIDE_CONTENT_PREVIEW_SECRET", SECRET)
        .user(
            "admin",
            1,
            &[
                "content.read",
                "content.write",
                "content.publish",
                "content.delete",
                "content.manage",
            ],
        )
        .user("reader", 1, &["content.read"])
        .principal(
            "agent",
            Principal::new(Uuid::from_u128(2), agent).via_api_key(Uuid::from_u128(3)),
        )
        .provide::<dyn Media>(media)
        .provide::<dyn TextModel>(text)
        .provide::<dyn Embedder>(embedder)
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

/// Gửi request; `who`: token (`None`: không đăng nhập); `headers`: header thêm vào.
async fn send(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> TestResponse {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
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

/// Vòng đời qua HTTP và mọi endpoint công khai: nháp không bao giờ lộ; `If-Match` bắt buộc;
/// bản dịch thiếu là 404; cache được và trả 304; sitemap, feed, menu, link xem trước.
#[tokio::test]
async fn vong_doi_va_endpoint_cong_khai() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let created = send(&app, "POST", "/entries", Some("admin"), &[], Some(json!({"kind": "post", "title": "Đường đến Đà Lạt"}))).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let read = send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();

        let change = json!({"excerpt": "Một chuyến đi"});
        let missing = send(&app, "PATCH", &format!("/entries/{id}"), Some("admin"), &[], Some(change.clone())).await;
        assert_eq!((missing.status(), missing.json()["code"].as_str()), (StatusCode::PRECONDITION_REQUIRED, Some("precondition_required")));
        let changed = send(&app, "PATCH", &format!("/entries/{id}"), Some("admin"), &[("if-match", &etag)], Some(change.clone())).await;
        assert_eq!(changed.status(), StatusCode::OK, "{}", changed.json());
        let stale = send(&app, "PATCH", &format!("/entries/{id}"), Some("admin"), &[("if-match", &etag)], Some(change)).await;
        assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);
        let etag = changed.header(header::ETAG).to_owned();

        // Nháp: không endpoint công khai nào thấy.
        let path = "/public/resolve?locale=en&path=/blog/duong-den-da-lat";
        assert_eq!(send(&app, "GET", path, None, &[], None).await.status(), StatusCode::NOT_FOUND);
        assert_eq!(send(&app, "GET", "/public/posts?locale=en", None, &[], None).await.json()["data"], json!([]));
        assert_eq!(send(&app, "GET", "/public/sitemap", None, &[], None).await.json()["data"], json!([]));
        assert_eq!(send(&app, "GET", "/public/feed?locale=en", None, &[], None).await.json()["data"], json!([]));

        let reader_publish = send(&app, "POST", &format!("/entries/{id}/publish"), Some("reader"), &[("if-match", &etag)], None).await;
        assert_eq!(reader_publish.status(), StatusCode::FORBIDDEN);
        let published = send(&app, "POST", &format!("/entries/{id}/publish"), Some("admin"), &[("if-match", &etag)], None).await;
        assert_eq!(published.status(), StatusCode::OK, "{}", published.json());

        let found = send(&app, "GET", path, None, &[], None).await;
        assert_eq!(found.status(), StatusCode::OK, "{}", found.json());
        assert_eq!((found.json()["data"]["type"].as_str(), found.json()["data"]["entry"]["excerpt"].as_str()), (Some("entry"), Some("Một chuyến đi")));
        assert_eq!(found.header(header::CACHE_CONTROL), "public, max-age=0, s-maxage=60, stale-while-revalidate=300");
        let public_etag = found.header(header::ETAG).to_owned();
        let again = send(&app, "GET", path, None, &[("if-none-match", &public_etag)], None).await;
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);
        let vi = send(&app, "GET", "/public/resolve?locale=vi&path=/blog/duong-den-da-lat", None, &[], None).await;
        assert_eq!((vi.status(), vi.json()["code"].as_str()), (StatusCode::NOT_FOUND, Some("content.not_found")), "no fallback");
        let unsupported = send(&app, "GET", "/public/resolve?locale=fr&path=/x", None, &[], None).await;
        assert_eq!(unsupported.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let posts = send(&app, "GET", "/public/posts?locale=en&q=da%20lat", None, &[], None).await.json();
        assert_eq!(posts["data"][0]["path"], "/blog/duong-den-da-lat");
        // Câu tìm công khai có giới hạn độ dài (như tìm theo ý nghĩa).
        let longest = send(&app, "GET", &format!("/public/posts?locale=en&q={}", "a".repeat(200)), None, &[], None).await;
        assert_eq!(longest.status(), StatusCode::OK);
        let too_long = send(&app, "GET", &format!("/public/posts?locale=en&q={}", "a".repeat(201)), None, &[], None).await;
        assert_eq!((too_long.status(), too_long.json()["errors"][0]["field"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("q")));
        let sitemap = send(&app, "GET", "/public/sitemap?locale=en", None, &[], None).await.json();
        assert_eq!((sitemap["data"][0]["path"].as_str(), sitemap["data"][0]["kind"].as_str()), (Some("/blog/duong-den-da-lat"), Some("post")));
        assert_eq!(send(&app, "GET", "/public/feed?locale=en&limit=5", None, &[], None).await.json()["data"].as_array().unwrap().len(), 1);
        assert_eq!(send(&app, "GET", "/public/feed?locale=en&limit=0", None, &[], None).await.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let author = send(&app, "GET", "/public/authors/nobody", None, &[], None).await;
        assert_eq!(author.json()["code"], "content.author_not_found");

        let menu = json!({"items": [{"label": "Blog", "entry_id": id}]});
        let put = send(&app, "PUT", "/menus/main?locale=en", Some("admin"), &[("if-match", "*")], Some(menu)).await;
        assert_eq!(put.status(), StatusCode::OK, "{}", put.json());
        let public_menu = send(&app, "GET", "/public/menus/main?locale=vi", None, &[], None).await.json();
        assert_eq!((public_menu["data"]["locale"].as_str(), public_menu["data"]["items"][0]["href"].as_str()), (Some("en"), Some("/blog/duong-den-da-lat")));

        // Link xem trước: thấy bản nháp mới nhất, không được cache.
        let draft = send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await;
        let draft_etag = draft.header(header::ETAG).to_owned();
        let edit = json!({"title": "Bản nháp mới"});
        send(&app, "PATCH", &format!("/entries/{id}"), Some("admin"), &[("if-match", &draft_etag)], Some(edit)).await;
        let denied = send(&app, "POST", &format!("/entries/{id}/preview-token"), Some("reader"), &[], None).await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let token = send(&app, "POST", &format!("/entries/{id}/preview-token"), Some("admin"), &[], None).await.json();
        let token = token["data"]["token"].as_str().unwrap().to_owned();
        let preview = send(&app, "GET", &format!("/public/preview?token={token}"), None, &[], None).await;
        assert_eq!((preview.status(), preview.json()["data"]["title"].as_str()), (StatusCode::OK, Some("Bản nháp mới")));
        assert_eq!(preview.header(header::CACHE_CONTROL), "no-store");
        // Token có thể tới tay người ngoài: không lộ ID người dùng hay trường nội bộ.
        let shown = preview.json()["data"].clone();
        for internal in ["created_by", "created_via", "review_requested_by", "review_requested_at", "translated_from_revision_id"] {
            assert!(shown.get(internal).is_none(), "preview shows {internal}");
        }
        assert_eq!(send(&app, "GET", path, None, &[], None).await.json()["data"]["entry"]["title"], "Đường đến Đà Lạt", "readers still see the published version");
        let bad = send(&app, "GET", "/public/preview?token=abc.def", None, &[], None).await;
        assert_eq!((bad.status(), bad.json()["code"].as_str()), (StatusCode::NOT_FOUND, Some("content.preview_token_invalid")));
    })
    .await
    .unwrap();
}

/// Tool MCP qua client thật: người chỉ đọc chỉ thấy tool đọc; tạo nháp, sửa với `version`
/// (cũ thì bị từ chối), đăng, `resolve`, revision, khôi phục, gỡ.
#[tokio::test]
async fn tool_mcp_voi_version() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let client = async |who: &str| app.mcp(who).await.unwrap();
        let reader = client("reader").await;
        let mut names: Vec<String> = reader.list_all_tools().await.unwrap().into_iter().map(|tool| tool.name.to_string()).collect();
        names.sort();
        assert_eq!(names, ["content.get", "content.get_translation_request", "content.legal_templates", "content.list", "content.list_banners", "content.list_revisions", "content.resolve", "content.search", "content.seo_check", "kernel.current_tenant", "kernel.decide_approval", "kernel.list_approvals", "kernel.list_custom_fields", "kernel.navigation"]);
        reader.cancel().await.unwrap();

        // Agent chỉ có `content.write_own`: thấy tool viết, không thấy tool đăng.
        let agent = client("agent").await;
        let mut names: Vec<String> = agent.list_all_tools().await.unwrap().into_iter().map(|tool| tool.name.to_string()).collect();
        names.sort();
        assert_eq!(names, ["content.create_draft", "content.draft_from_template", "content.get", "content.get_translation_request", "content.legal_templates", "content.list", "content.list_banners", "content.list_revisions", "content.request_review", "content.resolve", "content.restore_revision", "content.search", "content.seo_check", "content.suggest_seo", "content.translate", "content.update", "kernel.current_tenant", "kernel.decide_approval", "kernel.list_approvals", "kernel.list_custom_fields", "kernel.navigation"]);
        agent.cancel().await.unwrap();

        let admin = client("admin").await;
        let tools = admin.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), 23, "with `kernel.current_tenant` and `kernel.navigation` (admin reads banners, `content.read`)");
        let publish = tools.iter().find(|tool| tool.name == "content.publish").unwrap();
        assert!(publish.description.as_deref().unwrap_or_default().contains("Requires permission `content.publish` or `content.publish_own`."));
        for tool in &tools {
            assert!(!serde_json::to_string(&tool.input_schema).unwrap().contains("$ref"), "{} has a $ref", tool.name);
        }
        let call = async |name: &str, arguments: Value| {
            let result = admin.call_tool(tool(name, arguments)).await.unwrap();
            (result.is_error == Some(true), result.structured_content.unwrap_or(Value::Null))
        };
        let body = json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Xin chào"}]}]});
        let (failed, created) = call("content.create_draft", json!({"kind": "post", "title": "Hello MCP", "body": body})).await;
        assert!(!failed, "{created}");
        let id = created["data"]["id"].clone();
        let version = created["data"]["version"].as_str().unwrap().to_owned();
        assert_eq!(created["data"]["word_count"], 2);

        let (failed, updated) = call("content.update", json!({"id": id, "version": version, "excerpt": "Short", "save_revision": true})).await;
        assert!(!failed, "{updated}");
        let (stale, _) = call("content.update", json!({"id": id, "version": version, "excerpt": "Lost"})).await;
        assert!(stale, "an old version is refused");
        let (missing, _) = call("content.publish", json!({"id": id})).await;
        assert!(missing, "`version` is required");
        let version = updated["data"]["version"].as_str().unwrap().to_owned();
        let (failed, published) = call("content.publish", json!({"id": id, "version": version})).await;
        assert!(!failed, "{published}");
        assert_eq!(published["data"]["status"], "published");

        let (_, resolved) = call("content.resolve", json!({"locale": "en", "path": "/blog/hello-mcp"})).await;
        assert_eq!((resolved["data"]["type"].as_str(), resolved["data"]["entry"]["excerpt"].as_str()), (Some("entry"), Some("Short")));
        let (_, revisions) = call("content.list_revisions", json!({"id": id})).await;
        let revisions = revisions["data"].as_array().unwrap().clone();
        let initial = revisions.iter().find(|revision| revision["reason"] == "initial").unwrap();
        let (_, current) = call("content.get", json!({"id": id})).await;
        let version = current["data"]["version"].as_str().unwrap().to_owned();
        let (failed, restored) = call("content.restore_revision", json!({"id": id, "revision_id": initial["id"], "version": version})).await;
        assert!(!failed, "{restored}");
        assert_eq!((restored["data"]["excerpt"].as_str(), restored["data"]["has_unpublished_changes"].as_bool()), (Some(""), Some(true)));
        let version = restored["data"]["version"].as_str().unwrap().to_owned();
        let (failed, down) = call("content.unpublish", json!({"id": id, "version": version})).await;
        assert!(!failed, "{down}");
        let (gone, _) = call("content.resolve", json!({"locale": "en", "path": "/blog/hello-mcp"})).await;
        assert!(gone, "404 once taken down");
        let (_, listed) = call("content.list", json!({"status": "draft", "q": "xin chao"})).await;
        assert_eq!(listed["data"][0]["id"], id);
        admin.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

/// ETag của `resolve` đổi khi thứ gộp vào câu trả lời đổi (ở đây: gắn thẻ), dù bản công khai
/// không đổi — website không được nhận 304 với nội dung cũ.
#[tokio::test]
async fn etag_resolve_theo_ca_cau_tra_loi() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let created = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "post", "title": "Tagged"})),
        )
        .await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let read = send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();
        let published = send(
            &app,
            "POST",
            &format!("/entries/{id}/publish"),
            Some("admin"),
            &[("if-match", &etag)],
            None,
        )
        .await;
        assert_eq!(published.status(), StatusCode::OK, "{}", published.json());
        let path = "/public/resolve?locale=en&path=/blog/tagged";
        let first = send(&app, "GET", path, None, &[], None).await;
        let first_etag = first.header(header::ETAG).to_owned();
        assert!(!first_etag.is_empty());

        let tag = send(
            &app,
            "POST",
            "/terms",
            Some("admin"),
            &[],
            Some(json!({"taxonomy": "tag", "locale": "en", "name": "Food"})),
        )
        .await;
        assert_eq!(tag.status(), StatusCode::CREATED, "{}", tag.json());
        let tag_id = tag.json()["data"]["id"].as_str().unwrap().to_owned();
        let linked = send(
            &app,
            "PUT",
            &format!("/entries/{id}/terms"),
            Some("admin"),
            &[],
            Some(json!({"term_ids": [tag_id]})),
        )
        .await;
        assert_eq!(linked.status(), StatusCode::OK, "{}", linked.json());

        // Danh sách thẻ công khai: thẻ có bài đã đăng, kèm số bài; thẻ chưa có bài thì không hiện.
        let unused = json!({"taxonomy": "tag", "locale": "en", "name": "Unused"});
        let unused = send(&app, "POST", "/terms", Some("admin"), &[], Some(unused)).await;
        assert_eq!(unused.status(), StatusCode::CREATED, "{}", unused.json());
        let terms = send(&app, "GET", "/public/terms?locale=en", None, &[], None).await;
        assert_eq!(terms.status(), StatusCode::OK, "{}", terms.json());
        let listed = terms.json()["data"].clone();
        assert_eq!(listed.as_array().unwrap().len(), 1, "{listed}");
        assert_eq!(
            (listed[0]["id"].as_str(), listed[0]["posts"].as_u64()),
            (Some(tag_id.as_str()), Some(1))
        );
        assert_eq!(
            (listed[0]["name"].as_str(), listed[0]["taxonomy"].as_str()),
            (Some("Food"), Some("tag"))
        );
        let categories =
            send(&app, "GET", "/public/terms?locale=en&taxonomy=category", None, &[], None).await;
        assert_eq!(categories.json()["data"], json!([]));
        let bad = send(&app, "GET", "/public/terms?locale=fr", None, &[], None).await;
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let again = send(&app, "GET", path, None, &[("if-none-match", &first_etag)], None).await;
        assert_eq!(again.status(), StatusCode::OK, "the tag changed the answer");
        assert_ne!(again.header(header::ETAG), first_etag);
        let same =
            send(&app, "GET", path, None, &[("if-none-match", again.header(header::ETAG))], None)
                .await;
        assert_eq!(same.status(), StatusCode::NOT_MODIFIED);
    })
    .await
    .unwrap();
}

/// Xoá tài khoản người đã xác nhận duyệt nội dung AI: `ai_reviewed_by` bỏ trống nhưng người đọc
/// vẫn thấy `ai_reviewed` (tính từ thời điểm xác nhận).
#[tokio::test]
async fn da_duyet_ai_van_hien_sau_xoa_tai_khoan() {
    use autoxide_kernel::event::{Delivered, EventHandler};

    TestDatabase::run(async |test| {
        let (content, _, database) =
            crate::testing::service(test, crate::testing::config(&[])).await;
        let tenant = TenantContext::default_on(database.clone());
        let ada = crate::testing::editor();
        let mut input = crate::tests::post("Reviewed");
        input.fields.ai_generated = Some(true);
        let entry = content.create(&tenant, &ada, input).await.unwrap();
        content.publish(&tenant, &ada, entry.id, None, None, true).await.unwrap();
        let deleted = Delivered::new(
            tenant.clone(),
            "auth",
            autoxide_kernel::privacy::USER_DELETED.to_owned(),
            json!({"before": {"id": ada.user_id()}}),
        );
        content.handle(&deleted).await.unwrap();
        content.handle(&deleted).await.unwrap();
        let (resolution, _) = super::public::resolution(
            &content,
            &tenant,
            &autoxide_kernel::tenancy::Site::whole(&tenant),
            "en",
            "/blog/reviewed",
        )
        .await
        .unwrap();
        let shown = serde_json::to_value(resolution).unwrap();
        assert_eq!(
            (shown["entry"]["ai_generated"].as_bool(), shown["entry"]["ai_reviewed"].as_bool()),
            (Some(true), Some(true)),
            "{shown}"
        );
    })
    .await
    .unwrap();
}

/// Menu cũ vẫn chạy như trước nhưng báo deprecated, sunset và endpoint thay thế (D-238, D-059).
#[tokio::test]
async fn menu_cu_bao_deprecated() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let body = json!({"items": [{"label": "Home", "url": "/"}]});
        let put =
            send(&app, "PUT", "/menus/main", Some("admin"), &[("if-match", "*")], Some(body)).await;
        assert_eq!(put.status(), StatusCode::OK, "{}", put.json());
        for (method, uri, who) in [
            ("GET", "/menus", Some("admin")),
            ("GET", "/menus/main", Some("admin")),
            ("GET", "/public/menus/main?locale=en", None),
        ] {
            let response = send(&app, method, uri, who, &[], None).await;
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            assert!(response.headers().contains_key("deprecation"), "{uri}");
            assert!(response.header("sunset").contains("2027"), "{uri}");
            assert!(response.header(header::LINK).contains("/api/v1/menus"), "{uri}");
        }
        assert!(put.headers().contains_key("deprecation"));
        let document = serde_json::to_value(app.openapi()).unwrap();
        assert_eq!(
            document["paths"]["/api/v1/content/public/menus/{key}"]["get"]["deprecated"],
            true
        );
        assert!(
            document["paths"]["/api/v1/content/public/posts"]["get"].get("deprecated").is_none()
        );
    })
    .await
    .unwrap();
}

fn embed_body(items: &[(&str, &str)]) -> Value {
    let mut content =
        vec![json!({"type": "paragraph", "content": [{"type": "text", "text": "Xem"}]})];
    content.extend(
        items.iter().map(|(kind, id)| json!({"type": "embed", "attrs": {"kind": kind, "id": id}})),
    );
    json!({"type": "doc", "content": content})
}

/// Số event `content` đang nằm trong outbox (chưa giao: test không chạy bộ phát).
async fn outbox_count(app: &TestApp) -> i64 {
    use sea_orm::ConnectionTrait;
    let database = app.database().read();
    let sql = "SELECT COUNT(*) AS n FROM content_events";
    let row = database
        .query_one_raw(sea_orm::Statement::from_string(database.get_database_backend(), sql))
        .await
        .unwrap()
        .unwrap();
    row.try_get("", "n").unwrap()
}
