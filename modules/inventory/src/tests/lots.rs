//! Lô và hạn dùng (hết hạn trước xuất trước, job trừ lô quá hạn, `expiry_state`), serial (không
//! bán hai lần dù xuất cùng lúc), chuyển kho giữ lô và serial.

use std::sync::Arc;

use autoxide_kernel::job::{EnqueueOptions, Job, JobQueue};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{assert_consistent, call, drain, level, shop};
use crate::api::{FulfilLine, FulfilRequest, Inventory, ReserveLine, ReserveRequest};

fn day(offset: i64) -> String {
    (Utc::now().date_naive() + Duration::days(offset)).to_string()
}

/// Thêm hàng có lô / serial bằng điều chỉnh `found`.
async fn add(app: &TestApp, org: Uuid, line: Value) -> (StatusCode, Value) {
    let body = json!({"org_id": org, "reference": format!("r-{}", Uuid::now_v7().simple()), "lines": [line]});
    call(app, "POST", "/adjustments", "boss", Some(body)).await
}

async fn lots(app: &TestApp, query: &str) -> Vec<Value> {
    let (status, found) = call(app, "GET", &format!("/lots?{query}"), "boss", None).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    found["data"].as_array().unwrap().clone()
}

fn reserve(org: autoxide_kernel::tenancy::OrgId, variant: Uuid, quantity: i64) -> ReserveRequest {
    ReserveRequest::new(
        Uuid::now_v7(),
        org,
        Utc::now() + Duration::minutes(30),
        vec![ReserveLine::new("1", variant, quantity)],
    )
}

#[tokio::test]
async fn lo_het_han_truoc_xuat_truoc_va_job_tru_lo_qua_han() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let milk = shop.catalog.add(|variant| variant.lot_tracked = true);
        let a = shop.tree.a.as_uuid();
        // Thêm hàng theo lô mà không nêu lô: lỗi.
        let (status, problem) =
            add(&shop, a, json!({"variant_id": milk, "quantity": 5, "reason_code": "found"})).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        for (code, expires, quantity) in [("L-LATE", 60, 5), ("L-SOON", 10, 3), ("L-OLD", -1, 2)] {
            let line = json!({"variant_id": milk, "quantity": quantity, "reason_code": "found",
                              "lot_code": code, "expires_on": day(expires)});
            let (status, made) = add(&shop, a, line).await;
            assert_eq!(status, StatusCode::CREATED, "{made}");
        }
        assert_eq!(level(&shop, shop.tree.a, milk).await["on_hand"], 10);
        let states: Vec<(String, String)> = lots(&shop, &format!("org_id={a}"))
            .await
            .iter()
            .map(|lot| {
                (
                    lot["lot_code"].as_str().unwrap().to_owned(),
                    lot["expiry_state"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert!(states.contains(&("L-SOON".to_owned(), "expiring".to_owned())), "{states:?}");
        assert!(states.contains(&("L-LATE".to_owned(), "ok".to_owned())));
        assert!(states.contains(&("L-OLD".to_owned(), "expired".to_owned())));
        assert_eq!(lots(&shop, "expiry_state=expired").await.len(), 1);

        // Xuất 4: lấy 3 của lô sắp hết hạn rồi 1 của lô sau; không bao giờ lấy lô đã hết hạn.
        let api = shop.api::<dyn Inventory>().unwrap();
        let tenant = shop.default_tenant();
        let held = api.reserve(&tenant, &reserve(shop.tree.a, milk, 4)).await.unwrap().into_inner();
        api.fulfil(
            &tenant,
            &FulfilRequest::new(held.order_id, "s1", vec![FulfilLine::new("1", 4)]),
        )
        .await
        .unwrap();
        let on_hand = |code: &str, lots: &[Value]| {
            lots.iter().find(|lot| lot["lot_code"] == code).unwrap()["on_hand"].as_i64().unwrap()
        };
        let after = lots(&shop, &format!("org_id={a}")).await;
        assert_eq!(
            (on_hand("L-SOON", &after), on_hand("L-LATE", &after), on_hand("L-OLD", &after)),
            (0, 4, 2)
        );
        // Nêu lô đã hết hạn: không được.
        let held = api.reserve(&tenant, &reserve(shop.tree.a, milk, 1)).await.unwrap().into_inner();
        let error = api
            .fulfil(
                &tenant,
                &FulfilRequest::new(
                    held.order_id,
                    "s2",
                    vec![FulfilLine::new("1", 1).lot("L-OLD")],
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), "inventory.lot_unavailable");
        api.release(&tenant, held.order_id, crate::api::ReleaseReason::Cancelled).await.unwrap();

        // Job hằng ngày: lô quá hạn bị trừ khỏi tồn, chạy lại không trừ lần hai.
        expire_lots(&shop).await;
        expire_lots(&shop).await;
        let found = level(&shop, shop.tree.a, milk).await;
        assert_eq!((found["on_hand"].as_i64(), found["available"].as_i64()), (Some(4), Some(4)));
        let (_, ledger) = call(&shop, "GET", "/movements?kind=expiry", "boss", None).await;
        assert_eq!(ledger["data"].as_array().unwrap().len(), 1);
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[derive(Debug, Serialize, Deserialize)]
struct ExpireLotsNow {}

impl Job for ExpireLotsNow {
    const KIND: &'static str = crate::contract::EXPIRE_LOTS;
}

async fn expire_lots(app: &TestApp) {
    JobQueue::new("inventory")
        .enqueue_database(app.database().write(), &ExpireLotsNow {}, EnqueueOptions::default())
        .await
        .unwrap();
    drain(app).await;
}

#[tokio::test]
async fn serial_khong_ban_hai_lan() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let phone = shop.catalog.add(|variant| variant.serial_tracked = true);
        let a = shop.tree.a.as_uuid();
        // Thiếu serial: lỗi; đủ 3 serial: được.
        let (status, _) = add(
            &shop,
            a,
            json!({"variant_id": phone, "quantity": 3, "reason_code": "found", "serials": ["S1"]}),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, made) = add(
            &shop,
            a,
            json!({"variant_id": phone, "quantity": 3, "reason_code": "found",
                                                   "serials": ["S1", "S2", "S3"]}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let api = shop.api::<dyn Inventory>().unwrap();
        let tenant = shop.default_tenant();
        let first =
            api.reserve(&tenant, &reserve(shop.tree.a, phone, 1)).await.unwrap().into_inner();
        let second =
            api.reserve(&tenant, &reserve(shop.tree.a, phone, 1)).await.unwrap().into_inner();
        // Xuất thiếu serial: lỗi.
        let error = api
            .fulfil(
                &tenant,
                &FulfilRequest::new(first.order_id, "x", vec![FulfilLine::new("1", 1)]),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), "inventory.serials_required");
        // Hai đơn cùng xuất S2 cùng lúc: đúng một đơn được.
        let ship = |order: Uuid| {
            FulfilRequest::new(
                order,
                "ship",
                vec![FulfilLine::new("1", 1).serials(vec!["S2".to_owned()])],
            )
        };
        let attempts = [first.order_id, second.order_id].map(|order| {
            let (api, tenant, request) = (Arc::clone(&api), tenant.clone(), ship(order));
            async move { api.fulfil(&tenant, &request).await }
        });
        let outcomes = join_all(attempts).await;
        assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1, "{outcomes:?}");
        let error = outcomes.into_iter().find_map(Result::err).unwrap();
        assert_eq!(error.code(), "inventory.serial_unavailable");
        assert_eq!(level(&shop, shop.tree.a, phone).await["on_hand"], 2);
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chuyen_kho_giu_lo_va_serial() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let both = shop.catalog.add(|variant| {
            variant.lot_tracked = true;
            variant.serial_tracked = true;
        });
        let (a, b) = (shop.tree.a.as_uuid(), shop.tree.b.as_uuid());
        for (code, expires, serials) in [("SOON", 5, ["X1", "X2"]), ("LATE", 50, ["X3", "X4"])] {
            let line = json!({"variant_id": both, "quantity": 2, "reason_code": "found", "lot_code": code,
                              "expires_on": day(expires), "serials": serials});
            let (status, made) = add(&shop, a, line).await;
            assert_eq!(status, StatusCode::CREATED, "{made}");
        }
        let body = json!({"from_org_id": a, "to_org_id": b, "lines": [{"variant_id": both, "quantity": 3, "serials": ["X1", "X2", "X3"]}]});
        let (_, created) = call(&shop, "POST", "/transfers", "boss", Some(body)).await;
        let id = created["data"]["id"].as_str().unwrap().to_owned();
        let (status, shipped) = call(&shop, "POST", &format!("/transfers/{id}/ship"), "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{shipped}");
        // Nhận 2 rồi 1: kho B có lô SOON (2) và LATE (1), serial X1–X3 ở B.
        let part = json!({"lines": [{"variant_id": both, "quantity": 2}]});
        call(&shop, "POST", &format!("/transfers/{id}/receive"), "boss", Some(part)).await;
        let (status, done) = call(&shop, "POST", &format!("/transfers/{id}/receive"), "boss", Some(json!({}))).await;
        assert_eq!((status, &done["data"]["status"]), (StatusCode::OK, &json!("received")), "{done}");
        let at_b: Vec<(String, i64)> = lots(&shop, &format!("org_id={b}")).await.iter()
            .map(|lot| (lot["lot_code"].as_str().unwrap().to_owned(), lot["on_hand"].as_i64().unwrap()))
            .collect();
        assert!(at_b.contains(&("SOON".to_owned(), 2)) && at_b.contains(&("LATE".to_owned(), 1)), "{at_b:?}");
        // X3 bán được ở B, X4 vẫn ở A.
        let api = shop.api::<dyn Inventory>().unwrap();
        let tenant = shop.default_tenant();
        let held = api.reserve(&tenant, &reserve(shop.tree.b, both, 1)).await.unwrap().into_inner();
        let ship = FulfilRequest::new(held.order_id, "s", vec![FulfilLine::new("1", 1).serials(vec!["X4".to_owned()])]);
        assert_eq!(api.fulfil(&tenant, &ship).await.unwrap_err().code(), "inventory.serial_unavailable");
        let ship = FulfilRequest::new(held.order_id, "s", vec![FulfilLine::new("1", 1).serials(vec!["X3".to_owned()])]);
        api.fulfil(&tenant, &ship).await.unwrap();
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}
