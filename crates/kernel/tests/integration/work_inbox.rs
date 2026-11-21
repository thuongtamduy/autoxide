//! Hộp việc (bước 51 P2): gom từ các nguồn người gọi có permission, sắp theo mức gấp rồi hạn; nguồn
//! chậm bị bỏ qua và ghi vào `incomplete`; số đếm theo nguồn kèm tổng `work`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Date;
use autoxide_kernel::work::{Severity, WorkItem, WorkSource};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;

const DEMO_REVIEW: Permission = Permission::new("demo.review", "Review demo things.");
const DEMO_SLOW: Permission = Permission::new("demo.slow", "See the slow source.");

struct Waiting;

#[async_trait::async_trait]
impl WorkSource for Waiting {
    fn id(&self) -> &'static str {
        "demo.waiting"
    }

    fn permission(&self) -> Permission {
        DEMO_REVIEW
    }

    async fn count(&self, _: &TenantContext, _: &Principal, _: OrgTarget) -> Result<u64, Error> {
        Ok(3)
    }

    async fn items(
        &self,
        _: &TenantContext,
        _: &Principal,
        _: OrgTarget,
        _: u16,
    ) -> Result<Vec<WorkItem>, Error> {
        let due = |day: i8| Some(Date::new(2026, 1, day).unwrap().start_of_day_utc());
        Ok(vec![
            WorkItem::new("later", Severity::Attention, "Check {name}", "demo.list")
                .param("name", "B")
                .due_at(due(9)),
            WorkItem::new("info", Severity::Info, "Check {name}", "demo.list").param("name", "C"),
            WorkItem::new("block", Severity::Blocking, "Check {name}", "demo.list")
                .param("name", "A"),
            WorkItem::new("soon", Severity::Attention, "Check {name}", "demo.list")
                .param("name", "D")
                .due_at(due(2)),
        ])
    }
}

/// Nguồn quá hạn giờ: không làm hỏng hộp.
struct Slow;

#[async_trait::async_trait]
impl WorkSource for Slow {
    fn id(&self) -> &'static str {
        "demo.slow"
    }

    fn permission(&self) -> Permission {
        DEMO_SLOW
    }

    async fn count(&self, _: &TenantContext, _: &Principal, _: OrgTarget) -> Result<u64, Error> {
        tokio::time::sleep(Duration::from_secs(2)).await;
        Ok(1)
    }

    async fn items(
        &self,
        _: &TenantContext,
        _: &Principal,
        _: OrgTarget,
        _: u16,
    ) -> Result<Vec<WorkItem>, Error> {
        tokio::time::sleep(Duration::from_secs(2)).await;
        Ok(Vec::new())
    }
}

struct Demo;

impl Module for Demo {
    fn name(&self) -> &'static str {
        "demo"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![DEMO_REVIEW, DEMO_SLOW]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.work_source(Arc::new(Waiting))?;
        context.work_source(Arc::new(Slow))
    }
}

#[tokio::test]
async fn hop_viec_gom_sap_va_bo_nguon_cham() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .user("reviewer", 1, &["demo.review", "demo.slow"])
            .user("nobody", 2, &[])
            .module(Demo)
            .build()
            .await
            .unwrap();

        let answer = app.get("/api/v1/kernel/work").token("reviewer").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.json());
        let body = answer.json();
        let keys: Vec<&str> = body["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["key"].as_str().unwrap())
            .collect();
        assert_eq!(
            keys,
            ["demo.waiting:block", "demo.waiting:soon", "demo.waiting:later", "demo.waiting:info"]
        );
        assert_eq!(body["data"]["items"][0]["title"], "Check A");
        assert_eq!(body["data"]["items"][0]["target"]["screen"], "demo.list");
        assert_eq!(body["data"]["incomplete"], serde_json::json!(["demo.slow"]));

        let counts =
            app.get("/api/v1/kernel/work/counts").token("reviewer").send().await.unwrap().json();
        assert_eq!(counts["data"]["counts"]["demo.waiting"], 3);
        assert_eq!(counts["data"]["counts"]["work"], 3);
        assert_eq!(counts["data"]["incomplete"], serde_json::json!(["demo.slow"]));

        let none = app.get("/api/v1/kernel/work").token("nobody").send().await.unwrap().json();
        assert_eq!(none["data"]["items"], serde_json::json!([]), "không có permission của nguồn");
        let bad = app.get("/api/v1/kernel/work?limit=0").token("reviewer").send().await.unwrap();
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}
