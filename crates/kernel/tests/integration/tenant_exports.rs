//! Xuất dữ liệu khách (D-328) — ai được xuất khách nào (D-313, D-20261121-export-entry): nhân sự nền
//! tảng có `kernel.tenants.operate` không xuất được khách khác chỉ bằng đường dẫn, phải VÀO khách đó
//! (header `Autoxide-Tenant`, ở bản thật là lần cấp quyền vào hỗ trợ còn hạn); mỗi lượt xin ghi event
//! vào chính khách bị xuất. Chạy trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use autoxide_kernel::storage::Storage;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::{MemoryStorage, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};
use serde_json::Value;
use uuid::Uuid;

async fn app(test: &TestDatabase) -> TestApp {
    // Bản xuất cần nơi lưu file (ở bản thật: module `files`).
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    TestApp::builder(test)
        .provide::<dyn Storage>(storage)
        .tenant("acme")
        .user("ops", 1, &["kernel.tenants.operate"])
        .user("staff", 4, &["kernel.tenants.manage", "kernel.sandboxes.manage"])
        .user_in("acme", "owner", 2, &["kernel.data.export"])
        .tenant("beta")
        .domain("beta", "shop-beta.test")
        .user_in("beta", "stranger", 3, &["kernel.data.export"])
        .build()
        .await
        .unwrap()
}

async fn call(
    app: &TestApp,
    method: Method,
    uri: &str,
    who: &str,
    enter: Option<&str>,
) -> (StatusCode, Value) {
    let mut request = app.request(method, uri).token(who);
    if let Some(tenant) = enter {
        request = request.header("autoxide-tenant", tenant);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

/// Người làm của các event `name` trong database của khách `tenant`.
async fn actors(tenant: &TenantContext, name: &str) -> Vec<Option<Uuid>> {
    let select = Query::select()
        .column(Alias::new("actor_user_id"))
        .from(Alias::new("kernel_events"))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("name")).eq(name))
        .order_by(Alias::new("id"), sea_orm::sea_query::Order::Asc)
        .to_owned();
    let rows = tenant.database().read().query_all(&select).await.unwrap();
    rows.iter().map(|row| row.try_get::<Option<Uuid>>("", "actor_user_id").unwrap()).collect()
}

#[tokio::test]
async fn nhan_su_nen_tang_phai_vao_khach_moi_xuat_duoc_va_khach_thay_vet() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = app.tenant("acme").await.unwrap();
        let base = format!("/api/v1/kernel/tenants/{}/exports", acme.id());

        // Chỉ có `kernel.tenants.operate`, chưa vào khách: không xin, không xem được.
        for method in [Method::POST, Method::GET] {
            let (status, body) = call(&app, method, &base, "ops", None).await;
            assert_eq!(
                (status, body["code"].as_str()),
                (StatusCode::FORBIDDEN, Some("kernel.export_needs_tenant_access")),
                "{body}"
            );
        }
        let (status, _) =
            call(&app, Method::POST, "/api/v1/kernel/tenants/acme/exports", "ops", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(actors(&acme, "kernel.tenant_export_requested").await.is_empty());

        // Đã vào khách (ở bản thật: lần cấp quyền vào hỗ trợ): xin được, khách thấy ai xin.
        let (status, export) = call(&app, Method::POST, &base, "ops", Some("acme")).await;
        assert_eq!(status, StatusCode::CREATED, "{export}");
        assert_eq!(
            actors(&acme, "kernel.tenant_export_requested").await,
            [Some(Uuid::from_u128(1))]
        );
        let (status, listed) = call(&app, Method::GET, &base, "ops", Some("acme")).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert_eq!(listed["data"].as_array().unwrap().len(), 1);

        // Tải về và tạo link: khách thấy ai lấy dữ liệu của mình.
        let worker = app.modules().worker(4).unwrap();
        while worker.run_once().await.unwrap() > 0 {}
        let id = export["data"]["id"].as_str().unwrap();
        let content = app
            .get(&format!("{base}/{id}/content"))
            .token("ops")
            .header("autoxide-tenant", "acme")
            .send()
            .await
            .unwrap();
        assert_eq!(content.status(), StatusCode::OK);
        let (status, link) =
            call(&app, Method::POST, &format!("{base}/{id}/link"), "owner", None).await;
        assert_eq!(status, StatusCode::OK, "{link}");
        // Trang trên tên miền của khách khác không tải được bằng link của `acme` (D-20261125-tenant-cors).
        let foreign = app
            .get(link["data"]["url"].as_str().unwrap())
            .header("origin", "https://shop-beta.test")
            .send()
            .await
            .unwrap();
        assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
        assert_eq!(foreign.json()["code"], "kernel.export_link_invalid");
        let opened = app.get(link["data"]["url"].as_str().unwrap()).send().await.unwrap();
        assert_eq!(opened.status(), StatusCode::OK);
        assert_eq!(
            actors(&acme, "kernel.tenant_export_accessed").await,
            [Some(Uuid::from_u128(1)), Some(Uuid::from_u128(2)), None]
        );

        // Quản trị của chính khách vẫn xin được; người của khách khác thì như không có khách đó.
        let (status, _) = call(&app, Method::POST, &base, "owner", None).await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, body) = call(&app, Method::POST, &base, "stranger", None).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("tenant_not_found"))
        );

        // Khách nền tảng: nhân sự của nó xuất được chính nó.
        let (status, body) =
            call(&app, Method::POST, "/api/v1/kernel/tenants/default/exports", "ops", None).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    })
    .await
    .unwrap();
}

/// Sandbox sao cấu hình và sản phẩm của khách: nhân sự nền tảng cũng phải vào khách trước.
#[tokio::test]
async fn nhan_su_nen_tang_khong_tao_sandbox_cua_khach_chua_vao() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = app.tenant("acme").await.unwrap();
        let path = format!("/api/v1/kernel/tenants/{}/sandboxes", acme.id());
        let answer = app
            .request(Method::POST, &path)
            .token("staff")
            .json(serde_json::json!({"name": "Copy"}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            (answer.status(), answer.json()["code"].as_str().map(str::to_owned)),
            (StatusCode::FORBIDDEN, Some("tenancy.sandbox_needs_tenant_access".to_owned()))
        );
    })
    .await
    .unwrap();
}
