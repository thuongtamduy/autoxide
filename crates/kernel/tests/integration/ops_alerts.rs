//! Cảnh báo vận hành (`GET /kernel/ops/alerts`) và "Có gì mới" (`GET /kernel/whats-new`) — đề bài
//! giao diện, màn Sức khoẻ và nút "?".

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use autoxide_kernel::doctor::{Check, Finding, Probe};
use autoxide_kernel::module::Module;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;

/// Đếm số lần chạy để thấy kết quả được giữ một phút.
struct Counted(Arc<AtomicUsize>, Finding);

#[async_trait::async_trait]
impl Probe for Counted {
    async fn probe(&self) -> Finding {
        self.0.fetch_add(1, Ordering::SeqCst);
        self.1.clone()
    }
}

struct Demo(Arc<AtomicUsize>);

impl Module for Demo {
    fn name(&self) -> &'static str {
        "demo"
    }

    fn checks(&self) -> Vec<Check> {
        let warn = Finding::warning("The demo key is not set.").with_hint("Set DEMO_KEY.");
        let broken = Finding::failed("The demo server does not answer.");
        vec![
            Check::new(
                "fine",
                "Always fine.",
                Arc::new(Counted(self.0.clone(), Finding::ok("Fine."))),
            ),
            Check::new("key", "Checks the key.", Arc::new(Counted(self.0.clone(), warn))),
            Check::new("server", "Checks the server.", Arc::new(Counted(self.0.clone(), broken))),
        ]
    }
}

#[tokio::test]
async fn canh_bao_van_hanh_chi_lieu_buoc_chua_dat() {
    TestDatabase::run(async |test| {
        let runs = Arc::new(AtomicUsize::new(0));
        let app = TestApp::builder(test)
            .tenant("shop")
            .user("platform", 1, &["kernel.tenants.read"])
            .user("nobody", 2, &[])
            .user_in("shop", "owner", 3, &["kernel.tenants.read"])
            .module(Demo(runs.clone()))
            .build()
            .await
            .unwrap();
        let get = async |uri: &str, who: &str| app.get(uri).token(who).send().await.unwrap();

        let answer = get("/api/v1/kernel/ops/alerts", "platform").await;
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.json());
        let body = answer.json();
        let alerts = body["data"]["alerts"].as_array().unwrap();
        let names: Vec<&str> = alerts.iter().map(|alert| alert["name"].as_str().unwrap()).collect();
        // `kernel.backup_age`: app thử không có nơi lưu file nên backup theo lịch không có chỗ ghi.
        assert_eq!(
            names,
            ["demo.server", "demo.key", "kernel.backup_age"],
            "hỏng trước, cảnh báo sau; bước đạt không có"
        );
        assert_eq!(alerts[0]["severity"], "failed");
        assert_eq!(alerts[1]["hint"], "Set DEMO_KEY.");
        let first_runs = runs.load(Ordering::SeqCst);
        assert!(first_runs >= 3);

        // Trong một phút: dùng lại kết quả; `refresh=true` kiểm lại.
        get("/api/v1/kernel/ops/alerts", "platform").await;
        assert_eq!(runs.load(Ordering::SeqCst), first_runs);
        get("/api/v1/kernel/ops/alerts?refresh=true", "platform").await;
        assert!(runs.load(Ordering::SeqCst) > first_runs);

        assert_eq!(
            get("/api/v1/kernel/ops/alerts", "nobody").await.status(),
            StatusCode::FORBIDDEN
        );
        let shop = app
            .get("/api/v1/kernel/ops/alerts")
            .token("owner")
            .header("autoxide-tenant", "shop")
            .send()
            .await
            .unwrap();
        assert_eq!(
            shop.status(),
            StatusCode::FORBIDDEN,
            "chỉ nhân sự khách nền tảng: {}",
            shop.json()
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn co_gi_moi_doc_tu_changelog() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).build().await.unwrap();
        let all = app.get("/api/v1/kernel/whats-new").send().await.unwrap();
        assert_eq!(all.status(), StatusCode::OK, "{}", all.json());
        let releases = all.json()["data"].as_array().unwrap().clone();
        assert!(!releases.is_empty() && releases.len() <= 5);
        assert!(releases[0]["sections"][0]["items"][0].is_string());
        let newer = app.get("/api/v1/kernel/whats-new?since=0.13.0").send().await.unwrap();
        let versions: Vec<String> = newer.json()["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|release| release["version"].as_str().unwrap().to_owned())
            .collect();
        assert!(
            versions.contains(&"0.14.0".to_owned()) && !versions.contains(&"0.13.0".to_owned()),
            "{versions:?}"
        );
        let bad = app.get("/api/v1/kernel/whats-new?since=latest").send().await.unwrap();
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}
