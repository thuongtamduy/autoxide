//! Hàng đặt trước (D-204): không vượt suất khi đặt đồng thời, hàng về chia theo thứ tự đặt, huỷ
//! trả lại suất.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_catalog::api::SaleMode;
use autoxide_kernel::job::{EnqueueOptions, JobQueue};
use autoxide_kernel::tenancy::OrgId;
use autoxide_testkit::{TestApp, TestDatabase};
use futures_util::future::join_all;
use sea_orm::ConnectionTrait;
use uuid::Uuid;

use super::{assert_consistent, drain, level, shop, stock_in};
use crate::api::{Inventory, ReleaseReason, ReserveLine, ReserveRequest};
use crate::preorders::AllocatePreorders;

fn inventory(app: &TestApp) -> Arc<dyn Inventory> {
    app.api::<dyn Inventory>().unwrap()
}

fn request(org: OrgId, variant: Uuid, quantity: i64) -> ReserveRequest {
    ReserveRequest::new(
        Uuid::now_v7(),
        org,
        Timestamp::now() + SignedDuration::from_mins(30),
        vec![ReserveLine::new("1", variant, quantity)],
    )
}

/// `allocated_quantity` của dòng giữ chỗ thuộc đơn `order`.
async fn allocated(app: &TestApp, order: Uuid) -> i64 {
    let read = app.database().read();
    let backend = read.get_database_backend();
    let sql = match backend {
        sea_orm::DatabaseBackend::Postgres => {
            "SELECT l.allocated_quantity AS n FROM inventory_reservation_lines l \
             JOIN inventory_reservations r ON r.id = l.reservation_id WHERE r.order_id = $1"
        }
        _ => {
            "SELECT l.allocated_quantity AS n FROM inventory_reservation_lines l \
             JOIN inventory_reservations r ON r.id = l.reservation_id WHERE r.order_id = ?"
        }
    };
    let row = read
        .query_one_raw(autoxide_kernel::database::statement(backend, sql, [order.into()]))
        .await
        .unwrap()
        .unwrap();
    row.try_get::<i64>("", "n").unwrap()
}

#[tokio::test]
async fn dat_truoc_khong_vuot_suat_khi_dong_thoi_va_huy_tra_suat() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.add(|variant| {
            variant.sale_mode = SaleMode::Preorder;
            variant.preorder_limit = Some(10);
        });
        let api = inventory(&shop);
        let tenant = shop.default_tenant();
        let attempts = (0..100).map(|_| {
            let (api, tenant) = (api.clone(), tenant.clone());
            let request = request(shop.tree.a, item, 1);
            async move { api.reserve(&tenant, &request).await }
        });
        let outcomes = join_all(attempts).await;
        let won: Vec<_> = outcomes
            .iter()
            .filter_map(|outcome| outcome.as_ref().ok())
            .map(|submitted| submitted.clone().into_inner())
            .collect();
        assert_eq!(won.len(), 10);
        for outcome in &outcomes {
            if let Err(error) = outcome {
                assert_eq!(error.code(), "inventory.preorder_sold_out", "{error}");
            }
        }
        assert!(won.iter().all(|held| held.lines[0].preordered));
        // Huỷ một đơn: suất trở lại, người khác đặt được đúng một.
        let order = won[0].order_id;
        api.release(&tenant, order, ReleaseReason::Cancelled).await.unwrap();
        api.reserve(&tenant, &request(shop.tree.a, item, 1)).await.unwrap();
        let error = api.reserve(&tenant, &request(shop.tree.a, item, 1)).await.unwrap_err();
        assert_eq!(error.code(), "inventory.preorder_sold_out");
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hang_ve_chia_theo_thu_tu_dat() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.add(|variant| variant.sale_mode = SaleMode::Preorder);
        let api = inventory(&shop);
        let tenant = shop.default_tenant();
        let mut orders = Vec::new();
        for quantity in [2, 3, 1] {
            let held = api.reserve(&tenant, &request(shop.tree.a, item, quantity)).await.unwrap();
            orders.push(held.into_inner().order_id);
        }
        // Về 4: đơn đầu đủ 2, đơn hai được 2/3, đơn ba chưa có.
        stock_in(&shop, shop.tree.a, item, 4, None).await;
        drain(&shop).await;
        assert_eq!(allocated(&shop, orders[0]).await, 2);
        assert_eq!(allocated(&shop, orders[1]).await, 2);
        assert_eq!(allocated(&shop, orders[2]).await, 0);
        // Về thêm 5: đơn hai đủ, đơn ba đủ; phần dư không chia cho ai. Chạy lại job không chia hai lần.
        stock_in(&shop, shop.tree.a, item, 5, None).await;
        drain(&shop).await;
        shop_allocate_again(&shop, shop.tree.a, item).await;
        assert_eq!(allocated(&shop, orders[0]).await, 2);
        assert_eq!(allocated(&shop, orders[1]).await, 3);
        assert_eq!(allocated(&shop, orders[2]).await, 1);
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

/// Xếp lại job chia hàng cho dòng tồn của biến thể ở kho A (như job chạy lại) rồi chạy.
async fn shop_allocate_again(app: &TestApp, org: OrgId, variant: Uuid) {
    let found = level(app, org, variant).await;
    let level_id: Uuid = found["id"].as_str().unwrap().parse().unwrap();
    JobQueue::new("inventory")
        .enqueue(
            app.database().write(),
            &app.default_tenant(),
            &AllocatePreorders { level_id },
            EnqueueOptions::default(),
        )
        .await
        .unwrap();
    drain(app).await;
}
