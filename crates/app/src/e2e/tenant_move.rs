//! Dời khách trên chương trình có mọi module (bước 33 C2): khách `a` có bài viết (`content`), tài
//! khoản (`auth`), file (`files`, nội dung ở nơi lưu chung) và event chưa giao (`audit`, webhook…);
//! dời sang database riêng rồi đọc lại mọi thứ qua REST, số dòng mỗi bảng đúng như kế hoạch,
//! database cũ không còn dòng nào của `a`, khách `b` không bị chạm; dời về lại `main`. Chạy trên
//! database gate đang chọn (placement là một database thêm cùng loại).

use autoxide_auth::api::{Accounts, NewAccount, Users};
use autoxide_kernel::event::Actor;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::tenancy::{TenantContext, TenantMoves};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::body::Body;
use sea_orm::ConnectionTrait as _;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};
use serde_json::{Value, json};

const A: [&str; 4] = ["content.read", "content.write", "files.upload", "files.manage"];

async fn app(test: &TestDatabase) -> TestApp {
    let dir = std::env::temp_dir().join(format!("autoxide-move-{}", uuid::Uuid::now_v7()));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        // Dời chờ TTL cache + thời gian tối đa của request: rút ngắn cho test.
        .setting("AUTOXIDE_TENANCY_CACHE_SECONDS", "1")
        .setting("AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS", "1")
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
        assert_eq!(rows(&main, "auth_user", &a).await, 1);

        // Dời thật (đường của CLI).
        let started = moves.start(Actor::system(), "a", "spare", false, false).await.unwrap();
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
