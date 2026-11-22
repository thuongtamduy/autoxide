//! Giữ chỗ: không bán quá tồn khi nhiều request cùng lúc, xác nhận / trả / xuất / hết hạn, chọn
//! kho, combo, bán khi hết hàng, nhập lại hàng trả về.

use super::CatalogExt as _;
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_catalog::api::SaleMode;
use autoxide_kernel::job::{EnqueueOptions, Job, JobQueue};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::OrgId;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use futures_util::future::join_all;
use sea_orm::{ConnectionTrait, Statement};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use super::{
    Emitter, assert_consistent, call, drain, emit, level, raw_event, shop, shop_with, stock_in,
};
use crate::api::{
    FulfilLine, FulfilRequest, Inventory, ReleaseReason, ReservationStatus, ReserveLine,
    ReserveRequest, RestockLine, RestockRequest,
};

fn inventory(app: &TestApp) -> Arc<dyn Inventory> {
    app.api::<dyn Inventory>().unwrap()
}

fn request(org: OrgId, lines: &[(&str, Uuid, i64)]) -> ReserveRequest {
    ReserveRequest::new(
        Uuid::now_v7(),
        org,
        Timestamp::now() + SignedDuration::from_mins(30),
        lines
            .iter()
            .map(|(key, variant, quantity)| ReserveLine::new(*key, *variant, *quantity))
            .collect(),
    )
}

async fn numbers(app: &TestApp, org: OrgId, variant: Uuid) -> (i64, i64) {
    let found = level(app, org, variant).await;
    (found["on_hand"].as_i64().unwrap_or(0), found["available"].as_i64().unwrap_or(0))
}

#[tokio::test]
async fn tram_nguoi_mua_cai_cuoi() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.item();
        stock_in(&shop, shop.tree.a, item, 1, None).await;
        let api = inventory(&shop);
        let tenant = shop.default_tenant();
        let attempts = (0..100).map(|_| {
            let (api, tenant) = (api.clone(), tenant.clone());
            let request = request(shop.tree.a, &[("1", item, 1)]);
            async move { api.reserve(&tenant, &request).await }
        });
        let outcomes = join_all(attempts).await;
        let won = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
        assert_eq!(won, 1);
        for outcome in &outcomes {
            if let Err(error) = outcome {
                assert_eq!(error.code(), "inventory.insufficient_stock", "{error}");
            }
        }
        assert_eq!(numbers(&shop, shop.tree.a, item).await, (1, 0));
        let (_, held) = call(&shop, "GET", "/reservations?status=held", "boss", None).await;
        assert_eq!(held["data"].as_array().unwrap().len(), 1);

        // 100 người mua 3 cái khi còn 250 → đúng 83 người.
        let other = shop.catalog.item();
        stock_in(&shop, shop.tree.a, other, 250, None).await;
        let attempts = (0..100).map(|_| {
            let (api, tenant) = (api.clone(), tenant.clone());
            let request = request(shop.tree.a, &[("1", other, 3)]);
            async move { api.reserve(&tenant, &request).await }
        });
        let won = join_all(attempts).await.iter().filter(|outcome| outcome.is_ok()).count();
        assert_eq!(won, 83);
        assert_eq!(numbers(&shop, shop.tree.a, other).await, (250, 1));
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn giu_nhieu_dong_khong_deadlock() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let (a, b) = (shop.catalog.item(), shop.catalog.item());
        stock_in(&shop, shop.tree.a, a, 100, None).await;
        stock_in(&shop, shop.tree.a, b, 100, None).await;
        let api = inventory(&shop);
        let tenant = shop.default_tenant();
        let attempts = (0..50).map(|index| {
            let (api, tenant) = (api.clone(), tenant.clone());
            let lines = if index % 2 == 0 {
                [("x", a, 1), ("y", b, 1)]
            } else {
                [("x", b, 1), ("y", a, 1)]
            };
            let request = request(shop.tree.a, &lines);
            async move { api.reserve(&tenant, &request).await }
        });
        for outcome in join_all(attempts).await {
            outcome.unwrap();
        }
        assert_eq!(numbers(&shop, shop.tree.a, a).await, (100, 50));
        assert_eq!(numbers(&shop, shop.tree.a, b).await, (100, 50));
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn giu_lai_cung_order_id() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.item();
        stock_in(&shop, shop.tree.a, item, 10, None).await;
        let api = inventory(&shop);
        let tenant = shop.default_tenant();
        let first = request(shop.tree.a, &[("1", item, 2)]);
        let attempts = (0..16).map(|_| {
            let (api, tenant, request) = (api.clone(), tenant.clone(), first.clone());
            async move { api.reserve(&tenant, &request).await }
        });
        let outcomes = join_all(attempts).await;
        let created = outcomes.iter().filter(|o| matches!(o, Ok(Submitted::Created(_)))).count();
        let replayed = outcomes.iter().filter(|o| matches!(o, Ok(Submitted::Replayed(_)))).count();
        assert_eq!((created, replayed), (1, 15), "{outcomes:?}");
        assert_eq!(numbers(&shop, shop.tree.a, item).await, (10, 8));
        // Cùng đơn, khác nội dung.
        let mut other = first.clone();
        other.lines[0].quantity = 3;
        let error = api.reserve(&tenant, &other).await.unwrap_err();
        assert_eq!(error.code(), "reference_reused");
    })
    .await
    .unwrap();
}

#[derive(Debug, Serialize, Deserialize)]
struct ExpireNow {}

impl Job for ExpireNow {
    const KIND: &'static str = crate::contract::EXPIRE_RESERVATIONS;
}

async fn expire_now(app: &TestApp) {
    JobQueue::new("inventory")
        .enqueue_database(app.database().write(), &ExpireNow {}, EnqueueOptions::default())
        .await
        .unwrap();
    drain(app).await;
}

/// Đẩy hạn của mọi chỗ giữ về quá khứ.
async fn age_holds(app: &TestApp) {
    let write = app.database().write();
    let backend = write.get_database_backend();
    let past = (Timestamp::now() - SignedDuration::from_mins(1)).to_rfc3339();
    let sql = match backend {
        sea_orm::DatabaseBackend::Postgres => {
            format!(
                "UPDATE inventory_reservations SET expires_at = '{past}'::timestamptz WHERE status = 'held'"
            )
        }
        _ => {
            format!("UPDATE inventory_reservations SET expires_at = '{past}' WHERE status = 'held'")
        }
    };
    write.execute_raw(Statement::from_string(backend, sql)).await.unwrap();
}

#[tokio::test]
async fn xac_nhan_va_het_han_dua_nhau() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.item();
        stock_in(&shop, shop.tree.a, item, 10, None).await;
        let api = inventory(&shop);
        let tenant = shop.default_tenant();
        let mut orders = Vec::new();
        for _ in 0..5 {
            let held =
                api.reserve(&tenant, &request(shop.tree.a, &[("1", item, 2)])).await.unwrap();
            orders.push(held.into_inner().order_id);
        }
        assert_eq!(numbers(&shop, shop.tree.a, item).await, (10, 0));
        age_holds(&shop).await;
        // Xác nhận và job hết hạn chạy cùng lúc: mỗi đơn đúng một bên thắng.
        let confirms = orders.iter().map(|order| {
            let (api, tenant, order) = (api.clone(), tenant.clone(), *order);
            async move { api.confirm(&tenant, order).await }
        });
        let (confirmed, ()) = tokio::join!(join_all(confirms), expire_now(&shop));
        let mut kept = 0;
        for outcome in confirmed {
            match outcome {
                Ok(found) => {
                    assert_eq!(found.status, ReservationStatus::Confirmed);
                    kept += 1;
                }
                Err(error) => assert_eq!(error.code(), "inventory.reservation_expired"),
            }
        }
        // `available` không bị cộng hai lần: chỉ phần của đơn hết hạn quay về.
        assert_eq!(numbers(&shop, shop.tree.a, item).await, (10, 10 - 2 * kept));
        expire_now(&shop).await;
        assert_eq!(numbers(&shop, shop.tree.a, item).await, (10, 10 - 2 * kept));
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tra_cho_hai_lan_chi_cong_mot_lan() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.item();
        stock_in(&shop, shop.tree.a, item, 5, None).await;
        let api = inventory(&shop);
        let tenant = shop.default_tenant();
        let held = api.reserve(&tenant, &request(shop.tree.a, &[("1", item, 3)])).await.unwrap();
        let order = held.into_inner().order_id;
        api.confirm(&tenant, order).await.unwrap();
        let releases = (0..4).map(|_| {
            let (api, tenant) = (api.clone(), tenant.clone());
            async move { api.release(&tenant, order, ReleaseReason::Cancelled).await }
        });
        for outcome in join_all(releases).await {
            assert_eq!(outcome.unwrap().unwrap().status, ReservationStatus::Released);
        }
        assert_eq!(numbers(&shop, shop.tree.a, item).await, (5, 5));
        // Không có chỗ giữ: `None`; xác nhận chỗ đã trả: lỗi.
        assert!(
            api.release(&tenant, Uuid::now_v7(), ReleaseReason::Cancelled).await.unwrap().is_none()
        );
        let error = api.confirm(&tenant, order).await.unwrap_err();
        assert_eq!(error.code(), "inventory.reservation_expired");
        let error = api.confirm(&tenant, Uuid::now_v7()).await.unwrap_err();
        assert_eq!(error.code(), "inventory.reservation_not_found");
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xuat_mot_phan_roi_huy_phan_con_lai() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.item();
        stock_in(&shop, shop.tree.a, item, 10, None).await;
        let api = inventory(&shop);
        let tenant = shop.default_tenant();
        let order = api
            .reserve(&tenant, &request(shop.tree.a, &[("1", item, 4)]))
            .await
            .unwrap()
            .into_inner()
            .order_id;
        api.confirm(&tenant, order).await.unwrap();
        let ship = FulfilRequest::new(order, "VD-1", vec![FulfilLine::new("1", 1)]);
        let shipped = api.fulfil(&tenant, &ship).await.unwrap();
        assert_eq!(shipped.lines[0].fulfilled_quantity, 1);
        assert_eq!(shipped.status, ReservationStatus::Confirmed);
        // Gửi lại cùng mã: không xuất lần hai.
        api.fulfil(&tenant, &ship).await.unwrap();
        assert_eq!(numbers(&shop, shop.tree.a, item).await, (9, 6));
        // Xuất quá số giữ.
        let error = api
            .fulfil(&tenant, &FulfilRequest::new(order, "VD-2", vec![FulfilLine::new("1", 4)]))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "inventory.over_fulfilment");
        // Huỷ phần còn lại: 3 về tồn bán được, chỗ giữ thành `fulfilled`.
        let released =
            api.release(&tenant, order, ReleaseReason::Cancelled).await.unwrap().unwrap();
        assert_eq!(released.status, ReservationStatus::Fulfilled);
        assert_eq!(released.lines[0].released_quantity, 3);
        assert_eq!(numbers(&shop, shop.tree.a, item).await, (9, 9));
        let (_, ledger) = call(&shop, "GET", "/movements?kind=sale", "boss", None).await;
        assert_eq!(ledger["data"][0]["quantity"], -1);
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn combo_tru_tung_thanh_phan_va_bo_qua_hang_khong_theo_doi() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let (cup, saucer) = (shop.catalog.item(), shop.catalog.item());
        let set = shop.catalog.bundle(&[(cup, 2), (saucer, 1)]);
        let ebook = shop.catalog.add(|variant| variant.track_inventory = false);
        stock_in(&shop, shop.tree.a, cup, 10, None).await;
        stock_in(&shop, shop.tree.a, saucer, 10, None).await;
        let api = inventory(&shop);
        let tenant = shop.default_tenant();
        let held = api
            .reserve(&tenant, &request(shop.tree.a, &[("set", set, 3), ("book", ebook, 1)]))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(held.lines.len(), 2, "{held:?}");
        assert!(held.lines.iter().all(|line| line.bundle_variant_id == Some(set)));
        assert_eq!(numbers(&shop, shop.tree.a, cup).await, (10, 4));
        assert_eq!(numbers(&shop, shop.tree.a, saucer).await, (10, 7));
        // Xuất 1 combo: 2 tách, 1 đĩa.
        let ship = FulfilRequest::new(held.order_id, "s1", vec![FulfilLine::new("set", 1)]);
        api.fulfil(&tenant, &ship).await.unwrap();
        assert_eq!(numbers(&shop, shop.tree.a, cup).await, (8, 4));
        assert_eq!(numbers(&shop, shop.tree.a, saucer).await, (9, 7));
        // Xuất hết phần còn lại: dòng trống.
        let all =
            api.fulfil(&tenant, &FulfilRequest::new(held.order_id, "s2", vec![])).await.unwrap();
        assert_eq!(all.status, ReservationStatus::Fulfilled);
        assert_eq!(numbers(&shop, shop.tree.a, cup).await, (4, 4));
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cho_ban_khi_het_cho_available_am() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.add(|variant| variant.sale_mode = SaleMode::Backorder);
        stock_in(&shop, shop.tree.a, item, 1, None).await;
        let api = inventory(&shop);
        let tenant = shop.default_tenant();
        let held = api
            .reserve(&tenant, &request(shop.tree.a, &[("1", item, 3)]))
            .await
            .unwrap()
            .into_inner();
        assert!(held.lines[0].backordered);
        assert_eq!(numbers(&shop, shop.tree.a, item).await, (1, -2));
        // Xuất được phần có hàng; phần thiếu không xuất được (on_hand không âm).
        let error = api
            .fulfil(&tenant, &FulfilRequest::new(held.order_id, "f", vec![FulfilLine::new("1", 2)]))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "inventory.insufficient_stock");
        api.fulfil(&tenant, &FulfilRequest::new(held.order_id, "f", vec![FulfilLine::new("1", 1)]))
            .await
            .unwrap();
        assert_eq!(numbers(&shop, shop.tree.a, item).await, (0, -2));
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chon_kho_cung_tinh_truoc_va_chia_kho_khi_cho_phep() {
    TestDatabase::run(async |test| {
        let (organizations, outbox) =
            Emitter::new("organizations", &["organizations.public_profile_updated"]);
        let shop = shop_with(test, |builder, _| builder.module(organizations)).await;
        let item = shop.catalog.item();
        stock_in(&shop, shop.tree.a, item, 2, None).await;
        stock_in(&shop, shop.tree.b, item, 2, None).await;
        let store = shop.tree.store.as_uuid();
        // Cửa hàng bán từ A rồi B; B cùng tỉnh với người mua.
        let sources = json!({"sources": [
            {"warehouse_org_id": shop.tree.a.as_uuid(), "priority": 1, "active": true},
            {"warehouse_org_id": shop.tree.b.as_uuid(), "priority": 2, "active": true},
        ]});
        let (status, saved) =
            call(&shop, "PUT", &format!("/sources/{store}"), "boss", Some(sources)).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        let tenant = shop.default_tenant();
        // Hồ sơ công khai của kho B (tỉnh 79), như `organizations` phát; nhận hai lần vẫn vậy.
        raw_event!(ProfileUpdated, "organizations.public_profile_updated");
        let profile = ProfileUpdated(json!({"after": {
            "public": true, "display_name": "B", "pickup": true,
            "address": {"country": "VN", "province_code": "79", "commune_code": "26734"},
        }}));
        let subject = Some(format!("organizations.unit:{}", shop.tree.b.as_uuid()));
        emit(&shop, &outbox, subject.clone(), &profile).await;
        emit(&shop, &outbox, subject, &profile).await;
        drain(&shop).await;
        let api = inventory(&shop);
        // Chưa bật ưu tiên gần: theo thứ tự ưu tiên → A.
        let near = request(shop.tree.store, &[("1", item, 1)]).ship_to("79");
        let held = api.reserve(&tenant, &near).await.unwrap().into_inner();
        assert_eq!(held.lines[0].warehouse_org_id, shop.tree.a);
        let body = json!({"prefer_nearby": true, "allow_split": false});
        let (status, _) =
            call(&shop, "PUT", &format!("/settings/{store}"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        let near = request(shop.tree.store, &[("1", item, 1)]).ship_to("79");
        let held = api.reserve(&tenant, &near).await.unwrap().into_inner();
        assert_eq!(held.lines[0].warehouse_org_id, shop.tree.b);
        // Còn A:1, B:1. Cần 2: một kho không đủ, chưa cho chia → lỗi; cho chia → hai phần.
        let error =
            api.reserve(&tenant, &request(shop.tree.store, &[("1", item, 2)])).await.unwrap_err();
        assert_eq!(error.code(), "inventory.insufficient_stock");
        let body = json!({"prefer_nearby": false, "allow_split": true});
        call(&shop, "PUT", &format!("/settings/{store}"), "boss", Some(body)).await;
        let held = api
            .reserve(&tenant, &request(shop.tree.store, &[("1", item, 2)]))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(held.lines.len(), 2);
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_nguon_thi_loi_va_han_giu_qua_dai() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.item();
        let api = inventory(&shop);
        let tenant = shop.default_tenant();
        // Nút vùng không phải kho và không có kho phục vụ.
        let error =
            api.reserve(&tenant, &request(shop.tree.chain, &[("1", item, 1)])).await.unwrap_err();
        assert_eq!(error.code(), "inventory.no_source");
        let mut long = request(shop.tree.a, &[("1", item, 1)]);
        long.expires_at = Timestamp::now() + SignedDuration::from_hours(100);
        let error = api.reserve(&tenant, &long).await.unwrap_err();
        assert_eq!(error.code(), "inventory.hold_too_long");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhap_lai_hang_tra_ve_mot_lan() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.item();
        let api = inventory(&shop);
        let tenant = shop.default_tenant();
        let back = RestockRequest::new(
            "return:42",
            None,
            shop.tree.a,
            vec![RestockLine::new(item, 2, true), RestockLine::new(item, 1, false)],
        );
        api.restock(&tenant, &back).await.unwrap();
        api.restock(&tenant, &back).await.unwrap();
        assert_eq!(numbers(&shop, shop.tree.a, item).await, (2, 2));
        let mut other = back.clone();
        other.lines.pop();
        assert_eq!(api.restock(&tenant, &other).await.unwrap_err().code(), "reference_reused");
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn go_cho_giu_bang_tay_va_ro_ri_giua_cua_hang() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.item();
        stock_in(&shop, shop.tree.b, item, 3, None).await;
        let api = inventory(&shop);
        let tenant = shop.default_tenant();
        let held = api
            .reserve(&tenant, &request(shop.tree.b, &[("1", item, 2)]))
            .await
            .unwrap()
            .into_inner();
        // Nhân viên kho A không thấy chỗ giữ của B, không gỡ được.
        let (_, list) = call(&shop, "GET", "/reservations", "keeper_a", None).await;
        assert_eq!(list["data"], json!([]));
        let uri = format!("/reservations/{}/release", held.id);
        let (status, _) = call(&shop, "POST", &uri, "keeper_a", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, released) = call(&shop, "POST", &uri, "keeper_b", None).await;
        assert_eq!(status, StatusCode::OK, "{released}");
        assert_eq!(released["data"]["status"], "released");
        assert_eq!(numbers(&shop, shop.tree.b, item).await, (3, 3));
    })
    .await
    .unwrap();
}
