//! Lớp HTTP của kernel (`#[endpoint]`, `routes!`, D-20261126-kernel-web-layer): ba cách khai ai được gọi qua được lint
//! của tài liệu, phần mở rộng của tác giả không làm mất `x-permissions`, `approval` sinh `x-approval`, và gate ép đăng
//! nhập cho `signed_in` kể cả khi handler không nhận `Principal`.

use autoxide_kernel::access::Permission;
use autoxide_kernel::http::{NoContent, Routes, endpoint};
use autoxide_kernel::module::Module;
use autoxide_kernel::routes;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;

const PROBE_READ: Permission = Permission::new("probe.read", "Read probes.");
const PROBE_WRITE: Permission = Permission::new("probe.write", "Change probes.");

/// Ping, open to anyone.
#[endpoint(get, path = "/ping", public, responses((status = 200, description = "Alive.", body = String)))]
async fn ping() -> &'static str {
    "pong"
}

/// Who is calling; any signed-in caller. The handler does not take a `Principal`: the kernel checks sign-in.
#[endpoint(get, path = "/me", signed_in, responses((status = 200, description = "Signed in.", body = String)))]
async fn me() -> &'static str {
    "me"
}

/// Read a probe; needs `probe.read`.
#[endpoint(get, path = "/read", permission = "probe.read", responses((status = 200, description = "Read.", body = String)))]
async fn read() -> &'static str {
    "read"
}

/// Delete a probe; needs `probe.write` or `probe.read`. Buyers pass; an AI agent's call waits for approval.
#[endpoint(
    delete,
    path = "/item",
    permission = ["probe.write", "probe.read"],
    approval = "agent",
    extensions(("x-buyers" = json!(true))),
    responses((status = 204, description = "Deleted."))
)]
async fn remove() -> NoContent {
    NoContent
}

/// Nested route.
#[endpoint(get, path = "/", public, responses((status = 200, description = "Nested.", body = String)))]
async fn nested() -> &'static str {
    "nested"
}

struct Probe;

impl Module for Probe {
    fn name(&self) -> &'static str {
        "probe"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![PROBE_READ, PROBE_WRITE]
    }

    fn endpoints(&self) -> Routes {
        let first: Routes = routes![ping, me];
        first.merge(routes![read, remove]).nest("/inner", routes![nested])
    }
}

fn extension(app: &TestApp, path: &str, method: &str, name: &str) -> Option<serde_json::Value> {
    let document = serde_json::to_value(app.openapi()).unwrap();
    document["paths"][path][method].get(name).cloned()
}

#[tokio::test]
async fn ba_cach_khai_qua_lint_va_phan_mo_rong_khong_mat() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).module(Probe).build().await.unwrap();
        assert_eq!(autoxide_kernel::http::lint(app.openapi()), Vec::<String>::new());

        let public = extension(&app, "/api/v1/probe/ping", "get", "x-public");
        assert_eq!(public, Some(serde_json::json!(true)));
        let signed_in = extension(&app, "/api/v1/probe/me", "get", "x-permissions");
        assert_eq!(signed_in, Some(serde_json::json!([])));
        // `x-buyers` của tác giả đứng cạnh `x-permissions` và `x-approval` do macro sinh: không mục nào mất.
        let at = |name| extension(&app, "/api/v1/probe/item", "delete", name);
        assert_eq!(at("x-permissions"), Some(serde_json::json!(["probe.write", "probe.read"])));
        assert_eq!(at("x-buyers"), Some(serde_json::json!(true)));
        assert_eq!(at("x-approval"), Some(serde_json::json!("agent")));
        // `merge` và `nest` của `Routes`.
        assert!(extension(&app, "/api/v1/probe/inner", "get", "x-public").is_some());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn signed_in_bi_ep_dang_nhap_du_handler_khong_nhan_principal() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).user("ada", 1, &[]).module(Probe).build().await.unwrap();
        let anonymous = app.get("/api/v1/probe/me").send().await.unwrap();
        assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
        let signed_in = app.get("/api/v1/probe/me").token("ada").send().await.unwrap();
        assert_eq!(signed_in.status(), StatusCode::OK);
        let public = app.get("/api/v1/probe/ping").send().await.unwrap();
        assert_eq!(public.status(), StatusCode::OK);
        // Đăng nhập mà thiếu permission: 403 trước khi tới handler.
        let denied = app.get("/api/v1/probe/read").token("ada").send().await.unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}

/// List things; needs `probe.read`. The handler does not take a `Principal`: only the gate protects it.
#[endpoint(get, path = "/things", permission = "probe.read", responses((status = 200, description = "Things.", body = String)))]
async fn list_things() -> &'static str {
    "things"
}

/// Create a thing; needs `probe.write`.
#[endpoint(post, path = "/", permission = "probe.write", responses((status = 201, description = "Created.", body = String)))]
async fn create_thing() -> &'static str {
    "created"
}

/// `nest` lên một path đã có operation khác method.
struct Nester;

impl Module for Nester {
    fn name(&self) -> &'static str {
        "probe"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![PROBE_READ, PROBE_WRITE]
    }

    fn endpoints(&self) -> Routes {
        let list: Routes = routes![list_things];
        list.nest("/things", routes![create_thing])
    }
}

/// `Routes::nest` lên path đã có: tài liệu giữ CẢ hai operation (không thay mục path), nên gate có luật cho cả hai.
#[tokio::test]
async fn nest_len_path_da_co_giu_ca_hai_operation() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).user("ada", 1, &[]).module(Nester).build().await.unwrap();
        let document = serde_json::to_value(app.openapi()).unwrap();
        let item = &document["paths"]["/api/v1/probe/things"];
        assert!(item.get("get").is_some() && item.get("post").is_some(), "{item}");
        for method in ["GET", "POST"] {
            let request = |token: Option<&str>| {
                let mut request = app.request(method.parse().unwrap(), "/api/v1/probe/things");
                if let Some(token) = token {
                    request = request.token(token);
                }
                request
            };
            assert_eq!(
                request(None).send().await.unwrap().status(),
                StatusCode::UNAUTHORIZED,
                "{method}"
            );
            assert_eq!(
                request(Some("ada")).send().await.unwrap().status(),
                StatusCode::FORBIDDEN,
                "{method}"
            );
        }
    })
    .await
    .unwrap();
}

/// Chống lệch: kernel hỏi `matchit` (khai thẳng, cùng bản lớn với axum) để báo route xung đột lúc khởi động. Cặp route
/// mà `matchit` coi là xung đột thì axum cũng dừng tiến trình khi dựng router, và ngược lại. Axum đổi bản lớn của
/// `matchit` (luật khớp đổi) thì test này đỏ thay vì phép kiểm lúc khởi động lặng lẽ sai.
#[test]
fn matchit_khop_voi_axum() {
    let cases = [
        ("/{id}", "/{*rest}", true),
        ("/{id}", "/{key}", true),
        ("/files/{*rest}", "/{id}", false),
        ("/things", "/{id}", false),
        ("/{id}/items", "/{id}", false),
    ];
    for (first, second, conflict) in cases {
        let mut matcher = matchit::Router::new();
        matcher.insert(first, ()).unwrap();
        let matchit_conflict = matcher.insert(second, ()).is_err();
        let axum_conflict = std::panic::catch_unwind(|| {
            let _router: axum::Router = axum::Router::new()
                .route(first, axum::routing::get(|| async {}))
                .route(second, axum::routing::delete(|| async {}));
        })
        .is_err();
        assert_eq!(matchit_conflict, conflict, "matchit {first} {second}");
        assert_eq!(axum_conflict, conflict, "axum {first} {second}");
    }
}
