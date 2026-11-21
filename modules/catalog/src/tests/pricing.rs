//! Bảng giá (D-222): kế thừa theo cây, nhóm khách, giá hẹn giờ và tạm thời, giá ngay trước một
//! thời điểm, `reference` chống trùng, sàn giá, giá vốn từ `inventory.cost_changed`.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::OrgId;
use autoxide_testkit::{TestApp, TestDatabase, tool};
use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{app, call, create_product};
use crate::api::{Catalog, FloorAction, PriceQuery};

async fn price_list(app: &TestApp, who: &str, org: OrgId, extra: Value) -> String {
    let mut body = json!({"org_id": org.as_uuid(), "name": "Bảng giá", "currency": "VND", "tax_inclusive": true});
    if let (Value::Object(body), Value::Object(extra)) = (&mut body, extra) {
        body.extend(extra);
    }
    let (status, made) = call(app, "POST", "/price-lists", who, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    made["data"]["id"].as_str().unwrap().to_owned()
}

async fn add(app: &TestApp, list: &str, reference: &str, prices: Value) -> (StatusCode, Value) {
    let body = json!({"reference": reference, "prices": prices});
    call(app, "POST", &format!("/price-lists/{list}/prices"), "boss", Some(body)).await
}

fn vnd(amount: i64) -> Value {
    json!({"amount": amount.to_string(), "currency": "VND"})
}

async fn price_at(app: &TestApp, query: PriceQuery) -> Option<i64> {
    let catalog = app.api::<dyn Catalog>().unwrap();
    let found = catalog.prices(&app.default_tenant(), &query).await.unwrap();
    found.first().map(|price| price.amount.minor())
}

/// Giá của cửa hàng ghi đè giá của chuỗi; cửa hàng không có bảng riêng dùng giá chuỗi; nhóm khách
/// thắng giá chung cùng nút; bảng tắt không áp.
#[tokio::test]
async fn gia_cua_hang_ghi_de_gia_chuoi_va_nhom_khach_thang_gia_chung() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let (_, variant) = create_product(&app, "boss", tree.chain, "P-1", "Hàng").await;
        let id: Uuid = variant.parse().unwrap();
        let chain = price_list(&app, "boss", tree.chain, json!({})).await;
        let store = price_list(&app, "boss", tree.a, json!({})).await;
        let members = price_list(&app, "boss", tree.chain, json!({"customer_group": "vip"})).await;
        let _ = add(&app, &chain, "c1", json!([{"variant_id": id, "amount": vnd(100_000)}])).await;
        let _ = add(&app, &store, "s1", json!([{"variant_id": id, "amount": vnd(90_000)}])).await;
        let _ = add(&app, &members, "m1", json!([{"variant_id": id, "amount": vnd(80_000)}])).await;

        let now = Timestamp::now() + SignedDuration::from_secs(1);
        let ask = |org: OrgId| PriceQuery::new(org, Currency::VND, now, vec![id]);
        assert_eq!(price_at(&app, ask(tree.a)).await, Some(90_000), "store overrides chain");
        assert_eq!(price_at(&app, ask(tree.b)).await, Some(100_000), "store B inherits chain");
        // Nhóm khách ở chuỗi: cửa hàng A vẫn thắng (nút gần nhất trước), cửa hàng B lấy giá nhóm.
        assert_eq!(price_at(&app, ask(tree.a).for_customer_group("vip")).await, Some(90_000));
        assert_eq!(price_at(&app, ask(tree.b).for_customer_group("vip")).await, Some(80_000));
        assert_eq!(price_at(&app, ask(tree.b).for_customer_group("other")).await, Some(100_000));
        // Khác tiền tệ: không có giá.
        let usd = PriceQuery::new(tree.a, Currency::USD, now, vec![id]);
        assert_eq!(price_at(&app, usd).await, None);
        // Tắt bảng của cửa hàng: về giá chuỗi.
        let (status, _) = call(
            &app,
            "PATCH",
            &format!("/price-lists/{store}"),
            "boss",
            Some(json!({"active": false})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(price_at(&app, ask(tree.a)).await, Some(100_000));
        // Endpoint giải giá cho agent: bảng nào thắng, theo thứ tự.
        let uri = format!(
            "/variants/{id}/price?org_id={}&currency=VND&customer_group=vip",
            tree.b.as_uuid()
        );
        let (status, resolved) = call(&app, "GET", &uri, "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{resolved}");
        assert_eq!(resolved["data"]["price"]["price_list_id"], members.as_str());
        assert_eq!(resolved["data"]["candidates"].as_array().unwrap().len(), 2);
    })
    .await
    .unwrap();
}

/// Giá hẹn giờ có hiệu lực đúng lúc; giá tạm thời hết hạn thì giá cũ tự hiệu lực lại; giá ngay
/// trước `at` lấy dòng `< at`, không lấy dòng `= at`; dòng cũ không đổi.
#[tokio::test]
async fn gia_hen_gio_tam_thoi_va_gia_ngay_truoc_thoi_diem() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let (_, variant) = create_product(&app, "boss", tree.a, "P-2", "Hàng").await;
        let id: Uuid = variant.parse().unwrap();
        let list = price_list(&app, "boss", tree.a, json!({})).await;
        let start = Timestamp::now() - SignedDuration::from_hours(24 * 10);
        let sale = Timestamp::now() + SignedDuration::from_hours(24);
        let sale_end = sale + SignedDuration::from_hours(24 * 2);
        let (status, added) = add(
            &app,
            &list,
            "batch-1",
            json!([
                {"variant_id": id, "amount": vnd(200_000), "valid_from": start},
                {"variant_id": id, "amount": vnd(150_000), "compare_at": vnd(200_000), "valid_from": sale, "valid_to": sale_end},
            ]),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{added}");
        let ask = |at| PriceQuery::new(tree.a, Currency::VND, at, vec![id]);
        assert_eq!(price_at(&app, ask(Timestamp::now())).await, Some(200_000));
        assert_eq!(price_at(&app, ask(sale)).await, Some(150_000), "scheduled price starts at valid_from");
        assert_eq!(price_at(&app, ask(sale_end - SignedDuration::from_secs(1))).await, Some(150_000));
        assert_eq!(price_at(&app, ask(sale_end)).await, Some(200_000), "temporary price ended");

        let catalog = app.api::<dyn Catalog>().unwrap();
        let tenant = app.default_tenant();
        let before = catalog.reference_prices(&tenant, &ask(sale)).await.unwrap();
        assert_eq!(before[0].amount.minor(), 200_000, "the row starting exactly at `at` is not before it");
        let during = catalog.reference_prices(&tenant, &ask(sale + SignedDuration::from_hours(1))).await.unwrap();
        assert_eq!(during[0].amount.minor(), 150_000);
        let snapshot = catalog.prices(&tenant, &ask(sale)).await.unwrap();
        assert_eq!(snapshot[0].compare_at, Some(Money::new(200_000, Currency::VND)));

        // Lịch sử: hai dòng, mới trước; thêm giá mới không đổi dòng cũ.
        let _ = add(&app, &list, "batch-2", json!([{"variant_id": id, "amount": vnd(210_000)}])).await;
        let (status, history) =
            call(&app, "GET", &format!("/price-lists/{list}/prices?variant_id={id}"), "boss", None).await;
        assert_eq!(status, StatusCode::OK);
        let rows = history["data"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["amount"]["amount"], "210000");
        assert_eq!(rows[2]["amount"]["amount"], "200000");
        assert_eq!(rows[2]["source"], "manual");
    })
    .await
    .unwrap();
}

/// 16 request thêm giá cùng `reference` cùng lúc: chỉ một lần ghi; cùng `reference` nội dung khác
/// → `reference_reused`.
#[tokio::test]
async fn them_gia_cung_reference_khong_tao_hai_lan() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let (_, variant) = create_product(&app, "boss", tree.a, "P-3", "Hàng").await;
        let list = price_list(&app, "boss", tree.a, json!({})).await;
        let app = Arc::new(app);
        let prices = json!([{"variant_id": variant, "amount": vnd(50_000)}]);
        let tasks: Vec<_> = (0..16)
            .map(|_| {
                let (app, list, prices) = (app.clone(), list.clone(), prices.clone());
                tokio::spawn(async move { add(&app, &list, "same", prices).await })
            })
            .collect();
        let mut ids = Vec::new();
        for task in tasks {
            let (status, body) = task.await.unwrap();
            assert_eq!(status, StatusCode::OK, "{body}");
            ids.push(body["data"]["prices"][0]["id"].as_str().unwrap().to_owned());
        }
        ids.dedup();
        assert_eq!(ids.len(), 1, "every answer names the same row");
        let (_, history) =
            call(&app, "GET", &format!("/price-lists/{list}/prices"), "boss", None).await;
        assert_eq!(history["data"].as_array().unwrap().len(), 1);
        let other = json!([{"variant_id": variant, "amount": vnd(1)}]);
        let (status, problem) = add(&app, &list, "same", other).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("reference_reused"))
        );
        // Sai tiền tệ.
        let wrong = json!([{"variant_id": variant, "amount": {"amount": "5", "currency": "USD"}}]);
        let (status, problem) = add(&app, &list, "usd", wrong).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("catalog.currency_mismatch"))
        );
    })
    .await
    .unwrap();
}

/// Sàn giá (D-222): `block` chặn, `warn` ghi kèm cảnh báo; biên lợi nhuận trên giá vốn từ event
/// `inventory.cost_changed`; quy tắc của danh mục gần nhất thắng quy tắc chung.
#[tokio::test]
async fn gia_duoi_san_bi_chan_hoac_canh_bao() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let (_, variant) = create_product(&app, "boss", tree.a, "P-4", "Hàng").await;
        let id: Uuid = variant.parse().unwrap();
        let list = price_list(&app, "boss", tree.a, json!({})).await;
        let guard = json!({"org_id": tree.chain.as_uuid(), "currency": "VND", "min_price": vnd(10_000), "action": "block"});
        let (status, made) = call(&app, "POST", "/price-guards", "boss", Some(guard)).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let (status, problem) = add(&app, &list, "low", json!([{"variant_id": id, "amount": vnd(9_000)}])).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("catalog.price_below_floor")));

        // Quy tắc `warn` ở cửa hàng (nút gần hơn) theo biên 20% trên giá vốn 50.000.
        let guard = json!({"org_id": tree.a.as_uuid(), "currency": "VND", "min_margin_bp": 2000, "action": "warn"});
        let (status, _) = call(&app, "POST", "/price-guards", "boss", Some(guard)).await;
        assert_eq!(status, StatusCode::CREATED);
        // Giá vốn đến từ event `inventory.cost_changed` (module chưa có ở bước này): ghi thẳng
        // dòng mà bộ nghe event ghi.
        cost(&app, tree.a, id, 50_000).await;
        let catalog = app.api::<dyn Catalog>().unwrap();
        let floors =
            catalog.price_floors(&app.default_tenant(), tree.a, Currency::VND, &[id]).await.unwrap();
        assert_eq!(floors[0].action, FloorAction::Warn);
        assert_eq!(floors[0].floor, Some(Money::new(60_000, Currency::VND)));
        let (status, added) = add(&app, &list, "warned", json!([{"variant_id": id, "amount": vnd(55_000)}])).await;
        assert_eq!(status, StatusCode::OK, "{added}");
        assert_eq!(added["data"]["warnings"][0]["code"], "catalog.price_below_floor");
        // Cửa hàng B chỉ chịu quy tắc `block` của chuỗi.
        let floors =
            catalog.price_floors(&app.default_tenant(), tree.b, Currency::VND, &[id]).await.unwrap();
        assert_eq!(floors[0].action, FloorAction::Block);
    })
    .await
    .unwrap();
}

/// Dòng giá vốn như bộ nghe `inventory.cost_changed` ghi.
async fn cost(app: &TestApp, org: OrgId, variant: Uuid, minor: i64) {
    use sea_orm::{ActiveModelTrait, Set};
    crate::entity::variant_cost::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(app.default_tenant().id().as_uuid()),
        org_id: Set(org.as_uuid()),
        variant_id: Set(variant),
        unit_cost_minor: Set(minor),
        currency: Set("VND".to_owned()),
        updated_at: Set(Timestamp::now()),
    }
    .insert(app.database().write())
    .await
    .unwrap();
}

/// Bảng giá bán cho người tiêu dùng; MCP `resolve_price` như REST.
#[tokio::test]
async fn resolve_price_qua_mcp() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let (_, variant) = create_product(&app, "boss", tree.a, "P-5", "Hàng").await;
        let list = price_list(&app, "boss", tree.a, json!({})).await;
        let _ =
            add(&app, &list, "r", json!([{"variant_id": variant, "amount": vnd(42_000)}])).await;
        let client = app.mcp("boss").await.unwrap();
        let answer = client
            .call_tool(tool(
                "catalog.resolve_price",
                json!({"variant_id": variant, "org_id": tree.a.as_uuid(), "currency": "VND"}),
            ))
            .await
            .unwrap();
        let answer = answer.structured_content.unwrap();
        assert_eq!(answer["data"]["price"]["amount"]["amount"], "42000", "{answer}");
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
