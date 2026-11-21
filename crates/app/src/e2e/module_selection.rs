//! Bật / tắt module bằng `AUTOXIDE_MODULES` (bước 70 P1 và P3), trên chương trình thật và cả hai
//! database: tập tối thiểu chạy được; mỗi module khởi động được với đúng những module nó khai là
//! cần; database còn bảng của module đang tắt vẫn khởi động và dữ liệu còn nguyên.

use autoxide_kernel::config::ConfigSource;
use autoxide_kernel::database::Database;
use autoxide_kernel::module::{Module, Stability};
use autoxide_testkit::{TestDatabase, schema_snapshot};
use serde_json::json;

use crate::e2e::media::{admin_token, call, raw};
use crate::runtime::{self, Migrations, Runtime};
use crate::settings::Settings;

/// Cấu hình như một BẢN CÀI đọc nó (`settings::read_with`: không đặt `AUTOXIDE_MODULES` thì chỉ
/// module chính thức), khác `settings::read` của test (không đặt thì mọi module).
fn installed(url: &str, modules: Option<&str>) -> Result<Settings, String> {
    let scratch = std::env::temp_dir().join(format!("autoxide-modules-{}", uuid::Uuid::now_v7()));
    let files = scratch.join("files").display().to_string();
    let mail = scratch.join("mail").display().to_string();
    let mut pairs = vec![
        ("DATABASE_URL", url),
        ("AUTOXIDE_FILES_DIR", files.as_str()),
        ("AUTOXIDE_MAIL_FILE_DIR", mail.as_str()),
        // Cố định, để access token cấp trước khi khởi động lại vẫn dùng được sau đó.
        ("AUTOXIDE_AUTH_TOKEN_SECRET", "module-selection-test-secret-0123456789abcdef"),
    ];
    if let Some(modules) = modules {
        pairs.push(("AUTOXIDE_MODULES", modules));
    }
    let source = ConfigSource::from_pairs(pairs);
    crate::settings::read_with(&source, crate::modules::all()).map_err(|errors| errors.to_string())
}

/// Migrate rồi khởi động như `autoxide migrate up` + `autoxide serve`.
async fn serve(url: &str, modules: Option<&str>) -> Runtime {
    let settings = installed(url, modules).unwrap();
    let migrator = settings.modules.migrator().unwrap();
    let database = Database::connect(&settings.config.database).await.unwrap();
    migrator.up(&database).await.unwrap();
    database.close().await.unwrap();
    runtime::start(settings, Migrations::RequireApplied).await.unwrap()
}

fn enabled(settings: &Settings) -> Vec<&'static str> {
    settings
        .modules
        .names()
        .into_iter()
        .filter(|name| !["kernel", "antispam"].contains(name))
        .collect()
}

/// Danh sách hạng đã chốt (D-20261121-module-selection). Đổi hạng một module là đổi hành vi mặc
/// định của mọi bản cài: bài này buộc việc đó phải cố ý, và `docs/MODULES.md` phải theo.
#[test]
fn hang_cua_tung_module_dung_nhu_da_chot() {
    let experimental: Vec<&str> = crate::modules::all()
        .iter()
        .filter(|module| module.stability() == Stability::Experimental)
        .map(|module| module.name())
        .collect();
    assert_eq!(
        experimental,
        [
            "bookings",
            "newsletter",
            "comments",
            "support",
            "conversations",
            "analytics",
            "reporting",
            "automations"
        ]
    );
    let essential: Vec<&str> = crate::modules::all()
        .iter()
        .filter(|module| module.essential())
        .map(|module| module.name())
        .collect();
    assert_eq!(essential, ["auth", "idempotency"]);

    let url = "sqlite://data/not-opened.sqlite";
    assert_eq!(enabled(&installed(url, Some("core")).unwrap()), ["mail", "auth", "idempotency"]);
    // Không đặt biến: đúng các module chính thức.
    let default = installed(url, None).unwrap();
    assert_eq!(enabled(&default).len(), crate::modules::all().len() - experimental.len());
    let off: Vec<&str> = default.modules.inactive().iter().map(|module| module.name).collect();
    assert_eq!(off, experimental);
    assert_eq!(enabled(&installed(url, Some("all")).unwrap()).len(), crate::modules::all().len());
}

/// `docs/MODULES.md` và bản tiếng Việt có một dòng cho mỗi module, đúng hạng và phụ thuộc trong code.
#[test]
fn tai_lieu_module_khop_voi_code() {
    let documents = [
        ("docs/MODULES.md", include_str!("../../../../docs/MODULES.md")),
        ("docs/MODULES.vi.md", include_str!("../../../../docs/MODULES.vi.md")),
    ];
    let essential_names =
        enabled(&installed("sqlite://data/not-opened.sqlite", Some("core")).unwrap());
    for module in crate::modules::all() {
        let tier = if essential_names.contains(&module.name()) {
            "core"
        } else {
            module.stability().as_str()
        };
        let needs = module.depends_on();
        let needs = if needs.is_empty() {
            "—".to_owned()
        } else {
            needs.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ")
        };
        let row = format!("| `{}` | {tier} | {needs} |", module.name());
        for (path, text) in documents {
            assert!(
                text.lines().any(|line| line.starts_with(&row)),
                "{path} has no row starting with `{row}`; update the table to match the code"
            );
        }
    }
}

#[test]
fn go_sai_ten_hay_tat_thieu_phu_thuoc_bao_loi_ro() {
    let url = "sqlite://data/not-opened.sqlite";
    let error = installed(url, Some("official,order")).err().unwrap();
    assert!(
        error.contains("AUTOXIDE_MODULES: unknown module `order`. Modules: mail, ai,"),
        "{error}"
    );
    assert!(error.contains("orders"), "{error}");

    // `orders` cần `catalog`: tắt `catalog` mà còn `orders` thì nói rõ phải làm gì.
    let error = installed(url, Some("official,-catalog")).err().unwrap();
    assert!(
        error.contains(
            "`-catalog` turns off a module that `inventory`, `orders`, `promotions` depend on; \
             remove `-catalog`, or leave them out too (`-inventory,-orders,-promotions`)"
        ),
        "{error}"
    );
    let error = installed(url, Some("official,-auth")).err().unwrap();
    assert!(error.contains("`auth` cannot be turned off"), "{error}");

    // Nêu một module thì module nó cần được bật kèm, và sổ module ghi vì sao.
    let settings = installed(url, Some("content")).unwrap();
    assert_eq!(
        enabled(&settings),
        ["mail", "ai", "files", "auth", "idempotency", "media", "content"]
    );
    assert!(settings.modules.implied().contains(&("media", "content")));
}

/// Mỗi module, bật MỘT MÌNH (cùng những module nó khai ở `Module::depends_on` và module core), phải
/// khởi động được và dựng được router, tool MCP, worker, bộ phát event. Module `require` một trait
/// mà quên khai phụ thuộc thì hỏng ở đây chứ không ở máy người cài.
#[tokio::test]
async fn moi_module_khoi_dong_duoc_voi_dung_phu_thuoc_da_khai() {
    TestDatabase::run(async |test| {
        let names: Vec<&str> = crate::modules::all().iter().map(|module| module.name()).collect();
        for name in names.iter().copied().chain(["core", "official"]) {
            let settings = installed(test.url(), Some(name))
                .unwrap_or_else(|error| panic!("AUTOXIDE_MODULES={name}: {error}"));
            // `start` của module không truy vấn database (nó không `async`), nên không cần migrate.
            let runtime = runtime::start(settings, Migrations::Ignore)
                .await
                .unwrap_or_else(|error| panic!("AUTOXIDE_MODULES={name}: {error}"));
            let problems = autoxide_kernel::http::lint(&runtime.api.openapi);
            assert!(problems.is_empty(), "AUTOXIDE_MODULES={name}: {problems:?}");
            assert!(runtime.api.schema_conflicts.is_empty(), "AUTOXIDE_MODULES={name}");
            runtime.database.close().await.unwrap();
        }
    })
    .await
    .unwrap();
}

/// Tập tối thiểu (`core`): migrate, khởi động, đăng nhập và gọi API được; module tắt không có
/// bảng, route, tài liệu hay tool.
#[tokio::test]
async fn tap_toi_thieu_migrate_khoi_dong_va_phuc_vu() {
    TestDatabase::run(async |test| {
        let runtime = serve(test.url(), Some("core")).await;

        // Migration: chỉ bảng của module đang bật.
        let snapshot = schema_snapshot(runtime.database.write()).await.unwrap();
        let tables = snapshot.table_names();
        assert!(tables.contains(&"auth_migrations"), "{tables:?}");
        for prefix in ["content_", "catalog_", "orders_", "audit_", "organizations_"] {
            assert!(!tables.iter().any(|table| table.starts_with(prefix)), "{prefix}: {tables:?}");
        }

        // Một request thật: tạo quản trị viên, đăng nhập, hỏi "tôi là ai".
        let token = admin_token(&runtime, "root@example.com").await;
        let auth = format!("Bearer {token}");
        let headers = [("authorization", auth.as_str())];
        let (status, _, me) = call(&runtime, "GET", "/api/v1/auth/me", &headers, Vec::new()).await;
        assert_eq!(status, 200, "{me}");
        assert_eq!(me["data"]["email"], "root@example.com");

        // Route của module tắt không tồn tại; OpenAPI lúc chạy và `/llms.txt` không nhắc tới nó.
        let (status, _, _) =
            call(&runtime, "GET", "/api/v1/content/entries", &headers, Vec::new()).await;
        assert_eq!(status, 404);
        let (status, _, openapi) =
            call(&runtime, "GET", "/api/v1/openapi.json", &headers, Vec::new()).await;
        assert_eq!(status, 200);
        let paths: Vec<&String> = openapi["paths"].as_object().unwrap().keys().collect();
        assert!(paths.iter().any(|path| path.starts_with("/api/v1/auth/")), "{paths:?}");
        for prefix in ["/api/v1/content", "/api/v1/catalog", "/api/v1/webhooks", "/api/v1/audit"] {
            assert!(!paths.iter().any(|path| path.starts_with(prefix)), "{prefix} is documented");
        }
        let (status, _, llms) = raw(&runtime, "GET", "/llms.txt", &[], Vec::new()).await;
        assert_eq!(status, 200);
        let llms = String::from_utf8(llms).unwrap();
        assert!(llms.contains("[auth](/api/v1/kernel/modules/auth)"), "{llms}");
        assert!(!llms.contains("/modules/catalog") && !llms.contains("/modules/audit"), "{llms}");

        // Tool MCP, job, event: chỉ của module đang bật.
        let tools = runtime.modules.tools().unwrap();
        assert!(tools.iter().any(|tool| tool.name() == "auth.whoami"));
        assert!(tools.iter().all(|tool| {
            ["auth.", "kernel.", "mail.", "idempotency.", "antispam."]
                .iter()
                .any(|prefix| tool.name().starts_with(prefix))
        }));
        // Event của `auth` vẫn được phát (không ai nghe cũng không kẹt).
        runtime.dispatcher.run_once().await.unwrap();
        runtime.database.close().await.unwrap();
    })
    .await
    .unwrap();
}

/// Người nâng cấp từ bản mà mọi module đều chạy: database có bảng và dữ liệu của module thử
/// nghiệm, bản mới mặc định tắt chúng. Chương trình vẫn khởi động, không migration nào "còn thiếu",
/// dữ liệu còn nguyên, và người cài được báo đúng dòng cấu hình cần thêm; bật lại là dùng tiếp.
#[tokio::test]
async fn database_co_du_lieu_cua_module_tat_van_khoi_dong_va_con_nguyen() {
    TestDatabase::run(async |test| {
        // Bản cũ: mọi module chạy; có một danh sách bản tin (module thử nghiệm `newsletter`).
        let old = serve(test.url(), Some("all")).await;
        let token = admin_token(&old, "root@example.com").await;
        let auth = format!("Bearer {token}");
        let headers = [("authorization", auth.as_str()), ("content-type", "application/json")];
        let list = json!({"key": "news", "name": {"en": "News"}}).to_string().into_bytes();
        let (status, _, answer) =
            call(&old, "POST", "/api/v1/newsletter/lists", &headers, list).await;
        assert!(status.is_success(), "{status} {answer}");
        // Một role mang permission của module sắp tắt: role vẫn phải đọc được sau khi tắt.
        let role = json!({"name": "editor", "permissions": ["newsletter.manage", "content.read"]});
        let (status, _, answer) =
            call(&old, "POST", "/api/v1/auth/roles", &headers, role.to_string().into_bytes()).await;
        assert!(status.is_success(), "{status} {answer}");
        old.database.close().await.unwrap();

        // Bản mới, không đặt `AUTOXIDE_MODULES`: `newsletter` tắt.
        let settings = installed(test.url(), None).unwrap();
        let migrator = settings.modules.migrator().unwrap();
        let database = Database::connect(&settings.config.database).await.unwrap();
        assert!(migrator.pending(&database).await.unwrap().is_empty());
        database.close().await.unwrap();
        let new = runtime::start(settings, Migrations::RequireApplied).await.unwrap();
        let (status, _, _) =
            call(&new, "GET", "/api/v1/newsletter/lists", &headers, Vec::new()).await;
        assert_eq!(status, 404);
        // Đăng nhập cũ vẫn dùng được.
        let (status, _, _) = call(&new, "GET", "/api/v1/auth/me", &headers, Vec::new()).await;
        assert_eq!(status, 200);

        let (status, _, roles) =
            call(&new, "GET", "/api/v1/auth/roles", &headers, Vec::new()).await;
        assert_eq!(status, 200, "{roles}");
        assert!(roles.to_string().contains("newsletter.manage"), "{roles}");

        // Mọi module thử nghiệm đều còn bảng; chỉ `newsletter` có dữ liệu.
        let dormant = new.modules.dormant(&new.database).await.unwrap();
        assert_eq!(dormant.len(), new.modules.inactive().len());
        let with_data =
            crate::modules::dormant_with_data(&new.modules, &new.database).await.unwrap();
        let names: Vec<&str> = with_data.iter().map(|module| module.name).collect();
        assert_eq!(names, ["newsletter"], "{with_data:?}");
        assert!(with_data[0].tables_with_data.contains(&"newsletter_lists".to_owned()));
        let (message, hint) = crate::modules::dormant_notice(&new.modules, &with_data);
        assert!(message.contains("newsletter (1 table(s) with rows)"), "{message}");
        assert!(hint.contains("`AUTOXIDE_MODULES=official,newsletter`"), "{hint}");
        // Lệnh chép dữ liệu từ chối bỏ sót dữ liệu đó.
        let refused =
            crate::modules::refuse_dormant_data(&new.modules, &new.database, "copy").await;
        assert!(refused.unwrap_err().contains("AUTOXIDE_MODULES=official,newsletter"));
        new.dispatcher.run_once().await.unwrap();
        new.database.close().await.unwrap();

        // Bật lại theo đúng dòng được gợi ý: dữ liệu còn nguyên.
        let again = serve(test.url(), Some("official,newsletter")).await;
        let (status, _, lists) =
            call(&again, "GET", "/api/v1/newsletter/lists", &headers, Vec::new()).await;
        assert_eq!(status, 200, "{lists}");
        assert_eq!(lists["data"][0]["key"], "news");
        let none =
            crate::modules::dormant_with_data(&again.modules, &again.database).await.unwrap();
        assert!(none.is_empty(), "{none:?}");
        again.database.close().await.unwrap();
    })
    .await
    .unwrap();
}
