//! Giai đoạn khởi động của module: nhận database, công bố trait cho module khác,
//! lấy trait của module khác, và khai route. Gate chạy file này trên cả hai database.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
// Clippy chỉ tự miễn cho hàm `#[test]`, không miễn cho hàm phụ, nên khai ở đây.
// Trong `src/` thì KHÔNG được khai dòng này (`cargo xtask check` chặn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, Modules, StartContext};
use autoxide_testkit::TestDatabase;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// Hợp đồng mà module `users` công bố (trong module thật, nó nằm ở `users::api`).
trait UserDirectory: Send + Sync {
    fn display_name(&self, id: u32) -> String;
}

struct UserService;

impl UserDirectory for UserService {
    fn display_name(&self, id: u32) -> String {
        format!("user-{id}")
    }
}

/// Module công bố `dyn UserDirectory`.
struct Users;

impl Module for Users {
    fn name(&self) -> &'static str {
        "users"
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.provide::<dyn UserDirectory>(Arc::new(UserService))
    }
}

/// Module dùng `dyn UserDirectory` của module khác; ghi lại điều nó thấy vào `seen`.
struct Notes {
    seen: Arc<Mutex<Vec<String>>>,
    users: Option<Arc<dyn UserDirectory>>,
}

impl Notes {
    fn new(seen: &Arc<Mutex<Vec<String>>>) -> Self {
        Notes { seen: Arc::clone(seen), users: None }
    }
}

/// List notes.
#[utoipa::path(get, path = "/", responses((status = 200, description = "The notes")))]
async fn list_notes() -> &'static str {
    "notes"
}

impl Module for Notes {
    fn name(&self) -> &'static str {
        "notes"
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let users = context.require::<dyn UserDirectory>()?;
        self.seen.lock().unwrap().push(users.display_name(7));
        self.users = Some(users);
        Ok(())
    }

    fn routes(&self) -> OpenApiRouter {
        OpenApiRouter::new().routes(routes!(list_notes))
    }
}

/// Module dùng database lúc khởi động.
struct Pinger(Arc<Mutex<Vec<String>>>);

impl Module for Pinger {
    fn name(&self) -> &'static str {
        "pinger"
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        // Giữ một bản `Database` (clone rẻ) như một service thật sẽ làm.
        let database = context.database().clone();
        self.0.lock().unwrap().push(format!("{:?}", database.backend()));
        Ok(())
    }
}

/// Module khởi động hỏng.
struct Broken;

impl Module for Broken {
    fn name(&self) -> &'static str {
        "broken"
    }

    fn start(&mut self, _context: &mut StartContext<'_>) -> Result<(), Error> {
        Err(Error::unavailable("broken.unreachable", "The upstream is unreachable."))
    }
}

async fn connect(database: &TestDatabase) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    Database::connect(&reader.finish(config).unwrap().0).await.unwrap()
}

fn seen() -> Arc<Mutex<Vec<String>>> {
    Arc::new(Mutex::new(Vec::new()))
}

#[tokio::test]
async fn module_lay_duoc_trait_cua_module_dang_ky_truoc() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let seen = seen();
        let mut modules = Modules::new(vec![Box::new(Users), Box::new(Notes::new(&seen))]).unwrap();

        assert!(modules.api::<dyn UserDirectory>().is_none());

        modules.start(&db).unwrap();

        assert_eq!(*seen.lock().unwrap(), ["user-7"]);
        // Sau khởi động, binary lấy được trait module đã công bố.
        let users = modules.api::<dyn UserDirectory>().unwrap();
        assert_eq!(users.display_name(1), "user-1");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn can_trait_cua_module_dang_ky_sau_thi_bao_ro_cach_sua() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let seen = seen();
        let mut modules = Modules::new(vec![Box::new(Notes::new(&seen)), Box::new(Users)]).unwrap();

        let error = modules.start(&db).unwrap_err().to_string();

        assert!(error.starts_with("modules: module `notes` failed to start:"), "{error}");
        assert!(error.contains("requires `dyn module::UserDirectory`"), "{error}");
        assert!(error.contains("register the providing module before `notes`"), "{error}");
        assert!(seen.lock().unwrap().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_module_cong_bo_cung_mot_trait_la_loi() {
    TestDatabase::run(async |database| {
        struct OtherUsers;
        impl Module for OtherUsers {
            fn name(&self) -> &'static str {
                "accounts"
            }

            fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
                context.provide::<dyn UserDirectory>(Arc::new(UserService))
            }
        }
        let db = connect(database).await;
        let mut modules = Modules::new(vec![Box::new(Users), Box::new(OtherUsers)]).unwrap();

        let error = modules.start(&db).unwrap_err().to_string();

        assert!(error.contains("module `accounts` failed to start"), "{error}");
        assert!(error.contains("module `users` already does"), "{error}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn module_khoi_dong_hong_thi_dung_va_bao_ten_module() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let seen = seen();
        let mut modules =
            Modules::new(vec![Box::new(Broken), Box::new(Pinger(Arc::clone(&seen)))]).unwrap();

        let error = modules.start(&db).unwrap_err().to_string();

        assert_eq!(
            error,
            "modules: module `broken` failed to start: broken.unreachable: The upstream is unreachable."
        );
        // Module đứng sau module hỏng không được khởi động.
        assert!(seen.lock().unwrap().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn module_nhan_database_va_khai_route() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let seen = seen();
        let mut modules = Modules::new(vec![
            Box::new(Users),
            Box::new(Notes::new(&seen)),
            Box::new(Pinger(Arc::clone(&seen))),
        ])
        .unwrap();
        modules.start(&db).unwrap();

        assert_eq!(seen.lock().unwrap()[1], format!("{:?}", db.backend()));

        // Mỗi module một router; module không khai route thì router rỗng.
        let routes = modules.routes();
        let names: Vec<&str> = routes.iter().map(|(name, _)| *name).collect();
        // Module nội bộ `kernel` luôn đứng đầu (không có route).
        assert_eq!(names, ["kernel", "users", "notes", "pinger"]);
        let paths: Vec<Vec<String>> = routes
            .into_iter()
            .map(|(_, router)| router.into_openapi().paths.paths.keys().cloned().collect())
            .collect();
        assert_eq!(paths, [vec![], vec![], vec!["/".to_owned()], vec![]]);
    })
    .await
    .unwrap();
}

/// Module khai permission; tên permission do test chọn.
struct Declares(&'static str, Vec<Permission>);

impl Module for Declares {
    fn name(&self) -> &'static str {
        self.0
    }

    fn permissions(&self) -> Vec<Permission> {
        self.1.clone()
    }
}

#[tokio::test]
async fn module_thay_danh_muc_permission_cua_moi_module() {
    struct Reader(Arc<Mutex<Vec<String>>>);
    impl Module for Reader {
        fn name(&self) -> &'static str {
            "reader"
        }
        fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
            let names = context.permissions().iter().map(|p| p.name().to_owned());
            self.0.lock().unwrap().extend(names);
            Ok(())
        }
    }

    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let seen = seen();
        let notes = Declares("notes", vec![Permission::new("notes.write", "Write notes.")]);
        let mut modules =
            Modules::new(vec![Box::new(Reader(seen.clone())), Box::new(notes)]).unwrap();

        modules.start(&db).unwrap();

        // Module đăng ký trước vẫn thấy permission của module đăng ký sau.
        assert_eq!(*seen.lock().unwrap(), ["notes.write"]);
        assert!(modules.permissions().contains("notes.write"));
    })
    .await
    .unwrap();
}

#[test]
fn permission_sai_ten_bi_tu_choi_luc_dang_ky() {
    let wrong = Declares("notes", vec![Permission::new("tags.read", "Read tags.")]);

    let error = Modules::new(vec![Box::new(wrong)]).unwrap_err().to_string();

    assert!(error.contains("permission `tags.read` of module `notes` must be named"), "{error}");
}

#[test]
fn khai_deprecated_sai_bi_tu_choi_luc_dang_ky() {
    struct Old;
    impl Module for Old {
        fn name(&self) -> &'static str {
            "notes"
        }
        fn deprecations(&self) -> Vec<autoxide_kernel::http::Deprecation> {
            use autoxide_kernel::http::Deprecation;
            use axum::http::Method;
            vec![
                Deprecation::new(Method::GET, "/{id}", "01/11/2026"),
                Deprecation::new(Method::GET, "/", "2026-11-01").sunset("2026-10-01"),
            ]
        }
    }

    let error = Modules::new(vec![Box::new(Old)]).unwrap_err().to_string();

    assert!(error.contains("`since` must be a YYYY-MM-DD date"), "{error}");
    assert!(error.contains("`sunset` must come after `since`"), "{error}");
}
