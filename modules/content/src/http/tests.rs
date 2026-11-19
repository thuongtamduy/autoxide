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

const SECRET: &str = "a-test-secret-that-is-long-enough-0123456789";

/// Token → permission: `admin` mọi quyền của module; `reader` chỉ đọc; `agent` gọi bằng API key,
/// chỉ viết bài của mình. Thư viện media giả thay cho module `media`; module `ai` giả (đã tắt):
/// `content` cần `dyn TextModel` và `dyn Embedder`.
async fn app(test: &TestDatabase) -> TestApp {
    let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
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
        // Lớp cache của kernel (D-20261119-step39-cdn-cache) đổi `public, no-cache` thành chỉ thị cho CDN.
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
        assert_eq!(names, ["content.get", "content.get_translation_request", "content.legal_templates", "content.list", "content.list_revisions", "content.resolve", "content.search", "content.seo_check", "kernel.current_tenant", "kernel.decide_approval", "kernel.list_approvals", "kernel.list_custom_fields", "kernel.navigation"]);
        reader.cancel().await.unwrap();

        // Agent chỉ có `content.write_own`: thấy tool viết, không thấy tool đăng.
        let agent = client("agent").await;
        let mut names: Vec<String> = agent.list_all_tools().await.unwrap().into_iter().map(|tool| tool.name.to_string()).collect();
        names.sort();
        assert_eq!(names, ["content.create_draft", "content.draft_from_template", "content.get", "content.get_translation_request", "content.legal_templates", "content.list", "content.list_revisions", "content.request_review", "content.resolve", "content.restore_revision", "content.search", "content.seo_check", "content.suggest_seo", "content.translate", "content.update", "kernel.current_tenant", "kernel.decide_approval", "kernel.list_approvals", "kernel.list_custom_fields", "kernel.navigation"]);
        agent.cancel().await.unwrap();

        let admin = client("admin").await;
        let tools = admin.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), 22, "with `kernel.current_tenant` and `kernel.navigation`");
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

/// Module `auth` giả: chỉ đăng ký loại `auth.user` vào sổ đối tượng; người số 1 tên "Ada", người
/// khác không có (như tài khoản đã xoá).
struct FakeAuth;

struct Names;

#[async_trait::async_trait]
impl autoxide_kernel::resource::ResourceResolver for Names {
    async fn resolve(
        &self,
        _: &TenantContext,
        id: &str,
    ) -> Result<Option<autoxide_kernel::resource::Resource>, autoxide_kernel::error::Error> {
        let ada = Uuid::from_u128(1).to_string();
        Ok((id == ada).then(|| autoxide_kernel::resource::Resource::new("Ada")))
    }
}

impl autoxide_kernel::module::Module for FakeAuth {
    fn name(&self) -> &'static str {
        "auth"
    }

    fn start(
        &mut self,
        context: &mut autoxide_kernel::module::StartContext<'_>,
    ) -> Result<(), autoxide_kernel::error::Error> {
        context.resource_type("auth.user", Arc::new(Names))
    }
}

/// Đề bài giao diện (lịch sử phiên bản): mỗi phiên bản kèm tên người sửa, tra theo lô qua sổ đối
/// tượng; chương trình không có module tài khoản thì tên là `null`.
#[tokio::test]
async fn phien_ban_kem_ten_nguoi_sua() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let named = TestApp::builder(test)
            .user("admin", 1, &["content.read", "content.write"])
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .module(FakeAuth)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let body = json!({"kind": "post", "title": "Phiên bản"});
        let created = send(&named, "POST", "/entries", Some("admin"), &[], Some(body)).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let list =
            send(&named, "GET", &format!("/entries/{id}/revisions"), Some("admin"), &[], None)
                .await;
        assert_eq!(list.status(), StatusCode::OK, "{}", list.json());
        let first = &list.json()["data"][0];
        assert_eq!(first["created_by_name"], "Ada", "{first}");
        let revision = first["id"].as_str().unwrap().to_owned();
        let one = send(
            &named,
            "GET",
            &format!("/entries/{id}/revisions/{revision}"),
            Some("admin"),
            &[],
            None,
        )
        .await;
        assert_eq!(one.json()["data"]["created_by_name"], "Ada");

        // Không có module tài khoản: tên trống, không lỗi.
        let plain = app(test).await;
        let body = json!({"kind": "post", "title": "Không tên"});
        let created = send(&plain, "POST", "/entries", Some("admin"), &[], Some(body)).await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let list =
            send(&plain, "GET", &format!("/entries/{id}/revisions"), Some("admin"), &[], None)
                .await;
        assert_eq!(list.json()["data"][0]["created_by_name"], Value::Null);
    })
    .await
    .unwrap();
}

/// Đề bài giao diện W-059: `q` của danh sách chuyển hướng tìm trong đường dẫn cũ và địa chỉ đích,
/// không phân biệt hoa thường, có dấu hay không.
#[tokio::test]
async fn tim_chuyen_huong_theo_q() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let body = json!({"locale": "en", "from_path": "/khuyến-mãi-cũ", "to_url": "https://Shop.example.com/Sale", "status_code": 301});
        let created = send(&app, "POST", "/redirects", Some("admin"), &[], Some(body)).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let body = json!({"locale": "en", "from_path": "/about-us", "to_url": "/about", "status_code": 301});
        let created = send(&app, "POST", "/redirects", Some("admin"), &[], Some(body)).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());

        let found = async |q: &str| {
            let list = send(&app, "GET", &format!("/redirects?q={q}"), Some("admin"), &[], None).await;
            assert_eq!(list.status(), StatusCode::OK, "{}", list.json());
            list.json()["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["from_path"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(found("khuyen%20mai").await.len(), 0, "khoảng trắng không phải dấu gạch");
        assert_eq!(found("khuyen-mai").await, ["/khuyến-mãi-cũ"]);
        assert_eq!(found("SHOP.EXAMPLE").await, ["/khuyến-mãi-cũ"], "tìm cả địa chỉ đích");
        assert_eq!(found("about").await, ["/about-us"]);
        assert_eq!(found("50%25").await.len(), 0, "`%` là chữ thường");
        assert_eq!(found("").await.len(), 2);
    })
    .await
    .unwrap();
}

/// Module giả cung cấp `shop.item`: ID bắt đầu bằng `p` là công khai, `h` là đang ẩn, còn lại
/// không có.
struct Shop;

struct ShopItems;

#[async_trait::async_trait]
impl autoxide_kernel::resource::ResourceResolver for ShopItems {
    async fn resolve(
        &self,
        _: &TenantContext,
        id: &str,
    ) -> Result<Option<autoxide_kernel::resource::Resource>, autoxide_kernel::error::Error> {
        Ok((id.starts_with('p') || id.starts_with('h')).then(|| {
            let public = id.starts_with('p');
            autoxide_kernel::resource::Resource::new(if public {
                format!("Item {id}")
            } else {
                String::new()
            })
            .public(public)
            .summary(format!("About {id}"))
            .url(Some(format!("/items/{id}")))
        }))
    }
}

impl autoxide_kernel::module::Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn start(
        &mut self,
        context: &mut autoxide_kernel::module::StartContext<'_>,
    ) -> Result<(), autoxide_kernel::error::Error> {
        context.resource_type("shop.item", Arc::new(ShopItems))
    }
}

fn embed_body(items: &[(&str, &str)]) -> Value {
    let mut content =
        vec![json!({"type": "paragraph", "content": [{"type": "text", "text": "Xem"}]})];
    content.extend(
        items.iter().map(|(kind, id)| json!({"type": "embed", "attrs": {"kind": kind, "id": id}})),
    );
    json!({"type": "doc", "content": content})
}

/// Bước 51 P6: node `embed` dạng `{kind, id}` — loại chưa đăng ký hay bản ghi không có bị từ
/// chối khi lưu; đọc công khai chỉ trả bản ghi còn công khai.
#[tokio::test]
async fn embed_kind_chua_dang_ky_bi_tu_choi_va_bo_doi_tuong_khong_con_cong_khai() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let app = TestApp::builder(test)
            .user("admin", 1, &["content.read", "content.write", "content.publish"])
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .module(Shop)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let create = |body: Value| json!({"kind": "post", "title": "Nhúng sản phẩm", "body": body});

        let unknown = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(create(embed_body(&[("nope.item", "1")]))),
        )
        .await;
        assert_eq!(unknown.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", unknown.json());
        assert_eq!(unknown.json()["errors"][0]["code"], "embed_kind_unknown");
        let missing = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(create(embed_body(&[("shop.item", "x1")]))),
        )
        .await;
        assert_eq!(missing.json()["errors"][0]["code"], "embed_not_found", "{}", missing.json());
        let bad = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(create(embed_body(&[("Shop Item", "")]))),
        )
        .await;
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let body = embed_body(&[("shop.item", "p1"), ("shop.item", "h1")]);
        let created = send(&app, "POST", "/entries", Some("admin"), &[], Some(create(body))).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
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

        let public = send(
            &app,
            "GET",
            "/public/resolve?locale=en&path=/blog/nhung-san-pham",
            None,
            &[],
            None,
        )
        .await;
        assert_eq!(public.status(), StatusCode::OK, "{}", public.json());
        let embeds = public.json()["data"]["entry"]["embeds"].clone();
        assert_eq!(embeds.as_object().unwrap().len(), 1, "chỉ bản ghi còn công khai: {embeds}");
        assert_eq!(embeds["shop.item:p1"]["title"], "Item p1");
        assert_eq!(embeds["shop.item:p1"]["url"], "/items/p1");
    })
    .await
    .unwrap();
}

/// Bước 51 P3: ô tìm Ctrl K (`GET /kernel/find`) tìm bài không dấu, chỉ trong loại người gọi được
/// xem; từ tìm quá ngắn bị từ chối.
#[tokio::test]
async fn tim_xuyen_phan_he_tim_bai_khong_dau() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let created = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "post", "title": "Đường đến Đà Lạt"})),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();

        let found = app.get("/api/v1/kernel/find?q=da%20lat").token("reader").send().await.unwrap();
        assert_eq!(found.status(), StatusCode::OK, "{}", found.json());
        let groups = found.json()["data"]["groups"].clone();
        assert_eq!(groups[0]["kind"], "content.entry", "{groups}");
        assert_eq!(groups[0]["items"][0]["id"], id.as_str());
        assert_eq!(groups[0]["items"][0]["title"], "Đường đến Đà Lạt");
        assert_eq!(groups[0]["items"][0]["screen"], "content.entries");
        let nothing =
            app.get("/api/v1/kernel/find?q=hanoi").token("reader").send().await.unwrap().json();
        assert_eq!(nothing["data"]["groups"], json!([]));
        let short = app.get("/api/v1/kernel/find?q=d").token("reader").send().await.unwrap();
        assert_eq!(short.status(), StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

/// Schema bộ lọc và view đã lưu (bước 51 P7): danh sách bài có `x-autoxide-filters` sinh từ tham
/// số của nó; view lưu `params` sai schema thì 422 nêu từng tham số; view chung cần quyền.
#[tokio::test]
async fn view_da_luu_kiem_theo_schema_bo_loc() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let document = serde_json::to_value(app.openapi()).unwrap();
        let list = &document["paths"]["/api/v1/content/entries"]["get"];
        assert_eq!(list["x-autoxide-list"], "content.list_content_entries");
        let filters = list["x-autoxide-filters"].as_array().unwrap();
        let named = |name: &str| filters.iter().find(|filter| filter["name"] == name).cloned();
        assert_eq!(named("status").unwrap()["values"], json!(["draft", "published", "archived"]));
        assert_eq!(named("scheduled").unwrap()["kind"], "boolean");
        assert_eq!(named("q").unwrap()["operator"], "contains");
        assert!(named("cursor").is_none() && named("sort").is_none(), "không phải bộ lọc");

        let views = |method: Method, uri: &str, who: &str, body: Option<Value>| {
            let mut request = app.request(method, &format!("/api/v1/me/views{uri}")).token(who);
            if let Some(body) = body {
                request = request.json(body);
            }
            request.send()
        };
        let wrong = views(
            Method::POST,
            "",
            "reader",
            Some(json!({"resource": "content.list_content_entries", "name": "Sai",
                "params": {"status": "gone", "scheduled": "maybe", "colour": "red"}})),
        )
        .await
        .unwrap();
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", wrong.json());
        let mut fields: Vec<String> = wrong.json()["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|error| error["field"].as_str().unwrap().to_owned())
            .collect();
        fields.sort();
        assert_eq!(fields, ["params.colour", "params.scheduled", "params.status"]);

        let saved = views(
            Method::POST,
            "",
            "reader",
            Some(json!({"resource": "content.list_content_entries", "name": "Chờ duyệt",
                "params": {"review_requested": true, "sort": "updated_at"},
                "columns": ["title", "status"], "density": "compact"})),
        )
        .await
        .unwrap();
        assert_eq!(saved.status(), StatusCode::CREATED, "{}", saved.json());
        let id = saved.json()["data"]["id"].as_str().unwrap().to_owned();
        let shared = views(
            Method::POST,
            "",
            "reader",
            Some(json!({"resource": "content.list_content_entries", "name": "Cả nhà", "scope": "tenant"})),
        )
        .await
        .unwrap();
        assert_eq!(shared.status(), StatusCode::FORBIDDEN, "cần kernel.views.share");

        let mine = views(Method::GET, "?resource=content.list_content_entries", "reader", None)
            .await
            .unwrap()
            .json();
        assert_eq!(mine["data"][0]["params"]["review_requested"], true);
        let renamed =
            views(Method::PATCH, &format!("/{id}"), "reader", Some(json!({"name": "Duyệt"})))
                .await
                .unwrap();
        assert_eq!(renamed.json()["data"]["name"], "Duyệt");
        let deleted = views(Method::DELETE, &format!("/{id}"), "reader", None).await.unwrap();
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    })
    .await
    .unwrap();
}

/// Xuất danh sách theo bộ lọc (bước 51 P16): job của kernel đọc bài bằng chính bộ lọc của danh
/// sách, ghi CSV chống CSV injection, người yêu cầu tải về; cột lạ là 422, thiếu quyền là 403.
#[tokio::test]
async fn xuat_danh_sach_bai_theo_bo_loc() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let storage: Arc<dyn autoxide_kernel::storage::Storage> =
            Arc::new(autoxide_testkit::MemoryStorage::new());
        let app = TestApp::builder(test)
            .user("admin", 1, &["content.read", "content.write"])
            .user("nobody", 2, &[])
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .provide::<dyn autoxide_kernel::storage::Storage>(storage)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        for (kind, title) in [("post", "=cmd|'/c calc'!A1"), ("post", "Bài hai"), ("page", "Giới thiệu")] {
            let created = send(&app, "POST", "/entries", Some("admin"), &[], Some(json!({"kind": kind, "title": title}))).await;
            assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        }
        let export = |who: &str, body: Value| {
            app.request(Method::POST, "/api/v1/kernel/exports").token(who).json(body).send()
        };
        let resource = "content.list_content_entries";
        let bad = export("admin", json!({"resource": resource, "columns": ["title", "secret"]}))
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", bad.json());
        assert_eq!(bad.json()["errors"][0]["field"], "columns");
        let denied = export("nobody", json!({"resource": resource})).await.unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);

        let queued = export(
            "admin",
            json!({"resource": resource, "params": {"kind": "post", "order": "asc"}, "columns": ["title", "kind"]}),
        )
        .await
        .unwrap();
        assert_eq!(queued.status(), StatusCode::CREATED, "{}", queued.json());
        let id = queued.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(queued.json()["data"]["status"], "queued");
        let worker = app.modules().worker(2).unwrap();
        while worker.run_once().await.unwrap() > 0 {}

        let job = app.get(&format!("/api/v1/kernel/jobs/{id}")).token("admin").send().await.unwrap().json();
        assert_eq!(job["data"]["status"], "succeeded", "{job}");
        assert_eq!(job["data"]["result"]["rows"], 2);
        let url = job["data"]["file"]["url"].as_str().unwrap().to_owned();
        let file = app.get(&url).token("admin").send().await.unwrap();
        assert_eq!(file.status(), StatusCode::OK);
        let csv = file.text();
        assert_eq!(
            csv,
            "\u{feff}\"title\",\"kind\"\r\n\"'=cmd|'/c calc'!A1\",\"post\"\r\n\"Bài hai\",\"post\"\r\n"
        );
        let other = app.get(&url).token("nobody").send().await.unwrap();
        assert_eq!(other.status(), StatusCode::NOT_FOUND, "job của người khác");
        // Job xong thì kernel phát `kernel.job_finished` cho người yêu cầu (module thông báo gửi tin).
        use sea_orm::ConnectionTrait;
        let database = app.database().read();
        let sql = "SELECT payload FROM kernel_events WHERE name = 'kernel.job_finished'";
        let rows = database
            .query_all_raw(sea_orm::Statement::from_string(database.get_database_backend(), sql))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let payload: String = rows[0].try_get("", "payload").unwrap();
        let payload: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!((payload["status"].as_str(), payload["job_id"].as_str()), (Some("succeeded"), Some(id.as_str())));
    })
    .await
    .unwrap();
}

/// Thao tác hàng loạt (bước 51 P8): lưu trữ theo `ids` có kết quả từng bài (bài đã lưu trữ bị bỏ
/// qua, id lạ thất bại kèm `code`); chọn theo bộ lọc mà số khớp khác `expected_count` thì 409;
/// thiếu `Idempotency-Key` là 422.
#[tokio::test]
async fn luu_tru_hang_loat_co_ket_qua_tung_bai() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let mut ids = Vec::new();
        for title in ["Một", "Hai", "Ba"] {
            let created = send(
                &app,
                "POST",
                "/entries",
                Some("admin"),
                &[],
                Some(json!({"kind": "post", "title": title})),
            )
            .await;
            ids.push(created.json()["data"]["id"].as_str().unwrap().to_owned());
        }
        let bulk = |who: &str, key: Option<&str>, body: Value| {
            let mut request = app
                .request(Method::POST, "/api/v1/kernel/bulk/content.list_content_entries/archive")
                .token(who)
                .json(body);
            if let Some(key) = key {
                request = request.header("idempotency-key", key);
            }
            request.send()
        };
        let no_key = bulk("admin", None, json!({"ids": [ids[0]]})).await.unwrap();
        assert_eq!(no_key.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", no_key.json());
        let denied = bulk("reader", Some(&Uuid::now_v7().to_string()), json!({"ids": [ids[0]]}))
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let changed = bulk(
            "admin",
            Some(&Uuid::now_v7().to_string()),
            json!({"filter": {"kind": "post"}, "expected_count": 2}),
        )
        .await
        .unwrap();
        assert_eq!(
            (changed.status(), changed.json()["code"].as_str()),
            (StatusCode::CONFLICT, Some("kernel.bulk_count_changed"))
        );

        let preview = bulk("admin", None, json!({"filter": {"kind": "post"}, "dry_run": true}))
            .await
            .unwrap();
        assert_eq!(preview.status(), StatusCode::OK, "{}", preview.json());
        assert_eq!(preview.json()["data"]["count"], 3);
        let first = bulk("admin", Some(&Uuid::now_v7().to_string()), json!({"ids": [ids[0]]}))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::CREATED, "{}", first.json());
        let worker = app.modules().worker(2).unwrap();
        while worker.run_once().await.unwrap() > 0 {}
        let unknown = Uuid::now_v7().to_string();
        let queued = bulk(
            "admin",
            Some(&Uuid::now_v7().to_string()),
            json!({"ids": [ids[0], ids[1], unknown]}),
        )
        .await
        .unwrap();
        assert_eq!(queued.status(), StatusCode::CREATED, "{}", queued.json());
        let job_id = queued.json()["data"]["id"].as_str().unwrap().to_owned();
        while worker.run_once().await.unwrap() > 0 {}
        let job = app
            .get(&format!("/api/v1/kernel/jobs/{job_id}"))
            .token("admin")
            .send()
            .await
            .unwrap()
            .json();
        let result = &job["data"]["result"];
        assert_eq!(
            (result["ok"].as_u64(), result["skipped"].as_u64(), result["failed"].as_u64()),
            (Some(1), Some(1), Some(1)),
            "{job}"
        );
        assert_eq!(result["items"][0]["code"], "already_archived");
        assert_eq!(result["items"][2]["code"], "content.not_found");
        let third =
            send(&app, "GET", &format!("/entries/{}", ids[2]), Some("admin"), &[], None).await;
        assert_eq!(third.json()["data"]["status"], "draft", "bài không được chọn giữ nguyên");
    })
    .await
    .unwrap();
}

/// Tìm nâng cao và mở thẳng (bước 51 P9): `is:`, `status:`, phủ định, so ngày; trường lạ vào
/// `ignored`; `GET /kernel/resolve?code=` mở bài theo slug hay đường dẫn, mã lạ là 404.
#[tokio::test]
async fn tim_nang_cao_va_mo_thang_theo_ma() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        for (kind, title) in [("post", "Đà Lạt mùa hoa"), ("page", "Đà Lạt giới thiệu")]
        {
            let created = send(
                &app,
                "POST",
                "/entries",
                Some("admin"),
                &[],
                Some(json!({"kind": kind, "title": title})),
            )
            .await;
            assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        }
        let find = |q: &str| {
            let uri = format!(
                "/api/v1/kernel/find?q={}",
                q.replace(' ', "%20").replace(':', "%3A").replace('>', "%3E")
            );
            app.get(&uri).token("reader").send()
        };
        let titles = |body: &Value| -> Vec<String> {
            body["data"]["groups"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|group| {
                    group["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|item| item["title"].as_str().unwrap().to_owned())
                })
                .collect()
        };
        let pages = find("da lat kind:page").await.unwrap().json();
        assert_eq!(titles(&pages), ["Đà Lạt giới thiệu"], "{pages}");
        let not_pages = find("is:entry -kind:page status:draft").await.unwrap().json();
        assert_eq!(titles(&not_pages), ["Đà Lạt mùa hoa"], "{not_pages}");
        let future = find("da lat updated>2999-01-01").await.unwrap().json();
        assert!(titles(&future).is_empty(), "{future}");
        let odd = find("da lat colour:red").await.unwrap().json();
        assert_eq!(odd["data"]["ignored"], json!(["colour:red"]));
        assert_eq!(titles(&odd).len(), 2);

        let elsewhere = app
            .get(&format!("/api/v1/kernel/find?q=da%20lat&scope=node&org_id={}", Uuid::now_v7()))
            .token("reader")
            .send()
            .await
            .unwrap()
            .json();
        assert!(titles(&elsewhere).is_empty(), "nút khác không có bài: {elsewhere}");
        let no_unit = app
            .get("/api/v1/kernel/find?q=da%20lat&scope=subtree")
            .token("reader")
            .send()
            .await
            .unwrap();
        assert_eq!(no_unit.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let resolved = app
            .get("/api/v1/kernel/resolve?code=%2Fblog%2Fda-lat-mua-hoa")
            .token("reader")
            .send()
            .await
            .unwrap();
        assert_eq!(resolved.status(), StatusCode::OK, "{}", resolved.json());
        assert_eq!(
            (resolved.json()["data"]["kind"].as_str(), resolved.json()["data"]["title"].as_str()),
            (Some("content.entry"), Some("Đà Lạt mùa hoa"))
        );
        let missing = app
            .get("/api/v1/kernel/resolve?code=nothing-here")
            .token("reader")
            .send()
            .await
            .unwrap();
        assert_eq!(
            (missing.status(), missing.json()["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("kernel.code_not_found"))
        );
    })
    .await
    .unwrap();
}

/// Hộp việc theo người và việc thiết lập (bước 51 P10): hoãn ẩn việc khỏi hộp của người hoãn (bỏ
/// hoãn thì hiện lại); giao cho người không có quyền của nguồn là 422; `assigned=me` lọc; bước
/// thiết lập "trang đầu tiên" xong khi có trang đã đăng.
#[tokio::test]
async fn hoan_giao_viec_va_buoc_thiet_lap() {
    TestDatabase::run(async |test| {
        // Mỗi người một ID (app chung cho `admin` và `reader` cùng ID 1): giao việc tra quyền theo ID.
        let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let app = TestApp::builder(test)
            .user("admin", 1, &["content.read", "content.write", "content.publish"])
            .user("editor", 3, &["content.read", "content.publish"])
            .user("writer", 2, &["content.read", "content.write"])
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let created = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "page", "title": "Giới thiệu"})),
        )
        .await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let read = send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();
        let asked = send(
            &app,
            "POST",
            &format!("/entries/{id}/request-review"),
            Some("admin"),
            &[("if-match", &etag)],
            None,
        )
        .await;
        assert_eq!(asked.status(), StatusCode::OK, "{}", asked.json());
        let work =
            |query: &str| app.get(&format!("/api/v1/kernel/work{query}")).token("admin").send();
        let items = work("").await.unwrap().json();
        let key = items["data"]["items"][0]["key"].as_str().unwrap().to_owned();
        assert!(key.ends_with(&id), "{items}");
        let me = |method: Method, path: &str, body: Option<Value>| {
            let mut request =
                app.request(method, &format!("/api/v1/me/work/{key}/{path}")).token("admin");
            if let Some(body) = body {
                request = request.json(body);
            }
            request.send()
        };

        let past = me(Method::POST, "snooze", Some(json!({"until": "2000-01-01T00:00:00Z"})))
            .await
            .unwrap();
        assert_eq!(past.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let later = (chrono::Utc::now() + chrono::Duration::days(30))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let snoozed = me(Method::POST, "snooze", Some(json!({"until": later}))).await.unwrap();
        assert_eq!(snoozed.status(), StatusCode::OK, "{}", snoozed.json());
        assert_eq!(work("").await.unwrap().json()["data"]["items"], json!([]), "đã hoãn");
        let with = work("?include_snoozed=true").await.unwrap().json();
        assert!(with["data"]["items"][0]["snoozed_until"].is_string(), "{with}");
        assert_eq!(
            me(Method::DELETE, "snooze", None).await.unwrap().status(),
            StatusCode::NO_CONTENT
        );

        let refused =
            me(Method::POST, "assign", Some(json!({"user_id": Uuid::from_u128(2)}))).await.unwrap();
        assert_eq!(
            refused.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "writer không có content.publish"
        );
        let assigned =
            me(Method::POST, "assign", Some(json!({"user_id": Uuid::from_u128(1)}))).await.unwrap();
        assert_eq!(assigned.json()["data"]["assignee_id"], Uuid::from_u128(1).to_string());
        assert_eq!(work("?assigned=me").await.unwrap().json()["data"]["items"][0]["key"], key);
        assert_eq!(work("?assigned=nobody").await.unwrap().json()["data"]["items"], json!([]));
        let unknown = app
            .request(Method::POST, "/api/v1/me/work/nope:1/snooze")
            .token("admin")
            .json(json!({"until": later}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            (unknown.status(), unknown.json()["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("kernel.work_not_found"))
        );

        let setup = || app.get("/api/v1/kernel/setup").token("admin").send();
        let before = setup().await.unwrap().json();
        let step = before["data"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|step| step["id"] == "content.first_page")
            .cloned()
            .unwrap();
        assert_eq!(
            (step["done"].as_bool(), step["screen"].as_str()),
            (Some(false), Some("content.entries"))
        );
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
        let after = setup().await.unwrap().json();
        let step = after["data"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|step| step["id"] == "content.first_page")
            .cloned()
            .unwrap();
        assert_eq!(step["done"], true, "{after}");
    })
    .await
    .unwrap();
}

/// Ghi chú và dòng hoạt động trên bản ghi (bước 51 P13): người đọc được bài viết ghi chú, nhắc
/// người đọc được; nhắc người không đọc được là 422; người không có `content.read` thấy 404; chỉ
/// người viết xoá được ghi chú.
#[tokio::test]
async fn ghi_chu_tren_ban_ghi() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let app = TestApp::builder(test)
            .user("admin", 1, &["content.read", "content.write"])
            .user("editor", 3, &["content.read"])
            .user("stranger", 4, &[])
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let created = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "post", "title": "Ghi chú"})),
        )
        .await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let notes = format!("/api/v1/kernel/records/content.entry/{id}/notes");
        let post = |who: &str, body: Value| app.post(&notes).token(who).json(body).send();

        let wrong = post("admin", json!({"body": "Xem giúp", "mentions": [Uuid::from_u128(4)]}))
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", wrong.json());
        assert_eq!(wrong.json()["errors"][0]["field"], "mentions[0]");
        let written = post("admin", json!({"body": "Xem giúp", "mentions": [Uuid::from_u128(3)]}))
            .await
            .unwrap();
        assert_eq!(written.status(), StatusCode::CREATED, "{}", written.json());
        let note_id = written.json()["data"]["id"].as_str().unwrap().to_owned();
        let hidden = post("stranger", json!({"body": "x"})).await.unwrap();
        assert_eq!(
            (hidden.status(), hidden.json()["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("kernel.record_not_found"))
        );
        let missing = app
            .get(&format!("/api/v1/kernel/records/content.entry/{}/notes", Uuid::now_v7()))
            .token("admin")
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);

        let activity = app
            .get(&format!("/api/v1/kernel/records/content.entry/{id}/activity"))
            .token("editor")
            .send()
            .await
            .unwrap();
        assert_eq!(activity.status(), StatusCode::OK, "{}", activity.json());
        let first = &activity.json()["data"][0];
        assert_eq!(
            (first["kind"].as_str(), first["text"].as_str()),
            (Some("note"), Some("Xem giúp"))
        );
        let theirs =
            app.delete(&format!("{notes}/{note_id}")).token("editor").send().await.unwrap();
        assert_eq!(theirs.status(), StatusCode::NOT_FOUND, "chỉ người viết xoá được");
        let mine = app.delete(&format!("{notes}/{note_id}")).token("admin").send().await.unwrap();
        assert_eq!(mine.status(), StatusCode::NO_CONTENT);
    })
    .await
    .unwrap();
}

/// Lệnh cần duyệt khi agent gọi (bước 51 P12): agent xoá bài thì nhận 202 và yêu cầu duyệt, bài
/// còn nguyên; agent không tự duyệt được; gửi lại trước khi duyệt là 409; người duyệt xong thì gửi
/// lại cùng request (cùng `Idempotency-Key`) kèm `Autoxide-Approval` mới xoá thật, một lần.
#[tokio::test]
async fn agent_goi_lenh_rui_ro_nhan_202_approval_required() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let perms = ["content.read", "content.write", "content.delete"].map(str::to_owned);
        let robot = Principal::new(Uuid::from_u128(1), perms.clone())
            .via_api_key(Uuid::from_u128(9))
            .with_actor_kind(autoxide_kernel::access::ActorKind::Agent);
        let app = TestApp::builder(test)
            .user(
                "editor",
                2,
                &["content.read", "content.write", "content.delete", "kernel.devtools.read"],
            )
            .principal("robot", robot)
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let created = send(
            &app,
            "POST",
            "/entries",
            Some("editor"),
            &[],
            Some(json!({"kind": "post", "title": "Sắp xoá"})),
        )
        .await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let read = send(&app, "GET", &format!("/entries/{id}"), Some("editor"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();
        let key = Uuid::now_v7().to_string();
        let delete = |approval: Option<&str>| {
            let mut headers = vec![("if-match", etag.as_str()), ("idempotency-key", key.as_str())];
            if let Some(approval) = approval {
                headers.push(("autoxide-approval", approval));
            }
            let path = format!("/entries/{id}");
            let headers: Vec<(String, String)> =
                headers.into_iter().map(|(a, b)| (a.to_owned(), b.to_owned())).collect();
            let app = &app;
            async move {
                let pairs: Vec<(&str, &str)> =
                    headers.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
                send(app, "DELETE", &path, Some("robot"), &pairs, None).await
            }
        };

        let asked = delete(None).await;
        assert_eq!(asked.status(), StatusCode::ACCEPTED, "{}", asked.json());
        let approval = asked.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(asked.json()["data"]["status"], "pending");
        assert_eq!(
            send(&app, "GET", &format!("/entries/{id}"), Some("editor"), &[], None).await.status(),
            StatusCode::OK,
            "chưa xoá"
        );
        let early = delete(Some(&approval)).await;
        assert_eq!(
            (early.status(), early.json()["code"].as_str()),
            (StatusCode::CONFLICT, Some("kernel.approval_invalid"))
        );
        let decide = |who: &str| {
            app.post(&format!("/api/v1/kernel/approvals/{approval}/approve")).token(who).send()
        };
        let own = decide("robot").await.unwrap();
        assert_eq!(
            (own.status(), own.json()["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("kernel.approval_forbidden"))
        );
        let pending = app
            .get("/api/v1/kernel/approvals?status=pending")
            .token("editor")
            .send()
            .await
            .unwrap()
            .json();
        assert_eq!(pending["data"][0]["id"], approval, "người duyệt thấy yêu cầu");
        let approved = decide("editor").await.unwrap();
        assert_eq!(approved.json()["data"]["status"], "approved", "{}", approved.json());

        let done = delete(Some(&approval)).await;
        assert_eq!(
            done.status(),
            StatusCode::NO_CONTENT,
            "{}",
            String::from_utf8_lossy(done.body())
        );
        assert_eq!(
            send(&app, "GET", &format!("/entries/{id}"), Some("editor"), &[], None).await.status(),
            StatusCode::NOT_FOUND
        );
        // Nhật ký lệnh của agent (D-202): dòng 202 và dòng chạy thật cùng giữ `approval_id`.
        let mut rows = Vec::new();
        for _ in 0..100 {
            let page = app
                .get("/api/v1/kernel/devtools/requests?actor_kind=agent&path=/api/v1/content/entries/{id}")
                .token("editor")
                .send()
                .await
                .unwrap()
                .json();
            rows = page["data"].as_array().unwrap().clone();
            if rows.iter().filter(|row| row["method"] == "DELETE").count() >= 3 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        let statuses: Vec<(u64, &str)> = rows
            .iter()
            .filter(|row| row["method"] == "DELETE")
            .map(|row| (row["status"].as_u64().unwrap(), row["approval_id"].as_str().unwrap_or("")))
            .collect();
        assert_eq!(
            statuses,
            [(204, approval.as_str()), (409, approval.as_str()), (202, approval.as_str())],
            "lenh_cho_duyet_giu_approval_id"
        );
        let used = app
            .get(&format!("/api/v1/kernel/approvals/{approval}"))
            .token("robot")
            .send()
            .await
            .unwrap()
            .json();
        assert_eq!(used["data"]["status"], "used");
    })
    .await
    .unwrap();
}

/// Nhãn nội bộ (bước 51 P20): gắn cần quyền ghi bài; tên trùng khác hoa thường / dấu là một nhãn;
/// lọc `tags_any` / `tags_all`; bài trả `tags`; gộp nhãn chuyển hết bản ghi; khách bật
/// `kernel.tags.restricted` thì người không có `kernel.tags.manage` không tạo nhãn mới.
#[tokio::test]
async fn nhan_noi_bo_tren_bai_viet() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let app = TestApp::builder(test)
            .user(
                "admin",
                1,
                &["content.read", "content.write", "kernel.tags.manage", "kernel.settings.manage"],
            )
            .user("writer", 2, &["content.read", "content.write"])
            .user("reader", 3, &["content.read"])
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let mut ids = Vec::new();
        for title in ["Một", "Hai"] {
            let created = send(
                &app,
                "POST",
                "/entries",
                Some("admin"),
                &[],
                Some(json!({"kind": "post", "title": title})),
            )
            .await;
            ids.push(created.json()["data"]["id"].as_str().unwrap().to_owned());
        }
        let tag = |who: &str, id: &str, body: Value| {
            app.put(&format!("/api/v1/kernel/records/content.entry/{id}/tags"))
                .token(who)
                .json(body)
                .send()
        };
        let first = tag("writer", &ids[0], json!({"new_tags": ["VIP", "Cần gọi"]})).await.unwrap();
        assert_eq!(first.status(), StatusCode::OK, "{}", first.json());
        assert_eq!(first.json()["data"].as_array().unwrap().len(), 2);
        let second = tag("writer", &ids[1], json!({"new_tags": ["vip"]})).await.unwrap();
        let vip = second.json()["data"][0]["id"].as_str().unwrap().to_owned();
        assert_eq!(second.json()["data"][0]["name"], "VIP", "cùng tên khác hoa thường là một nhãn");
        assert_eq!(
            tag("reader", &ids[0], json!({"tag_ids": []})).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );

        let all = app.get("/api/v1/kernel/tags").token("reader").send().await.unwrap().json();
        let call = all["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "Cần gọi")
            .cloned()
            .unwrap();
        assert_eq!(
            all["data"].as_array().unwrap().iter().find(|t| t["name"] == "VIP").unwrap()["uses"],
            2
        );
        let call_id = call["id"].as_str().unwrap().to_owned();
        let list = |query: String| {
            let app = &app;
            async move {
                send(app, "GET", &format!("/entries?{query}"), Some("reader"), &[], None).await
            }
        };
        assert_eq!(
            list(format!("tags_any={vip}")).await.json()["data"].as_array().unwrap().len(),
            2
        );
        let both = list(format!("tags_all={vip},{call_id}")).await.json();
        assert_eq!(both["data"].as_array().unwrap().len(), 1, "{both}");
        assert_eq!(both["data"][0]["tags"].as_array().unwrap().len(), 2);

        let merged = app
            .post(&format!("/api/v1/kernel/tags/{call_id}/merge"))
            .token("admin")
            .json(json!({"into": vip}))
            .send()
            .await
            .unwrap();
        assert_eq!(merged.status(), StatusCode::OK, "{}", merged.json());
        let after = list(format!("tags_any={vip}")).await.json();
        assert!(
            after["data"].as_array().unwrap().iter().all(|entry| entry["tags"]
                .as_array()
                .unwrap()
                .len()
                == 1)
        );

        let restricted = app
            .put("/api/v1/kernel/settings/kernel.tags.restricted")
            .token("admin")
            .json(json!({"value": "true"}))
            .send()
            .await
            .unwrap();
        assert_eq!(restricted.status(), StatusCode::OK, "{}", restricted.json());
        let refused =
            tag("writer", &ids[0], json!({"tag_ids": [vip], "new_tags": ["Mới"]})).await.unwrap();
        assert_eq!(
            (refused.status(), refused.json()["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("kernel.tags_restricted"))
        );
        assert_eq!(
            tag("writer", &ids[0], json!({"tag_ids": []})).await.unwrap().status(),
            StatusCode::OK,
            "bỏ nhãn vẫn được"
        );
    })
    .await
    .unwrap();
}

/// Trường tự định nghĩa (bước 51 P19): khách định nghĩa trường cho bài viết; lỗi gom một lần; giá
/// trị mặc định; lọc `cf.<khoá>` qua index ghi cùng transaction; trường không lọc được là 422; đổi
/// kiểu trường đã dùng là 409.
#[tokio::test]
async fn truong_tu_dinh_nghia_tren_bai_viet() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let app = TestApp::builder(test)
            .user("admin", 1, &["content.read", "content.write", "kernel.custom_fields.manage"])
            .user("reader", 2, &["content.read"])
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let define = |body: Value| app.post("/api/v1/kernel/custom-fields").token("admin").json(body).send();
        let owner = define(json!({"target": "content.entry", "key": "owner", "data_type": "text", "labels": {"en": "Owner"}, "filterable": true})).await.unwrap();
        assert_eq!(owner.status(), StatusCode::CREATED, "{}", owner.json());
        let owner_id = owner.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(define(json!({"target": "content.entry", "key": "budget", "data_type": "integer", "labels": {"en": "Budget"}, "min": "0", "filterable": true})).await.unwrap().status(), StatusCode::CREATED);
        assert_eq!(define(json!({"target": "content.entry", "key": "stage", "data_type": "option", "labels": {"en": "Stage"}, "options": ["draft", "review"], "required": true, "default": "draft"})).await.unwrap().status(), StatusCode::CREATED);
        let bad = define(json!({"target": "content.entry", "key": "Bad Key", "data_type": "colour", "labels": {}})).await.unwrap();
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let fields = app.get("/api/v1/kernel/custom-fields?target=content.entry").token("reader").send().await.unwrap().json();
        assert_eq!(fields["data"].as_array().unwrap().len(), 3, "người đọc bài đọc được định nghĩa");

        let wrong = send(&app, "POST", "/entries", Some("admin"), &[], Some(json!({"kind": "post", "title": "Sai", "custom_fields": {"budget": -5, "colour": "x"}}))).await;
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", wrong.json());
        let mut paths: Vec<String> = wrong.json()["errors"].as_array().unwrap().iter().map(|e| e["field"].as_str().unwrap().to_owned()).collect();
        paths.sort();
        assert_eq!(paths, ["custom_fields.budget", "custom_fields.colour"]);
        let first = send(&app, "POST", "/entries", Some("admin"), &[], Some(json!({"kind": "post", "title": "Một", "custom_fields": {"owner": "Ada Lovelace", "budget": 100}}))).await;
        assert_eq!(first.status(), StatusCode::CREATED, "{}", first.json());
        assert_eq!(first.json()["data"]["custom_fields"]["stage"], "draft", "giá trị mặc định");
        let id = first.json()["data"]["id"].as_str().unwrap().to_owned();
        let second = send(&app, "POST", "/entries", Some("admin"), &[], Some(json!({"kind": "post", "title": "Hai", "custom_fields": {"owner": "Bob", "budget": 50}}))).await;
        assert_eq!(second.status(), StatusCode::CREATED);

        let count = |query: &'static str| {
            let app = &app;
            async move {
                let answer = send(app, "GET", &format!("/entries?{query}"), Some("reader"), &[], None).await;
                (answer.status(), answer.json()["data"].as_array().map_or(0, Vec::len))
            }
        };
        assert_eq!(count("cf.owner=prefix:ada").await, (StatusCode::OK, 1));
        assert_eq!(count("cf.budget=gte:60").await, (StatusCode::OK, 1));
        assert_eq!(count("cf.budget=in:50,100").await, (StatusCode::OK, 2));
        assert_eq!(count("cf.stage=eq:draft").await.0, StatusCode::UNPROCESSABLE_ENTITY, "không lọc được");

        let read = send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();
        let cleared = send(&app, "PATCH", &format!("/entries/{id}"), Some("admin"), &[("if-match", &etag)], Some(json!({"custom_fields": {"budget": null}}))).await;
        assert_eq!(cleared.status(), StatusCode::OK, "{}", cleared.json());
        assert!(cleared.json()["data"]["custom_fields"].get("budget").is_none());
        assert_eq!(count("cf.budget=gte:60").await, (StatusCode::OK, 0), "index ghi lại cùng lúc");

        let locked = app.patch(&format!("/api/v1/kernel/custom-fields/{owner_id}")).token("admin").json(json!({"data_type": "integer"})).send().await.unwrap();
        assert_eq!((locked.status(), locked.json()["code"].as_str()), (StatusCode::CONFLICT, Some("kernel.custom_field_type_locked")));
    })
    .await
    .unwrap();
}

/// Hai nhánh anh em của cây tổ chức (bước 51, test bắt buộc): bài chờ duyệt ở nhánh A không hiện
/// trong hộp việc, số đếm, kết quả tìm hay mở theo mã của người chỉ làm ở nhánh B.
#[tokio::test]
async fn viec_va_tim_khong_lo_sang_nhanh_anh_em() {
    use autoxide_kernel::access::Scope;
    use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree};
    TestDatabase::run(async |test| {
        let (a, b) = (OrgId::from_uuid(Uuid::now_v7()), OrgId::from_uuid(Uuid::now_v7()));
        let grants = [
            ("content.read", Scope::Subtree),
            ("content.write", Scope::Subtree),
            ("content.publish", Scope::Subtree),
        ];
        let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let app = TestApp::builder(test)
            .member("north", 1, a, &grants)
            .member("south", 2, b, &grants)
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let write = app.database().begin_write().await.unwrap();
        let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, a, root, 8).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, b, root, 8).await.unwrap();
        write.commit().await.unwrap();

        let created = send(
            &app,
            "POST",
            "/entries",
            Some("north"),
            &[],
            Some(json!({"kind": "post", "title": "Nhanh bac bi mat"})),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let read = send(&app, "GET", &format!("/entries/{id}"), Some("north"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();
        let asked = send(
            &app,
            "POST",
            &format!("/entries/{id}/request-review"),
            Some("north"),
            &[("if-match", &etag)],
            None,
        )
        .await;
        assert_eq!(asked.status(), StatusCode::OK, "{}", asked.json());

        let get = |who: &'static str, uri: String| {
            let app = &app;
            async move { app.get(&uri).token(who).send().await.unwrap().json() }
        };
        let (a_id, b_id) = (a.as_uuid(), b.as_uuid());
        let north = get("north", format!("/api/v1/kernel/work?org_id={a_id}")).await;
        assert_eq!(north["data"]["items"].as_array().unwrap().len(), 1, "{north}");
        for unit in [format!("org_id={b_id}"), String::new()] {
            let south = get("south", format!("/api/v1/kernel/work?{unit}")).await;
            assert_eq!(south["data"]["items"], json!([]), "{south}");
            let counts = get("south", format!("/api/v1/kernel/work/counts?{unit}")).await;
            assert_eq!(counts["data"]["counts"]["work"], 0, "{counts}");
            let found = get("south", format!("/api/v1/kernel/find?q=bi%20mat&{unit}")).await;
            assert_eq!(found["data"]["groups"], json!([]), "{found}");
        }
        let theirs = get("north", format!("/api/v1/kernel/find?q=bi%20mat&org_id={a_id}")).await;
        assert_eq!(theirs["data"]["groups"].as_array().unwrap().len(), 1, "{theirs}");
        let resolved = app
            .get("/api/v1/kernel/resolve?code=nhanh-bac-bi-mat")
            .token("south")
            .send()
            .await
            .unwrap();
        assert_eq!(resolved.status(), StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
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

/// Hoàn tác an toàn (bước 52): bài đã lưu trữ về lại nháp nguyên nội dung; bài chưa lưu trữ thì
/// 409; chạy thử thao tác hàng loạt không ghi gì, không phát event.
#[tokio::test]
async fn restore_tra_lai_trang_thai_truoc() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let created = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "post", "title": "Lưu rồi lấy lại", "excerpt": "Tóm tắt"})),
        )
        .await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let etag_of = async |id: &str| {
            send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None)
                .await
                .header(header::ETAG)
                .to_owned()
        };
        let etag = etag_of(&id).await;
        let early = send(
            &app,
            "POST",
            &format!("/entries/{id}/unarchive"),
            Some("admin"),
            &[("if-match", etag.as_str())],
            None,
        )
        .await;
        assert_eq!(
            (early.status(), early.json()["code"].as_str()),
            (StatusCode::CONFLICT, Some("content.not_archived"))
        );
        let archived = send(
            &app,
            "POST",
            &format!("/entries/{id}/archive"),
            Some("admin"),
            &[("if-match", etag.as_str())],
            None,
        )
        .await;
        assert_eq!(archived.json()["data"]["status"], "archived", "{}", archived.json());
        let etag = etag_of(&id).await;
        let denied = send(
            &app,
            "POST",
            &format!("/entries/{id}/unarchive"),
            Some("reader"),
            &[("if-match", etag.as_str())],
            None,
        )
        .await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let restored = send(
            &app,
            "POST",
            &format!("/entries/{id}/unarchive"),
            Some("admin"),
            &[("if-match", etag.as_str())],
            None,
        )
        .await;
        assert_eq!(restored.status(), StatusCode::OK, "{}", restored.json());
        let data = restored.json()["data"].clone();
        assert_eq!(
            (data["status"].as_str(), data["title"].as_str(), data["excerpt"].as_str()),
            (Some("draft"), Some("Lưu rồi lấy lại"), Some("Tóm tắt"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dry_run_khong_ghi_khong_phat_event() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        for title in ["Một", "Hai"] {
            send(
                &app,
                "POST",
                "/entries",
                Some("admin"),
                &[],
                Some(json!({"kind": "post", "title": title})),
            )
            .await;
        }
        let before = outbox_count(&app).await;
        let preview = app
            .request(Method::POST, "/api/v1/kernel/bulk/content.list_content_entries/archive")
            .token("admin")
            .json(json!({"filter": {"kind": "post"}, "dry_run": true}))
            .send()
            .await
            .unwrap();
        assert_eq!(preview.status(), StatusCode::OK, "{}", preview.json());
        assert_eq!(preview.json()["data"]["count"], 2);
        assert_eq!(outbox_count(&app).await, before, "không phát event");
        let list = send(&app, "GET", "/entries?status=archived", Some("admin"), &[], None).await;
        assert_eq!(list.json()["data"], json!([]), "không ghi gì");
    })
    .await
    .unwrap();
}

/// Gắn nhãn hàng loạt (bước 51 P20): một nhãn lên nhiều bài một lần, bài đã có hay không có thì
/// bỏ qua; gỡ hàng loạt; cần quyền ghi bài.
#[tokio::test]
async fn gan_nhan_hang_loat() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let mut ids = Vec::new();
        for title in ["Một", "Hai"] {
            let created = send(
                &app,
                "POST",
                "/entries",
                Some("admin"),
                &[],
                Some(json!({"kind": "post", "title": title})),
            )
            .await;
            ids.push(created.json()["data"]["id"].as_str().unwrap().to_owned());
        }
        // Nhãn mới tạo bằng cách gắn lên một bài thứ ba (người viết tạo được nhãn khi khách không hạn chế).
        let third = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "post", "title": "Ba"})),
        )
        .await;
        let third = third.json()["data"]["id"].as_str().unwrap().to_owned();
        let tagged = app
            .put(&format!("/api/v1/kernel/records/content.entry/{third}/tags"))
            .token("admin")
            .json(json!({"new_tags": ["Mùa hè"]}))
            .send()
            .await
            .unwrap();
        assert_eq!(tagged.status(), StatusCode::OK, "{}", tagged.json());
        let tag = tagged.json()["data"][0]["id"].as_str().unwrap().to_owned();
        let apply = |who: &'static str, body: Value| {
            let app = &app;
            let path = format!("/api/v1/kernel/tags/{tag}/apply");
            async move { app.post(&path).token(who).json(body).send().await.unwrap() }
        };
        let missing = Uuid::now_v7().to_string();
        let added =
            apply("admin", json!({"kind": "content.entry", "add": [ids[0], ids[1], missing]}))
                .await;
        assert_eq!(added.status(), StatusCode::OK, "{}", added.json());
        assert_eq!(added.json()["data"]["added"], json!([ids[0], ids[1]]));
        assert_eq!(added.json()["data"]["skipped"], json!([missing]));
        let again =
            apply("admin", json!({"kind": "content.entry", "add": [ids[0]], "remove": [ids[1]]}))
                .await;
        assert_eq!(again.json()["data"]["skipped"], json!([ids[0]]), "đã có thì bỏ qua");
        assert_eq!(again.json()["data"]["removed"], json!([ids[1]]));
        let tagged =
            send(&app, "GET", &format!("/entries?tags_any={tag}"), Some("admin"), &[], None).await;
        let tagged = tagged.json();
        let found: Vec<&str> =
            tagged["data"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap()).collect();
        assert_eq!(found.len(), 2);
        assert!(found.contains(&ids[0].as_str()) && !found.contains(&ids[1].as_str()));
        assert_eq!(
            apply("reader", json!({"kind": "content.entry", "add": [ids[1]]})).await.status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            apply("admin", json!({"kind": "content.entry"})).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    })
    .await
    .unwrap();
}

/// Trường tự định nghĩa (bước 51 P19, phần còn lại): trường mức `public` vào bản công khai lúc đăng,
/// trường nội bộ không lộ; file xuất có cột `cf.<khoá>`; cột `cf.` lạ là 422.
#[tokio::test]
async fn truong_cong_khai_va_cot_xuat() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let storage: Arc<dyn autoxide_kernel::storage::Storage> =
            Arc::new(autoxide_testkit::MemoryStorage::new());
        let app = TestApp::builder(test)
            .user(
                "admin",
                1,
                &["content.read", "content.write", "content.publish", "kernel.custom_fields.manage"],
            )
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .provide::<dyn autoxide_kernel::storage::Storage>(storage)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let define = |body: Value| app.post("/api/v1/kernel/custom-fields").token("admin").json(body).send();
        let teaser = define(json!({"target": "content.entry", "key": "teaser", "data_type": "text", "labels": {"en": "Teaser"}, "visibility": "public"})).await.unwrap();
        assert_eq!(teaser.status(), StatusCode::CREATED, "{}", teaser.json());
        let owner = define(json!({"target": "content.entry", "key": "owner", "data_type": "text", "labels": {"en": "Owner"}})).await.unwrap();
        assert_eq!(owner.status(), StatusCode::CREATED);
        let created = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "post", "title": "Truong cong khai", "custom_fields": {"teaser": "Đọc ngay", "owner": "Ann"}})),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let etag = send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await.header(header::ETAG).to_owned();
        let published = send(&app, "POST", &format!("/entries/{id}/publish"), Some("admin"), &[("if-match", &etag)], None).await;
        assert_eq!(published.status(), StatusCode::OK, "{}", published.json());
        let found = send(&app, "GET", "/public/resolve?locale=en&path=/blog/truong-cong-khai", None, &[], None).await;
        assert_eq!(found.status(), StatusCode::OK, "{}", found.json());
        assert_eq!(found.json()["data"]["entry"]["custom_fields"], json!({"teaser": "Đọc ngay"}), "trường nội bộ không lộ");

        let export = |body: Value| app.request(Method::POST, "/api/v1/kernel/exports").token("admin").json(body).send();
        let resource = "content.list_content_entries";
        let bad = export(json!({"resource": resource, "columns": ["title", "cf.nothing"]})).await.unwrap();
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let queued = export(json!({"resource": resource, "columns": ["title", "cf.owner", "cf.teaser"]})).await.unwrap();
        assert_eq!(queued.status(), StatusCode::CREATED, "{}", queued.json());
        let job_id = queued.json()["data"]["id"].as_str().unwrap().to_owned();
        let worker = app.modules().worker(2).unwrap();
        while worker.run_once().await.unwrap() > 0 {}
        let job = app.get(&format!("/api/v1/kernel/jobs/{job_id}")).token("admin").send().await.unwrap().json();
        assert_eq!(job["data"]["status"], "succeeded", "{job}");
        let url = job["data"]["file"]["url"].as_str().unwrap().to_owned();
        let csv = app.get(&url).token("admin").send().await.unwrap().text();
        assert_eq!(
            csv,
            "\u{feff}\"title\",\"cf.owner\",\"cf.teaser\"\r\n\"Truong cong khai\",\"Ann\",\"Đọc ngay\"\r\n"
        );
    })
    .await
    .unwrap();
}

/// "Đang dùng ở đâu" (bước 51, D-267): bài nhúng bài khác thì bài kia biết mình được dùng ở đâu;
/// người không đọc được bản ghi được hỏi thì 404.
#[tokio::test]
async fn dang_dung_o_dau() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let target = send(&app, "POST", "/entries", Some("admin"), &[], Some(json!({"kind": "post", "title": "Được nhúng"}))).await;
        let target = target.json()["data"]["id"].as_str().unwrap().to_owned();
        let user = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "post", "title": "Bài nhúng", "body": embed_body(&[("content.entry", target.as_str())])})),
        )
        .await;
        assert_eq!(user.status(), StatusCode::CREATED, "{}", user.json());
        let user = user.json()["data"]["id"].as_str().unwrap().to_owned();
        let usages = app
            .get(&format!("/api/v1/kernel/records/content.entry/{target}/usages"))
            .token("admin")
            .send()
            .await
            .unwrap();
        assert_eq!(usages.status(), StatusCode::OK, "{}", usages.json());
        assert_eq!(
            usages.json()["data"],
            json!([{"kind": "content.entry", "id": user, "title": "Bài nhúng", "field": "body"}])
        );
        let none = app
            .get(&format!("/api/v1/kernel/records/content.entry/{user}/usages"))
            .token("admin")
            .send()
            .await
            .unwrap();
        assert_eq!(none.json()["data"], json!([]), "không ai dùng bài nhúng");
        let missing = app
            .get(&format!("/api/v1/kernel/records/content.entry/{}/usages", Uuid::now_v7()))
            .token("admin")
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}
