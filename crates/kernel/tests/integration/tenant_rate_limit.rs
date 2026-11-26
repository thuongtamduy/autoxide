//! Giới hạn request theo khách (D-20261126-tenant-rate-limit): khách A gửi quá mức thì 429 `tenant_rate_limited` kèm
//! `Retry-After`; khách B không bị khách A làm nghẽn. Đếm trong bộ nhớ và đếm chung qua database.

use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;

async fn app(test: &TestDatabase, store: &str) -> TestApp {
    TestApp::builder(test)
        .setting("AUTOXIDE_HTTP_RATE_LIMIT_TENANT_PER_MINUTE", "2")
        .setting("AUTOXIDE_HTTP_RATE_LIMIT_TENANT_BURST", "2")
        .setting("AUTOXIDE_HTTP_RATE_LIMIT_STORE", store)
        .tenant("a")
        .tenant("b")
        .user_in("a", "owner-a", 1, &["kernel.settings.manage"])
        .user_in("b", "owner-b", 2, &["kernel.settings.manage"])
        .build()
        .await
        .unwrap()
}

#[tokio::test]
async fn moi_khach_mot_muc_rieng() {
    for store in ["memory", "database"] {
        TestDatabase::run(async |test| {
            let app = app(test, store).await;
            let get = |token: &'static str| app.get("/api/v1/kernel/settings").token(token).send();
            assert_eq!(get("owner-a").await.unwrap().status(), StatusCode::OK, "{store}");
            assert_eq!(get("owner-a").await.unwrap().status(), StatusCode::OK, "{store}");
            let limited = get("owner-a").await.unwrap();
            assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS, "{store}");
            assert_eq!(limited.json()["code"], "tenant_rate_limited", "{store}");
            assert!(!limited.header("retry-after").is_empty(), "{store}");
            // Khách B vẫn chạy.
            assert_eq!(get("owner-b").await.unwrap().status(), StatusCode::OK, "{store}");
        })
        .await
        .unwrap();
    }
}

/// Gói giả: khách `limited` có `kernel.api_rate_per_minute` = 1; khách khác có gói "không giới hạn" mức này.
struct Rate {
    limited: std::sync::Mutex<Option<autoxide_kernel::tenancy::TenantId>>,
}

#[async_trait::async_trait]
impl autoxide_kernel::entitlement::Entitlements for Rate {
    async fn snapshot(
        &self,
        tenant: autoxide_kernel::tenancy::TenantId,
    ) -> Result<
        std::sync::Arc<autoxide_kernel::entitlement::TenantEntitlements>,
        autoxide_kernel::error::Error,
    > {
        let level = (*self.limited.lock().unwrap() == Some(tenant)).then_some(1);
        Ok(std::sync::Arc::new(autoxide_kernel::entitlement::TenantEntitlements {
            limits: [("kernel.api_rate_per_minute".to_owned(), level)].into(),
            ..Default::default()
        }))
    }

    async fn reserve(
        &self,
        _: autoxide_kernel::tenancy::TenantId,
        _: &autoxide_kernel::entitlement::Limit,
        _: i64,
        _: &str,
    ) -> Result<(), autoxide_kernel::error::Error> {
        Ok(())
    }

    async fn confirm(
        &self,
        _: autoxide_kernel::tenancy::TenantId,
        _: &autoxide_kernel::entitlement::Limit,
        _: &str,
    ) -> Result<(), autoxide_kernel::error::Error> {
        Ok(())
    }

    async fn release(
        &self,
        _: autoxide_kernel::tenancy::TenantId,
        _: &autoxide_kernel::entitlement::Limit,
        _: &str,
    ) -> Result<(), autoxide_kernel::error::Error> {
        Ok(())
    }

    async fn record_usage(
        &self,
        _: autoxide_kernel::tenancy::TenantId,
        _: &autoxide_kernel::entitlement::Limit,
        _: i64,
        _: &str,
    ) -> Result<(), autoxide_kernel::error::Error> {
        Ok(())
    }
}

/// Mức theo gói thay mức của máy chủ (ở đây máy chủ tắt): khách `a` 1 request/phút, khách `b` không giới hạn.
#[tokio::test]
async fn muc_theo_goi() {
    TestDatabase::run(async |test| {
        let plan = std::sync::Arc::new(Rate { limited: std::sync::Mutex::new(None) });
        let provider: std::sync::Arc<dyn autoxide_kernel::entitlement::Entitlements> = plan.clone();
        let app = TestApp::builder(test)
            .tenant("a")
            .tenant("b")
            .user_in("a", "owner-a", 1, &["kernel.settings.manage"])
            .user_in("b", "owner-b", 2, &["kernel.settings.manage"])
            .provide::<dyn autoxide_kernel::entitlement::Entitlements>(provider)
            .build()
            .await
            .unwrap();
        *plan.limited.lock().unwrap() = Some(app.tenant("a").await.unwrap().id());
        let get = |token: &'static str| app.get("/api/v1/kernel/settings").token(token).send();
        assert_eq!(get("owner-a").await.unwrap().status(), StatusCode::OK);
        assert_eq!(get("owner-a").await.unwrap().status(), StatusCode::TOO_MANY_REQUESTS);
        for _ in 0..5 {
            assert_eq!(get("owner-b").await.unwrap().status(), StatusCode::OK);
        }
    })
    .await
    .unwrap();
}
