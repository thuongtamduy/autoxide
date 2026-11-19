//! Chạy thử, xung đột, lịch, số liệu (D-222).

use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use serde_json::json;
use uuid::Uuid;

use super::{body, call, create, running, setup};
use crate::api::ReserveRequest;

#[tokio::test]
async fn chay_thu_ban_nhap_tren_gio_mau_khong_ghi() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let variant = setup.catalog.add(setup.tree.a, 100_000, &[]);
        let id = create(&setup, body(setup.tree.a, json!({}))).await;
        let cart = serde_json::to_value(setup.cart(&[(variant, 2)])).unwrap();
        let (status, found) = call(
            &setup.app,
            "POST",
            &format!("/promotions/{id}/simulate"),
            "boss",
            Some(json!({"carts": [cart]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{found}");
        let result = &found["data"][0];
        assert_eq!(result["difference_minor"], 20_000);
        assert_eq!(result["without"]["applied"], json!([]));
        // Không ghi: vẫn là bản nháp, không ai giữ lượt.
        let (_, view) = call(&setup.app, "GET", &format!("/promotions/{id}"), "boss", None).await;
        assert_eq!(view["data"]["status"], "draft");

        let (status, error) = call(
            &setup.app,
            "POST",
            &format!("/promotions/{id}/simulate"),
            "boss",
            Some(json!({"carts": []})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xung_dot_khi_mot_ben_doc_quyen() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let first = running(&setup, body(setup.tree.a, json!({"value_bp": 500}))).await;
        let second =
            create(&setup, body(setup.tree.a, json!({"value_bp": 500, "exclusivity": "class"})))
                .await;
        let (status, found) =
            call(&setup.app, "GET", &format!("/promotions/{second}/conflicts"), "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{found}");
        assert_eq!(found["data"][0]["id"], first.to_string());
        // Chương trình của cửa hàng B không xung đột với A.
        let other = create(&setup, body(setup.tree.b, json!({"exclusivity": "class"}))).await;
        let (_, found) =
            call(&setup.app, "GET", &format!("/promotions/{other}/conflicts"), "boss", None).await;
        assert_eq!(found["data"], json!([]));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lich_theo_khoang_ngay_bo_ban_nhap() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let now = running(&setup, body(setup.tree.a, json!({}))).await;
        create(&setup, body(setup.tree.a, json!({}))).await;
        let today = crate::compliance::legal_day(Utc::now());
        let range = |from: chrono::NaiveDate, to: chrono::NaiveDate| {
            format!("/promotions/calendar?from={from}&to={to}")
        };
        let (status, found) = call(&setup.app, "GET", &range(today, today), "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{found}");
        let ids: Vec<_> =
            found["data"].as_array().unwrap().iter().map(|row| row["id"].clone()).collect();
        assert_eq!(ids, vec![json!(now.to_string())]);
        // Một tháng sau: chương trình (hết hạn sau 7 ngày) không còn.
        let later = today + chrono::Days::new(30);
        let (_, found) = call(&setup.app, "GET", &range(later, later), "boss", None).await;
        assert_eq!(found["data"], json!([]));
        // Khoảng ngược hay quá dài: 422.
        let (status, _) = call(&setup.app, "GET", &range(later, today), "boss", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, _) =
            call(&setup.app, "GET", &range(today, today + chrono::Days::new(400)), "boss", None)
                .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn so_lieu_theo_ngay_sau_khi_xac_nhan() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let variant = setup.catalog.add(setup.tree.a, 100_000, &[]);
        let id = running(&setup, body(setup.tree.a, json!({}))).await;
        let tenant = setup.tenant();
        let promotions = setup.promotions();
        let order = Uuid::now_v7();
        let request = ReserveRequest::new(
            order,
            setup.cart(&[(variant, 1)]),
            Utc::now() + Duration::hours(1),
        );
        promotions.reserve(&tenant, &request).await.unwrap();
        promotions.confirm(&tenant, order).await.unwrap();
        let today = crate::compliance::legal_day(Utc::now());
        let (status, found) = call(
            &setup.app,
            "GET",
            &format!("/promotions/{id}/stats?from={today}&to={today}"),
            "boss",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{found}");
        let day = &found["data"][0];
        assert_eq!(day["day"], today.to_string());
        assert_eq!(
            (day["redemptions"].clone(), day["discount_minor"].clone()),
            (json!(1), json!(10_000))
        );
    })
    .await
    .unwrap();
}
