//! Bảng giá suy ra, quy tắc làm tròn, đổi giá hàng loạt có xem trước và duyệt (D-222).

use autoxide_kernel::money::Currency;
use autoxide_kernel::tenancy::OrgId;
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use jiff::SignedDuration;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{app, call, create_product};
use crate::api::{Catalog, PriceQuery};

async fn run_jobs(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap();
    while worker.run_once().await.unwrap() > 0 {}
}

async fn list(app: &TestApp, org: OrgId, extra: Value) -> String {
    let mut body =
        json!({"org_id": org.as_uuid(), "name": "Bảng", "currency": "VND", "tax_inclusive": true});
    if let (Value::Object(body), Value::Object(extra)) = (&mut body, extra) {
        body.extend(extra);
    }
    let (status, made) = call(app, "POST", "/price-lists", "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    made["data"]["id"].as_str().unwrap().to_owned()
}

fn vnd(amount: i64) -> Value {
    json!({"amount": amount.to_string(), "currency": "VND"})
}

async fn history(app: &TestApp, list: &str) -> Vec<Value> {
    let (_, rows) =
        call(app, "GET", &format!("/price-lists/{list}/prices?limit=100"), "boss", None).await;
    rows["data"].as_array().unwrap().clone()
}

#[tokio::test]
async fn bang_suy_ra_ghi_dong_gia_that_va_khong_nhan_doi() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let (_, variant) = create_product(&app, "boss", tree.a, "D-1", "Hàng").await;
        let base = list(&app, tree.chain, json!({})).await;
        let body = json!({"reference": "base", "prices": [{"variant_id": variant, "amount": vnd(121_500)}]});
        let (status, _) = call(&app, "POST", &format!("/price-lists/{base}/prices"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        let rule = json!({"name": "Đuôi 9.000", "step_minor": 10_000, "mode": "up", "ending_minor": 9_000});
        let (status, rule) = call(&app, "POST", "/rounding-rules", "boss", Some(rule)).await;
        assert_eq!(status, StatusCode::CREATED, "{rule}");
        // Bảng suy ra cho nhóm khách `vip`: −10%, làm tròn lên đuôi 9.000.
        let derived = list(
            &app,
            tree.chain,
            json!({"customer_group": "vip", "derived_from": base, "adjust_bp": -1000, "rounding_rule_id": rule["data"]["id"]}),
        )
        .await;
        // Bảng suy ra không làm bảng gốc của bảng khác.
        let body = json!({"name": "x", "currency": "VND", "tax_inclusive": true, "derived_from": derived});
        let (status, problem) = call(&app, "POST", "/price-lists", "boss", Some(body)).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("catalog.derived_chain")));

        run_jobs(&app).await;
        let rows = history(&app, &derived).await;
        assert_eq!(rows.len(), 1, "{rows:?}");
        // 121.500 × 0,9 = 109.350, làm tròn lên dạng k × 10.000 + 9.000 → 119.000.
        assert_eq!(rows[0]["amount"]["amount"], "119000");
        assert_eq!(rows[0]["source"], "derived");
        // Chạy lại job (thêm giá mới ở bảng gốc xếp job lần nữa): dòng cũ không nhân đôi.
        let body = json!({"reference": "base-2", "prices": [{"variant_id": variant, "amount": vnd(200_000), "valid_from": Timestamp::now() + SignedDuration::from_hours(24)}]});
        let (status, _) = call(&app, "POST", &format!("/price-lists/{base}/prices"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        run_jobs(&app).await;
        run_jobs(&app).await;
        let rows = history(&app, &derived).await;
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0]["amount"]["amount"], "189000", "200.000 × 0,9 = 180.000 → 189.000");
        // Đọc giá qua `dyn Catalog`: nhóm `vip` lấy bảng suy ra.
        let catalog = app.api::<dyn Catalog>().unwrap();
        let id: Uuid = variant.parse().unwrap();
        let query = PriceQuery::new(tree.a, Currency::VND, Timestamp::now() + SignedDuration::from_secs(1), vec![id])
            .for_customer_group("vip");
        let found = catalog.prices(&app.default_tenant(), &query).await.unwrap();
        assert_eq!(found[0].amount.minor(), 119_000);
    })
    .await
    .unwrap();
}

/// Dòng tay đang hiệu lực trong bảng suy ra ghim giá: job bỏ qua biến thể đó.
#[tokio::test]
async fn dong_tay_ghim_gia_trong_bang_suy_ra() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let (_, variant) = create_product(&app, "boss", tree.a, "PIN-1", "Hàng").await;
        let base = list(&app, tree.chain, json!({})).await;
        let derived = list(&app, tree.chain, json!({"customer_group": "vip", "derived_from": base, "adjust_bp": -2000})).await;
        let pin = json!({"reference": "pin", "prices": [{"variant_id": variant, "amount": vnd(55_000), "valid_from": Timestamp::now() - SignedDuration::from_hours(24)}]});
        let (status, _) = call(&app, "POST", &format!("/price-lists/{derived}/prices"), "boss", Some(pin)).await;
        assert_eq!(status, StatusCode::OK);
        let body = json!({"reference": "base", "prices": [{"variant_id": variant, "amount": vnd(100_000)}]});
        let (status, _) = call(&app, "POST", &format!("/price-lists/{base}/prices"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        run_jobs(&app).await;
        let rows = history(&app, &derived).await;
        assert_eq!(rows.len(), 1, "only the manual row: {rows:?}");
        assert_eq!(rows[0]["source"], "manual");
    })
    .await
    .unwrap();
}

/// Xem trước không ghi giá; duyệt thì job ghi `bulk`; giá tạm thời hết hạn thì giá cũ trở lại;
/// chưa xem trước thì không duyệt được.
#[tokio::test]
async fn doi_gia_hang_loat_xem_truoc_khong_ghi_va_gia_tam_thoi() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let (_, one) = create_product(&app, "boss", tree.a, "B-1", "Một").await;
        let (_, two) = create_product(&app, "boss", tree.a, "B-2", "Hai").await;
        let retail = list(&app, tree.a, json!({})).await;
        let body = json!({"reference": "open", "prices": [
            {"variant_id": one, "amount": vnd(100_000), "valid_from": Timestamp::now() - SignedDuration::from_hours(24 * 2)},
            {"variant_id": two, "amount": vnd(50_000), "valid_from": Timestamp::now() - SignedDuration::from_hours(24 * 2)},
        ]});
        let (status, _) = call(&app, "POST", &format!("/price-lists/{retail}/prices"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        let start = Timestamp::now() + SignedDuration::from_hours(1);
        let end = start + SignedDuration::from_hours(24 * 3);
        let body = json!({"operation": "percent", "value": -2000, "valid_from": start, "valid_to": end});
        let (status, change) = call(&app, "POST", &format!("/price-lists/{retail}/changes"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{change}");
        let change_uri = format!("/price-lists/{retail}/changes/{}", change["data"]["id"].as_str().unwrap());
        let (status, problem) = call(&app, "POST", &format!("{change_uri}/approve"), "boss", None).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::CONFLICT, Some("catalog.price_change_not_previewed")));

        let (status, previewed) = call(&app, "POST", &format!("{change_uri}/preview"), "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{previewed}");
        assert_eq!(previewed["data"]["affected"], 2);
        let samples = previewed["data"]["preview"]["samples"].as_array().unwrap();
        assert!(samples.iter().any(|row| row["before"] == 100_000 && row["after"] == 80_000), "{samples:?}");
        assert_eq!(history(&app, &retail).await.len(), 2, "preview writes nothing");

        let (status, approved) = call(&app, "POST", &format!("{change_uri}/approve"), "boss", None).await;
        assert_eq!((status, approved["data"]["status"].as_str()), (StatusCode::OK, Some("approved")));
        run_jobs(&app).await;
        let (_, applied) = call(&app, "GET", &change_uri, "boss", None).await;
        assert_eq!(applied["data"]["status"], "applied");
        assert_eq!(history(&app, &retail).await.len(), 4);

        let catalog = app.api::<dyn Catalog>().unwrap();
        let id: Uuid = one.parse().unwrap();
        let at = |t| PriceQuery::new(tree.a, Currency::VND, t, vec![id]);
        let price = |found: Vec<crate::api::PriceSnapshot>| found[0].amount.minor();
        let tenant = app.default_tenant();
        assert_eq!(price(catalog.prices(&tenant, &at(Timestamp::now())).await.unwrap()), 100_000);
        assert_eq!(price(catalog.prices(&tenant, &at(start + SignedDuration::from_mins(1))).await.unwrap()), 80_000);
        assert_eq!(price(catalog.prices(&tenant, &at(end + SignedDuration::from_mins(1))).await.unwrap()), 100_000, "the temporary price ended");
    })
    .await
    .unwrap();
}

/// Thứ tự chọn bảng giá (D-222) với đủ năm chiều: nút gần nhất → công ty → nhóm khách → kênh →
/// thị trường → `priority`. Cột công ty / kênh / thị trường chưa có API (bước 59, 48, 63): chèn
/// thẳng bảng giá như các bước đó sẽ ghi.
#[tokio::test]
async fn chon_bang_gia_theo_thu_tu_cu_the() {
    use sea_orm::{ActiveModelTrait, Set};
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let (_, variant) = create_product(&app, "boss", tree.a, "ORD-1", "Hàng").await;
        let id: Uuid = variant.parse().unwrap();
        let company = Uuid::now_v7();
        let tenant = app.default_tenant();
        // (nút, công ty, nhóm, kênh, thị trường, ưu tiên, giá)
        let lists = [
            (tree.chain, None, None, None, None, 0, 100),
            (tree.chain, None, None, None, None, 5, 101),
            (tree.chain, None, None, None, Some("VN"), 0, 102),
            (tree.chain, None, None, Some("pos"), None, 0, 103),
            (tree.chain, None, Some("vip"), None, None, 0, 104),
            (tree.chain, Some(company), None, None, None, 0, 105),
            (tree.a, None, None, None, None, 0, 106),
        ];
        for (org, company_id, group, channel, market, priority, amount) in lists {
            let list = crate::entity::price_list::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                org_id: Set(org.as_uuid()),
                name: Set(format!("list {amount}")),
                currency: Set("VND".to_owned()),
                tax_inclusive: Set(true),
                audience: Set("consumer".to_owned()),
                company_id: Set(company_id),
                customer_group: Set(group.map(str::to_owned)),
                channel: Set(channel.map(str::to_owned)),
                market: Set(market.map(str::to_owned)),
                derived_from: Set(None),
                adjust_bp: Set(0),
                adjust_minor: Set(None),
                rounding_rule_id: Set(None),
                priority: Set(priority),
                active: Set(true),
                created_by: Set(None),
                created_at: Set(Timestamp::now()),
                updated_at: Set(Timestamp::now()),
            }
            .insert(app.database().write())
            .await
            .unwrap();
            crate::entity::price::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                price_list_id: Set(list.id),
                variant_id: Set(id),
                amount_minor: Set(amount),
                amount_currency: Set("VND".to_owned()),
                compare_at_minor: Set(None),
                valid_from: Set(Timestamp::now() - SignedDuration::from_hours(24)),
                valid_to: Set(None),
                source: Set("manual".to_owned()),
                reference: Set(None),
                reference_fingerprint: Set(None),
                created_by: Set(None),
                created_at: Set(Timestamp::now()),
            }
            .insert(app.database().write())
            .await
            .unwrap();
        }
        let catalog = app.api::<dyn Catalog>().unwrap();
        let ask = |query: PriceQuery| {
            let (catalog, tenant) = (catalog.clone(), tenant.clone());
            async move { catalog.prices(&tenant, &query).await.unwrap()[0].amount.minor() }
        };
        let base = |org| PriceQuery::new(org, Currency::VND, Timestamp::now(), vec![id]);
        // Cửa hàng A có bảng riêng: thắng mọi bảng của chuỗi dù khớp bao nhiêu chiều.
        assert_eq!(ask(base(tree.a).for_company(company).for_customer_group("vip")).await, 106);
        // Cửa hàng B: theo chiều khớp, công ty trước nhất.
        let all = base(tree.b)
            .for_company(company)
            .for_customer_group("vip")
            .for_channel("pos")
            .for_market("VN");
        assert_eq!(ask(all).await, 105);
        let no_company = base(tree.b).for_customer_group("vip").for_channel("pos").for_market("VN");
        assert_eq!(ask(no_company).await, 104);
        assert_eq!(ask(base(tree.b).for_channel("pos").for_market("VN")).await, 103);
        assert_eq!(ask(base(tree.b).for_market("VN")).await, 102);
        // Không chiều nào: `priority` lớn hơn thắng.
        assert_eq!(ask(base(tree.b)).await, 101);
        // Kênh khác: bảng kênh `pos` không áp.
        assert_eq!(ask(base(tree.b).for_channel("web")).await, 101);
    })
    .await
    .unwrap();
}
