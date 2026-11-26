//! Tự đăng ký tài khoản nhân sự theo khách (`auth.staff_registration`, mục 2 của đợt cấu hình theo khách): khách
//! tự mở hay đóng, chưa đặt thì theo biến chung `AUTOXIDE_AUTH_SELF_REGISTRATION`; khách A mở không làm khách B mở.
//! Chạy trên cả SQLite và Postgres.

use std::collections::HashMap;
use std::sync::Mutex;

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, TenantContext, TenantId};
use autoxide_organizations::api::{EffectiveSetting, OrgDirectory, Unit};
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::http::test_support::{PASSWORD, app_built, call_with};

/// `OrgDirectory` giả: cài đặt ở nút gốc theo từng khách; cặp (khách, khoá) không có trong bảng thì chưa đặt.
#[derive(Default)]
pub(crate) struct OrgValues(Mutex<HashMap<(TenantId, String), Value>>);

impl OrgValues {
    pub(crate) fn set(&self, tenant: TenantId, key: &str, value: Value) {
        self.0.lock().unwrap().insert((tenant, key.to_owned()), value);
    }
}

#[async_trait::async_trait]
impl OrgDirectory for OrgValues {
    async fn unit(&self, _: &TenantContext, _: OrgId) -> Result<Option<Unit>, Error> {
        Ok(None)
    }
    async fn root(&self, _: &TenantContext) -> Result<Unit, Error> {
        Err(Error::internal("not used"))
    }
    async fn ancestors(&self, _: &TenantContext, _: OrgId) -> Result<Vec<Unit>, Error> {
        Ok(Vec::new())
    }
    async fn is_within(&self, _: &TenantContext, _: OrgId, _: OrgId) -> Result<bool, Error> {
        Ok(false)
    }
    async fn setting(
        &self,
        tenant: &TenantContext,
        id: OrgId,
        key: &str,
    ) -> Result<Option<EffectiveSetting>, Error> {
        let value = self.0.lock().unwrap().get(&(tenant.id(), key.to_owned())).cloned();
        Ok(Some(EffectiveSetting::new(key, value, Some(id), false)))
    }
}

async fn register(
    app: &autoxide_testkit::TestApp,
    tenant: &str,
    email: &str,
) -> (StatusCode, Value) {
    let body = json!({"email": email, "name": "Bob", "password": PASSWORD});
    let headers = [("autoxide-tenant", tenant)];
    let (status, _, body) = call_with(app, "POST", "/register", &headers, Some(body)).await;
    (status, body)
}

/// Máy chủ tắt tự đăng ký: khách `a` tự mở thì người lạ tạo được tài khoản ở `a`, khách `b` (chưa đặt) vẫn đóng.
#[tokio::test]
async fn khach_tu_mo_dang_ky_khong_mo_khach_khac() {
    TestDatabase::run(async |test| {
        let orgs = std::sync::Arc::new(OrgValues::default());
        let provided: std::sync::Arc<dyn OrgDirectory> = orgs.clone();
        let (app, _) = app_built(test, false, |builder| {
            builder.tenant("a").tenant("b").provide::<dyn OrgDirectory>(provided)
        })
        .await;
        orgs.set(app.tenant("a").await.unwrap().id(), "auth.staff_registration", json!(true));
        let (status, body) = register(&app, "a", "bob@example.com").await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let (status, body) = register(&app, "b", "bob@example.com").await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("auth.registration_disabled"))
        );
    })
    .await
    .unwrap();
}

/// Máy chủ bật tự đăng ký: khách `a` tự đóng được; khách `b` chưa đặt thì theo máy chủ (mở).
#[tokio::test]
async fn khach_tu_dong_dang_ky_khi_may_chu_mo() {
    TestDatabase::run(async |test| {
        let orgs = std::sync::Arc::new(OrgValues::default());
        let provided: std::sync::Arc<dyn OrgDirectory> = orgs.clone();
        let (app, _) = app_built(test, true, |builder| {
            builder.tenant("a").tenant("b").provide::<dyn OrgDirectory>(provided)
        })
        .await;
        orgs.set(app.tenant("a").await.unwrap().id(), "auth.staff_registration", json!(false));
        let (status, body) = register(&app, "a", "bob@example.com").await;
        assert_eq!(body["code"], "auth.registration_disabled", "{status}");
        let (status, body) = register(&app, "b", "bob@example.com").await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    })
    .await
    .unwrap();
}
