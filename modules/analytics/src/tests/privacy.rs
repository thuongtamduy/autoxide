//! Theo dõi theo người chỉ khi đồng ý, rút đồng ý thì gỡ phần gắn người (giữ số gộp), xoá tài khoản,
//! xuất dữ liệu cá nhân, hết hạn thì xoá sự kiện thô.

use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::privacy::PersonalData;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use jiff::SignedDuration;
use sea_orm::EntityTrait;
use serde_json::json;

use super::{CHROME, HOST, SAFARI, at, browser, harness, page, user_id};
use crate::api::Analytics;
use crate::entity::daily;
use crate::privacy::Forget;

#[tokio::test]
async fn theo_doi_theo_nguoi_chi_khi_dong_y() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let buyer = user_id(9);
        let client = browser("203.0.113.1", CHROME);
        let product =
            json!({"type": "shop.product_viewed", "path": "/p/1", "object": "shop.product:p-1"});
        // Chưa đồng ý: không gắn người, kể cả khi đã đăng nhập.
        h.send(None, &client, Some(buyer), vec![product.clone()], at(10, 9)).await;
        assert_eq!(h.raw().await[0].user_id, None);
        // Đồng ý `analytics.personal`: gắn người, "vừa xem" có dữ liệu.
        super::grant_personal(&h.ledger, buyer);
        h.send(None, &client, Some(buyer), vec![product.clone()], at(10, 10)).await;
        let other =
            json!({"type": "shop.product_viewed", "path": "/p/2", "object": "shop.product:p-2"});
        h.send(None, &client, Some(buyer), vec![other], at(10, 11)).await;
        let rows = h.raw().await;
        assert_eq!(rows[1].user_id, Some(buyer));
        let recent = h.service.recently_viewed(&h.tenant, buyer, "shop.product", 10).await.unwrap();
        assert_eq!(recent, ["shop.product:p-2", "shop.product:p-1"]);

        // Qua REST: người mua đăng nhập (token) và đã đồng ý.
        let answer = h
            .app
            .post("/api/v1/analytics/public/events")
            .token("buyer")
            .host(HOST)
            .header("origin", &format!("https://{HOST}"))
            .header("user-agent", SAFARI)
            .json(json!({"events": [page("/account")]}))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::ACCEPTED);
        assert_eq!(h.raw().await.last().unwrap().user_id, Some(buyer));
        // Xuất dữ liệu cá nhân: sự kiện còn gắn với người đó.
        let exported = h.service.export(&h.tenant, buyer).await.unwrap();
        assert_eq!(exported["events"].as_array().unwrap().len(), 3);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn rut_dong_y_xoa_phan_gan_nguoi() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let (buyer, other) = (user_id(9), user_id(10));
        super::grant_personal(&h.ledger, buyer);
        super::grant_personal(&h.ledger, other);
        let client = browser("203.0.113.1", CHROME);
        h.send(None, &client, Some(buyer), vec![page("/a"), page("/b")], at(10, 9)).await;
        h.send(None, &client, Some(other), vec![page("/c")], at(10, 9)).await;
        h.service.rollup_tenant(&h.tenant, at(10, 10)).await.unwrap();
        let before = daily::Entity::find().all(h.app.database().read()).await.unwrap();

        // Rút đồng ý: ngừng ngay (đọc sổ đồng ý ở mỗi lần), rồi bên nhận event gỡ phần gắn người.
        super::withdraw_personal(&h.ledger, buyer);
        let recent = h.service.recently_viewed(&h.tenant, buyer, "shop.product", 10).await;
        assert!(recent.unwrap().is_empty());
        h.send(None, &client, Some(buyer), vec![page("/d")], at(10, 11)).await;
        assert_eq!(h.raw().await.last().unwrap().user_id, None);
        let withdrawn = Delivered::new(
            h.tenant.clone(),
            "compliance",
            autoxide_compliance::api::CONSENT_WITHDRAWN,
            json!({"consent_id": uuid::Uuid::now_v7(), "purpose": "analytics.personal", "scope": "", "channel": ""}),
        );
        let forget = Forget(h.service.clone());
        forget.handle(&withdrawn).await.unwrap();
        // Nhận lại: vô hại.
        forget.handle(&withdrawn).await.unwrap();
        let linked: Vec<_> = h.raw().await.into_iter().filter_map(|row| row.user_id).collect();
        assert_eq!(linked, [other]);
        // Số gộp giữ nguyên (không chứa người).
        let after = daily::Entity::find().all(h.app.database().read()).await.unwrap();
        assert_eq!(after, before);

        // Xoá tài khoản: gỡ phần của người đó.
        let deleted = Delivered::new(
            h.tenant.clone(),
            "auth",
            autoxide_kernel::privacy::USER_DELETED,
            json!({"before": {"id": other}}),
        );
        forget.handle(&deleted).await.unwrap();
        assert!(h.raw().await.iter().all(|row| row.user_id.is_none()));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn het_han_thi_xoa_su_kien_tho() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let client = browser("203.0.113.1", CHROME);
        h.send(None, &client, None, vec![page("/old")], at(1, 9)).await;
        h.send(None, &client, None, vec![page("/new")], at(9, 9)).await;
        h.service.rollup_tenant(&h.tenant, at(9, 10)).await.unwrap();
        // Hạn lưu sự kiện thô mặc định 7 ngày: sự kiện ngày 1 hết hạn vào ngày 9, ngày 9 còn.
        let cleaned = h.service.cleanup(h.app.database(), at(9, 12)).await.unwrap();
        assert_eq!(cleaned.events, 1);
        let paths: Vec<String> = h.raw().await.into_iter().map(|row| row.path).collect();
        assert_eq!(paths, ["/new"]);
        // Số gộp còn (giữ 790 ngày), tới khi quá hạn của chúng.
        let days = daily::Entity::find().all(h.app.database().read()).await.unwrap();
        assert!(days.iter().any(|row| row.day == "2026-11-01"));
        let later = at(9, 12) + SignedDuration::from_hours(24 * 800);
        let cleaned = h.service.cleanup(h.app.database(), later).await.unwrap();
        assert!(cleaned.daily > 0);
        assert!(daily::Entity::find().all(h.app.database().read()).await.unwrap().is_empty());
        assert!(h.raw().await.is_empty());
    })
    .await
    .unwrap();
}
