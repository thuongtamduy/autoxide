//! Dời khách trên chương trình có mọi module (bước 33 C2): khách `a` có bài viết (`content`), tài
//! khoản (`auth`), file (`files`, nội dung ở nơi lưu chung) và event chưa giao (`audit`, webhook…);
//! dời sang database riêng rồi đọc lại mọi thứ qua REST, số dòng mỗi bảng đúng như kế hoạch,
//! database cũ không còn dòng nào của `a`, khách `b` không bị chạm; dời về lại `main`. Chạy trên
//! database gate đang chọn (placement là một database thêm cùng loại).
//!
//! Placement có nơi lưu file riêng (thư mục khác): dời sang đó chép file của khách (`files`) sang
//! nơi lưu riêng và xoá ở nơi lưu chung; tải lên sau đó vào nơi lưu riêng; mọi thứ đọc lại được.

use autoxide_auth::api::{Accounts, NewAccount, Users};
use autoxide_kernel::event::Actor;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::tenancy::{TenantContext, TenantMoves};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::body::Body;
use sea_orm::ConnectionTrait as _;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};
use serde_json::{Value, json};

const A: [&str; 5] =
    ["content.read", "content.write", "content.publish", "files.upload", "files.manage"];

async fn app(test: &TestDatabase) -> TestApp {
    let dir = std::env::temp_dir().join(format!("autoxide-move-{}", uuid::Uuid::now_v7()));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        // Dời chờ TTL cache + thời gian tối đa của request: test bỏ qua bằng `skip_move_wait`
        // thay vì hạ thời gian tối đa của request xuống 1 giây (request chậm trên CI bị cắt).
        .setting("AUTOXIDE_TENANCY_CACHE_SECONDS", "1")
        .tenant("a")
        .tenant("b")
        .placement("spare")
        .user("platform", 1, &["kernel.tenants.move", "kernel.tenants.read"])
        .user_in("a", "token-a", 2, &A)
        .user_in("b", "token-b", 3, &A);
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    builder.build().await.unwrap()
}

async fn tenant(app: &TestApp, slug: &str) -> TenantContext {
    let id = app.tenant(slug).await.unwrap().id();
    app.registry().invalidate(id);
    app.tenant(slug).await.unwrap()
}

/// Số dòng của khách `tenant` trong `table` của database của `on`.
async fn rows(on: &TenantContext, table: &str, tenant: &TenantContext) -> i64 {
    let select = Query::select()
        .expr_as(Expr::cust("COUNT(*)"), Alias::new("n"))
        .from(Alias::new(table))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .to_owned();
    let row = on.database().write().query_one(&select).await.unwrap().unwrap();
    row.try_get("", "n").unwrap()
}

/// Bài viết `title` của người `token`; trả ID.
async fn entry(app: &TestApp, token: &str, title: &str) -> String {
    let body = json!({"kind": "post", "title": title, "slug": title.to_lowercase()});
    let answer = app.post("/api/v1/content/entries").token(token).json(body).send().await.unwrap();
    assert_eq!(answer.status(), 201, "{}", answer.json());
    answer.json()["data"]["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn doi_khach_co_du_lieu_cua_nhieu_module_roi_doi_ve() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let a = tenant(&app, "a").await;
        let b = tenant(&app, "b").await;
        let main = a.clone();

        // Dữ liệu của `a` ở nhiều module; `b` có một bài.
        let post = entry(&app, "token-a", "Hello").await;
        // Bài đã đăng: dòng của bản đăng và của chỉ mục từ (`content_published_words`) đi theo khách.
        let read = app.get(&format!("/api/v1/content/entries/{post}")).token("token-a").send();
        let etag = read.await.unwrap().headers()["etag"].to_str().unwrap().to_owned();
        let published = app
            .post(&format!("/api/v1/content/entries/{post}/publish"))
            .token("token-a")
            .header("if-match", &etag)
            .send()
            .await
            .unwrap();
        assert_eq!(published.status(), 200, "{}", published.json());
        let other = entry(&app, "token-b", "Other").await;
        let accounts = app.api::<dyn Accounts>().unwrap();
        let account = NewAccount {
            email: "ada@a.example".to_owned(),
            name: "Ada".to_owned(),
            password: None,
            admin: false,
        };
        let ada = accounts.create(&a, account).await.unwrap();
        let upload = app
            .post("/api/v1/files?name=hello.txt")
            .token("token-a")
            .header("content-type", "text/plain")
            .body(Body::from("hello from a"))
            .send()
            .await
            .unwrap();
        assert_eq!(upload.status(), 201, "{}", upload.json());
        let file = upload.json()["data"]["id"].as_str().unwrap().to_owned();
        // Event chưa giao (bộ phát chưa chạy): đi theo dữ liệu.
        let undelivered = rows(&main, "content_events", &a).await;
        assert!(undelivered > 0);

        // Dry-run: kế hoạch, không đổi gì.
        let moves = app.api::<TenantMoves>().unwrap();
        let plan = moves.plan("a", "spare", false, Language::En).await.unwrap();
        assert!(plan.ready, "{plan:?}");
        let planned = |table: &str| {
            plan.tables.iter().find(|entry| entry.table == table).map_or(0, |entry| entry.rows)
        };
        assert_eq!(planned("auth_user"), 1);
        assert_eq!(planned("files_file"), 1);
        assert!(planned("content_entries") >= 1);
        assert_eq!(planned("content_published"), 1);
        assert!(planned("content_published_words") >= 1, "the word index moves with the tenant");
        assert_eq!(rows(&main, "auth_user", &a).await, 1);

        // Dời thật (đường của CLI).
        let started = moves.start(Actor::system(), "a", "spare", false, false).await.unwrap();
        app.skip_move_wait().await.unwrap();
        let moved = moves.run(started.id, "e2e").await.unwrap();
        assert_eq!(moved.state, "done");
        for table in &moved.tables {
            assert_eq!(table.rows, planned(&table.table), "{}", table.table);
        }
        let a = tenant(&app, "a").await;
        assert_ne!(a.placement(), main.placement());
        for table in &moved.tables {
            assert_eq!(rows(&main, &table.table, &a).await, 0, "{}", table.table);
            let extra = i64::from(table.table == "kernel_events");
            let expected = i64::try_from(table.rows).unwrap() + extra;
            assert_eq!(rows(&a, &table.table, &a).await, expected, "{}", table.table);
        }
        assert_eq!(rows(&main, "content_entries", &b).await, 1);

        // Mọi thứ đọc lại được qua REST và service, từ database mới; file vẫn ở nơi lưu chung.
        let read = app.get(&format!("/api/v1/content/entries/{post}")).token("token-a").send();
        let read = read.await.unwrap();
        assert_eq!(read.status(), 200);
        assert_eq!(read.json()["data"]["title"], json!("Hello"));
        let content =
            app.get(&format!("/api/v1/files/{file}/content")).token("token-a").send().await;
        let content = content.unwrap();
        assert_eq!(content.status(), 200);
        assert_eq!(content.body(), b"hello from a");
        let users = app.api::<dyn Users>().unwrap();
        assert!(users.find(&a, ada.id).await.unwrap().is_some());
        let theirs = app.get(&format!("/api/v1/content/entries/{other}")).token("token-b").send();
        assert_eq!(theirs.await.unwrap().status(), 200);

        // Event chưa giao được giao ở database mới.
        let dispatcher = app.modules().dispatcher().unwrap();
        while dispatcher.run_once().await.unwrap() > 0 {}
        let pending = Query::select()
            .expr_as(Expr::cust("COUNT(*)"), Alias::new("n"))
            .from(Alias::new("content_events"))
            .and_where(Expr::col(Alias::new("delivered_at")).is_null())
            .to_owned();
        let row = a.database().write().query_one(&pending).await.unwrap().unwrap();
        assert_eq!(row.try_get::<i64>("", "n").unwrap(), 0);

        // Dời về `main`.
        let back = moves.start(Actor::system(), "a", "main", false, false).await.unwrap();
        app.skip_move_wait().await.unwrap();
        assert_eq!(moves.run(back.id, "e2e").await.unwrap().state, "done");
        let a = tenant(&app, "a").await;
        assert_eq!(a.placement(), main.placement());
        let read = app.get(&format!("/api/v1/content/entries/{post}")).token("token-a").send();
        assert_eq!(read.await.unwrap().json()["data"]["title"], json!("Hello"));
        assert_eq!(rows(&main, "auth_user", &a).await, 1);
        let value: Value = serde_json::to_value(&moved).unwrap();
        assert_eq!(value["to"], json!("spare"));
    })
    .await
    .unwrap();
}

/// Khoá file của khách `tenant` trong thư mục `dir` (nơi lưu cục bộ).
fn keys_in(dir: &std::path::Path, tenant: &TenantContext) -> Vec<String> {
    let root = dir.join("tenants").join(tenant.id().to_string());
    let mut found = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(folder) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path.strip_prefix(&root).unwrap().display().to_string());
            }
        }
    }
    found.sort();
    found
}

#[tokio::test]
async fn doi_khach_sang_placement_co_noi_luu_file_rieng() {
    TestDatabase::run(async |test| {
        let root = std::env::temp_dir().join(format!("autoxide-move-{}", uuid::Uuid::now_v7()));
        let (shared_dir, own_dir) = (root.join("files"), root.join("spare-files"));
        let mut builder = TestApp::builder(test)
            .setting("AUTOXIDE_FILES_DIR", &shared_dir.display().to_string())
            .setting("AUTOXIDE_MAIL_FILE_DIR", &root.join("mail").display().to_string())
            .setting("AUTOXIDE_TENANCY_CACHE_SECONDS", "1")
            .tenant("a")
            .placement("spare")
            .user_in("a", "token-a", 2, &["files.upload", "files.manage"]);
        for module in crate::modules::all() {
            builder = builder.boxed_module(module);
        }
        let app = builder.build().await.unwrap();
        let placements = app.modules().placements(app.registry()).unwrap();
        let location =
            autoxide_kernel::storage::StorageLocation::Local { dir: own_dir.display().to_string() };
        placements.set_storage("spare", Some(location), false).await.unwrap();

        let upload = |text: &'static str| {
            app.post("/api/v1/files?name=hello.txt")
                .token("token-a")
                .header("content-type", "text/plain")
                .body(Body::from(text))
        };
        let first = upload("before the move").send().await.unwrap();
        assert_eq!(first.status(), 201, "{}", first.json());
        let file = first.json()["data"]["id"].as_str().unwrap().to_owned();
        let a = tenant(&app, "a").await;
        assert_eq!(keys_in(&shared_dir, &a), [format!("files/{file}")]);

        let moves = app.api::<TenantMoves>().unwrap();
        let plan = moves.plan("a", "spare", false, Language::En).await.unwrap();
        assert!(plan.ready, "{plan:?}");
        assert_eq!(plan.files, 1);
        let started = moves.start(Actor::system(), "a", "spare", false, false).await.unwrap();
        app.skip_move_wait().await.unwrap();
        assert_eq!(moves.run(started.id, "e2e").await.unwrap().state, "done");

        let a = tenant(&app, "a").await;
        assert_eq!(keys_in(&own_dir, &a), [format!("files/{file}")]);
        assert!(keys_in(&shared_dir, &a).is_empty(), "the old copy is deleted");
        let content =
            app.get(&format!("/api/v1/files/{file}/content")).token("token-a").send().await;
        let content = content.unwrap();
        assert_eq!(
            (content.status().as_u16(), content.body()),
            (200, b"before the move".as_slice())
        );
        let second = upload("after the move").send().await.unwrap();
        assert_eq!(second.status(), 201);
        let after = second.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(keys_in(&own_dir, &a).len(), 2);
        assert!(keys_in(&shared_dir, &a).is_empty());
        let content =
            app.get(&format!("/api/v1/files/{after}/content")).token("token-a").send().await;
        assert_eq!(content.unwrap().body(), b"after the move");
    })
    .await
    .unwrap();
}
