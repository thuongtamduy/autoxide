//! Chính sách đăng nhập theo khách (D-20261126-tenant-login-policy): độ dài mật khẩu tối thiểu, thời hạn phiên, tên
//! trong app xác thực. Khách chỉ siết hơn máy chủ; khách khác không bị ảnh hưởng. Chạy trên cả SQLite và Postgres.

use std::sync::Arc;

use autoxide_kernel::time::Timestamp;
use autoxide_organizations::api::OrgDirectory;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::http::test_support::{PASSWORD, app_built, call, call_with};
use crate::registration_tests::OrgValues;

/// Mật khẩu 14 ký tự: đủ mức của máy chủ (12), thiếu mức 16 của khách.
const FOURTEEN: &str = "fourteen chars";
/// Khách của `ada` (khách mặc định), nêu bằng header như mọi request chưa đăng nhập khi có nhiều khách.
const HOME: [(&str, &str); 1] = [("autoxide-tenant", "default")];

async fn login(app: &autoxide_testkit::TestApp) -> Value {
    let body = json!({"email": "ada@example.com", "password": PASSWORD});
    let (status, _, login) = call_with(app, "POST", "/login", &HOME, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{login}");
    login["data"].clone()
}

fn expires_in_days(tokens: &Value) -> i64 {
    let at: Timestamp = tokens["refresh_token_expires_at"].as_str().unwrap().parse().unwrap();
    (at - Timestamp::now()).as_secs().div_euclid(86_400)
}

#[tokio::test]
async fn chinh_sach_cua_khach_chi_ap_cho_khach_do() {
    TestDatabase::run(async |test| {
        let orgs = Arc::new(OrgValues::default());
        let provided: Arc<dyn OrgDirectory> = orgs.clone();
        let (app, services) = app_built(test, true, |builder| {
            builder.tenant("b").provide::<dyn OrgDirectory>(provided)
        })
        .await;
        let home = services.tenant.id();
        orgs.set(home, "auth.password_min_length", json!(16));
        orgs.set(home, "auth.session_days", json!(1));
        orgs.set(home, "auth.totp_issuer", json!("Shop A"));

        // Mật khẩu: khách siết lên 16; khách `b` vẫn theo máy chủ (12).
        let body = json!({"email": "bob@example.com", "name": "Bob", "password": FOURTEEN});
        let (status, _, refused) = call_with(&app, "POST", "/register", &HOME, Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["errors"][0]["field"], "password", "{refused}");
        assert!(refused["errors"][0].to_string().contains("16"), "{refused}");
        let other = json!({"email": "bob@example.com", "name": "Bob", "password": FOURTEEN});
        let headers = [("autoxide-tenant", "b")];
        let (status, _, created) =
            call_with(&app, "POST", "/register", &headers, Some(other)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");

        // Phiên: 1 ngày thay vì 30 của máy chủ; làm mới cũng theo mức của khách.
        let tokens = login(&app).await;
        assert_eq!(expires_in_days(&tokens), 0, "{tokens}");
        let body = json!({"refresh_token": tokens["refresh_token"]});
        let (status, _, refreshed) = call_with(&app, "POST", "/refresh", &HOME, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{refreshed}");
        assert_eq!(expires_in_days(&refreshed["data"]), 0, "{refreshed}");

        // Tên trong app xác thực.
        let bearer = format!("Bearer {}", tokens["access_token"].as_str().unwrap());
        let body = json!({"current_password": PASSWORD});
        let (status, _, setup) =
            call(&app, "POST", "/me/mfa/totp", Some(&bearer), Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{setup}");
        let url = setup["data"]["otpauth_url"].as_str().unwrap();
        assert!(url.contains("issuer=Shop%20A"), "{url}");

        // Khách đặt lỏng hơn máy chủ (90 ngày > 30): bị bỏ qua, vẫn 30 ngày.
        orgs.set(home, "auth.session_days", json!(90));
        let days = expires_in_days(&login(&app).await);
        assert!((29..=30).contains(&days), "{days}");
    })
    .await
    .unwrap();
}

/// `auth.session_days` lưu bằng đường khác (chuyển dữ liệu) vượt 365 hay rất lớn: kẹp, không tràn phép tính, không
/// dài hơn máy chủ.
#[tokio::test]
async fn session_days_luu_ngoai_khoang_bi_kep() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let orgs = Arc::new(OrgValues::default());
        let provided: Arc<dyn OrgDirectory> = orgs.clone();
        let policy = crate::policy::TenantPolicy::new(Some(provided));
        let tenant = &services.tenant;
        let server = jiff::SignedDuration::from_hours(400 * 24);
        for (stored, days) in [(json!(400), 365), (json!(i64::MAX), 365), (json!(0), 400)] {
            orgs.set(tenant.id(), "auth.session_days", stored.clone());
            let ttl = policy.session_ttl(tenant, server).await.unwrap();
            assert_eq!(ttl, jiff::SignedDuration::from_hours(days * 24), "{stored}");
        }
    })
    .await
    .unwrap();
}
