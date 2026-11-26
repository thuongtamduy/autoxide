//! `GET /templates` theo trang (bước 70 P9, D-20261125-list-pages): theo loại, rồi thứ tự ghi lần đầu (ID);
//! theo `next_cursor` tới hết không trùng không sót, hai chiều; ghi lại giữ chỗ (giữ ID). Cả SQLite và Postgres.

use super::*;

#[tokio::test]
async fn cau_thay_the_phan_trang_theo_loai() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        let text = json!({"title": "T", "body": "B", "short": "S"});
        let saved = [
            "shop.promo/email/vi",
            "shop.order_shipped/in_app/en",
            "shop.order_shipped/email/vi",
            "shop.new_sign_in/email/en",
            "shop.order_shipped/email/en",
        ];
        for path in saved {
            let uri = format!("/templates/{path}");
            let (status, body) = call(&fx.app, "PUT", &uri, "admin", Some(text.clone())).await;
            assert_eq!(status, StatusCode::OK, "{path}: {body}");
        }
        // Ghi lại một câu đã có: vẫn ở chỗ cũ (ID không đổi).
        let again = json!({"title": "T2", "body": "B", "short": "S"});
        let (status, _) =
            call(&fx.app, "PUT", "/templates/shop.order_shipped/in_app/en", "admin", Some(again))
                .await;
        assert_eq!(status, StatusCode::OK);
        let walk = async |query: &str| {
            let mut found = Vec::new();
            let mut uri = format!("/templates?limit=2{query}");
            loop {
                let (status, page) = call(&fx.app, "GET", &uri, "admin", None).await;
                assert_eq!(status, StatusCode::OK, "{page}");
                let rows = page["data"].as_array().unwrap();
                assert!(rows.len() <= 2);
                found.extend(rows.iter().map(|row| {
                    format!(
                        "{}/{}/{}",
                        row["kind"].as_str().unwrap(),
                        row["channel"].as_str().unwrap(),
                        row["locale"].as_str().unwrap()
                    )
                }));
                match page["pagination"]["next_cursor"].as_str() {
                    Some(cursor) => uri = format!("/templates?limit=2{query}&cursor={cursor}"),
                    None => return found,
                }
            }
        };
        let expected = [
            "shop.new_sign_in/email/en",
            "shop.order_shipped/in_app/en",
            "shop.order_shipped/email/vi",
            "shop.order_shipped/email/en",
            "shop.promo/email/vi",
        ];
        assert_eq!(walk("").await, expected);
        let down: Vec<&str> = expected.iter().rev().copied().collect();
        assert_eq!(walk("&order=desc").await, down);
        // Không có quyền: 403 trước lỗi tham số trang.
        let (status, _) = call(&fx.app, "GET", "/templates?limit=0", "alice", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = call(&fx.app, "GET", "/templates?limit=0", "admin", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}
