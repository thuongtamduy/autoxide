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

// Danh sách nhãn phân trang keyset trong database theo tên đã chuẩn hoá (bỏ dấu, chữ thường): nhiều nhãn hơn
// `limit`, theo `next_cursor` tới hết không trùng / sót, `order=desc` đảo; con trỏ hỏng là 422.
#[tokio::test]
async fn danh_sach_nhan_phan_trang_theo_ten() {
    TestDatabase::run(async |test| {
        let app =
            TestApp::builder(test).user("boss", 1, &["kernel.tags.manage"]).build().await.unwrap();
        for name in ["Đỏ", "banana", "Apple", "cherry", "Ếch"] {
            let made = app
                .post("/api/v1/kernel/tags")
                .token("boss")
                .json(json!({"name": name}))
                .send()
                .await
                .unwrap();
            assert_eq!(made.status(), StatusCode::CREATED, "{}", made.json());
        }
        let walk = async |order: &str| -> Vec<String> {
            let mut names = Vec::new();
            let mut cursor: Option<String> = None;
            loop {
                let mut path = format!("/api/v1/kernel/tags?limit=2&order={order}");
                if let Some(cursor) = &cursor {
                    path.push_str(&format!("&cursor={cursor}"));
                }
                let page = app.get(&path).token("boss").send().await.unwrap().json();
                names.extend(
                    page["data"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|tag| tag["name"].as_str().unwrap().to_owned()),
                );
                match page["pagination"]["next_cursor"].as_str() {
                    Some(next) => cursor = Some(next.to_owned()),
                    None => return names,
                }
            }
        };
        let ascending = walk("asc").await;
        assert_eq!(ascending, ["Apple", "banana", "cherry", "Đỏ", "Ếch"]);
        let mut descending = walk("desc").await;
        descending.reverse();
        assert_eq!(descending, ascending);
        let broken = app.get("/api/v1/kernel/tags?cursor=abc").token("boss").send().await.unwrap();
        assert_eq!(broken.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(broken.json()["errors"][0]["field"], "cursor");
    })
    .await
    .unwrap();
}
