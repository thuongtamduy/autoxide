//! Ghi chú theo khách (bước 33): không rò rỉ giữa khách (một khách trên database chung, một khách
//! trên database riêng), trùng tên ở hai khách được phép, dữ liệu cũ thuộc khách mặc định, event
//! ghi và giao đúng khách. Chạy trên cả SQLite và Postgres.

use autoxide_kernel::event::{Actor, Delivered, EventHandler, Outbox};
use autoxide_kernel::migration::Migrator;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, TenantContext};
use autoxide_testkit::{TestApp, TestDatabase, leak_check};
use axum::http::StatusCode;
use sea_orm::sea_query::{Alias, Query};
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::service::{NewNote, NoteService};

/// Cùng một người dùng (cùng ID) ở hai khách: chỉ ranh giới khách giữ dữ liệu tách nhau.
const ADA: u128 = 0xada;

async fn app(test: &TestDatabase) -> TestApp {
    let all = ["notes.read", "notes.write"];
    TestApp::builder(test)
        .tenant("shared")
        .tenant_dedicated("own")
        .user_in("shared", "token-shared", ADA, &all)
        .user_in("own", "token-own", ADA, &all)
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

async fn create(app: &TestApp, token: &str, title: &str) -> (StatusCode, Value) {
    let body = json!({"title": title, "body": format!("body of {title}")});
    let answer = app.post("/api/v1/notes").token(token).json(body).send().await.unwrap();
    (answer.status(), answer.json())
}

/// Tên và ID của event trong bảng `notes_events` của `tenant` (database của khách).
async fn events(tenant: &TenantContext) -> Vec<(String, Uuid)> {
    let database = tenant.database();
    let backend = database.read().get_database_backend();
    let sql = "SELECT name, tenant_id FROM notes_events ORDER BY id";
    let rows = database.read().query_all_raw(Statement::from_string(backend, sql)).await;
    rows.unwrap()
        .iter()
        .map(|row| (row.try_get("", "name").unwrap(), row.try_get("", "tenant_id").unwrap()))
        .collect()
}

#[tokio::test]
async fn khong_ro_ri_giua_khach() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        for (owner, other) in [("token-shared", "token-own"), ("token-own", "token-shared")] {
            let marker = format!("MARKER-{owner}");
            let (status, created) = create(&app, owner, &marker).await;
            assert_eq!(status, StatusCode::CREATED, "{created}");
            let id = created["data"]["id"].as_str().unwrap().to_owned();

            let leaks = leak_check(&app, other, &marker, &[("id", id.clone())]).await.unwrap();
            assert_eq!(leaks, Vec::<String>::new());
            // Ghi cũng không với tới: sửa, xoá ghi chú của khách kia là "không có".
            let uri = format!("/api/v1/notes/{id}");
            let changed =
                app.patch(&uri).token(other).json(json!({"body": "x"})).send().await.unwrap();
            assert_eq!(changed.status(), StatusCode::NOT_FOUND);
            let removed = app.delete(&uri).token(other).send().await.unwrap();
            assert_eq!(removed.status(), StatusCode::NOT_FOUND);
            let mine = app.get(&uri).token(owner).send().await.unwrap();
            assert_eq!(mine.json()["data"]["body"], format!("body of {marker}"));
        }
    })
    .await
    .unwrap();
}

/// Con trỏ pagination lấy ở khách này gửi bằng người của khách kia (một khách database chung, một
/// database riêng): chỉ thấy dữ liệu của khách kia, không bao giờ của khách sở hữu con trỏ (D-259
/// mục 5 — con trỏ chỉ là vị trí trong danh sách đã lọc theo khách).
#[tokio::test]
async fn con_tro_cua_khach_khac_khong_lo_du_lieu() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        for (owner, other) in [("token-shared", "token-own"), ("token-own", "token-shared")] {
            let marker = format!("CURSOR-{owner}");
            let mut ids = Vec::new();
            for index in 0..3 {
                let (status, created) = create(&app, owner, &format!("{marker}-{index}")).await;
                assert_eq!(status, StatusCode::CREATED, "{created}");
                ids.push(("id", created["data"]["id"].as_str().unwrap().to_owned()));
            }
            let (status, _) = create(&app, other, &format!("theirs-{owner}")).await;
            assert_eq!(status, StatusCode::CREATED);

            let (violations, tried) =
                autoxide_testkit::cursor_leak_check(&app, owner, other, &marker, &ids)
                    .await
                    .unwrap();
            assert_eq!(violations, Vec::<String>::new());
            assert!(tried >= 1, "no list gave a cursor");

            // Cụ thể: con trỏ của trang đầu (mới nhất trước) ở khách sở hữu; khách kia chỉ nhận dòng
            // của chính nó.
            let first = app.get("/api/v1/notes?limit=1").token(owner).send().await.unwrap();
            let cursor = first.json()["pagination"]["next_cursor"].as_str().unwrap().to_owned();
            let uri = format!("/api/v1/notes?order=asc&cursor={cursor}");
            let theirs = app.get(&uri).token(other).send().await.unwrap();
            assert_eq!(theirs.status(), StatusCode::OK);
            let rows = theirs.json()["data"].as_array().unwrap().clone();
            assert!(rows.iter().all(|row| !row["title"].as_str().unwrap().contains(&marker)));
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn trung_ten_o_hai_khach_duoc_phep() {
    TestDatabase::run(async |test| {
        let app = app(test).await;

        assert_eq!(create(&app, "token-shared", "Same").await.0, StatusCode::CREATED);
        assert_eq!(create(&app, "token-own", "Same").await.0, StatusCode::CREATED);
        let (status, problem) = create(&app, "token-shared", "Same").await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("notes.title_taken"))
        );
    })
    .await
    .unwrap();
}

/// Bản cài cũ (ghi chú trước bước 33) nâng cấp: ghi chú thuộc khách mặc định, đọc lại được.
#[tokio::test]
async fn nang_cap_gan_du_lieu_cu_cho_khach_mac_dinh() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .user("ada", ADA, &["notes.read"])
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let migrator: Migrator = app.modules().migrator().unwrap();
        // Về schema trước bước 33 (bỏ `m0005_tenancy`), ghi một ghi chú kiểu cũ, rồi nâng lại.
        migrator.down(app.database(), "notes", 1).await.unwrap();
        let database = app.database();
        let backend = database.write().get_database_backend();
        let id = Uuid::now_v7();
        let at = chrono::Utc::now();
        // `sea_query` đổi UUID, thời gian sang dạng lưu của từng database (như SeaORM ghi).
        let insert = Query::insert()
            .into_table(Alias::new("notes_note"))
            .columns(
                ["id", "owner_id", "title", "body", "created_at", "updated_at"].map(Alias::new),
            )
            .values_panic([
                id.into(),
                Uuid::from_u128(ADA).into(),
                "Old".into(),
                "from before".into(),
                at.into(),
                at.into(),
            ])
            .to_owned();
        database.write().execute(&insert).await.unwrap();
        migrator.up(app.database()).await.unwrap();

        let answer = app.get(&format!("/api/v1/notes/{id}")).token("ada").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
        assert_eq!(answer.json()["data"]["title"], "Old");
        let select = "SELECT tenant_id FROM notes_note";
        let rows = database.read().query_all_raw(Statement::from_string(backend, select)).await;
        let owners: Vec<Uuid> =
            rows.unwrap().iter().map(|row| row.try_get("", "tenant_id").unwrap()).collect();
        assert_eq!(owners, [DEFAULT_TENANT.as_uuid()]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn event_va_job_chay_dung_khach() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let shared = app.tenant("shared").await.unwrap();
        let own = app.tenant("own").await.unwrap();
        create(&app, "token-shared", "A").await;
        create(&app, "token-own", "B").await;

        // Event nằm ở database của đúng khách, mang ID của khách đó.
        assert_eq!(
            events(&shared).await,
            [("notes.note_created".to_owned(), shared.id().as_uuid())]
        );
        assert_eq!(events(&own).await, [("notes.note_created".to_owned(), own.id().as_uuid())]);

        // Tài khoản bị xoá ở khách `own`: chỉ ghi chú của người đó ở `own` bị xoá, cùng ID người
        // dùng ở `shared` không bị đụng.
        let service = NoteService::new(Outbox::standalone("notes"));
        let deleted = Delivered::new(
            own.clone(),
            "auth",
            autoxide_kernel::privacy::USER_DELETED.to_owned(),
            json!({"before": {"id": Uuid::from_u128(ADA)}}),
        );
        service.handle(&deleted).await.unwrap();
        let list = |token: &'static str| {
            let app = &app;
            async move { app.get("/api/v1/notes").token(token).send().await.unwrap().json() }
        };
        assert_eq!(list("token-own").await["data"], json!([]));
        assert_eq!(list("token-shared").await["data"].as_array().unwrap().len(), 1);

        // Trait công bố cho module khác cũng theo khách.
        let note = service
            .create(
                &shared,
                Actor::user(Uuid::from_u128(ADA)),
                NewNote { title: "Via trait".to_owned(), body: String::new() },
            )
            .await
            .unwrap();
        let notes = app.api::<dyn crate::api::Notes>().unwrap();
        assert!(notes.find(&shared, note.id).await.unwrap().is_some());
        assert!(notes.find(&own, note.id).await.unwrap().is_none());
    })
    .await
    .unwrap();
}

/// Bảng của `notes` đúng luật bảng cấp khách hàng: `tenant_id` NOT NULL, khoá và index bắt đầu
/// bằng nó (binary chưa đăng ký `notes`, nên bài chung của app không phủ module này).
#[tokio::test]
async fn bang_dung_luat_theo_khach() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).module(crate::module()).build().await.unwrap();
        let snapshot = autoxide_testkit::schema_snapshot(app.database().write()).await.unwrap();
        let control_plane = app.modules().control_plane_tables();
        let violations = autoxide_testkit::schema_tenancy_violations(&snapshot, &control_plane);
        assert_eq!(violations, Vec::<String>::new());
    })
    .await
    .unwrap();
}
