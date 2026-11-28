//! Hợp đồng module kiểu mới (D-20261126-module-manifest): module khai mọi thứ tĩnh bằng `Manifest`, đăng ký route,
//! tool MCP, event cần nhận trong `start`; kernel đọc đúng các thứ đó như với module kiểu cũ. Chạy trên cả hai database.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use autoxide_kernel::access::{Permission, Principal};
use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler, EventInfo, Outbox, Subscription};
use autoxide_kernel::http::endpoint;
use autoxide_kernel::mcp::{NoInput, Tool};
use autoxide_kernel::module::{Manifest, Module, Modules, Stability, StartContext};
use autoxide_kernel::privacy::USER_DELETED;
use autoxide_kernel::routes;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::TestDatabase;

const MEMO_READ: Permission = Permission::new("memo.read", "Read memos.");
const MEMO_CREATED: EventInfo = EventInfo::new("memo.memo_created", "A memo was created.");

/// Ping.
#[endpoint(get, path = "/ping", public, responses((status = 200, description = "Alive.", body = String)))]
async fn ping() -> &'static str {
    "pong"
}

struct Forget;

#[autoxide_kernel::async_trait]
impl EventHandler for Forget {
    async fn handle(&self, _event: &Delivered) -> Result<(), Error> {
        Ok(())
    }
}

/// Module khai bằng manifest, đăng ký trong `start`.
struct Memo;

impl Module for Memo {
    fn manifest(&self) -> Manifest {
        Manifest::new("memo", "Short memos.")
            .stability(Stability::Experimental)
            .permissions([MEMO_READ])
            .events([MEMO_CREATED])
            .translation("vi", r#"{"memo.empty": "Ghi nhớ trống."}"#)
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        // Có khai event nên lấy được outbox; kiểu đích chọn handle.
        let _outbox: Outbox = context.kernel()?;
        context.subscribe(Subscription::new(USER_DELETED, Arc::new(Forget)))?;
        context.routes(routes![ping])?;
        context.tools([Tool::new(
            "count",
            "Count memos.",
            |_: TenantContext, _: Principal, _: NoInput| async move { Ok(0) },
        )
        .requires(MEMO_READ)
        .read_only()])
    }
}

/// Module kiểu cũ (chưa cài `manifest`) mà đăng ký kiểu mới: bị từ chối, không lặng lẽ bỏ qua.
struct Mixed;

impl Module for Mixed {
    fn name(&self) -> &'static str {
        "mixed"
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.subscribe(Subscription::new(USER_DELETED, Arc::new(Forget)))
    }
}

/// Module khai manifest không có event mà xin outbox.
struct NoEvents;

impl Module for NoEvents {
    fn manifest(&self) -> Manifest {
        Manifest::new("quiet", "Emits nothing.")
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.kernel::<Outbox>().map(|_| ())
    }
}

/// Module không cài cả `manifest` lẫn `name`.
struct Nameless;

impl Module for Nameless {}

async fn connect(database: &TestDatabase) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    Database::connect(&reader.finish(config).unwrap().0).await.unwrap()
}

#[test]
fn manifest_thay_cac_ham_khai_bao_cu() {
    let memo = Memo;
    assert_eq!(memo.name(), "memo");
    assert_eq!(memo.description(), "Short memos.");
    assert_eq!(memo.stability(), Stability::Experimental);
    assert_eq!(memo.permissions(), [MEMO_READ]);
    assert_eq!(memo.events(), [MEMO_CREATED]);
    assert_eq!(memo.translations().len(), 1);
    assert!(memo.migrations().is_empty());

    let modules = Modules::new(vec![Box::new(Memo)]).unwrap();
    assert!(modules.names().contains(&"memo"));
    assert!(modules.permissions().contains("memo.read"));
    assert_eq!(modules.experimental(), ["memo"]);
}

#[test]
fn module_khong_ten_bi_tu_choi_va_chi_cach_sua() {
    let error = Modules::new(vec![Box::new(Nameless)]).unwrap_err().to_string();
    assert!(error.contains("implement `Module::manifest`"), "{error}");
}

#[tokio::test]
async fn thu_dang_ky_trong_start_den_duoc_kernel() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let mut modules = Modules::new(vec![Box::new(Memo)]).unwrap();
        modules.start(&db).unwrap();

        let tools = modules.tools().unwrap();
        let names: Vec<&str> =
            tools.iter().map(Tool::name).filter(|name| name.starts_with("memo.")).collect();
        assert_eq!(names, ["memo.count"]);

        let routes = modules.routes();
        let (_, router) = routes.iter().find(|(name, _)| *name == "memo").unwrap();
        assert!(router.get_openapi().paths.paths.contains_key("/ping"));

        assert!(modules.user_deleted_receivers().contains(&"memo"));
        // Sau `start`, module vẫn trả đúng phần khai tĩnh (qua manifest của module bên trong).
        assert!(modules.permissions().contains("memo.read"));
        assert!(modules.dispatcher().is_ok());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn module_kieu_cu_khong_duoc_dang_ky_kieu_moi() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let mut modules = Modules::new(vec![Box::new(Mixed)]).unwrap();
        let error = modules.start(&db).unwrap_err().to_string();
        assert!(error.contains("does not implement `Module::manifest`"), "{error}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn kernel_handle_giu_luat_cua_getter_cu() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let mut modules = Modules::new(vec![Box::new(NoEvents)]).unwrap();
        let error = modules.start(&db).unwrap_err().to_string();
        assert!(error.contains("declares no events"), "{error}");
    })
    .await
    .unwrap();
}
