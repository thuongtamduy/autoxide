//! Link trong thư theo từng khách (D-20261125-tenant-link-templates): khách đặt trang riêng có đường dẫn tuỳ
//! ý, chỉ trên host khách sở hữu; khách A đặt không ảnh hưởng khách B; khách B không mượn được tên miền của A.

use autoxide_kernel::module::Module;
use autoxide_kernel::tenancy::{SiteLinks, TenantSetting};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};

const PAGE: TenantSetting = TenantSetting::link(
    "linker.link.page",
    "Page of this tenant's site, with `{token}`. When unset, the server's AUTOXIDE_LINKER_URL is used.",
    &["{token}"],
);

/// Mẫu link của máy chủ (vai biến `AUTOXIDE_…_URL`).
const SERVER: &str = "https://www.example.com/confirm?token={token}";

struct Linker;

impl Module for Linker {
    fn name(&self) -> &'static str {
        "linker"
    }

    fn tenant_settings(&self) -> Vec<TenantSetting> {
        vec![PAGE]
    }
}

async fn app(test: &TestDatabase) -> TestApp {
    TestApp::builder(test)
        .tenant("a")
        .tenant("b")
        .domain("a", "shop-a.test")
        .user_in("a", "owner-a", 1, &["kernel.settings.manage"])
        .user_in("b", "owner-b", 2, &["kernel.settings.manage"])
        .module(Linker)
        .build()
        .await
        .unwrap()
}

/// Đặt `name` = `value` cho khách của `token`; trả mã HTTP và thân.
async fn put(app: &TestApp, token: &str, name: &str, value: &str) -> (StatusCode, Value) {
    let path = format!("/api/v1/kernel/settings/{name}");
    let response = app.put(&path).token(token).json(json!({"value": value})).send().await.unwrap();
    (response.status(), response.json())
}

fn codes(body: &Value) -> Vec<String> {
    body["errors"]
        .as_array()
        .map(|errors| errors.iter().map(|e| e["code"].as_str().unwrap_or("").to_owned()).collect())
        .unwrap_or_default()
}

#[tokio::test]
async fn link_rieng_chi_tren_host_cua_khach() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let name = PAGE.name();
        for (value, code) in [
            ("https://evil.test/reset?t={token}", "kernel.link_host_not_allowed"),
            ("https://shop-a.test/reset", "kernel.link_placeholder_missing"),
            ("http://shop-a.test/reset?t={token}", "kernel.link_template_invalid"),
            ("https://shop-a.test@evil.test/{token}", "kernel.link_template_invalid"),
            ("https://{token}.shop-a.test/", "kernel.link_template_invalid"),
            ("https://shop-a.test.evil.test/{token}", "kernel.link_host_not_allowed"),
        ] {
            let (status, body) = put(&app, "owner-a", name, value).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{value}: {body}");
            assert_eq!(codes(&body), [code], "{value}: {body}");
        }
        // Tên miền đã xác minh của chính khách: nhận, đường dẫn tuỳ ý.
        let own = "https://shop-a.test/dat-lai-mat-khau?t={token}";
        let (status, body) = put(&app, "owner-a", name, own).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        // Khách B không mượn được tên miền của A (mạo danh / lấy token của người dùng B qua trang của A).
        let (status, body) = put(&app, "owner-b", name, own).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(codes(&body), ["kernel.link_host_not_allowed"]);

        let links = app.api::<SiteLinks>().unwrap();
        let a = app.tenant("a").await.unwrap();
        let b = app.tenant("b").await.unwrap();
        assert_eq!(links.link(&a, &PAGE, Some(SERVER)).await.unwrap().as_deref(), Some(own));
        // B chưa đặt gì: link của máy chủ như cũ.
        assert_eq!(links.link(&b, &PAGE, Some(SERVER)).await.unwrap().as_deref(), Some(SERVER));
        // Máy chủ không đặt biến: A vẫn có link của mình, B không có link (tính năng tắt với B).
        assert_eq!(links.link(&a, &PAGE, None).await.unwrap().as_deref(), Some(own));
        assert_eq!(links.link(&b, &PAGE, None).await.unwrap(), None);

        // B đặt `kernel.public_url` rồi trỏ link về host đó: nhận; link máy chủ đổi host theo B.
        let (status, body) = put(&app, "owner-b", "kernel.public_url", "https://b.example").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            links.link(&b, &PAGE, Some(SERVER)).await.unwrap().as_deref(),
            Some("https://b.example/confirm?token={token}")
        );
        let page_b = "https://b.example/xac-nhan/{token}";
        let (status, body) = put(&app, "owner-b", name, page_b).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(links.link(&b, &PAGE, Some(SERVER)).await.unwrap().as_deref(), Some(page_b));
        assert_eq!(links.link(&a, &PAGE, Some(SERVER)).await.unwrap().as_deref(), Some(own));

        // Host của link máy chủ luôn được tin (người vận hành đặt).
        let (status, body) = put(&app, "owner-b", "kernel.public_url", "https://b2.example").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        // Giá trị cũ của B giờ trỏ host B không còn khai: không dùng nữa, quay về link máy chủ (đổi host).
        assert_eq!(
            links.link(&b, &PAGE, Some(SERVER)).await.unwrap().as_deref(),
            Some("https://b2.example/confirm?token={token}")
        );
    })
    .await
    .unwrap();
}
