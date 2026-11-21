//! Chuyển kho (đang đi đường, nhận một phần, giá vốn theo kho gửi) và kiểm kê (chênh lệch thành
//! dòng sổ có lý do, kiểm kê khi đang bán, nhập CSV).

use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{TestDatabase, tool};
use axum::http::StatusCode;
use jiff::SignedDuration;
use serde_json::json;
use uuid::Uuid;

use super::{assert_consistent, call, level, shop, stock_in};
use crate::api::{FulfilLine, FulfilRequest, Inventory, ReserveLine, ReserveRequest};

#[tokio::test]
async fn chuyen_kho_dang_di_duong_nhan_mot_phan() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.item();
        stock_in(&shop, shop.tree.a, item, 10, Some(1000)).await;
        stock_in(&shop, shop.tree.b, item, 10, Some(2000)).await;
        // 4 trong 10 đã giữ cho đơn: chỉ chuyển được 6.
        let api = shop.api::<dyn Inventory>().unwrap();
        let tenant = shop.default_tenant();
        let hold = ReserveRequest::new(
            Uuid::now_v7(),
            shop.tree.a,
            Timestamp::now() + SignedDuration::from_mins(30),
            vec![ReserveLine::new("1", item, 4)],
        );
        api.reserve(&tenant, &hold).await.unwrap();

        let body = |quantity: i64| {
            json!({"from_org_id": shop.tree.a.as_uuid(), "to_org_id": shop.tree.b.as_uuid(),
            "lines": [{"variant_id": item, "quantity": quantity}]})
        };
        let (status, too_much) = call(&shop, "POST", "/transfers", "boss", Some(body(7))).await;
        assert_eq!(status, StatusCode::CREATED, "{too_much}");
        let id = too_much["data"]["id"].as_str().unwrap().to_owned();
        let (status, problem) =
            call(&shop, "POST", &format!("/transfers/{id}/ship"), "boss", None).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("inventory.insufficient_stock"))
        );
        let (status, _) =
            call(&shop, "POST", &format!("/transfers/{id}/cancel"), "boss", None).await;
        assert_eq!(status, StatusCode::OK);

        let (_, created) = call(&shop, "POST", "/transfers", "boss", Some(body(6))).await;
        assert!(created["data"]["number"].as_str().unwrap().starts_with("CK-"));
        let id = created["data"]["id"].as_str().unwrap().to_owned();
        // Nhân viên kho B không gửi được hàng của kho A.
        let (status, _) =
            call(&shop, "POST", &format!("/transfers/{id}/ship"), "keeper_b", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, shipped) =
            call(&shop, "POST", &format!("/transfers/{id}/ship"), "keeper_a", None).await;
        assert_eq!(status, StatusCode::OK, "{shipped}");
        assert_eq!(shipped["data"]["status"], "in_transit");
        assert_eq!(shipped["data"]["lines"][0]["unit_cost"]["amount"], "1000");
        let a = level(&shop, shop.tree.a, item).await;
        assert_eq!((a["on_hand"].as_i64(), a["available"].as_i64()), (Some(4), Some(0)));
        assert_eq!(level(&shop, shop.tree.b, item).await["incoming"], 6);
        // Gửi lại: không được.
        let (status, problem) =
            call(&shop, "POST", &format!("/transfers/{id}/ship"), "keeper_a", None).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("inventory.invalid_transition"))
        );

        // Kho A không nhận thay kho B; nhận 2 rồi quá số.
        let receive =
            |quantity: i64| json!({"lines": [{"variant_id": item, "quantity": quantity}]});
        let uri = format!("/transfers/{id}/receive");
        let (status, _) = call(&shop, "POST", &uri, "keeper_a", Some(receive(2))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, part) = call(&shop, "POST", &uri, "keeper_b", Some(receive(2))).await;
        assert_eq!(
            (status, &part["data"]["status"]),
            (StatusCode::OK, &json!("partially_received")),
            "{part}"
        );
        let (status, problem) = call(&shop, "POST", &uri, "keeper_b", Some(receive(5))).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("inventory.over_receipt"))
        );
        let b = level(&shop, shop.tree.b, item).await;
        assert_eq!((b["on_hand"].as_i64(), b["incoming"].as_i64()), (Some(12), Some(4)));
        // Giá vốn ở B: (10 × 2000 + 2 × 1000) / 12 = 1833,3 → 1833.
        assert_eq!(b["average_cost"]["amount"], "1833");
        // Đóng phiếu: 4 chưa tới bỏ khỏi `incoming`.
        let (status, closed) =
            call(&shop, "POST", &format!("/transfers/{id}/close"), "keeper_b", None).await;
        assert_eq!((status, &closed["data"]["status"]), (StatusCode::OK, &json!("closed")));
        assert_eq!(level(&shop, shop.tree.b, item).await["incoming"], 0);
        let (_, list) = call(&shop, "GET", "/transfers?status=closed", "keeper_b", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1);
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn kiem_ke_lech_thanh_dieu_chinh_co_ly_do_va_khi_dang_ban() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let (shirt, cap) = (shop.catalog.item(), shop.catalog.item());
        stock_in(&shop, shop.tree.a, shirt, 10, None).await;
        stock_in(&shop, shop.tree.a, cap, 5, None).await;
        let (status, opened) = call(
            &shop,
            "POST",
            "/counts",
            "keeper_a",
            Some(json!({"org_id": shop.tree.a.as_uuid()})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{opened}");
        let id = opened["data"]["id"].as_str().unwrap().to_owned();
        // Đếm áo: 8 (thiếu 2, mất); mũ: 5 (khớp).
        let lines = json!({"lines": [
            {"variant_id": shirt, "counted": 8, "reason_code": "lost"},
            {"variant_id": cap, "counted": 5},
        ]});
        let (status, counted) =
            call(&shop, "PUT", &format!("/counts/{id}/lines"), "keeper_a", Some(lines)).await;
        assert_eq!(status, StatusCode::OK, "{counted}");
        assert_eq!(counted["data"]["lines"][0]["expected_at_count"], 10);
        // Bán 3 áo xen giữa lúc đếm và lúc áp dụng.
        let api = shop.api::<dyn Inventory>().unwrap();
        let tenant = shop.default_tenant();
        let order = Uuid::now_v7();
        let hold = ReserveRequest::new(
            order,
            shop.tree.a,
            Timestamp::now() + SignedDuration::from_mins(30),
            vec![ReserveLine::new("1", shirt, 3)],
        );
        api.reserve(&tenant, &hold).await.unwrap();
        api.fulfil(&tenant, &FulfilRequest::new(order, "ship-1", vec![FulfilLine::new("1", 3)]))
            .await
            .unwrap();
        // Lý do lạ bị từ chối.
        let bad = json!({"lines": [{"variant_id": cap, "counted": 5, "reason_code": "eaten"}]});
        let (_, problem) =
            call(&shop, "PUT", &format!("/counts/{id}/lines"), "keeper_a", Some(bad)).await;
        assert_eq!(problem["code"], "kernel.reason_unknown");
        // Áp dụng: 10 − 3 bán − 2 lệch = 5 (không ghi đè bằng số đếm 8).
        let (status, applied) =
            call(&shop, "POST", &format!("/counts/{id}/apply"), "keeper_a", None).await;
        assert_eq!(status, StatusCode::OK, "{applied}");
        assert_eq!(applied["data"]["status"], "applied");
        assert_eq!(level(&shop, shop.tree.a, shirt).await["on_hand"], 5);
        assert_eq!(level(&shop, shop.tree.a, cap).await["on_hand"], 5);
        let (_, ledger) = call(&shop, "GET", "/movements?kind=count", "boss", None).await;
        assert_eq!(ledger["data"].as_array().unwrap().len(), 1);
        assert_eq!(
            (ledger["data"][0]["quantity"].as_i64(), ledger["data"][0]["reason_code"].as_str()),
            (Some(-2), Some("lost"))
        );
        // Áp dụng lại: không được.
        let (status, problem) =
            call(&shop, "POST", &format!("/counts/{id}/apply"), "keeper_a", None).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("inventory.invalid_transition"))
        );
        // Kho B không thấy phiếu của kho A.
        let (status, _) = call(&shop, "GET", &format!("/counts/{id}"), "keeper_b", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn kiem_ke_nhap_csv_bao_loi_theo_dong() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.item();
        let sku = format!("SKU-{}", item.simple());
        stock_in(&shop, shop.tree.a, item, 4, None).await;
        let (_, opened) =
            call(&shop, "POST", "/counts", "boss", Some(json!({"org_id": shop.tree.a.as_uuid()})))
                .await;
        let id = opened["data"]["id"].as_str().unwrap().to_owned();
        let uri = format!("/counts/{id}/import");
        let bad = format!("sku,counted\n{sku},x\nNOPE,3\n");
        let (status, problem) = call(&shop, "POST", &uri, "boss", Some(json!({"csv": bad}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        let fields: Vec<&str> = problem["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["field"].as_str().unwrap())
            .collect();
        assert_eq!(fields, ["csv[2].counted", "csv[3].sku"]);
        let good = format!("sku,counted,reason_code\n{sku},6,found\n");
        let (status, counted) = call(&shop, "POST", &uri, "boss", Some(json!({"csv": good}))).await;
        assert_eq!(status, StatusCode::OK, "{counted}");
        call(&shop, "POST", &format!("/counts/{id}/apply"), "boss", None).await;
        assert_eq!(level(&shop, shop.tree.a, item).await["on_hand"], 6);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tool_tao_va_nhan_phieu_chuyen() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.item();
        stock_in(&shop, shop.tree.a, item, 3, None).await;
        let client = shop.mcp("boss").await.unwrap();
        let input = json!({"from_org_id": shop.tree.a.as_uuid(), "to_org_id": shop.tree.b.as_uuid(),
            "lines": [{"variant_id": item, "quantity": 3}]});
        let made = client.call_tool(tool("inventory.create_transfer", input)).await.unwrap();
        let text = format!("{made:?}");
        let id =
            text.split("\\\"id\\\":\\\"").nth(1).unwrap().split("\\\"").next().unwrap().to_owned();
        call(&shop, "POST", &format!("/transfers/{id}/ship"), "boss", None).await;
        let done =
            client.call_tool(tool("inventory.receive_transfer", json!({"id": id}))).await.unwrap();
        assert!(format!("{done:?}").contains("\\\"status\\\":\\\"received\\\""), "{done:?}");
        assert_eq!(level(&shop, shop.tree.b, item).await["on_hand"], 3);
    })
    .await
    .unwrap();
}
