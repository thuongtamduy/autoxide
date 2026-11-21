//! Phí vận chuyển (bước 44 mục 2): vùng theo tỉnh thắng vùng cả nước, bậc cân và cân quy đổi,
//! miễn phí từ giá trị đơn, cửa hàng ghi đè phương thức của chuỗi, hàng không cần giao, bậc chồng
//! nhau bị từ chối, nhận tại cửa hàng, rò rỉ giữa hai cửa hàng anh em, gọi qua `dyn ShippingRates`.
//! Cả SQLite và Postgres.

use autoxide_kernel::access::Scope;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, root_of};
use autoxide_kernel::time::Timestamp;

use super::*;
use crate::api::{QuoteItem, QuoteRequest, ShippingRates};

const STAFF: [&str; 6] = [
    "shipping.settings.manage",
    "shipping.read",
    "shipping.carriers.manage",
    "shipping.shipments.read",
    "shipping.shipments.manage",
    "shipping.cod.manage",
];

/// Cây gốc → chuỗi → cửa hàng A, B.
pub(crate) struct Tree {
    pub chain: OrgId,
    pub a: OrgId,
    pub b: OrgId,
}

/// App với `boss` (cả khách), `store_a` / `store_b` (chỉ cửa hàng của mình).
pub(crate) async fn shop(test: &TestDatabase) -> (TestApp, Tree) {
    let org = || OrgId::from_uuid(Uuid::now_v7());
    let tree = Tree { chain: org(), a: org(), b: org() };
    let scoped = |scope: Scope| STAFF.map(|name| (name, scope));
    let app = TestApp::builder(test)
        .member("boss", 1, root_of(DEFAULT_TENANT), &scoped(Scope::Tenant))
        .member("store_a", 2, tree.a, &scoped(Scope::Unit))
        .member("store_b", 3, tree.b, &scoped(Scope::Unit))
        .provide::<dyn AdministrativeUnits>(Arc::new(FakeUnits))
        .module(crate::module())
        .build()
        .await
        .unwrap();
    insert_tree(&app, &tree).await;
    (app, tree)
}

/// Dựng cây gốc → chuỗi → cửa hàng A, B trong khách mặc định.
pub(crate) async fn insert_tree(app: &TestApp, tree: &Tree) {
    let write = app.database().begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.chain, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.a, tree.chain, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.b, tree.chain, 8).await.unwrap();
    write.commit().await.unwrap();
}

fn vnd(amount: i64) -> Value {
    json!({"amount": amount.to_string(), "currency": "VND"})
}

pub(crate) async fn zone(
    app: &TestApp,
    who: &str,
    org: OrgId,
    name: &str,
    regions: Value,
) -> String {
    let body = json!({"org_id": org.as_uuid(), "name": name, "regions": regions});
    let (status, created) = call(app, "POST", "/zones", who, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["data"]["id"].as_str().unwrap().to_owned()
}

pub(crate) async fn method(
    app: &TestApp,
    who: &str,
    org: OrgId,
    code: &str,
    extra: Value,
) -> String {
    let mut body = json!({
        "org_id": org.as_uuid(), "code": code, "kind": "table_rate",
        "names": {"vi": format!("Giao {code}"), "en": format!("{code} delivery")},
        "currency": "VND",
    });
    for (key, value) in extra.as_object().unwrap() {
        body[key] = value.clone();
    }
    let (status, created) = call(app, "POST", "/methods", who, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["data"]["id"].as_str().unwrap().to_owned()
}

pub(crate) async fn rates(app: &TestApp, who: &str, method: &str, rates: Value) {
    let (status, body) =
        call(app, "PUT", &format!("/methods/{method}/rates"), who, Some(json!({"rates": rates})))
            .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

fn hanoi() -> Value {
    json!({"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Trang Tien"})
}

fn saigon() -> Value {
    json!({"country": "VN", "province_code": "79", "commune_code": "26734", "line": "1 Le Loi"})
}

async fn quote(
    app: &TestApp,
    who: &str,
    org: OrgId,
    destination: Value,
    items: Value,
    value: i64,
) -> Value {
    let body = json!({
        "org_id": org.as_uuid(), "destination": destination, "items": items,
        "order_value": vnd(value), "locale": "vi",
    });
    let (status, answer) = call(app, "POST", "/quote", who, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    answer["data"].clone()
}

fn item(weight: i64, quantity: i64) -> Value {
    json!({"variant_id": Uuid::now_v7(), "sku": "SKU", "quantity": quantity, "weight_grams": weight})
}

fn amount(option: &Value) -> i64 {
    option["amount"]["amount"].as_str().unwrap().parse().unwrap()
}

/// Chuỗi có "Toàn quốc" 40 000đ và "Hà Nội" 20 000đ (0–1 kg), +5 000đ mỗi 500 g vượt 1 kg.
async fn standard(app: &TestApp, tree: &Tree) -> String {
    let all = zone(app, "boss", tree.chain, "Toan quoc", json!([{"country": "VN"}])).await;
    let hn =
        zone(app, "boss", tree.chain, "Ha Noi", json!([{"country": "VN", "province_code": "01"}]))
            .await;
    let id = method(app, "boss", tree.chain, "standard", json!({})).await;
    rates(
        app,
        "boss",
        &id,
        json!([
            {"zone_id": all, "min_weight_grams": 0, "max_weight_grams": 1000, "amount": vnd(40_000)},
            {"zone_id": all, "min_weight_grams": 1000, "amount": vnd(40_000), "step_grams": 500, "step_amount": vnd(5_000)},
            {"zone_id": hn, "min_weight_grams": 0, "max_weight_grams": 1000, "amount": vnd(20_000)},
            {"zone_id": hn, "min_weight_grams": 1000, "amount": vnd(20_000), "step_grams": 500, "step_amount": vnd(5_000)},
        ]),
    )
    .await;
    id
}

#[tokio::test]
async fn vung_theo_tinh_thang_ca_nuoc() {
    TestDatabase::run(async |test| {
        let (app, tree) = shop(test).await;
        standard(&app, &tree).await;
        let to_hanoi = quote(&app, "boss", tree.a, hanoi(), json!([item(300, 1)]), 100_000).await;
        assert_eq!(amount(&to_hanoi["options"][0]), 20_000, "{to_hanoi}");
        assert_eq!(to_hanoi["options"][0]["name"], "Giao standard");
        let to_saigon = quote(&app, "boss", tree.a, saigon(), json!([item(300, 1)]), 100_000).await;
        assert_eq!(amount(&to_saigon["options"][0]), 40_000);
        // Xã không thuộc tỉnh: đích không hợp lệ.
        let mut wrong = hanoi();
        wrong["province_code"] = json!("79");
        let body = json!({"org_id": tree.a.as_uuid(), "destination": wrong, "items": [item(300, 1)], "order_value": vnd(1)});
        let (status, problem) = call(&app, "POST", "/quote", "boss", Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["code"], "shipping.invalid_address");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bac_can_nang_va_quy_doi() {
    TestDatabase::run(async |test| {
        let (app, tree) = shop(test).await;
        standard(&app, &tree).await;
        // 2 × 800 g = 1 600 g: vượt 600 g = 2 bước → 20 000 + 10 000.
        let heavy = quote(&app, "boss", tree.a, hanoi(), json!([item(800, 2)]), 0).await;
        assert_eq!(amount(&heavy["options"][0]), 30_000, "{heavy}");
        assert_eq!(heavy["options"][0]["chargeable_weight_grams"], 1600);
        // Hộp nhẹ mà to: 400 × 300 × 200 mm / 6000 = 4 000 g quy đổi > 100 g thật → 6 bước.
        let bulky = json!([{"variant_id": Uuid::now_v7(), "quantity": 1, "weight_grams": 100,
                            "length_mm": 400, "width_mm": 300, "height_mm": 200}]);
        let bulky = quote(&app, "boss", tree.a, hanoi(), bulky, 0).await;
        assert_eq!(bulky["options"][0]["chargeable_weight_grams"], 4000, "{bulky}");
        assert_eq!(amount(&bulky["options"][0]), 50_000);
        // Thiếu cân nặng, phương thức không có cân mặc định → không dùng được.
        let unknown = json!([{"variant_id": Uuid::now_v7(), "sku": "NO-W", "quantity": 1}]);
        let missing = quote(&app, "boss", tree.a, hanoi(), unknown, 0).await;
        assert_eq!(missing["options"].as_array().unwrap().len(), 0);
        assert_eq!(missing["unavailable"][0]["reason_code"], "shipping.weight_missing");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mien_phi_tu_gia_tri_don() {
    TestDatabase::run(async |test| {
        let (app, tree) = shop(test).await;
        let id = standard(&app, &tree).await;
        let (status, _) = call(
            &app,
            "PATCH",
            &format!("/methods/{id}"),
            "boss",
            Some(json!({"free_over": vnd(500_000)})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let below = quote(&app, "boss", tree.a, hanoi(), json!([item(300, 1)]), 499_999).await;
        assert_eq!(amount(&below["options"][0]), 20_000);
        assert_eq!(below["options"][0]["free_over"]["amount"], "500000");
        let free = quote(&app, "boss", tree.a, hanoi(), json!([item(300, 1)]), 500_000).await;
        assert_eq!(amount(&free["options"][0]), 0);
        assert_eq!(free["options"][0]["free_shipping_applied"], true);
        assert_ne!(below["options"][0]["option_key"], free["options"][0]["option_key"]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn phuong_thuc_cua_hang_ghi_de_chuoi() {
    TestDatabase::run(async |test| {
        let (app, tree) = shop(test).await;
        standard(&app, &tree).await;
        // Cửa hàng A tự đặt "standard" rẻ hơn: A dùng của mình, B vẫn dùng của chuỗi.
        let all = zone(&app, "store_a", tree.a, "A toan quoc", json!([{"country": "VN"}])).await;
        let own = method(&app, "store_a", tree.a, "standard", json!({})).await;
        rates(
            &app,
            "store_a",
            &own,
            json!([{"zone_id": all, "min_weight_grams": 0, "amount": vnd(15_000)}]),
        )
        .await;
        let at_a = quote(&app, "store_a", tree.a, hanoi(), json!([item(300, 1)]), 0).await;
        assert_eq!(at_a["options"].as_array().unwrap().len(), 1, "{at_a}");
        assert_eq!(amount(&at_a["options"][0]), 15_000);
        let at_b = quote(&app, "store_b", tree.b, hanoi(), json!([item(300, 1)]), 0).await;
        assert_eq!(amount(&at_b["options"][0]), 20_000);
        // Trùng mã ở cùng nút → 409.
        let body = json!({"org_id": tree.a.as_uuid(), "code": "standard", "kind": "table_rate",
                          "names": {"vi": "X"}, "currency": "VND"});
        let (status, problem) = call(&app, "POST", "/methods", "store_a", Some(body)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(problem["code"], "shipping.method_code_taken");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hang_khong_can_giao() {
    TestDatabase::run(async |test| {
        let (app, tree) = shop(test).await;
        standard(&app, &tree).await;
        let digital = json!([{"variant_id": Uuid::now_v7(), "quantity": 1, "requires_shipping": false}]);
        let none = quote(&app, "boss", tree.a, hanoi(), digital, 100_000).await;
        assert_eq!(none["options"].as_array().unwrap().len(), 0);
        assert_eq!(none["unavailable"].as_array().unwrap().len(), 0);
        // Lẫn hàng cần giao: chỉ hàng cần giao tính cân (hàng số không cân nặng không làm hỏng).
        let mixed = json!([item(300, 1), {"variant_id": Uuid::now_v7(), "quantity": 3, "requires_shipping": false}]);
        let mixed = quote(&app, "boss", tree.a, hanoi(), mixed, 0).await;
        assert_eq!(mixed["options"][0]["chargeable_weight_grams"], 300);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bac_chong_nhau_bi_tu_choi_qua_rest() {
    TestDatabase::run(async |test| {
        let (app, tree) = shop(test).await;
        let all = zone(&app, "boss", tree.chain, "VN", json!([{"country": "VN"}])).await;
        let id = method(&app, "boss", tree.chain, "standard", json!({})).await;
        let body = json!({"rates": [
            {"zone_id": all, "min_weight_grams": 0, "max_weight_grams": 2000, "amount": vnd(1)},
            {"zone_id": all, "min_weight_grams": 1000, "amount": vnd(2)},
        ]});
        let (status, problem) =
            call(&app, "PUT", &format!("/methods/{id}/rates"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["code"], "shipping.rate_overlap");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhan_tai_cua_hang() {
    TestDatabase::run(async |test| {
        let (app, tree) = shop(test).await;
        let id = method(&app, "boss", tree.chain, "pickup", json!({"kind": "pickup", "hold_days": 5})).await;
        let points = json!({"pickup_points": [
            {"org_id": tree.a.as_uuid(), "instructions": "Quay 1", "prep_hours": 2},
            {"org_id": tree.b.as_uuid(), "prep_hours": 4},
        ]});
        let (status, saved) = call(&app, "PUT", &format!("/methods/{id}/pickup-points"), "boss", Some(points)).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        // Không cần đích, không cần cân nặng.
        let body = json!({"org_id": tree.a.as_uuid(), "items": [{"variant_id": Uuid::now_v7(), "quantity": 1}],
                          "order_value": vnd(1)});
        let (_, answer) = call(&app, "POST", "/quote", "boss", Some(body)).await;
        let options = answer["data"]["options"].as_array().unwrap();
        assert_eq!(options.len(), 2, "{answer}");
        assert_eq!(options[0]["kind"], "pickup");
        assert_eq!(options[0]["hold_days"], 5);
        assert_eq!(amount(&options[0]), 0);
        // Điểm nhận ngoài nhánh của phương thức bị từ chối.
        let outside = json!({"pickup_points": [{"org_id": Uuid::now_v7()}]});
        let (status, _) = call(&app, "PUT", &format!("/methods/{id}/pickup-points"), "boss", Some(outside)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ro_ri_giua_hai_cua_hang_anh_em() {
    TestDatabase::run(async |test| {
        let (app, tree) = shop(test).await;
        let zone_b = zone(&app, "store_b", tree.b, "B", json!([{"country": "VN"}])).await;
        let method_b = method(&app, "store_b", tree.b, "express", json!({})).await;
        for path in [format!("/zones/{zone_b}"), format!("/methods/{method_b}")] {
            let (status, problem) = call(&app, "GET", &path, "store_a", None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {problem}");
            let (status, _) = call(&app, "DELETE", &path, "store_a", None).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
        }
        let (_, list) = call(&app, "GET", "/methods", "store_a", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 0, "{list}");
        let (_, list) = call(&app, "GET", "/zones", "store_a", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 0);
        // A không báo giá được cho cửa hàng B.
        let body = json!({"org_id": tree.b.as_uuid(), "items": [], "order_value": vnd(0)});
        let (status, _) = call(&app, "POST", "/quote", "store_a", Some(body)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // Phương thức riêng của B không áp cho A.
        rates(
            &app,
            "store_b",
            &method_b,
            json!([{"zone_id": zone_b, "min_weight_grams": 0, "amount": vnd(1)}]),
        )
        .await;
        let at_a = quote(&app, "store_a", tree.a, hanoi(), json!([item(300, 1)]), 0).await;
        assert_eq!(at_a["options"].as_array().unwrap().len(), 0, "{at_a}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn api_shipping_qua_testapp() {
    TestDatabase::run(async |test| {
        let (app, tree) = shop(test).await;
        standard(&app, &tree).await;
        let rates = app.api::<dyn ShippingRates>().unwrap();
        let tenant = app.default_tenant();
        let destination: crate::api::Address = serde_json::from_value(json!({
            "country": "VN", "province_code": "01", "province_name": "Thanh pho Ha Noi",
            "commune_code": "00004", "commune_name": "Phuong Ba Dinh", "line": "12 Trang Tien",
        }))
        .unwrap();
        let vnd = Currency::parse("VND").unwrap();
        let request = QuoteRequest::new(
            tree.a,
            vec![QuoteItem::new(Uuid::now_v7(), 1).weight(Some(300))],
            Money::new(100_000, vnd),
            Timestamp::now(),
        )
        .destination(destination)
        .locale("en");
        let quote = rates.quote(&tenant, &request).await.unwrap();
        assert_eq!(quote.options.len(), 1);
        assert_eq!(quote.options[0].amount, Money::new(20_000, vnd));
        assert_eq!(quote.options[0].name, "standard delivery");
        let key = quote.options[0].option_key.clone();
        assert!(rates.confirm_option(&tenant, &request, &key).await.is_ok());
        let error = rates.confirm_option(&tenant, &request, "stale").await.unwrap_err();
        assert_eq!(error.code(), "shipping.option_changed");
        assert!(app.api::<dyn crate::api::CarrierRegistry>().unwrap().codes().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tool_mcp_bao_gia_nhu_rest() {
    TestDatabase::run(async |test| {
        let (app, tree) = shop(test).await;
        standard(&app, &tree).await;
        let client = app.mcp("boss").await.unwrap();
        let input = json!({"org_id": tree.a.as_uuid(),
            "destination": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "1"},
            "items": [{"variant_id": Uuid::now_v7(), "quantity": 1, "weight_grams": 300}],
            "order_value": "100000"});
        let answer = client.call_tool(autoxide_testkit::tool("shipping.quote", input)).await.unwrap();
        assert_eq!(answer.is_error, Some(false), "{answer:?}");
        let data = answer.structured_content.unwrap();
        assert_eq!(data["data"]["options"][0]["amount"]["amount"], "20000", "{data}");
        // Người không có `shipping.read` ở cửa hàng B: như REST, bị từ chối.
        let client = app.mcp("store_a").await.unwrap();
        let input = json!({"org_id": tree.b.as_uuid(), "items": [], "order_value": "0"});
        let refused = client.call_tool(autoxide_testkit::tool("shipping.quote", input)).await.unwrap();
        assert_eq!(refused.is_error, Some(true));
        let listed = client
            .call_tool(autoxide_testkit::tool("shipping.list_shipments", json!({})))
            .await
            .unwrap();
        assert_eq!(listed.is_error, Some(false), "{listed:?}");
    })
    .await
    .unwrap();
}
