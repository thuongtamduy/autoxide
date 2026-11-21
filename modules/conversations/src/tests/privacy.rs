//! Dữ liệu cá nhân: hạn lưu, xoá tài khoản, bản xuất.

use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::privacy::PersonalData;
use autoxide_kernel::tenancy::ScopeExt;
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use jiff::SignedDuration;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use serde_json::json;
use uuid::Uuid;

use super::{BUYER, call, harness, reply, service_of, start};
use crate::entity::{message, thread};

#[tokio::test]
async fn het_han_luu_thi_xoa() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let old = start(&h.app, "buyer", "Cuộc cũ").await;
        reply(&h.app, "agent", &old, "Trả lời cũ").await;
        let fresh = start(&h.app, "other", "Cuộc mới").await;
        let tenant = h.app.default_tenant();
        let old_id: Uuid = old.parse().unwrap();
        thread::Entity::update_many()
            .col_expr(
                thread::Column::LastMessageAt,
                Expr::value(Timestamp::now() - SignedDuration::from_hours(24 * 366)),
            )
            .filter(thread::Column::Id.eq(old_id))
            .scoped(&tenant)
            .exec(tenant.database().write())
            .await
            .unwrap();
        let service = service_of();
        let removed = service.purge(h.app.database(), crate::service::now()).await.unwrap();
        assert_eq!(removed, 1);
        // Chạy lại: không còn gì để xoá.
        assert_eq!(service.purge(h.app.database(), crate::service::now()).await.unwrap(), 0);
        let (status, _) = call(&h.app, "GET", &format!("/threads/{old}"), "agent", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let left = message::Entity::find()
            .filter(message::Column::ThreadId.eq(old_id))
            .count(tenant.database().read())
            .await
            .unwrap();
        assert_eq!(left, 0, "messages go with their conversation");
        let (status, _) = call(&h.app, "GET", &format!("/threads/{fresh}"), "agent", None).await;
        assert_eq!(status, StatusCode::OK);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xoa_tai_khoan_an_danh_hoa_phia_nguoi_mua() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let id = start(&h.app, "buyer", "Số tôi 0901234567").await;
        reply(&h.app, "agent", &id, "Dạ shop nhận").await;
        let service = service_of();
        let exported =
            service.export(&h.app.default_tenant(), Uuid::from_u128(BUYER)).await.unwrap();
        assert_eq!(exported[0]["messages"][0]["body"], "Số tôi 0901234567");

        let event = Delivered::new(
            h.app.default_tenant(),
            "auth",
            autoxide_kernel::privacy::USER_DELETED,
            json!({"before": {"id": Uuid::from_u128(BUYER)}}),
        );
        service.handle(&event).await.unwrap();
        service.handle(&event).await.unwrap();

        let (_, got) = call(&h.app, "GET", &format!("/threads/{id}"), "agent", None).await;
        assert!(got["data"]["customer_user_id"].is_null(), "{got}");
        assert_eq!(got["data"]["status"], "closed");
        let (_, page) =
            call(&h.app, "GET", &format!("/threads/{id}/messages?order=asc"), "agent", None).await;
        assert_eq!(page["data"][0]["body"], "", "the customer's text is gone");
        assert_eq!(page["data"][1]["body"], "Dạ shop nhận", "staff answers stay");
        let (_, mine) = call(&h.app, "GET", "/me/threads", "buyer", None).await;
        assert!(mine["data"].as_array().unwrap().is_empty());
    })
    .await
    .unwrap();
}
