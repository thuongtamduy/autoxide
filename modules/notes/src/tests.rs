//! Test của module mẫu, trên database gate đang chọn (gate chạy cả SQLite và Postgres).
//! `TestApp` của testkit dựng module như binary dựng: migration → start → route + tool → lớp
//! HTTP của kernel; gọi qua REST và qua client MCP thật.

use std::sync::Arc;

use autoxide_kernel::access::{Grant, Principal, Scope};
use autoxide_kernel::database::Database;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};

use autoxide_kernel::db::Queries as _;
use autoxide_testkit::mcp::McpClient;
use autoxide_testkit::{TestApp, TestDatabase, tool};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::Notes;
use crate::service::{NewNote, NoteChanges};

/// Hai người dùng: `ada` đọc và ghi được, `bob` chỉ đọc được.
const ADA: Uuid = Uuid::from_u128(0xada);
const BOB: Uuid = Uuid::from_u128(0xb0b);

/// Ghi chú cá nhân: `notes.*` với độ sâu `own` (ở nút gốc) — mỗi người chỉ thấy ghi chú của
/// mình (bước 34).
fn own(user: Uuid) -> Principal {
    let root = root_of(DEFAULT_TENANT);
    Principal::with_grants(
        user,
        ["notes.read", "notes.write"].map(|name| (name.to_owned(), Grant::new(root, Scope::Own))),
    )
}

async fn app(test: &TestDatabase) -> TestApp {
    let root = root_of(DEFAULT_TENANT);
    TestApp::builder(test)
        .member("ada", 0xada, root, &[("notes.read", Scope::Own), ("notes.write", Scope::Own)])
        .member("bob", 0xb0b, root, &[("notes.read", Scope::Own)])
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

/// Gọi `/api/v1/notes{uri}` bằng tiếng Việt; trả trạng thái và thân JSON.
async fn rest(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app
        .request(method, &format!("/api/v1/notes{uri}"))
        .token(who)
        .header("accept-language", "vi");
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    (answer.status(), answer.json())
}

async fn mcp(app: &TestApp, who: &str) -> McpClient {
    app.mcp(who).await.unwrap()
}

async fn events(database: &Database) -> Vec<String> {
    let sql = "SELECT name FROM notes_events ORDER BY id";
    let rows: Vec<(String,)> =
        autoxide_testkit::all_tenants(database).read().sql(sql).all().await.unwrap();
    rows.into_iter().map(|(name,)| name).collect()
}

/// REST và MCP gọi cùng service: ghi bằng đường này, đọc được bằng đường kia.
#[tokio::test]
async fn rest_va_mcp_dung_chung_service() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let ada = mcp(&app, "ada").await;

        let body = json!({"title": "Shopping", "body": "Milk"});
        let (status, created) = rest(&app, "POST", "", "ada", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let listed = ada.call_tool(tool("notes.list_notes", json!({}))).await.unwrap();
        let listed = listed.structured_content.unwrap();
        // Cùng hình dạng với REST: `data` + `pagination`, không lồng thêm.
        assert_eq!(listed["data"][0]["id"], created["data"]["id"]);
        assert!(listed["pagination"].is_object(), "{listed}");

        let made = ada.call_tool(tool("notes.create_note", json!({"title": "Ideas"}))).await;
        let made = made.unwrap().structured_content.unwrap();
        let id = made["data"]["id"].as_str().unwrap();
        let (status, got) = rest(&app, "GET", &format!("/{id}"), "ada", None).await;
        assert_eq!((status, got["data"]["title"].as_str()), (StatusCode::OK, Some("Ideas")));

        // Cùng lỗi, cùng mã, qua cả hai đường.
        let taken = ada.call_tool(tool("notes.create_note", json!({"title": "Ideas"}))).await;
        let taken = taken.unwrap();
        assert_eq!(taken.is_error, Some(true));
        assert_eq!(taken.structured_content.unwrap()["code"], "notes.title_taken");
        let (status, problem) =
            rest(&app, "POST", "", "ada", Some(json!({"title": "Ideas"}))).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("notes.title_taken"))
        );
        assert_eq!(problem["detail"], "Bạn đã có một ghi chú tên `Ideas`. Hãy chọn tên khác.");

        let updated =
            ada.call_tool(tool("notes.update_note", json!({"id": id, "body": "More"}))).await;
        assert_eq!(updated.unwrap().structured_content.unwrap()["data"]["body"], "More");
        let deleted = ada.call_tool(tool("notes.delete_note", json!({"id": id}))).await.unwrap();
        assert_eq!(deleted.is_error, Some(false));
        let (status, _) = rest(&app, "GET", &format!("/{id}"), "ada", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Mỗi lệnh ghi thành công đúng một event (audit log đọc từ đây).
        assert_eq!(
            events(app.database()).await,
            [
                "notes.note_created",
                "notes.note_created",
                "notes.note_updated",
                "notes.note_deleted"
            ]
        );
        ada.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

/// Người chỉ có `notes.read`: không thấy tool ghi, REST ghi bị 403; không thấy ghi chú
/// của người khác.
#[tokio::test]
async fn permission_va_du_lieu_rieng_tung_nguoi() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (_, created) = rest(&app, "POST", "", "ada", Some(json!({"title": "Mine"}))).await;
        let id = created["data"]["id"].as_str().unwrap();

        let bob = mcp(&app, "bob").await;
        let tools = bob.list_all_tools().await.unwrap();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
        assert_eq!(
            names,
            [
                "kernel.navigation",
                "kernel.current_tenant",
                "kernel.list_custom_fields",
                "kernel.list_approvals",
                "kernel.decide_approval",
                "notes.list_notes",
                "notes.get_note"
            ]
        );
        let (status, problem) = rest(&app, "POST", "", "bob", Some(json!({"title": "x"}))).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("permission_denied"))
        );

        let (status, _) = rest(&app, "GET", &format!("/{id}"), "bob", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let theirs = bob.call_tool(tool("notes.get_note", json!({"id": id}))).await.unwrap();
        assert_eq!(theirs.structured_content.unwrap()["code"], "notes.note_not_found");
        let (_, listed) = rest(&app, "GET", "", "bob", None).await;
        assert_eq!(listed["data"], json!([]));
        bob.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn kiem_dau_vao_nhu_nhau_o_rest_va_mcp() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let ada = mcp(&app, "ada").await;
        let long = "x".repeat(201);

        let (status, problem) = rest(&app, "POST", "", "ada", Some(json!({"title": long}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["errors"][0]["field"], "title");
        // Câu của từng ô cũng theo `Accept-Language` (D-262).
        assert_eq!(
            problem["errors"][0]["detail"],
            "phải dài 1 đến 200 ký tự, không chỉ có khoảng trắng"
        );
        let invalid = ada.call_tool(tool("notes.create_note", json!({"title": long}))).await;
        let invalid = invalid.unwrap().structured_content.unwrap();
        assert_eq!(
            (invalid["code"].as_str(), invalid["errors"][0]["field"].as_str()),
            (Some("validation_failed"), Some("title"))
        );
        let blank = ada.call_tool(tool("notes.create_note", json!({"title": "   "}))).await;
        assert_eq!(blank.unwrap().structured_content.unwrap()["code"], "validation_failed");
        ada.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

/// Cùng một người tạo cùng một tên từ nhiều request cùng lúc: đúng một cái thành công
/// (index unique quyết) — trên cả hai database.
#[tokio::test]
async fn tao_trung_ten_dong_thoi_chi_mot_thanh_cong() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let tenant = app.default_tenant();
        let service = crate::service::NoteService::new(
            autoxide_kernel::event::Outbox::standalone("notes"),
            Default::default(),
            autoxide_kernel::sync::SyncLog::standalone("notes"),
        );
        let service = Arc::new(service);

        let attempts: Vec<_> = (0..6)
            .map(|_| {
                let service = service.clone();
                let tenant = tenant.clone();
                tokio::spawn(async move {
                    let input =
                        NewNote { title: "Same".to_owned(), body: String::new(), org_id: None };
                    service.create(&tenant, &own(ADA), input).await
                })
            })
            .collect();
        let mut created = 0;
        for attempt in attempts {
            match attempt.await.unwrap() {
                Ok(_) => created += 1,
                Err(error) => assert_eq!(error.code(), "notes.title_taken", "{error:?}"),
            }
        }
        assert_eq!(created, 1);

        // Người khác dùng cùng tên được; đổi tên trùng thì bị chặn.
        let other = NewNote { title: "Same".to_owned(), body: String::new(), org_id: None };
        service.create(&tenant, &own(BOB), other).await.unwrap();
        let second = NewNote { title: "Second".to_owned(), body: String::new(), org_id: None };
        let second = service.create(&tenant, &own(ADA), second).await.unwrap();
        let rename = NoteChanges { title: Some("Same".to_owned()), body: None };
        let taken = service.update(&tenant, &own(ADA), second.id, rename, None).await.unwrap_err();
        assert_eq!(taken.code(), "notes.title_taken");
    })
    .await
    .unwrap();
}

/// Module khác tra ghi chú qua trait công bố, không đọc bảng của `notes`.
#[tokio::test]
async fn module_khac_dung_trait_cong_bo() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (_, created) = rest(&app, "POST", "", "ada", Some(json!({"title": "Shared"}))).await;
        let id: Uuid = created["data"]["id"].as_str().unwrap().parse().unwrap();

        let notes = app.modules().api::<dyn Notes>().unwrap();

        let found = notes.find(&app.default_tenant(), id).await.unwrap().unwrap();
        assert_eq!((found.owner_id, found.title.as_str()), (ADA, "Shared"));
        assert!(notes.find(&app.default_tenant(), Uuid::now_v7()).await.unwrap().is_none());
    })
    .await
    .unwrap();
}

/// Tài liệu OpenAPI của module đạt luật của kernel (mỗi endpoint đủ mô tả để gọi đúng
/// mà không đọc mã nguồn) — như gate kiểm cho các module đã đăng ký.
#[tokio::test]
async fn tai_lieu_openapi_du_mo_ta() {
    TestDatabase::run(async |test| {
        let app = app(test).await;

        assert_eq!(autoxide_kernel::http::lint(app.openapi()), Vec::<String>::new());
        assert!(app.openapi().paths.paths.contains_key("/api/v1/notes/{id}"));
    })
    .await
    .unwrap();
}

/// Xuất dữ liệu cá nhân có ghi chú của người dùng; tài khoản bị xoá thì ghi chú của
/// người đó bị xoá theo (nhận lại event cũng không sao) — trên cả hai database.
#[tokio::test]
async fn xuat_du_lieu_va_xoa_theo_tai_khoan() {
    use autoxide_kernel::event::{Delivered, EventHandler, Outbox};

    TestDatabase::run(async |test| {
        let app = app(test).await;
        let tenant = app.default_tenant();
        for (who, title) in [("ada", "one"), ("ada", "two"), ("bob", "three")] {
            let body = json!({"title": title, "body": "…"});
            let (status, _) = rest(&app, "POST", "", who, Some(body)).await;
            // bob chỉ đọc được: tạo ghi chú cho bob thẳng qua service.
            if who == "bob" {
                assert_eq!(status, StatusCode::FORBIDDEN);
                let service = crate::service::NoteService::new(
                    Outbox::standalone("notes"),
                    Default::default(),
                    autoxide_kernel::sync::SyncLog::standalone("notes"),
                );
                let note = NewNote { title: title.to_owned(), body: String::new(), org_id: None };
                service.create(&tenant, &own(BOB), note).await.unwrap();
            } else {
                assert_eq!(status, StatusCode::CREATED);
            }
        }
        // Bộ phát event dựng được dù chưa có module tài khoản nào khai event xoá.
        app.modules().dispatcher().unwrap();

        let exported =
            app.modules().personal_data().export(&app.default_tenant(), ADA).await.unwrap();
        let titles: Vec<_> =
            exported["notes"].as_array().unwrap().iter().map(|n| n["title"].clone()).collect();
        assert_eq!(titles, vec![json!("one"), json!("two")]);

        let service = crate::service::NoteService::new(
            Outbox::standalone("notes"),
            Default::default(),
            autoxide_kernel::sync::SyncLog::standalone("notes"),
        );
        let deleted = Delivered::new(
            app.default_tenant(),
            "auth",
            autoxide_kernel::privacy::USER_DELETED.to_owned(),
            json!({"before": {"id": ADA}}),
        );
        service.handle(&deleted).await.unwrap();
        service.handle(&deleted).await.unwrap();
        let exported =
            app.modules().personal_data().export(&app.default_tenant(), ADA).await.unwrap();
        assert_eq!(exported["notes"], json!([]));
        let exported =
            app.modules().personal_data().export(&app.default_tenant(), BOB).await.unwrap();
        assert_eq!(exported["notes"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

/// Nhiều người cùng sửa một ghi chú với cùng một phiên bản (`If-Match`): đúng một người
/// thành công, những người khác nhận 412 — trên cả hai database (D-084).
#[tokio::test]
async fn sua_dong_thoi_cung_phien_ban_chi_mot_nguoi_thang() {
    use autoxide_kernel::event::Outbox;

    TestDatabase::run(async |test| {
        let app = app(test).await;
        let tenant = app.default_tenant();
        let service = Arc::new(crate::service::NoteService::new(
            Outbox::standalone("notes"),
            Default::default(),
            autoxide_kernel::sync::SyncLog::standalone("notes"),
        ));
        let note = NewNote { title: "Shared".to_owned(), body: String::new(), org_id: None };
        let note = service.create(&tenant, &own(ADA), note).await.unwrap();

        let attempts: Vec<_> = (0..8)
            .map(|index| {
                let service = service.clone();
                let tenant = tenant.clone();
                let changes = NoteChanges { title: None, body: Some(format!("edit {index}")) };
                let version = Some(note.updated_at);
                tokio::spawn(async move {
                    service.update(&tenant, &own(ADA), note.id, changes, version).await
                })
            })
            .collect();
        let mut won = 0;
        for attempt in attempts {
            match attempt.await.unwrap() {
                Ok(_) => won += 1,
                Err(error) => assert_eq!(error.code(), "precondition_failed"),
            }
        }
        assert_eq!(won, 1);
    })
    .await
    .unwrap();
}

/// `GET /api/v1/me/permissions` (đề bài giao diện A5): permission của chính người gọi, mô tả theo
/// `Accept-Language`, kèm mức rủi ro — không cần quyền đọc sổ permission.
#[tokio::test]
async fn permission_cua_toi_kem_mo_ta_va_rui_ro() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let answer =
            app.get("/api/v1/me/permissions").token("bob").header("accept-language", "vi").send();
        let answer = answer.await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK);
        assert_eq!(
            answer.json()["data"],
            json!([{"name": "notes.read", "description": "Xem ghi chú trong phạm vi của bạn (độ sâu `own`: chỉ ghi chú của chính bạn).", "risk": "read"}])
        );
        let english = app.get("/api/v1/me/permissions").token("ada").send().await.unwrap();
        let listed = english.json();
        let names: Vec<&str> =
            listed["data"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["notes.read", "notes.write"]);
        assert_eq!(english.json()["data"][1]["description"], "Create, change and delete the notes in your scope (with scope `own`: only your own).");
        let anonymous = app.get("/api/v1/me/permissions").send().await.unwrap();
        assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    })
    .await
    .unwrap();
}

/// Module giả cùng tên `notes`, khai record (hay bảng control-plane) lệch với bảng thật mà migration của
/// `notes` dựng.
struct Declares(Vec<autoxide_kernel::db::RecordInfo>, Vec<&'static str>);

impl autoxide_kernel::module::Module for Declares {
    fn name(&self) -> &'static str {
        "notes"
    }

    fn records(&self) -> Vec<autoxide_kernel::db::RecordInfo> {
        self.0.clone()
    }

    fn control_plane_tables(&self) -> Vec<&'static str> {
        self.1.clone()
    }
}

/// `notes_note` không có cột `pinned`.
#[derive(autoxide_kernel::db::Record, Debug, Clone)]
#[record(table = "notes_note", scope = tenant)]
struct NoteWithExtra {
    id: Uuid,
    tenant_id: autoxide_kernel::db::TenantKey,
    pinned: bool,
}

/// `notes_note` có `tenant_id` mà record khai `global`: derive không thấy bảng thật, kernel chặn lúc khởi động.
#[derive(autoxide_kernel::db::Record, Debug, Clone)]
#[record(table = "notes_note", scope = global)]
struct NoteAsGlobal {
    id: Uuid,
    title: String,
}

/// `notes_note.org_id` cho NULL mà trường không phải `Option`.
#[derive(autoxide_kernel::db::Record, Debug, Clone)]
#[record(table = "notes_note", scope = tenant)]
struct NoteStrictOrg {
    id: Uuid,
    tenant_id: autoxide_kernel::db::TenantKey,
    org_id: Uuid,
}

/// Bước đối chiếu lúc khởi động (D-20261126-kernel-data-layer): record khớp bảng thì qua; lệch thì báo đủ chỗ lệch.
#[tokio::test]
async fn record_lech_schema_thi_khong_khoi_dong() {
    use autoxide_kernel::db::RecordInfo;
    use autoxide_kernel::module::Modules;
    TestDatabase::run(async |test| {
        // `TestApp` đã chạy chính bước này cho record thật của `notes` (`Module::records`).
        let app = TestApp::builder(test).module(crate::module()).build().await.unwrap();
        let wrong = Modules::new(vec![Box::new(Declares(
            vec![
                RecordInfo::of::<NoteWithExtra>(),
                RecordInfo::of::<NoteAsGlobal>(),
                RecordInfo::of::<NoteStrictOrg>(),
            ],
            Vec::new(),
        ))])
        .unwrap();
        let message = wrong.check_records(app.database()).await.unwrap_err().to_string();
        assert!(message.contains("field `pinned` has no column"), "{message}");
        assert!(message.contains("is `scope = global` but the table has `tenant_id`"), "{message}");
        assert!(message.contains("is not in the module's `control_plane_tables`"), "{message}");
        assert!(message.contains("field `org_id` must be an `Option`"), "{message}");
    })
    .await
    .unwrap();
}

/// Module không lách được chốt record `global` bằng bảng control-plane: khai bảng của module khác thì không
/// dựng được `Modules`; khai bảng của khách (có `tenant_id`) là control-plane thì record `global` trên nó vẫn
/// bị chặn lúc khởi động (bảng control-plane có `tenant_id` là hợp lệ theo D-253, record `global` thì không).
#[tokio::test]
async fn khai_bang_control_plane_khong_lach_duoc_chot_global() {
    use autoxide_kernel::db::RecordInfo;
    use autoxide_kernel::module::Modules;
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).module(crate::module()).build().await.unwrap();
        let foreign = Modules::new(vec![Box::new(Declares(Vec::new(), vec!["kernel_tenants"]))]);
        let message = foreign.err().map(|error| error.to_string()).unwrap_or_default();
        assert!(
            message
                .contains("control-plane table `kernel_tenants`, which is not one of its tables"),
            "{message}"
        );
        let bare = Modules::new(vec![Box::new(Declares(Vec::new(), vec!["notes_"]))]);
        assert!(bare.is_err());
        let tenant = Modules::new(vec![Box::new(Declares(
            vec![RecordInfo::of::<NoteAsGlobal>()],
            vec!["notes_note"],
        ))])
        .unwrap();
        let message = tenant.check_records(app.database()).await.unwrap_err().to_string();
        assert!(message.contains("is `scope = global` but the table has `tenant_id`"), "{message}");
        assert!(!message.contains("is not in the module's `control_plane_tables`"), "{message}");
    })
    .await
    .unwrap();
}
