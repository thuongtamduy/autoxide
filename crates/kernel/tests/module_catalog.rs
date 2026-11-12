//! Sổ module đã cài (D-151): `GET /api/v1/kernel/modules`, `GET /api/v1/kernel/modules/{name}`
//! và hai tool MCP `kernel.list_modules`, `kernel.get_module`. Gate chạy file này trên cả hai
//! database.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
// Clippy chỉ tự miễn cho hàm `#[test]`, không miễn cho hàm phụ, nên khai ở đây.
// Trong `src/` thì KHÔNG được khai dòng này (`cargo xtask check` chặn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::{Authenticator, Permission, Principal};
use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig, HttpConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::EventInfo;
use autoxide_kernel::http::{self, Deprecation};
use autoxide_kernel::job::{JobContext, JobHandler, JobRegistration, Schedule};
use autoxide_kernel::mcp::{NoInput, Tool};
use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::module::{Module, Modules};
use autoxide_testkit::TestDatabase;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

const NOTES_WRITE: Permission = Permission::new("notes.write", "Write notes.");

/// Migration tạo bảng `notes_items`.
struct CreateItems;

impl MigrationName for CreateItems {
    fn name(&self) -> &str {
        "m0001_create_items"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateItems {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("notes_items")
                    .col(schema::uuid("id").primary_key())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("notes_items").to_owned()).await
    }
}

/// Migration thứ hai, test để nó chưa áp dụng.
struct AddIndex;

impl MigrationName for AddIndex {
    fn name(&self) -> &str {
        "m0002_add_index"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddIndex {
    async fn up(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

struct Noop;

#[async_trait::async_trait]
impl JobHandler for Noop {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        Ok(())
    }
}

/// Module khai đủ mọi thứ mà sổ liệt kê.
struct Notes {
    /// Có migration thứ hai không (để giả lập migration chưa áp dụng).
    second_migration: bool,
}

impl Module for Notes {
    fn name(&self) -> &'static str {
        "notes"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        let mut migrations: Vec<Box<dyn MigrationTrait>> = vec![Box::new(CreateItems)];
        if self.second_migration {
            migrations.push(Box::new(AddIndex));
        }
        migrations
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![NOTES_WRITE]
    }

    fn events(&self) -> Vec<EventInfo> {
        vec![EventInfo::new("notes.note_created", "A note was created.")]
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        vec![JobRegistration::new("notes.cleanup", Arc::new(Noop))]
    }

    fn schedules(&self) -> Vec<Schedule> {
        vec![Schedule::new("notes.nightly", "notes.cleanup", Duration::from_secs(86_400))]
    }

    fn deprecations(&self) -> Vec<Deprecation> {
        vec![Deprecation::new(Method::GET, "/{id}", "2026-11-01").successor("/api/v2/notes/{id}")]
    }

    fn tools(&self) -> Vec<Tool> {
        vec![
            Tool::new("count", "Count notes.", |_: Principal, _: NoInput| async {
                Ok(json!({ "count": 0 }))
            })
            .requires(NOTES_WRITE)
            .read_only(),
        ]
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", "{}")]
    }
}

/// `admin` có mọi permission; `writer` chỉ có `notes.write`.
struct FakeAuthenticator;

#[async_trait::async_trait]
impl Authenticator for FakeAuthenticator {
    async fn authenticate(&self, authorization: Option<&str>) -> Result<Principal, Error> {
        match authorization {
            Some("Bearer admin") => Ok(Principal::new(
                Uuid::nil(),
                ["kernel.modules.read".to_owned(), "notes.write".to_owned()],
            )),
            Some("Bearer writer") => Ok(Principal::new(Uuid::nil(), ["notes.write".to_owned()])),
            _ => Err(Error::unauthenticated("auth.missing_token", "Sign in.")),
        }
    }
}

async fn connect(database: &TestDatabase) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    Database::connect(&reader.finish(config).unwrap().0).await.unwrap()
}

/// Dựng chương trình như binary: đăng ký, áp dụng migration của bản đầu, khởi động, ghép router.
async fn start(db: &Database, second_migration: bool) -> (Modules, Router) {
    // Áp migration của bản chỉ có migration đầu; bản chạy thật có thể có thêm migration
    // thứ hai, chưa áp dụng.
    let first = Modules::new(vec![Box::new(Notes { second_migration: false })]).unwrap();
    first.migrator().unwrap().up(db).await.unwrap();

    let mut modules = Modules::new(vec![Box::new(Notes { second_migration })]).unwrap();
    modules.start(db).unwrap();
    let source = ConfigSource::default();
    let mut reader = ConfigReader::new(&source);
    let config = HttpConfig::read(&mut reader);
    let config = reader.finish(config).unwrap().0;
    let extras = http::Extras {
        authenticator: Some(Arc::new(FakeAuthenticator)),
        tools: modules.tools().unwrap(),
        translations: modules.translations().clone(),
        ..Default::default()
    };
    let router =
        http::build(&config, http::health_routes(db.clone()), modules.routes(), extras).router;
    (modules, router)
}

async fn get(app: Router, path: &str, token: Option<&str>) -> (StatusCode, Value) {
    let mut request = Request::get(path);
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app.oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn liet_ke_moi_module_theo_thu_tu_dang_ky_kem_khai_bao() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let (_modules, app) = start(&db, false).await;

        let (status, body) = get(app, "/api/v1/kernel/modules", Some("admin")).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        let modules = body["data"].as_array().unwrap();
        let names: Vec<&str> = modules.iter().map(|m| m["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["kernel", "antispam", "notes"]);
        assert_eq!(modules[0]["internal"], true);
        assert_eq!(modules[0]["permissions"][0]["name"], "kernel.modules.read");
        let tools: Vec<&str> =
            modules[0]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(tools, ["kernel.list_modules", "kernel.get_module"]);

        let notes = &modules[2];
        assert_eq!(
            *notes,
            json!({
                "name": "notes",
                "internal": false,
                "position": 2,
                "permissions": [{ "name": "notes.write", "description": "Write notes." }],
                "events": [{ "name": "notes.note_created", "description": "A note was created." }],
                "subscriptions": [],
                "jobs": ["notes.cleanup"],
                "schedules": [{ "name": "notes.nightly", "kind": "notes.cleanup", "every_seconds": 86_400 }],
                "tools": [{
                    "name": "notes.count",
                    "description": "Count notes.",
                    "permissions": ["notes.write"],
                    "read_only": true,
                    "destructive": false,
                }],
                "deprecations": [{
                    "method": "GET",
                    "path": "/api/v1/notes/{id}",
                    "since": "2026-11-01",
                    "sunset": null,
                    "successor": "/api/v2/notes/{id}",
                }],
                "languages": ["vi"],
                // Danh sách không đọc database.
                "migrations": [{ "name": "m0001_create_items", "applied": null }],
            })
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mot_module_kem_trang_thai_migration_doc_tu_database() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let (_modules, app) = start(&db, true).await;

        let (status, body) = get(app.clone(), "/api/v1/kernel/modules/notes", Some("admin")).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["data"]["migrations"],
            json!([
                { "name": "m0001_create_items", "applied": true },
                { "name": "m0002_add_index", "applied": false },
            ])
        );

        let (status, body) = get(app, "/api/v1/kernel/modules/kernel", Some("admin")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        // Database của test chưa có bảng của `kernel`: đọc trạng thái không tạo bảng, chỉ báo.
        assert_eq!(body["data"]["migrations"][0]["name"], "m0001_create_secrets");
        assert!(body["data"]["migrations"][0]["applied"].is_boolean());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn module_khong_co_thi_404_kem_ma_on_dinh() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let (_modules, app) = start(&db, false).await;

        let (status, body) = get(app, "/api/v1/kernel/modules/shop", Some("admin")).await;

        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["code"], "kernel.module_not_found");
        assert!(body["detail"].as_str().unwrap().contains("`shop`"), "{body}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn can_permission_va_dang_nhap() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let (_modules, app) = start(&db, false).await;

        let (status, body) = get(app.clone(), "/api/v1/kernel/modules", Some("writer")).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["code"], "permission_denied");

        let (status, _) = get(app.clone(), "/api/v1/kernel/modules/notes", Some("writer")).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = get(app, "/api/v1/kernel/modules", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tool_mcp_tra_cung_ket_qua_voi_rest() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let (modules, app) = start(&db, true).await;
        let tools = modules.tools().unwrap();
        let tool = |name: &str| tools.iter().find(|tool| tool.name() == name).unwrap().clone();
        let admin = Principal::new(Uuid::nil(), ["kernel.modules.read".to_owned()]);
        let writer = Principal::new(Uuid::nil(), ["notes.write".to_owned()]);

        let list = tool("kernel.list_modules");
        assert!(list.is_read_only());
        assert!(list.allows(&admin) && !list.allows(&writer));
        let listed = list.call(admin.clone(), json!({})).await.unwrap();
        let (_, rest) = get(app.clone(), "/api/v1/kernel/modules", Some("admin")).await;
        assert_eq!(listed, rest["data"]);

        let one = tool("kernel.get_module");
        let described = one.call(admin.clone(), json!({ "name": "notes" })).await.unwrap();
        let (_, rest) = get(app, "/api/v1/kernel/modules/notes", Some("admin")).await;
        assert_eq!(described, rest["data"]);

        let missing = one.call(admin.clone(), json!({ "name": "shop" })).await.unwrap_err();
        assert_eq!(missing.code(), "kernel.module_not_found");
        let invalid = one.call(admin, json!({ "name": "" })).await.unwrap_err();
        assert_eq!(invalid.code(), "validation_failed");
    })
    .await
    .unwrap();
}
