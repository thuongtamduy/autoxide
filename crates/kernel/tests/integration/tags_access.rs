//! Pentest hộp đen 2026-11: nhãn là ghi chú nội bộ của nhân sự; tài khoản người mua của cửa hàng
//! không liệt kê được (`kernel.tags_staff_only`), nhân sự không permission nào vẫn đọc được.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::access::Principal;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn nguoi_mua_khong_doc_duoc_nhan_noi_bo() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .user("boss", 1, &["kernel.tags.manage"])
            .user("clerk", 2, &[])
            .principal("buyer", Principal::customer(Uuid::from_u128(3)))
            .build()
            .await
            .unwrap();
        let made = app
            .post("/api/v1/kernel/tags")
            .token("boss")
            .json(json!({"name": "Fraud suspect"}))
            .send()
            .await
            .unwrap();
        assert_eq!(made.status(), StatusCode::CREATED, "{}", made.json());

        let staff = app.get("/api/v1/kernel/tags").token("clerk").send().await.unwrap();
        assert_eq!(staff.status(), StatusCode::OK);
        assert_eq!(staff.json()["data"][0]["name"], "Fraud suspect");

        let buyer = app.get("/api/v1/kernel/tags").token("buyer").send().await.unwrap();
        assert_eq!(buyer.status(), StatusCode::FORBIDDEN);
        assert_eq!(buyer.json()["code"], "kernel.tags_staff_only");
        assert!(!buyer.text().contains("Fraud suspect"));
    })
    .await
    .unwrap();
}
