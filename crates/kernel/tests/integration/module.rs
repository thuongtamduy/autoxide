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
use autoxide_kernel::module::{Later, Module, Modules, StartContext};
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

/// Module giữ tay cầm `Later` tới trait của module đăng ký SAU nó.
struct LateReader(Arc<Mutex<Option<Later<dyn UserDirectory>>>>);

impl Module for LateReader {
    fn name(&self) -> &'static str {
        "late_reader"
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let later = context.later::<dyn UserDirectory>();
        // Trong `start` chưa điền: đọc lúc phục vụ.
        assert!(later.get().is_none());
        *self.0.lock().unwrap() = Some(later);
        Ok(())
    }
}

#[tokio::test]
async fn later_lay_trait_cua_module_dang_ky_sau() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let held = Arc::new(Mutex::new(None));
        let mut modules =
            Modules::new(vec![Box::new(LateReader(held.clone())), Box::new(Users)]).unwrap();
        modules.start(&db).unwrap();
        let later = held.lock().unwrap().clone().unwrap();
        assert_eq!(later.get().unwrap().display_name(3), "user-3");

        // Không module nào công bố: tay cầm trả `None`, khởi động vẫn được.
        let alone = Arc::new(Mutex::new(None));
        let mut modules = Modules::new(vec![Box::new(LateReader(alone.clone()))]).unwrap();
        modules.start(&db).unwrap();
        assert!(alone.lock().unwrap().clone().unwrap().get().is_none());
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
        // Tên kiểu đầy đủ tuỳ chỗ khai trait (ở đây có tiền tố của chương trình test).
        assert!(
            error.contains("requires `dyn ") && error.contains("module::UserDirectory`"),
            "{error}"
        );
        assert!(
            error.contains("The providing module must be registered before `notes`")
                && error.contains("`Module::depends_on`")
                && error.contains("AUTOXIDE_MODULES"),
            "{error}"
        );
        assert!(seen.lock().unwrap().is_empty());
    })
    .await
    .unwrap();
}

// Sổ thời hạn lưu được liệt kê theo trang với khoá (module, tên): một module khai hai mục trùng tên thì khởi động
// dừng, nói rõ module và tên — không để trang `compliance/retention` lỗi 500 mỗi lần đọc. Hai module khác nhau dùng
// cùng tên thì được (khoá có tên module).
#[tokio::test]
async fn hai_muc_thoi_han_luu_trung_ten_trong_mot_module_la_loi() {
    use autoxide_kernel::privacy::{DataCategory, Keep, RetentionPolicy};
    TestDatabase::run(async |database| {
        struct Keeps(&'static str, usize);
        impl Module for Keeps {
            fn name(&self) -> &'static str {
                self.0
            }

            fn retention(&self) -> Vec<RetentionPolicy> {
                (0..self.1)
                    .map(|_| {
                        RetentionPolicy::new(
                            "shared.logs",
                            "Logs.",
                            DataCategory::Basic,
                            Keep::Days(30),
                        )
                    })
                    .collect()
            }
        }
        let db = connect(database).await;
        let mut fine =
            Modules::new(vec![Box::new(Keeps("one", 1)), Box::new(Keeps("two", 1))]).unwrap();
        fine.start(&db).unwrap();

        let mut modules = Modules::new(vec![Box::new(Keeps("one", 2))]).unwrap();
        let error = modules.start(&db).unwrap_err().to_string();

        assert!(
            error.contains("module `one` declares the retention policy `shared.logs` twice"),
            "{error}"
        );
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

/// Nhiều module cùng góp một trait (`contribute`, bước 36); module sau thấy mọi bộ góp trước nó theo
/// thứ tự đăng ký; một module góp hai lần là lỗi.
#[tokio::test]
async fn nhieu_module_cung_gop_mot_trait() {
    TestDatabase::run(async |database| {
        struct Named(&'static str);
        impl UserDirectory for Named {
            fn display_name(&self, id: u32) -> String {
                format!("{}-{id}", self.0)
            }
        }
        struct Contributor(&'static str, usize);
        impl Module for Contributor {
            fn name(&self) -> &'static str {
                self.0
            }

            fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
                for _ in 0..self.1 {
                    context.contribute::<dyn UserDirectory>(Arc::new(Named(self.0)))?;
                }
                Ok(())
            }
        }
        struct Reader(Arc<Mutex<Vec<String>>>);
        impl Module for Reader {
            fn name(&self) -> &'static str {
                "reader"
            }

            fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
                let all = context.contributions::<dyn UserDirectory>();
                *self.0.lock().unwrap() = all.iter().map(|api| api.display_name(1)).collect();
                Ok(())
            }
        }
        let db = connect(database).await;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut modules = Modules::new(vec![
            Box::new(Contributor("vnpay", 1)),
            Box::new(Contributor("momo", 1)),
            Box::new(Reader(Arc::clone(&seen))),
        ])
        .unwrap();
        modules.start(&db).unwrap();
        assert_eq!(*seen.lock().unwrap(), ["vnpay-1", "momo-1"]);

        let mut twice = Modules::new(vec![Box::new(Contributor("vnpay", 2))]).unwrap();
        let error = twice.start(&db).unwrap_err().to_string();
        assert!(error.contains("contributes"), "{error}");
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
        // Hai module nội bộ luôn đứng đầu; `kernel` có sổ module, `antispam` có endpoint lấy
        // token form. Cuối cùng là `me`: endpoint "của tôi" của kernel dưới `/api/v1/me` (D-207).
        assert_eq!(names, ["kernel", "antispam", "users", "notes", "pinger", "me"]);
        let paths: Vec<Vec<String>> = routes
            .into_iter()
            .map(|(_, router)| router.into_openapi().paths.paths.keys().cloned().collect())
            .collect();
        assert_eq!(
            paths,
            [
                vec![
                    "/approvals".to_owned(),
                    "/approvals/{id}".to_owned(),
                    "/approvals/{id}/approve".to_owned(),
                    "/approvals/{id}/reject".to_owned(),
                    "/blocks".to_owned(),
                    "/bulk/{resource}/{action}".to_owned(),
                    "/client-config".to_owned(),
                    "/connections/webhooks".to_owned(),
                    "/custom-fields".to_owned(),
                    "/custom-fields/{id}".to_owned(),
                    "/devtools/events".to_owned(),
                    "/devtools/requests".to_owned(),
                    "/domains".to_owned(),
                    "/domains/allowed".to_owned(),
                    "/domains/{id}".to_owned(),
                    "/domains/{id}/verify".to_owned(),
                    "/events/stream".to_owned(),
                    "/events/ws".to_owned(),
                    "/exports".to_owned(),
                    "/filters/parse".to_owned(),
                    "/find".to_owned(),
                    "/jobs".to_owned(),
                    "/jobs/{id}".to_owned(),
                    "/jobs/{id}/file".to_owned(),
                    "/jobs/{id}/retry".to_owned(),
                    "/modules".to_owned(),
                    "/modules/{name}".to_owned(),
                    "/navigation".to_owned(),
                    "/ops/alerts".to_owned(),
                    "/ops/backlog".to_owned(),
                    "/placements".to_owned(),
                    "/placements/migrations".to_owned(),
                    "/placements/migrations/{id}".to_owned(),
                    "/public/texts".to_owned(),
                    "/reasons/{list}".to_owned(),
                    "/reasons/{list}/{code}".to_owned(),
                    "/records/types".to_owned(),
                    "/records/{kind}/{id}/activity".to_owned(),
                    "/records/{kind}/{id}/notes".to_owned(),
                    "/records/{kind}/{id}/notes/{note_id}".to_owned(),
                    "/records/{kind}/{id}/tags".to_owned(),
                    "/records/{kind}/{id}/usages".to_owned(),
                    "/relocations".to_owned(),
                    "/relocations/{id}".to_owned(),
                    "/resolve".to_owned(),
                    "/sandbox/messages".to_owned(),
                    "/sandboxes/promotable".to_owned(),
                    "/sandboxes/{id}".to_owned(),
                    "/sandboxes/{id}/extend".to_owned(),
                    "/sandboxes/{id}/promote".to_owned(),
                    "/settings".to_owned(),
                    "/settings/{name}".to_owned(),
                    "/settings/{name}/test".to_owned(),
                    "/setup".to_owned(),
                    "/summary".to_owned(),
                    "/tags".to_owned(),
                    "/tags/{id}".to_owned(),
                    "/tags/{id}/apply".to_owned(),
                    "/tags/{id}/merge".to_owned(),
                    "/tenant".to_owned(),
                    "/tenant/relocation-requests".to_owned(),
                    "/tenant/relocation-requests/{id}".to_owned(),
                    "/tenants".to_owned(),
                    "/tenants/{id}".to_owned(),
                    "/tenants/{id}/erase".to_owned(),
                    "/tenants/{id}/move".to_owned(),
                    "/tenants/{id}/moves/{move_id}".to_owned(),
                    "/tenants/{id}/sandboxes".to_owned(),
                    "/tenants/{id}/settings".to_owned(),
                    "/tenants/{id}/settings/{name}".to_owned(),
                    "/texts".to_owned(),
                    "/texts/{locale}/{key}".to_owned(),
                    "/whats-new".to_owned(),
                    "/work".to_owned(),
                    "/work/counts".to_owned()
                ],
                vec!["/token".to_owned()],
                vec![],
                vec!["/".to_owned()],
                vec![],
                vec![
                    "/can".to_owned(),
                    "/flags".to_owned(),
                    "/flags/{key}".to_owned(),
                    "/permissions".to_owned(),
                    "/pins".to_owned(),
                    "/pins/order".to_owned(),
                    "/pins/{id}".to_owned(),
                    "/views".to_owned(),
                    "/views/{id}".to_owned(),
                    "/work/{key}/assign".to_owned(),
                    "/work/{key}/snooze".to_owned()
                ]
            ]
        );
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
        // (`kernel.*` là của module nội bộ `kernel`.)
        assert_eq!(
            *seen.lock().unwrap(),
            [
                "kernel.custom_fields.manage",
                "kernel.data.export",
                "kernel.devtools.manage",
                "kernel.devtools.read",
                "kernel.domains.manage",
                "kernel.jobs.manage",
                "kernel.jobs.read",
                "kernel.modules.read",
                "kernel.ops.read",
                "kernel.placements.read",
                "kernel.reasons.manage",
                "kernel.relocations.request",
                "kernel.sandboxes.manage",
                "kernel.sandboxes.read",
                "kernel.settings.manage",
                "kernel.tags.manage",
                "kernel.tenants.delete",
                "kernel.tenants.manage",
                "kernel.tenants.move",
                "kernel.tenants.operate",
                "kernel.tenants.read",
                "kernel.tenants.relocate",
                "kernel.texts.manage",
                "kernel.views.share",
                "notes.write"
            ]
        );
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

/// `GET /` khai bằng `#[endpoint]` (lớp HTTP của kernel).
#[autoxide_kernel::http::endpoint(get, path = "/", public, responses((status = 200, description = "Hello.")))]
async fn hello() -> &'static str {
    "hello"
}

/// `GET /other` khai bằng `#[endpoint]`.
#[autoxide_kernel::http::endpoint(get, path = "/other", public, responses((status = 200, description = "Other.")))]
async fn other() -> &'static str {
    "other"
}

/// Module khai cùng một route ở cả `routes` (cách cũ) và `endpoints` (cách mới) khi `twice`.
struct Both {
    twice: bool,
}

impl Module for Both {
    fn name(&self) -> &'static str {
        "both"
    }

    fn routes(&self) -> OpenApiRouter {
        OpenApiRouter::new().routes(routes!(list_notes))
    }

    fn endpoints(&self) -> autoxide_kernel::http::Routes {
        if self.twice {
            autoxide_kernel::routes![hello, other]
        } else {
            autoxide_kernel::routes![other]
        }
    }
}

/// `Module::routes` và `Module::endpoints` trùng path + method: không khởi động, báo đúng route trùng (thay vì axum
/// dừng tiến trình lúc ghép router); không trùng thì hai cách cùng chạy.
#[tokio::test]
async fn route_trung_giua_routes_va_endpoints_thi_khong_khoi_dong() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let mut modules = Modules::new(vec![Box::new(Both { twice: true })]).unwrap();
        let error = modules.start(&db).unwrap_err().to_string();
        assert!(error.contains("both: GET /"), "{error}");
        assert!(!error.contains("/other"), "{error}");

        let mut modules = Modules::new(vec![Box::new(Both { twice: false })]).unwrap();
        modules.start(&db).unwrap();
        let routes = modules.routes();
        let (_, router) = routes.iter().find(|(name, _)| *name == "both").unwrap();
        let paths: Vec<&String> = router.get_openapi().paths.paths.keys().collect();
        assert_eq!(paths, ["/", "/other"]);
    })
    .await
    .unwrap();
}
