//! Số liệu tóm tắt (bước 51 P4, D-207): lọc theo nhóm và permission, mỗi số kèm định nghĩa; số
//! chậm vào `incomplete`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::summary::{MetricInfo, MetricValue, SummaryMetric};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;

const SHOP_READ: Permission = Permission::new("shop.read", "See the shop.");

struct Revenue;

#[async_trait::async_trait]
impl SummaryMetric for Revenue {
    fn info(&self) -> MetricInfo {
        MetricInfo::new(
            "shop.revenue_today",
            "Revenue today",
            "Paid orders since midnight.",
            "sales",
        )
    }

    fn permission(&self) -> Permission {
        SHOP_READ
    }

    async fn value(
        &self,
        _: &TenantContext,
        _: &Principal,
        _: OrgTarget,
    ) -> Result<MetricValue, Error> {
        Ok(MetricValue::Money { amount_minor: 125_000, currency: "VND".to_owned() })
    }
}

struct Slow;

#[async_trait::async_trait]
impl SummaryMetric for Slow {
    fn info(&self) -> MetricInfo {
        MetricInfo::new("shop.slow", "Slow", "Takes too long.", "home")
    }

    fn permission(&self) -> Permission {
        SHOP_READ
    }

    async fn value(
        &self,
        _: &TenantContext,
        _: &Principal,
        _: OrgTarget,
    ) -> Result<MetricValue, Error> {
        // Không bao giờ trả lời kịp: nguồn chỉ có thể bị bỏ vì quá trần.
        tokio::time::sleep(Duration::from_secs(600)).await;
        Ok(MetricValue::Count(1))
    }
}

struct Shop;

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![SHOP_READ]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.summary_metric(Arc::new(Revenue))?;
        context.summary_metric(Arc::new(Slow))
    }
}

#[tokio::test]
async fn so_lieu_tom_tat_kem_dinh_nghia() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            // Trần ngắn hơn mặc định của testkit (60 s) để nguồn chậm bị bỏ; vẫn đủ rộng cho nguồn nhanh khi máy
            // bận.
            .setting("AUTOXIDE_SOURCE_TIMEOUT_MS", "5000")
            .user("clerk", 1, &["shop.read"])
            .user("nobody", 2, &[])
            .module(Shop)
            .build()
            .await
            .unwrap();
        let all = app.get("/api/v1/kernel/summary").token("clerk").send().await.unwrap();
        assert_eq!(all.status(), StatusCode::OK, "{}", all.json());
        let body = all.json();
        let metric = &body["data"]["metrics"][0];
        assert_eq!(metric["id"], "shop.revenue_today");
        assert_eq!(metric["definition"], "Paid orders since midnight.");
        assert_eq!(
            (
                metric["value"]["kind"].as_str(),
                metric["value"]["amount"].as_i64(),
                metric["value"]["currency"].as_str()
            ),
            (Some("money"), Some(125_000), Some("VND"))
        );
        assert_eq!(body["data"]["incomplete"], serde_json::json!(["shop.slow"]));

        let sales = app
            .get("/api/v1/kernel/summary?group=home")
            .token("clerk")
            .send()
            .await
            .unwrap()
            .json();
        assert_eq!(sales["data"]["metrics"], serde_json::json!([]), "chỉ nhóm hỏi");
        let none = app.get("/api/v1/kernel/summary").token("nobody").send().await.unwrap().json();
        assert_eq!(none["data"]["metrics"], serde_json::json!([]));
    })
    .await
    .unwrap();
}
