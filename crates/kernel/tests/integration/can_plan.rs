//! Giải thích quyền theo gói (bước 51 P11): việc cần tính năng gói chưa có → `feature_not_in_plan`,
//! hạn mức đã dùng hết → `quota_exceeded` (kèm lượng dùng thật), cả hai kèm `upgrade_to` — mã gói
//! đang bán mở được việc đó.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::entitlement::{
    Entitlements, Limit, PlanNeed, TenantEntitlements, UsageCounter,
};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use autoxide_testkit::{TestApp, TestDatabase};
use serde_json::json;

const SHOP_WRITE: Permission = Permission::new("shop.write", "Change the shop.");
const ITEMS: Limit = Limit::count("shop.items", "Items a shop may list.");

struct Shop;

/// Khách đã có 3 mặt hàng.
struct Three;

#[async_trait::async_trait]
impl UsageCounter for Three {
    async fn count(&self, _: &TenantContext) -> Result<i64, Error> {
        Ok(3)
    }
}

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![SHOP_WRITE]
    }

    fn limits(&self) -> Vec<Limit> {
        vec![ITEMS]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.usage_counter(&ITEMS, Arc::new(Three))
    }
}

/// Gói hiện tại: có module `shop`, không có tính năng `shop.pos`, tối đa 3 mặt hàng. Gói `pro`
/// có cả hai.
struct Basic;

#[async_trait::async_trait]
impl Entitlements for Basic {
    async fn snapshot(&self, _: TenantId) -> Result<Arc<TenantEntitlements>, Error> {
        Ok(Arc::new(TenantEntitlements {
            modules: ["shop".to_owned()].into(),
            limits: [("shop.items".to_owned(), Some(3))].into(),
            ..TenantEntitlements::default()
        }))
    }

    async fn plans_offering(&self, need: &PlanNeed) -> Result<Vec<String>, Error> {
        let pro = need.feature.as_deref().is_none_or(|feature| feature == "shop.pos")
            && need.limit.as_ref().is_none_or(|(key, above)| key == "shop.items" && *above < 100);
        Ok(if pro { vec!["pro".to_owned()] } else { Vec::new() })
    }

    async fn reserve(&self, _: TenantId, _: &Limit, _: i64, _: &str) -> Result<(), Error> {
        Ok(())
    }

    async fn confirm(&self, _: TenantId, _: &Limit, _: &str) -> Result<(), Error> {
        Ok(())
    }

    async fn release(&self, _: TenantId, _: &Limit, _: &str) -> Result<(), Error> {
        Ok(())
    }

    async fn record_usage(&self, _: TenantId, _: &Limit, _: i64, _: &str) -> Result<(), Error> {
        Ok(())
    }
}

#[tokio::test]
async fn giai_thich_tinh_nang_va_han_muc_cua_goi() {
    TestDatabase::run(async |test| {
        let plan: Arc<dyn Entitlements> = Arc::new(Basic);
        let app = TestApp::builder(test)
            .tenant("acme")
            .user_in("acme", "clerk", 1, &["shop.write"])
            .provide::<dyn Entitlements>(plan)
            .module(Shop)
            .build()
            .await
            .unwrap();
        let answers = app
            .post("/api/v1/me/can")
            .token("clerk")
            .header("autoxide-tenant", "acme")
            .json(json!({"checks": [
                {"permission": "shop.write"},
                {"permission": "shop.write", "feature": "shop.pos"},
                {"permission": "shop.write", "limit": "shop.items"},
            ]}))
            .send()
            .await
            .unwrap()
            .json();
        let data = &answers["data"];
        assert_eq!(data[0]["allowed"], true, "{answers}");
        assert_eq!(
            (data[1]["reason"].as_str(), data[1]["required_feature"].as_str()),
            (Some("feature_not_in_plan"), Some("shop.pos"))
        );
        assert_eq!(data[1]["upgrade_to"], json!(["pro"]));
        assert_eq!(data[2]["reason"], "quota_exceeded");
        assert_eq!((data[2]["used"].as_i64(), data[2]["max"].as_i64()), (Some(3), Some(3)));
        assert_eq!(data[2]["upgrade_to"], json!(["pro"]));
    })
    .await
    .unwrap();
}
