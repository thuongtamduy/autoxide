//! Hạn mức trường tự định nghĩa theo gói (bước 51 P19): tạo trường giữ một suất `kernel.custom_fields`,
//! hết suất thì từ chối với lỗi của gói, xoá (tắt) trường thì trả suất.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use autoxide_kernel::access::Permission;
use autoxide_kernel::custom_fields::CustomFieldTarget;
use autoxide_kernel::entitlement::{Entitlements, Limit, TenantEntitlements};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::Module;
use autoxide_kernel::tenancy::TenantId;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::json;

const SHOP_READ: Permission = Permission::new("shop.read", "See the shop.");
const SHOP_WRITE: Permission = Permission::new("shop.write", "Change the shop.");

struct Shop;

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![SHOP_READ, SHOP_WRITE]
    }

    fn custom_field_targets(&self) -> Vec<CustomFieldTarget> {
        vec![CustomFieldTarget::new("shop.item", SHOP_READ, SHOP_WRITE)]
    }
}

/// Gói cho đúng một trường: giữ danh sách suất đang dùng.
#[derive(Default)]
struct OneField(Mutex<Vec<String>>);

#[async_trait::async_trait]
impl Entitlements for OneField {
    async fn snapshot(&self, _: TenantId) -> Result<Arc<TenantEntitlements>, Error> {
        Ok(Arc::new(TenantEntitlements {
            modules: ["shop".to_owned()].into(),
            ..TenantEntitlements::default()
        }))
    }

    async fn reserve(
        &self,
        _: TenantId,
        limit: &Limit,
        _: i64,
        reference: &str,
    ) -> Result<(), Error> {
        assert_eq!(limit.key(), "kernel.custom_fields");
        let mut used = self.0.lock().unwrap();
        if !used.is_empty() {
            return Err(Error::conflict("plans.limit_reached", "The plan allows one field."));
        }
        used.push(reference.to_owned());
        Ok(())
    }

    async fn confirm(&self, _: TenantId, _: &Limit, _: &str) -> Result<(), Error> {
        Ok(())
    }

    async fn release(&self, _: TenantId, _: &Limit, reference: &str) -> Result<(), Error> {
        self.0.lock().unwrap().retain(|known| known != reference);
        Ok(())
    }

    async fn record_usage(&self, _: TenantId, _: &Limit, _: i64, _: &str) -> Result<(), Error> {
        Ok(())
    }
}

#[tokio::test]
async fn han_muc_truong_tu_dinh_nghia_theo_goi() {
    TestDatabase::run(async |test| {
        let plan: Arc<dyn Entitlements> = Arc::new(OneField::default());
        let app = TestApp::builder(test)
            .tenant("acme")
            .user_in("acme", "admin", 1, &["shop.read", "kernel.custom_fields.manage"])
            .provide::<dyn Entitlements>(plan)
            .module(Shop)
            .build()
            .await
            .unwrap();
        let define = async |key: &str| {
            app.post("/api/v1/kernel/custom-fields")
                .token("admin")
                .header("autoxide-tenant", "acme")
                .json(json!({"target": "shop.item", "key": key, "data_type": "text", "labels": {"en": key}}))
                .send()
                .await
                .unwrap()
        };
        let first = define("color").await;
        assert_eq!(first.status(), StatusCode::CREATED, "{}", first.json());
        let second = define("size").await;
        assert_eq!(
            (second.status(), second.json()["code"].as_str()),
            (StatusCode::CONFLICT, Some("plans.limit_reached"))
        );
        let id = first.json()["data"]["id"].as_str().unwrap().to_owned();
        let removed = app
            .delete(&format!("/api/v1/kernel/custom-fields/{id}"))
            .token("admin")
            .header("autoxide-tenant", "acme")
            .send()
            .await
            .unwrap();
        assert_eq!(removed.status(), StatusCode::NO_CONTENT);
        assert_eq!(define("size").await.status(), StatusCode::CREATED, "xoá trả suất");
    })
    .await
    .unwrap();
}
