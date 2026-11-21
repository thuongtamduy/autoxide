//! Khách thử nghiệm (sandbox, bước 38, D-232), phần nền: tạo (cùng placement với khách gốc), job sao
//! cấu hình kích hoạt, nhân viên vào bằng uỷ quyền, header `Autoxide-Sandbox` + `noindex`, hạn mức
//! (gói không đặt thì 1), không tạo sandbox từ sandbox, gia hạn, xoá, hết hạn tự xoá. Mỗi bài chạy với
//! khách gốc ở database chung và ở database riêng, trên cả SQLite và Postgres.

use autoxide_auth::api::{Accounts, NewAccount};
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use jiff::SignedDuration;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::ExprTrait as _;
use serde_json::{Value, json};
use std::time::Duration;

const PASSWORD: &str = "correct horse battery staple";

async fn app(test: &TestDatabase, dedicated: bool) -> TestApp {
    let dir = std::env::temp_dir().join(format!("autoxide-sandbox-{}", uuid::Uuid::now_v7()));
    let builder = TestApp::builder(test)
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string());
    let builder = if dedicated { builder.tenant_dedicated("shop") } else { builder.tenant("shop") };
    builder
        .tenant("other")
        .module(autoxide_mail::module())
        .module(autoxide_auth::module())
        .build()
        .await
        .unwrap()
}

async fn call(
    app: &TestApp,
    method: Method,
    path: &str,
    token: &str,
    tenant: &str,
    body: Option<Value>,
) -> (StatusCode, Value, axum::http::HeaderMap) {
    let mut request =
        app.request(method, &format!("/api/v1{path}")).header("autoxide-tenant", tenant);
    if !token.is_empty() {
        request = request.token(token);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    let body = if response.body().is_empty() { Value::Null } else { response.json() };
    (status, body, response.headers().clone())
}

/// Chủ khách `slug` (role `admin`), đã đăng nhập; trả access token.
async fn owner(app: &TestApp, slug: &str) -> String {
    let tenant = app.tenant(slug).await.unwrap();
    let email = format!("owner@{slug}.example");
    let account = NewAccount {
        email: email.clone(),
        name: "Owner".to_owned(),
        password: Some(PASSWORD.to_owned()),
        admin: true,
    };
    app.api::<dyn Accounts>().unwrap().create(&tenant, account).await.unwrap();
    let body = json!({"email": email, "password": PASSWORD});
    let (status, answer, _) = call(app, Method::POST, "/auth/login", "", slug, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    answer["data"]["access_token"].as_str().unwrap().to_owned()
}

/// Chạy job và giao event cho tới khi hết việc.
async fn settle(app: &TestApp) {
    let worker = app.modules().worker(8).unwrap().blocked_delay(Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..5 {
        let ran = worker.run_once().await.unwrap();
        let delivered = dispatcher.run_once().await.unwrap();
        if ran == 0 && delivered == 0 {
            break;
        }
    }
}

/// Tạo sandbox của `shop`, chạy job; trả ID.
async fn create(app: &TestApp, token: &str, shop: &TenantContext) -> String {
    let path = format!("/kernel/tenants/{}/sandboxes", shop.id());
    let body = json!({"name": "Try the new theme"});
    let (status, created, headers) =
        call(app, Method::POST, &path, token, "shop", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert!(headers.get("location").is_some());
    assert_eq!(created["data"]["status"], "provisioning");
    assert!(created["data"]["job_id"].is_string(), "{created}");
    let id = created["data"]["id"].as_str().unwrap().to_owned();
    settle(app).await;
    id
}

async fn sandbox_status(app: &TestApp, id: &str) -> String {
    let read = app.registry().control().read();
    let row = read
        .query_one_raw(autoxide_kernel::database::statement(
            read.get_database_backend(),
            "SELECT status FROM kernel_tenants WHERE id = $1",
            [id.parse::<uuid::Uuid>().unwrap().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get("", "status").unwrap()
}

async fn lifecycle(dedicated: bool) {
    TestDatabase::run(async |test| {
        let app = app(test, dedicated).await;
        let shop = app.tenant("shop").await.unwrap();
        let token = owner(&app, "shop").await;
        let id = create(&app, &token, &shop).await;
        assert_eq!(sandbox_status(&app, &id).await, "active");
        let sandbox =
            app.registry().tenant(TenantId::from_uuid(id.parse().unwrap())).await.unwrap();
        assert!(sandbox.is_sandbox());
        assert_eq!(sandbox.sandbox_of(), Some(shop.id()));
        assert_eq!(sandbox.placement(), shop.placement(), "cùng placement với khách gốc");
        assert!(
            app.registry().tenant_descendants(shop.id()).await.unwrap().is_empty(),
            "sandbox không phải khách con"
        );

        // Chủ khách vào sandbox bằng uỷ quyền (header `Autoxide-Tenant`), response mang hai header.
        let (status, current, headers) =
            call(&app, Method::GET, "/kernel/tenant", &token, &id, None).await;
        assert_eq!(status, StatusCode::OK, "{current}");
        assert_eq!(current["data"]["kind"], "sandbox");
        assert_eq!(current["data"]["sandbox_of"], shop.id().to_string());
        assert!(current["data"]["sandbox_expires_at"].is_string());
        assert_eq!(headers.get("autoxide-sandbox").unwrap(), "true");
        assert_eq!(headers.get("x-robots-tag").unwrap(), "noindex, nofollow");
        // moi_response_sandbox_co_header_va_noindex: cả lỗi cũng mang header.
        let (status, _, headers) =
            call(&app, Method::GET, "/auth/users/not-a-uuid", &token, &id, None).await;
        assert!(status.is_client_error());
        assert_eq!(headers.get("autoxide-sandbox").unwrap(), "true");
        // Khách thật không mang header.
        let (_, current, headers) =
            call(&app, Method::GET, "/kernel/tenant", &token, "shop", None).await;
        assert_eq!(current["data"]["kind"], "production");
        assert!(headers.get("autoxide-sandbox").is_none());
        assert!(headers.get("x-robots-tag").is_none());

        // tao_sandbox_tu_sandbox_bi_tu_choi.
        let path = format!("/kernel/tenants/{id}/sandboxes");
        let (status, refused, _) =
            call(&app, Method::POST, &path, &token, &id, Some(json!({"name": "Nested"}))).await;
        assert_eq!(
            (status, refused["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("tenancy.sandbox_nested")),
            "{refused}"
        );
        // vuot_han_muc_sandbox_tra_limit_exceeded: không thuê bao, gói không đặt → 1.
        let path = format!("/kernel/tenants/{}/sandboxes", shop.id());
        let (status, refused, _) =
            call(&app, Method::POST, &path, &token, "shop", Some(json!({"name": "Second"}))).await;
        assert_eq!(
            (status, refused["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("limit_exceeded"))
        );
        let (_, listed, _) = call(&app, Method::GET, &path, &token, "shop", None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 1, "{listed}");

        // Khách khác không thấy, không gia hạn được sandbox này.
        let stranger = owner(&app, "other").await;
        let (status, hidden, _) = call(
            &app,
            Method::POST,
            &format!("/kernel/sandboxes/{id}/extend"),
            &stranger,
            "other",
            None,
        )
        .await;
        assert_eq!(
            (status, hidden["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("tenancy.sandbox_not_found"))
        );
        let (status, _, _) = call(&app, Method::GET, "/kernel/tenant", &stranger, &id, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "người của khách khác không vào được");

        // Gia hạn.
        let before = current_expiry(&app, &token, &id).await;
        let (status, extended, _) = call(
            &app,
            Method::POST,
            &format!("/kernel/sandboxes/{id}/extend"),
            &token,
            "shop",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{extended}");
        assert!(extended["data"]["expires_at"].as_str().unwrap() >= before.as_str());

        // Xoá: sandbox bị xoá, uỷ quyền bị thu hồi.
        let (status, _, _) =
            call(&app, Method::DELETE, &format!("/kernel/sandboxes/{id}"), &token, "shop", None)
                .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        settle(&app).await;
        assert_eq!(sandbox_status(&app, &id).await, "deleted");
        let (status, _, _) = call(&app, Method::GET, "/kernel/tenant", &token, &id, None).await;
        assert!(status.is_client_error());

        // sandbox_het_han_bi_xoa: hạn đã qua → lịch mỗi giờ xoá.
        let again = create(&app, &token, &shop).await;
        let write = app.registry().control().write();
        // Builder thay câu viết tay: giá trị thời gian cần kiểu `timestamptz` trên Postgres, builder tự thêm.
        let expired = sea_orm::sea_query::Query::update()
            .table(sea_orm::sea_query::Alias::new("kernel_tenants"))
            .value(
                sea_orm::sea_query::Alias::new("sandbox_expires_at"),
                sea_orm::Value::from(Timestamp::now() - SignedDuration::from_hours(1)),
            )
            .and_where(
                sea_orm::sea_query::Expr::col(sea_orm::sea_query::Alias::new("id"))
                    .eq(again.parse::<uuid::Uuid>().unwrap()),
            )
            .to_owned();
        write.execute(&expired).await.unwrap();
        app.registry().invalidate(TenantId::from_uuid(again.parse().unwrap()));
        // Lịch mỗi giờ đã chạy ở lượt đầu: gọi thẳng bộ chạy của nó.
        let sandboxes = app.api::<autoxide_kernel::tenancy::TenantSandboxes>().unwrap();
        assert_eq!(sandboxes.expire_due().await.unwrap(), 1);
        assert_eq!(sandbox_status(&app, &again).await, "deleted");
    })
    .await
    .unwrap();
}

async fn current_expiry(app: &TestApp, token: &str, id: &str) -> String {
    let (_, current, _) = call(app, Method::GET, "/kernel/tenant", token, id, None).await;
    current["data"]["sandbox_expires_at"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn sandbox_vong_doi_database_chung() {
    lifecycle(false).await;
}

#[tokio::test]
async fn sandbox_vong_doi_database_rieng() {
    lifecycle(true).await;
}
