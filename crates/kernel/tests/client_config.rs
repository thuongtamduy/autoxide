//! Cấu hình cho giao diện (D-271): `GET /api/v1/kernel/client-config` gom phần của từng module,
//! không cần đăng nhập, và lược đồ OpenAPI ghép từ lược đồ của từng phần (testkit kiểm câu trả lời
//! thật theo lược đồ đó). Gate chạy file này trên cả hai database.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::error::Error;
use autoxide_kernel::module::{ClientSection, Module};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde::Serialize;
use serde_json::json;
use utoipa::ToSchema;

/// Settings of the test module.
#[derive(Serialize, ToSchema)]
struct ShopConfig {
    /// Currencies prices may use.
    currencies: Vec<String>,
    /// Largest cart.
    max_items: u32,
}

struct Shop;

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn client_config(&self) -> Result<Option<ClientSection>, Error> {
        let config =
            ShopConfig { currencies: vec!["VND".to_owned(), "USD".to_owned()], max_items: 50 };
        ClientSection::new(&config).map(Some)
    }
}

/// Module không khai gì thì vắng.
struct Quiet;

impl Module for Quiet {
    fn name(&self) -> &'static str {
        "quiet"
    }
}

#[tokio::test]
async fn cau_hinh_cho_giao_dien_gom_phan_cua_tung_module() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .setting("AUTOXIDE_HTTP_PUBLIC_URL", "https://api.example.com")
            .module(Shop)
            .module(Quiet)
            .build()
            .await
            .unwrap();
        let answer = app.request(Method::GET, "/api/v1/kernel/client-config").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.json());
        let data = &answer.json()["data"];
        assert_eq!(data["shop"], json!({"currencies": ["VND", "USD"], "max_items": 50}));
        assert_eq!(data["kernel"]["public_url"], "https://api.example.com");
        assert!(data["kernel"]["version"].is_string());
        assert!(data.get("quiet").is_none(), "{data}");

        let document = serde_json::to_value(app.openapi()).unwrap();
        let schema = &document["components"]["schemas"]["ClientConfigResponse"];
        assert_eq!(schema["required"], json!(["kernel", "shop"]), "{schema}");
        assert_eq!(
            schema["properties"]["shop"]["properties"]["max_items"]["description"],
            "Largest cart."
        );
        let operation = &document["paths"]["/api/v1/kernel/client-config"]["get"];
        assert_eq!(operation["x-public"], true);
    })
    .await
    .unwrap();
}
