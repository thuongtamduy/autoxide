//! Người bán tham gia flash sale của sàn: đăng ký, duyệt, thành suất, người bán trả phần giảm.

use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use serde_json::json;

use super::{activate, body, call, create, setup};
use crate::api::PayerKind;

#[tokio::test]
async fn nguoi_ban_dang_ky_san_duyet_nguoi_ban_tra_phan_giam() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let mine = setup.catalog.add(setup.tree.a, 200_000, &[]);
        // Flash sale của chuỗi (sàn), chưa bắt đầu.
        let starts = Utc::now() + Duration::hours(2);
        let id = create(
            &setup,
            body(
                setup.tree.chain,
                json!({"kind": "flash_sale", "value_bp": null, "starts_at": starts}),
            ),
        )
        .await;
        let uri = format!("/promotions/{id}/participations");
        let offer = |org: uuid::Uuid| {
            json!({"org_id": org, "items": [{"variant_id": mine, "sale_price_minor": 150000, "quantity_limit": 10}]})
        };
        // Cửa hàng B đăng ký sản phẩm không phải của mình: 422.
        let (status, error) =
            call(&setup.app, "POST", &uri, "store_b", Some(offer(setup.tree.b.as_uuid()))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        let (status, applied) =
            call(&setup.app, "POST", &uri, "store_a", Some(offer(setup.tree.a.as_uuid()))).await;
        assert_eq!(status, StatusCode::CREATED, "{applied}");
        assert_eq!(applied["data"]["status"], "pending");
        let participation = applied["data"]["id"].as_str().unwrap().to_owned();
        // Người bán thấy lần của mình; cửa hàng B không thấy.
        let (_, list) = call(&setup.app, "GET", &uri, "store_a", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1);
        let (_, list) = call(&setup.app, "GET", &uri, "store_b", None).await;
        assert_eq!(list["data"], json!([]));
        // Người bán không tự duyệt được.
        let approve = format!("{uri}/{participation}/approve");
        let (status, _) = call(&setup.app, "POST", &approve, "store_a", None).await;
        assert!(status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND, "{status}");
        let (status, approved) = call(&setup.app, "POST", &approve, "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{approved}");
        assert_eq!(approved["data"]["status"], "approved");
        // Đã duyệt: đăng ký lại bị chặn.
        let (status, error) =
            call(&setup.app, "POST", &uri, "store_a", Some(offer(setup.tree.a.as_uuid()))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{error}");

        // Cho chương trình tới giờ rồi bật: giỏ ở cửa hàng A, phần giảm do cửa hàng A trả.
        {
            use sea_orm::sea_query::Expr;
            use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
            crate::entity::promotion::Entity::update_many()
                .col_expr(
                    crate::entity::promotion::Column::StartsAt,
                    Expr::value(Utc::now() - Duration::hours(1)),
                )
                .filter(crate::entity::promotion::Column::Id.eq(id.parse::<uuid::Uuid>().unwrap()))
                .exec(setup.app.database().write())
                .await
                .unwrap();
        }
        let (status, activated) = activate(&setup, &id).await;
        assert_eq!(status, StatusCode::OK, "{activated}");
        let result =
            setup.promotions().evaluate(&setup.tenant(), &setup.cart(&[(mine, 1)])).await.unwrap();
        let line = &result.lines[0];
        assert_eq!(line.amount.minor(), 50_000);
        assert_eq!(line.funding.len(), 1);
        assert_eq!(line.funding[0].payer_kind, PayerKind::Seller);
        assert_eq!(line.funding[0].payer_org_id, Some(setup.tree.a));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dang_ky_dong_thoi_chi_mot_dong() {
    TestDatabase::run(async |test| {
        let setup = std::sync::Arc::new(setup(test, true).await);
        let mine = setup.catalog.add(setup.tree.a, 200_000, &[]);
        let starts = Utc::now() + Duration::hours(2);
        let id = create(
            &setup,
            body(
                setup.tree.chain,
                json!({"kind": "flash_sale", "value_bp": null, "starts_at": starts}),
            ),
        )
        .await;
        let uri = format!("/promotions/{id}/participations");
        let offer = json!({"org_id": setup.tree.a.as_uuid(), "items": [{"variant_id": mine, "value_bp": 2000}]});
        let tasks: Vec<_> = (0..6)
            .map(|_| {
                let (setup, uri, offer) = (setup.clone(), uri.clone(), offer.clone());
                tokio::spawn(async move {
                    call(&setup.app, "POST", &uri, "store_a", Some(offer)).await.0
                })
            })
            .collect();
        for task in tasks {
            assert!(task.await.unwrap().is_success());
        }
        let (_, list) = call(&setup.app, "GET", &uri, "boss", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn duyet_va_tu_choi_cung_luc_khong_lech_suat() {
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        for _ in 0..4 {
            let mine = setup.catalog.add(setup.tree.a, 200_000, &[]);
            let starts = Utc::now() + Duration::hours(2);
            let id = create(
                &setup,
                body(
                    setup.tree.chain,
                    json!({"kind": "flash_sale", "value_bp": null, "starts_at": starts}),
                ),
            )
            .await;
            let uri = format!("/promotions/{id}/participations");
            let offer = json!({"org_id": setup.tree.a.as_uuid(), "items": [{"variant_id": mine, "value_bp": 2000}]});
            let (status, applied) = call(&setup.app, "POST", &uri, "store_a", Some(offer)).await;
            assert_eq!(status, StatusCode::CREATED, "{applied}");
            let participation = applied["data"]["id"].as_str().unwrap().to_owned();
            // Duyệt và từ chối cùng lúc. Postgres từng cho cả hai cùng thấy `pending`: suất flash
            // sale đã thêm nhưng hồ sơ ghi `rejected` (giá flash có hiệu lực cho lần bị từ chối).
            let (approve, reject) =
                (format!("{uri}/{participation}/approve"), format!("{uri}/{participation}/reject"));
            let (approved, rejected) = tokio::join!(
                call(&setup.app, "POST", &approve, "boss", None),
                call(&setup.app, "POST", &reject, "boss", None),
            );
            assert_eq!(approved.0, StatusCode::OK, "{}", approved.1);
            assert!(
                rejected.0 == StatusCode::OK || rejected.0 == StatusCode::CONFLICT,
                "{}",
                rejected.1
            );
            let (_, list) = call(&setup.app, "GET", &uri, "boss", None).await;
            let program = id.parse::<uuid::Uuid>().unwrap();
            let items = crate::entity::flash_item::Entity::find()
                .filter(crate::entity::flash_item::Column::PromotionId.eq(program))
                .count(setup.app.database().read())
                .await
                .unwrap();
            assert_eq!(list["data"][0]["status"], "approved", "{list}");
            assert_eq!(items, 1);
        }
    })
    .await
    .unwrap();
}
