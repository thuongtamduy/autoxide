//! Đổi câu chữ theo khách (bước 51 P21): câu của khách thay câu mặc định; chỗ điền lệch bị từ chối;
//! mã lỗi và câu pháp lý không đổi được; `ui.*` đọc công khai có `ETag`; xoá là về mặc định.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::tenancy::DatabaseRegistry;
use autoxide_kernel::texts::{OverridableText, TextOverrides};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::json;

struct Shop(Arc<Mutex<Option<TextOverrides>>>);

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn overridable_texts(&self) -> Vec<OverridableText> {
        vec![
            OverridableText::new("shop.term.order", "Order"),
            OverridableText::new("shop.greeting", "Hello {name}, your order {number} is ready"),
            OverridableText::new("shop.consent", "I agree to the terms").legal(),
        ]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(context.text_overrides());
        Ok(())
    }
}

#[tokio::test]
async fn cau_cua_khach_thay_cau_mac_dinh() {
    TestDatabase::run(async |test| {
        let handle = Arc::new(Mutex::new(None));
        let app = TestApp::builder(test)
            .user("manager", 1, &["kernel.texts.manage"])
            .user("nobody", 2, &[])
            .module(Shop(handle.clone()))
            .build()
            .await
            .unwrap();
        let put = async |key: &str, text: &str| {
            app.put(&format!("/api/v1/kernel/texts/vi/{key}"))
                .token("manager")
                .json(json!({"text": text}))
                .send()
                .await
                .unwrap()
        };
        let set = put("shop.term.order", "Phiếu đặt").await;
        assert_eq!(set.status(), StatusCode::OK, "{}", set.json());
        assert_eq!(set.json()["data"]["text"], "Phiếu đặt");
        let overrides = handle.lock().unwrap().clone().unwrap();
        let tenant = DatabaseRegistry::single(app.database().clone()).default_tenant();
        assert_eq!(
            overrides.text(&tenant, "vi", "shop.term.order").await.unwrap().as_deref(),
            Some("Phiếu đặt")
        );

        let mismatch = put("shop.greeting", "Chào {name}").await;
        assert_eq!(
            (mismatch.status(), mismatch.json()["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("kernel.text_placeholders_mismatch"))
        );
        assert_eq!(
            put("shop.greeting", "Chào {name}, phiếu {number} xong").await.status(),
            StatusCode::OK
        );
        for key in ["shop.consent", "kernel.view_not_found", "shop.unknown"] {
            let refused = put(key, "x").await;
            assert_eq!(refused.json()["code"], "kernel.text_not_overridable", "{key}");
        }
        assert_eq!(
            put("ui.orders.title", "<b>x</b>").await.status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(put("ui.orders.title", "Phiếu của tôi").await.status(), StatusCode::OK);

        let listed = app
            .get("/api/v1/kernel/texts?locale=vi&overridden=true")
            .token("manager")
            .send()
            .await
            .unwrap()
            .json();
        let keys: Vec<&str> =
            listed["data"].as_array().unwrap().iter().map(|t| t["key"].as_str().unwrap()).collect();
        assert_eq!(keys, ["shop.greeting", "shop.term.order", "ui.orders.title"]);
        assert_eq!(
            app.get("/api/v1/kernel/texts").token("nobody").send().await.unwrap().status(),
            StatusCode::FORBIDDEN
        );

        let public =
            app.get("/api/v1/kernel/public/texts?locale=vi&prefix=ui.").send().await.unwrap();
        assert_eq!(public.status(), StatusCode::OK, "{}", public.json());
        assert_eq!(public.json()["data"], json!({"ui.orders.title": "Phiếu của tôi"}));
        let etag = public.header(axum::http::header::ETAG).to_owned();
        let again = app
            .get("/api/v1/kernel/public/texts?locale=vi&prefix=ui.")
            .header("if-none-match", &etag)
            .send()
            .await
            .unwrap();
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);

        let removed = app
            .delete("/api/v1/kernel/texts/vi/shop.term.order")
            .token("manager")
            .send()
            .await
            .unwrap();
        assert_eq!(removed.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            overrides.text(&tenant, "vi", "shop.term.order").await.unwrap(),
            None,
            "về mặc định"
        );
    })
    .await
    .unwrap();
}
