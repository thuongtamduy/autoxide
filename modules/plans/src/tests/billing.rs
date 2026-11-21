//! Hồ sơ xuất hoá đơn và các màn của nhân sự nền tảng (bước 35, đợt cho giao diện): kỳ thanh toán
//! theo khách, số khách theo gói, ghi đè của mọi khách, quyền dùng của một khách, tên gói trong thuê
//! bao, `expand=subscription,usage` của danh sách khách. Cả SQLite và Postgres.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::mail::Mailer;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::json;

use super::lifecycle::{Outbox, call, ok, priced, sql, tick};

/// Danh mục hành chính giả: tỉnh 01 có xã 00004.
struct FakeUnits;

#[async_trait::async_trait]
impl autoxide_address::api::AdministrativeUnits for FakeUnits {
    async fn provinces(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Vec<autoxide_address::api::AdministrativeUnit>, autoxide_kernel::error::Error> {
        Ok(Vec::new())
    }

    async fn communes(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> Result<Vec<autoxide_address::api::AdministrativeUnit>, autoxide_kernel::error::Error> {
        Ok(Vec::new())
    }

    async fn validate(
        &self,
        input: autoxide_address::api::AddressInput,
        field: &str,
    ) -> Result<autoxide_address::api::Address, Vec<autoxide_kernel::error::FieldError>> {
        if input.province_code != "01" || input.commune_code != "00004" {
            return Err(vec![autoxide_kernel::error::FieldError::new(
                format!("{field}commune_code"),
                "address.commune_not_in_province",
                "commune `{commune}` is not in province `{province}`",
            )]);
        }
        Ok(serde_json::from_value(json!({
            "country": input.country,
            "province_code": "01",
            "province_name": "Thanh pho Ha Noi",
            "commune_code": "00004",
            "commune_name": "Phuong Ba Dinh",
            "line": input.line,
        }))
        .unwrap())
    }
}

const OPS: [&str; 5] = [
    "plans.plans.read",
    "plans.plans.manage",
    "plans.subscriptions.manage",
    "plans.subscriptions.read",
    "kernel.tenants.read",
];

async fn app(test: &TestDatabase, mails: &Arc<Outbox>) -> TestApp {
    let mailer: Arc<dyn Mailer> = mails.clone();
    TestApp::builder(test)
        .setting("AUTOXIDE_PLANS_CACHE_SECONDS", "0")
        .provide::<dyn Mailer>(mailer)
        .provide::<dyn autoxide_address::api::AdministrativeUnits>(Arc::new(FakeUnits))
        .module(crate::module())
        .tenant("acme")
        .tenant("beta")
        .user("ops", 1, &OPS)
        .user_in("acme", "owner", 2, &["plans.billing.read", "plans.billing.manage"])
        .user_in("beta", "other", 3, &["plans.billing.read", "plans.billing.manage"])
        .user_in("beta", "pretender", 4, &OPS)
        .build()
        .await
        .unwrap()
}

async fn tenant(app: &TestApp, slug: &str) -> uuid::Uuid {
    app.tenant(slug).await.unwrap().id().as_uuid()
}

#[tokio::test]
async fn ho_so_xuat_hoa_don_kiem_mot_lan_va_khong_lo_giua_khach() {
    TestDatabase::run(async |test| {
        let mails = Arc::new(Outbox::default());
        let app = app(test, &mails).await;
        let acme = tenant(&app, "acme").await;
        let (status, body) = call(&app, Method::GET, "/plans/billing-profile", "owner", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["code"], "plans.billing_profile_not_found");

        // Mọi lỗi báo một lần.
        let bad = json!({
            "legal_name": " ",
            "tax_code": "12345",
            "email": "not-an-email",
            "phone": "0".repeat(21),
            "address": {"country": "VN", "province_code": "79", "commune_code": "00004", "line": "1 A"},
        });
        let (status, body) =
            call(&app, Method::PUT, "/plans/billing-profile", "owner", Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        let mut fields: Vec<&str> =
            body["errors"].as_array().unwrap().iter().map(|e| e["field"].as_str().unwrap()).collect();
        fields.sort_unstable();
        assert_eq!(fields, ["address.commune_code", "email", "legal_name", "phone", "tax_code"]);

        let good = json!({
            "legal_name": "Cong ty TNHH Acme",
            "tax_code": "0101234567-001",
            "email": "billing@acme.example",
            "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Trang Tien"},
        });
        let saved = ok(&app, Method::PUT, "/plans/billing-profile", "owner", Some(good)).await;
        assert_eq!(saved["data"]["address"]["commune_name"], "Phuong Ba Dinh");
        assert_eq!(saved["data"]["updated_by"].as_str().map(str::len), Some(36));
        // Đặt lại thay hẳn: bỏ địa chỉ và mã số thuế.
        let again = json!({"legal_name": "Acme", "email": "invoices@acme.example"});
        ok(&app, Method::PUT, "/plans/billing-profile", "owner", Some(again)).await;
        let got = ok(&app, Method::GET, "/plans/billing-profile", "owner", None).await;
        assert_eq!(got["data"]["legal_name"], "Acme");
        assert!(got["data"]["address"].is_null() && got["data"]["tax_code"].is_null());

        // Nhân sự nền tảng đọc được; khách khác không thấy; người của khách thường bị từ chối.
        let path = format!("/plans/tenants/{acme}/billing-profile");
        let staff = ok(&app, Method::GET, &path, "ops", None).await;
        assert_eq!(staff["data"]["email"], "invoices@acme.example");
        let (status, _) = call(&app, Method::GET, "/plans/billing-profile", "other", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, body) = call(&app, Method::GET, &path, "pretender", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], "tenant_platform_only");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mail_nhac_ve_email_cua_ho_so_khi_thue_bao_khong_dat_rieng() {
    TestDatabase::run(async |test| {
        let mails = Arc::new(Outbox::default());
        let app = app(test, &mails).await;
        let acme = tenant(&app, "acme").await;
        let pro = priced(&app, "pro", 199_000, 0).await;
        let body = json!({"plan_id": pro.0, "price_id": pro.1, "trial": false});
        ok(&app, Method::PUT, &format!("/plans/tenants/{acme}/subscription"), "ops", Some(body))
            .await;
        let profile = json!({"legal_name": "Acme", "email": "invoices@acme.example"});
        ok(&app, Method::PUT, "/plans/billing-profile", "owner", Some(profile)).await;
        let soon = Timestamp::now() + SignedDuration::from_hours(12);
        sql(
            &app,
            &format!("UPDATE plans_subscription SET current_period_end = '{}'", soon.to_rfc3339()),
        )
        .await;
        tick(&app).await;
        tick(&app).await;
        let sent = mails.0.lock().unwrap().clone();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0].to, "invoices@acme.example");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhan_su_nen_tang_xem_ky_thanh_toan_so_khach_ghi_de_va_quyen_dung_cua_khach() {
    TestDatabase::run(async |test| {
        let mails = Arc::new(Outbox::default());
        let app = app(test, &mails).await;
        let (acme, beta) = (tenant(&app, "acme").await, tenant(&app, "beta").await);
        let pro = priced(&app, "pro", 199_000, 0).await;
        for id in [acme, beta] {
            let body = json!({"plan_id": pro.0, "price_id": pro.1, "trial": false});
            ok(&app, Method::PUT, &format!("/plans/tenants/{id}/subscription"), "ops", Some(body))
                .await;
        }

        // Kỳ sắp hết: job lập kỳ `renewal` cho cả hai khách.
        let soon = Timestamp::now() + SignedDuration::from_hours(12);
        sql(&app, &format!("UPDATE plans_subscription SET current_period_end = '{}'", soon.to_rfc3339())).await;
        tick(&app).await;

        // Kỳ thanh toán của một khách, cho "đánh dấu đã thu".
        let periods =
            ok(&app, Method::GET, &format!("/plans/tenants/{acme}/billing-periods"), "ops", None)
                .await;
        let list = periods["data"].as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert!(list.iter().all(|period| period["tenant_id"] == json!(acme)));

        // Số khách theo gói (chỉ cho nhà vận hành), tên gói trong thuê bao.
        let plan = ok(&app, Method::GET, &format!("/plans/plans/{}", pro.0), "ops", None).await;
        assert_eq!(plan["data"]["subscriber_count"], 2);
        let catalog = app.get("/api/v1/plans/catalog").send().await.unwrap().json();
        assert!(catalog["data"][0]["subscriber_count"].is_null());
        ok(&app, Method::POST, &format!("/plans/tenants/{beta}/subscription/cancel"), "ops", None)
            .await;
        let page = ok(&app, Method::GET, "/plans/plans", "ops", None).await;
        assert_eq!(page["data"][0]["subscriber_count"], 1);
        ok(&app, Method::POST, &format!("/plans/plans/{}/retire", pro.0), "ops", None).await;
        let own = ok(&app, Method::GET, "/plans/subscription", "owner", None).await;
        assert_eq!((own["data"]["plan_code"].as_str(), own["data"]["plan_name"].as_str()), (Some("pro"), Some("pro")));
        let one = ok(&app, Method::GET, &format!("/plans/subscriptions?tenant_id={acme}"), "ops", None).await;
        assert_eq!(one["data"].as_array().unwrap().len(), 1);
        assert_eq!(one["data"][0]["plan_name"], "pro");

        // Ghi đè của mọi khách, lọc theo khách và theo hiệu lực.
        let past = (Timestamp::now() - SignedDuration::from_hours(24)).to_rfc3339();
        let overrides = [
            (acme, json!({"kind": "limit", "key": "kernel.api_requests", "max_value": 5, "reason": "test"})),
            (beta, json!({"kind": "limit", "key": "kernel.api_requests", "max_value": 9, "reason": "old", "expires_at": past})),
        ];
        for (id, body) in overrides {
            let path = format!("/plans/tenants/{id}/overrides");
            let (status, body) = call(&app, Method::POST, &path, "ops", Some(body)).await;
            assert_eq!(status, StatusCode::CREATED, "{body}");
        }
        let all = ok(&app, Method::GET, "/plans/overrides", "ops", None).await;
        assert_eq!(all["data"].as_array().unwrap().len(), 2);
        let mine = ok(&app, Method::GET, &format!("/plans/overrides?tenant_id={acme}"), "ops", None).await;
        assert_eq!(mine["data"].as_array().unwrap().len(), 1);
        assert_eq!(mine["data"][0]["tenant_id"], json!(acme));
        let expired = ok(&app, Method::GET, "/plans/overrides?active=false", "ops", None).await;
        assert_eq!(expired["data"][0]["tenant_id"], json!(beta));
        let (status, _) = call(&app, Method::GET, "/plans/overrides", "pretender", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Quyền dùng của một khách bất kỳ; khách không có: 404.
        let rights =
            ok(&app, Method::GET, &format!("/plans/tenants/{acme}/entitlements"), "ops", None).await;
        let api = rights["data"]["limits"]
            .as_array()
            .unwrap()
            .iter()
            .find(|limit| limit["key"] == "kernel.api_requests")
            .unwrap()
            .clone();
        assert_eq!(api["max"], 5);
        let unknown = uuid::Uuid::now_v7();
        let (status, body) =
            call(&app, Method::GET, &format!("/plans/tenants/{unknown}/entitlements"), "ops", None).await;
        assert_eq!((status, body["code"].as_str()), (StatusCode::NOT_FOUND, Some("tenant_not_found")));

        // Danh sách khách của kernel: `expand=subscription,usage` có dữ liệu thật.
        let tenants = ok(&app, Method::GET, "/kernel/tenants?expand=subscription,usage&limit=50", "ops", None).await;
        let row = tenants["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == json!(acme))
            .unwrap()
            .clone();
        assert_eq!(row["expanded"]["subscription"]["plan_code"], "pro");
        assert_eq!(row["expanded"]["subscription"]["status"], "active");
        assert!(row["expanded"]["usage"].as_array().unwrap().iter().any(|u| u["key"] == "kernel.api_requests"));
        assert!(row["expanded"]["placement"].is_null());
    })
    .await
    .unwrap();
}
