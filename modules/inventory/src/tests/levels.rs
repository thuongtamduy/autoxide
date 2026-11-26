//! Dòng tồn, sổ biến động, điều chỉnh có lý do, giá vốn bình quân, cảnh báo tồn thấp, rò rỉ.

use super::CatalogExt as _;
use autoxide_testkit::{TestApp, TestDatabase, branch_leak_check, leak_check, tool};
use axum::http::StatusCode;
use sea_orm::ConnectionTrait;
use serde_json::json;
use uuid::Uuid;

use super::{ALL, assert_consistent, call, level, shop, stock_in};

/// Số event `name` trong outbox của module.
async fn events_named(app: &TestApp, name: &str) -> i64 {
    let read = app.database().read();
    let backend = read.get_database_backend();
    let row = read
        .query_one_raw(autoxide_kernel::database::statement(
            backend,
            match backend {
                sea_orm::DatabaseBackend::Postgres => {
                    "SELECT COUNT(*) AS n FROM inventory_events WHERE name = $1"
                }
                _ => "SELECT COUNT(*) AS n FROM inventory_events WHERE name = ?",
            },
            [name.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get::<i64>("", "n").unwrap()
}

#[tokio::test]
async fn dieu_chinh_co_ly_do_ghi_so_va_chay_mot_lan() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.item();
        let a = shop.tree.a.as_uuid();
        stock_in(&shop, shop.tree.a, item, 10, None).await;

        // Thiếu lý do: một lỗi gom mọi dòng.
        let body = json!({"org_id": a, "reference": "adj-1", "lines": [
            {"variant_id": item, "quantity": -2},
            {"variant_id": item, "quantity": 0, "reason_code": "damaged"},
        ]});
        let (status, problem) = call(&shop, "POST", "/adjustments", "boss", Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["code"], "inventory.reason_required");
        assert_eq!(problem["errors"].as_array().unwrap().len(), 2, "{problem}");

        // Lý do không có trong danh sách.
        let body = json!({"org_id": a, "reference": "adj-1", "lines": [
            {"variant_id": item, "quantity": -2, "reason_code": "eaten"},
        ]});
        let (status, problem) = call(&shop, "POST", "/adjustments", "boss", Some(body)).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("kernel.reason_unknown"))
        );

        // Đúng: trừ 2 hàng hỏng.
        let body = json!({"org_id": a, "reference": "adj-1", "lines": [
            {"variant_id": item, "quantity": -2, "reason_code": "damaged", "note": "Rơi vỡ"},
        ]});
        let (status, made) = call(&shop, "POST", "/adjustments", "boss", Some(body.clone())).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        assert_eq!(made["data"]["movements"][0]["quantity"], -2);
        assert_eq!(made["data"]["movements"][0]["reason_code"], "damaged");
        // Gửi lại cùng nội dung: không trừ lần hai.
        let (status, again) = call(&shop, "POST", "/adjustments", "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(again["data"]["id"], made["data"]["id"]);
        // Cùng mã, khác nội dung.
        let body = json!({"org_id": a, "reference": "adj-1", "lines": [
            {"variant_id": item, "quantity": -3, "reason_code": "damaged"},
        ]});
        let (status, problem) = call(&shop, "POST", "/adjustments", "boss", Some(body)).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("reference_reused"))
        );

        let found = level(&shop, shop.tree.a, item).await;
        assert_eq!((found["on_hand"].as_i64(), found["available"].as_i64()), (Some(8), Some(8)));
        let id = made["data"]["id"].as_str().unwrap();
        let (status, got) = call(&shop, "GET", &format!("/adjustments/{id}"), "boss", None).await;
        assert_eq!((status, &got["data"]["reference"]), (StatusCode::OK, &json!("adj-1")));

        // Không xuống dưới 0.
        let body = json!({"org_id": a, "reference": "adj-2", "lines": [
            {"variant_id": item, "quantity": -9, "reason_code": "lost"},
        ]});
        let (status, problem) = call(&shop, "POST", "/adjustments", "boss", Some(body)).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("inventory.insufficient_stock")),
            "{problem}"
        );

        // Sổ: mới trước; lọc theo loại.
        let (_, ledger) =
            call(&shop, "GET", &format!("/movements?org_id={a}&kind=adjustment"), "boss", None)
                .await;
        let quantities: Vec<i64> = ledger["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line["quantity"].as_i64().unwrap())
            .collect();
        assert_eq!(quantities, vec![-2, 10]);
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gia_von_binh_quan_gia_quyen() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.item();
        stock_in(&shop, shop.tree.a, item, 10, Some(1000)).await;
        // 10 × 1000 + 5 × 1300 = 16 500 / 15 = 1100.
        stock_in(&shop, shop.tree.a, item, 5, Some(1300)).await;
        let found = level(&shop, shop.tree.a, item).await;
        assert_eq!(found["average_cost"], json!({"amount": "1100", "currency": "VND"}));
        // 15 × 1100 + 4 × 1001 = 20 504 / 19 = 1079,16… → 1079.
        stock_in(&shop, shop.tree.a, item, 4, Some(1001)).await;
        let found = level(&shop, shop.tree.a, item).await;
        assert_eq!(found["average_cost"]["amount"], "1079");
        // Hàng thêm không giá (vd tìm thấy) giữ nguyên giá vốn.
        stock_in(&shop, shop.tree.a, item, 1, None).await;
        assert_eq!(level(&shop, shop.tree.a, item).await["average_cost"]["amount"], "1079");
        // Giá trị tồn: 20 × 1079 = 21 580.
        let (_, report) = call(&shop, "GET", "/reports/valuation", "boss", None).await;
        assert_eq!(report["data"][0]["units"], 20, "{report}");
        assert_eq!(report["data"][0]["value"], json!({"amount": "21580", "currency": "VND"}));
        // Ba lần đổi giá vốn → ba event `inventory.cost_changed` cho `catalog`.
        assert_eq!(events_named(&shop, "inventory.cost_changed").await, 3);
        // Khác tiền tệ khi đang có tồn.
        let body = json!({"org_id": shop.tree.a.as_uuid(), "reference": "usd", "lines": [
            {"variant_id": item, "quantity": 1, "reason_code": "found",
             "unit_cost": {"amount": "100", "currency": "USD"}},
        ]});
        let (status, problem) = call(&shop, "POST", "/adjustments", "boss", Some(body)).await;
        assert_eq!(problem["code"], "inventory.currency_mismatch", "{status} {problem}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn on_hand_bang_tong_so() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let items = [shop.catalog.item(), shop.catalog.item(), shop.catalog.item()];
        let orgs = [shop.tree.a, shop.tree.b];
        // Chuỗi ngẫu nhiên có seed cố định (LCG): cộng / trừ, có lần bị từ chối vì không đủ.
        let mut seed: u64 = 42;
        let mut next = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            seed >> 33
        };
        for step in 0..40 {
            let item = items[(next() % 3) as usize];
            let org = orgs[(next() % 2) as usize];
            let quantity = (next() % 20) as i64 - 8;
            let quantity = if quantity == 0 { 1 } else { quantity };
            let reason = if quantity > 0 { "found" } else { "lost" };
            let body = json!({"org_id": org.as_uuid(), "reference": format!("r-{step}"), "lines": [
                {"variant_id": item, "quantity": quantity, "reason_code": reason},
            ]});
            let (status, answer) = call(&shop, "POST", "/adjustments", "boss", Some(body)).await;
            assert!(
                status == StatusCode::CREATED || answer["code"] == "inventory.insufficient_stock",
                "{status} {answer}"
            );
        }
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chi_kho_hay_cua_hang_giu_ton() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.item();
        // Nút vùng không giữ tồn.
        let body = json!({"org_id": shop.tree.chain.as_uuid(), "reference": "x", "lines": [
            {"variant_id": item, "quantity": 1, "reason_code": "found"},
        ]});
        let (status, problem) = call(&shop, "POST", "/adjustments", "boss", Some(body)).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("inventory.not_a_warehouse")));
        // Cửa hàng và nút gốc giữ được.
        stock_in(&shop, shop.tree.store, item, 1, None).await;
        stock_in(&shop, autoxide_kernel::tenancy::root_of(autoxide_kernel::tenancy::DEFAULT_TENANT), item, 1, None).await;
        // Biến thể không theo dõi tồn (combo) và biến thể không có.
        let bundle = shop.catalog.bundle(&[(item, 2)]);
        for (variant, code) in [(bundle, "inventory.not_tracked"), (Uuid::now_v7(), "inventory.variant_not_found")] {
            let body = json!({"org_id": shop.tree.a.as_uuid(), "reference": format!("v-{variant}"), "lines": [
                {"variant_id": variant, "quantity": 1, "reason_code": "found"},
            ]});
            let (_, problem) = call(&shop, "POST", "/adjustments", "boss", Some(body)).await;
            assert_eq!(problem["code"], code, "{problem}");
        }
        let _ = &shop.orgs;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn canh_bao_ton_thap_chi_khi_vuot_nguong() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.item();
        stock_in(&shop, shop.tree.a, item, 10, None).await;
        let id = level(&shop, shop.tree.a, item).await["id"].as_str().unwrap().to_owned();
        let (status, set) = call(
            &shop,
            "PATCH",
            &format!("/levels/{id}"),
            "boss",
            Some(json!({"reorder_point": 5})),
        )
        .await;
        assert_eq!((status, &set["data"]["reorder_point"]), (StatusCode::OK, &json!(5)));
        let take = |n: i64, reference: &str| {
            json!({"org_id": shop.tree.a.as_uuid(), "reference": reference, "lines": [
                {"variant_id": item, "quantity": -n, "reason_code": "sample_use"},
            ]})
        };
        // 10 → 7: trên ngưỡng, không cảnh báo.
        call(&shop, "POST", "/adjustments", "boss", Some(take(3, "t1"))).await;
        assert_eq!(events_named(&shop, "inventory.low_stock").await, 0);
        // 7 → 4: vượt ngưỡng, một cảnh báo; 4 → 3: đã dưới ngưỡng, không cảnh báo nữa.
        call(&shop, "POST", "/adjustments", "boss", Some(take(3, "t2"))).await;
        call(&shop, "POST", "/adjustments", "boss", Some(take(1, "t3"))).await;
        assert_eq!(events_named(&shop, "inventory.low_stock").await, 1);
        let (_, low) = call(&shop, "GET", "/levels?below_reorder=true", "boss", None).await;
        assert_eq!(low["data"].as_array().unwrap().len(), 1);
        // Về 0: hết hàng.
        call(&shop, "POST", "/adjustments", "boss", Some(take(3, "t4"))).await;
        assert_eq!(events_named(&shop, "inventory.out_of_stock").await, 1);
        assert_eq!(events_named(&shop, "inventory.level_changed").await, 5);
    })
    .await
    .unwrap();
}

/// Nhân viên kho A (độ sâu `unit`) không đọc / điều chỉnh tồn kho B; khách khác không thấy gì.
#[tokio::test]
async fn ro_ri_giua_hai_kho_anh_em() {
    TestDatabase::run(async |test| {
        let shop = super::shop_with(test, |builder, _| {
            builder.tenant("other").user_in("other", "stranger", 9, &ALL)
        })
        .await;
        let item = shop.catalog.item();
        let made = stock_in(&shop, shop.tree.b, item, 7, None).await;
        let level_b = level(&shop, shop.tree.b, item).await["id"].as_str().unwrap().to_owned();
        let adjustment = made["data"]["id"].as_str().unwrap().to_owned();
        let movement = made["data"]["movements"][0]["id"].as_str().unwrap().to_owned();

        let (status, _) = call(&shop, "GET", &format!("/levels/{level_b}"), "keeper_a", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (_, list) = call(&shop, "GET", "/levels", "keeper_a", None).await;
        assert_eq!(list["data"], json!([]));
        let body = json!({"org_id": shop.tree.b.as_uuid(), "reference": "steal", "lines": [
            {"variant_id": item, "quantity": -1, "reason_code": "lost"},
        ]});
        let (status, problem) = call(&shop, "POST", "/adjustments", "keeper_a", Some(body)).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("org_out_of_scope"))
        );
        let (status, _) = call(
            &shop,
            "PATCH",
            &format!("/levels/{level_b}"),
            "keeper_a",
            Some(json!({"reorder_point": 1})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // keeper_b thấy.
        let (_, list) = call(&shop, "GET", "/levels", "keeper_b", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1);

        let ids = [
            ("id", level_b.clone()),
            ("id", adjustment.clone()),
            ("level_id", level_b.clone()),
            ("variant_id", item.to_string()),
        ];
        let marker = movement.as_str();
        assert_eq!(
            branch_leak_check(&shop, "keeper_a", marker, &ids).await.unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            leak_check(&shop, "stranger", marker, &ids).await.unwrap(),
            Vec::<String>::new()
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tool_mcp_nhu_rest() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.item();
        let client = shop.mcp("boss").await.unwrap();
        let input = json!({"org_id": shop.tree.a.as_uuid(), "reference": "mcp-1", "lines": [
            {"variant_id": item, "quantity": 4, "reason_code": "found"},
        ]});
        let done = client.call_tool(tool("inventory.adjust_stock", input)).await.unwrap();
        assert_ne!(done.is_error, Some(true), "{done:?}");
        let found = client
            .call_tool(tool("inventory.get_levels", json!({"variant_id": item})))
            .await
            .unwrap();
        let text = format!("{found:?}");
        assert!(text.contains("\\\"on_hand\\\":4"), "{text}");
    })
    .await
    .unwrap();
}

/// Bước 70 P9 (D-20261125-list-pages): giá trị tồn là một trang theo kho rồi tiền tệ; theo `next_cursor` tới
/// hết không trùng không sót, `order=desc` ngược lại.
#[tokio::test]
async fn gia_tri_ton_phan_trang_theo_kho() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let item = shop.catalog.item();
        for org in [shop.tree.a, shop.tree.b, shop.tree.store] {
            stock_in(&shop, org, item, 2, Some(1000)).await;
        }
        let mut orgs: Vec<String> = [shop.tree.a, shop.tree.b, shop.tree.store]
            .iter()
            .map(|org| org.as_uuid().to_string())
            .collect();
        orgs.sort();
        for (order, expected) in
            [("asc", orgs.clone()), ("desc", orgs.iter().rev().cloned().collect())]
        {
            let first = format!("/reports/valuation?limit=2&order={order}");
            let mut uri = first.clone();
            let mut seen = Vec::new();
            loop {
                let (status, page) = call(&shop, "GET", &uri, "boss", None).await;
                assert_eq!(status, StatusCode::OK, "{page}");
                let rows = page["data"].as_array().unwrap();
                assert!(rows.len() <= 2);
                seen.extend(rows.iter().map(|row| row["org_id"].as_str().unwrap().to_owned()));
                match page["pagination"]["next_cursor"].as_str() {
                    Some(cursor) => uri = format!("{first}&cursor={cursor}"),
                    None => break,
                }
            }
            assert_eq!(seen, expected, "{order}");
        }
    })
    .await
    .unwrap();
}
