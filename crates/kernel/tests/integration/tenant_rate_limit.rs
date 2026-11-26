//! Giới hạn request theo khách (D-20261126-tenant-rate-limit): khách A gửi quá mức thì 429 `tenant_rate_limited` kèm
//! `Retry-After`; khách B không bị khách A làm nghẽn. Đếm trong bộ nhớ và đếm chung qua database.

use autoxide_testkit::{TestApp, TestDatabase, tool};
use axum::http::StatusCode;
use serde_json::json;

async fn app(test: &TestDatabase, store: &str) -> TestApp {
    TestApp::builder(test)
        .setting("AUTOXIDE_HTTP_RATE_LIMIT_TENANT_PER_MINUTE", "2")
        .setting("AUTOXIDE_HTTP_RATE_LIMIT_TENANT_BURST", "2")
        .setting("AUTOXIDE_HTTP_RATE_LIMIT_STORE", store)
        // Giới hạn theo địa chỉ không là thứ được đo ở đây.
        .setting("AUTOXIDE_HTTP_RATE_LIMIT_PER_MINUTE", "0")
        .tenant("a")
        .tenant("b")
        .user_in("a", "owner-a", 1, &["kernel.settings.manage"])
        .user_in("b", "owner-b", 2, &["kernel.settings.manage"])
        .user_in("a", "nobody-a", 3, &[])
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

/// Lớp giới hạn theo khách nằm ngoài lớp permission (D-20261126-permission-gate) và chỉ có MỘT lớp: request bị 403
/// vẫn trừ ngân sách của khách, mỗi request đúng một lần — mức 2 thì hai request bị 403, request thứ ba 429.
#[tokio::test]
async fn request_bi_tu_choi_tru_ngan_sach_mot_lan() {
    for store in ["memory", "database"] {
        TestDatabase::run(async |test| {
            let app = app(test, store).await;
            let get = || app.get("/api/v1/kernel/settings").token("nobody-a").send();
            for _ in 0..2 {
                let refused = get().await.unwrap();
                assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{store}");
                assert_eq!(refused.json()["code"], "permission_denied", "{store}");
            }
            let limited = get().await.unwrap();
            assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS, "{store}");
            assert_eq!(limited.json()["code"], "tenant_rate_limited", "{store}");
        })
        .await
        .unwrap();
    }
}

/// Request không đăng nhập (hay mang token sai) nêu tên khách `a` bằng `Autoxide-Tenant` không trừ vào hạn mức của
/// `a`: người ngoài không làm cạn được hạn mức của khách; route công khai (và webhook gửi đến, cùng là request không
/// có thông tin xác thực của Autoxide) không bao giờ bị 429 vì khách ở trần.
#[tokio::test]
async fn an_danh_khong_lam_can_han_muc_cua_khach() {
    for store in ["memory", "database"] {
        TestDatabase::run(async |test| {
            let app = app(test, store).await;
            for _ in 0..5 {
                let public = app
                    .get("/api/v1/kernel/client-config")
                    .header("autoxide-tenant", "a")
                    .send()
                    .await
                    .unwrap();
                assert_eq!(public.status(), StatusCode::OK, "{store}");
                let forged = app
                    .get("/api/v1/kernel/settings")
                    .header("autoxide-tenant", "a")
                    .header("authorization", "Bearer khong-hop-le")
                    .send()
                    .await
                    .unwrap();
                assert_eq!(forged.status(), StatusCode::UNAUTHORIZED, "{store}");
            }
            let get = || app.get("/api/v1/kernel/settings").token("owner-a").send();
            assert_eq!(get().await.unwrap().status(), StatusCode::OK, "{store}");
            assert_eq!(get().await.unwrap().status(), StatusCode::OK, "{store}");
            // Khách đã ở trần: route công khai vẫn trả lời.
            assert_eq!(get().await.unwrap().status(), StatusCode::TOO_MANY_REQUESTS, "{store}");
            let public = app
                .get("/api/v1/kernel/client-config")
                .header("autoxide-tenant", "a")
                .send()
                .await
                .unwrap();
            assert_eq!(public.status(), StatusCode::OK, "{store}");
        })
        .await
        .unwrap();
    }
}

/// Lời gọi tool MCP đi qua cùng bộ đếm với REST: hết lượt thì tool trả `tenant_rate_limited`.
#[tokio::test]
async fn tool_mcp_tinh_chung_voi_rest() {
    TestDatabase::run(async |test| {
        let app = app(test, "memory").await;
        let rest = app.get("/api/v1/kernel/settings").token("owner-a").send().await.unwrap();
        assert_eq!(rest.status(), StatusCode::OK);
        let mcp = app.mcp("owner-a").await.unwrap();
        let first = mcp.call_tool(tool("kernel.navigation", json!({}))).await.unwrap();
        assert_eq!(first.is_error, Some(false), "{first:?}");
        let second = mcp.call_tool(tool("kernel.navigation", json!({}))).await.unwrap();
        assert_eq!(second.is_error, Some(true), "{second:?}");
        assert_eq!(second.structured_content.unwrap()["code"], "tenant_rate_limited");
        // REST của khách cũng hết lượt; khách `b` không sao.
        let rest = app.get("/api/v1/kernel/settings").token("owner-a").send().await.unwrap();
        assert_eq!(rest.status(), StatusCode::TOO_MANY_REQUESTS);
        let other = app.mcp("owner-b").await.unwrap();
        let call = other.call_tool(tool("kernel.navigation", json!({}))).await.unwrap();
        assert_eq!(call.is_error, Some(false), "{call:?}");
    })
    .await
    .unwrap();
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
