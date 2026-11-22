//! Ngày giao dự kiến và ước tính công khai (bước 44 mục 6, 7, 9): lịch giao với ngày nghỉ và giờ
//! chốt riêng, `eta` trong báo giá, giờ sẵn sàng và hạn giữ hàng của nhận tại cửa hàng, ước tính
//! công khai bằng đúng báo giá của giỏ một dòng. Cả SQLite và Postgres.

use autoxide_kernel::time::{Date, Timestamp};
use jiff::SignedDuration;
use std::collections::HashMap;

use autoxide_catalog::api::{
    BundleComponent, Catalog, PriceFloor, PriceQuery, PriceSnapshot, VariantSnapshot,
};
use autoxide_kernel::error::Error;
use autoxide_kernel::money::Currency;
use autoxide_kernel::tenancy::{OrgId, TenantContext};

use super::rates::{Tree, method, rates, zone};
use super::*;

fn vnd(amount: i64) -> Value {
    json!({"amount": amount.to_string(), "currency": "VND"})
}

/// Catalog giả: một biến thể 300 g của cửa hàng A, giá 200 000đ.
struct FakeCatalog {
    variant: VariantSnapshot,
}

impl FakeCatalog {
    fn new(org: OrgId) -> Self {
        let variant = serde_json::from_value(json!({
            "variant_id": Uuid::from_u128(0xA1), "product_id": Uuid::from_u128(0xB1), "org_id": org,
            "kind": "physical", "status": "active", "variant_active": true, "sku": "TEE-1", "barcode": null,
            "name": "Ao", "variant_label": "", "locale": "vi", "tax_category": "vat.standard",
            "category_ids": [], "weight_grams": 300, "length_mm": null, "width_mm": null, "height_mm": null,
            "requires_shipping": true, "track_inventory": true, "sale_mode": "in_stock",
            "preorder_available_on": null, "preorder_limit": null, "preorder_ends_at": null,
            "allow_backorder": false, "lot_tracked": false, "serial_tracked": false,
            "warranty_months": null, "image_media_id": null, "updated_at": Timestamp::now(),
        }))
        .unwrap();
        FakeCatalog { variant }
    }
}

#[async_trait::async_trait]
impl Catalog for FakeCatalog {
    async fn variants(
        &self,
        _: &TenantContext,
        ids: &[Uuid],
        _: Option<&str>,
    ) -> Result<Vec<VariantSnapshot>, Error> {
        Ok(ids
            .iter()
            .filter(|id| **id == self.variant.variant_id)
            .map(|_| self.variant.clone())
            .collect())
    }
    async fn variant_by_code(
        &self,
        _: &TenantContext,
        _: &str,
        _: Option<&str>,
    ) -> Result<Option<VariantSnapshot>, Error> {
        Ok(None)
    }
    async fn prices(
        &self,
        _: &TenantContext,
        query: &PriceQuery,
    ) -> Result<Vec<PriceSnapshot>, Error> {
        Ok(vec![
            serde_json::from_value(json!({
                "variant_id": self.variant.variant_id, "price_list_id": Uuid::from_u128(1),
                "price_id": Uuid::from_u128(2), "amount": vnd(200_000), "tax_inclusive": true,
                "compare_at": null, "valid_from": query.at, "valid_to": null,
            }))
            .unwrap(),
        ])
    }
    async fn reference_prices(
        &self,
        _: &TenantContext,
        _: &PriceQuery,
    ) -> Result<Vec<PriceSnapshot>, Error> {
        Ok(Vec::new())
    }
    async fn price_floors(
        &self,
        _: &TenantContext,
        _: OrgId,
        _: Currency,
        _: &[Uuid],
    ) -> Result<Vec<PriceFloor>, Error> {
        Ok(Vec::new())
    }
    async fn bundle_components(
        &self,
        _: &TenantContext,
        _: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<BundleComponent>>, Error> {
        Ok(HashMap::new())
    }
}

/// Cửa hàng có catalog giả, phương thức "standard" 20 000đ toàn quốc, giao 1–2 ngày, miễn phí từ
/// 300 000đ, gắn lịch có giờ chốt 11:00 và cả tuần làm việc.
async fn dated_shop(test: &TestDatabase) -> (TestApp, Tree, String, String) {
    let org = || OrgId::from_uuid(Uuid::now_v7());
    let tree = Tree { chain: org(), a: org(), b: org() };
    let staff = ["shipping.settings.manage", "shipping.read"]
        .map(|name| (name, autoxide_kernel::access::Scope::Tenant));
    let app = TestApp::builder(test)
        .member(
            "boss",
            1,
            autoxide_kernel::tenancy::root_of(autoxide_kernel::tenancy::DEFAULT_TENANT),
            &staff,
        )
        .provide::<dyn AdministrativeUnits>(Arc::new(FakeAdministrativeUnits::sample()))
        .provide::<dyn Catalog>(Arc::new(FakeCatalog::new(tree.a)))
        .module(crate::module())
        .build()
        .await
        .unwrap();
    super::rates::insert_tree(&app, &tree).await;
    let body = json!({"org_id": tree.chain.as_uuid(), "name": "Kho", "working_days": [1, 2, 3, 4, 5, 6, 7]});
    let (status, calendar) = call(&app, "POST", "/calendars", "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{calendar}");
    let calendar = calendar["data"]["id"].as_str().unwrap().to_owned();
    let all = zone(&app, "boss", tree.chain, "VN", json!([{"country": "VN"}])).await;
    let id = method(
        &app,
        "boss",
        tree.chain,
        "standard",
        json!({"free_over": vnd(300_000), "estimated_days": {"min": 1, "max": 2},
               "calendar_id": calendar, "cutoff_minute": 1439}),
    )
    .await;
    rates(
        &app,
        "boss",
        &id,
        json!([{"zone_id": all, "min_weight_grams": 0, "amount": vnd(20_000)}]),
    )
    .await;
    (app, tree, id, calendar)
}

#[tokio::test]
async fn ngay_giao_theo_lich_va_ngay_nghi() {
    TestDatabase::run(async |test| {
        let (app, tree, _, calendar) = dated_shop(test).await;
        let tz = autoxide_kernel::time::zone("Asia/Ho_Chi_Minh").unwrap();
        let today = Timestamp::now().date_in(&tz);
        let body = json!({"org_id": tree.a.as_uuid(),
            "destination": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "1"},
            "items": [{"variant_id": Uuid::now_v7(), "quantity": 1, "weight_grams": 300}], "order_value": vnd(1)});
        let (_, quote) = call(&app, "POST", "/quote", "boss", Some(body.clone())).await;
        let eta = &quote["data"]["options"][0]["eta"];
        assert_eq!(eta["time_zone"], "Asia/Ho_Chi_Minh", "{quote}");
        let earliest: Date = serde_json::from_value(eta["earliest_date"].clone()).unwrap();
        assert!(earliest > today && earliest <= today.checked_add_days(2).unwrap(), "{eta}");
        // Ngày mai nghỉ: ngày giao sớm nhất lùi qua ngày nghỉ.
        let tomorrow = today.succ().unwrap();
        let (status, _) = call(
            &app,
            "PUT",
            &format!("/calendars/{calendar}/days/{tomorrow}"),
            "boss",
            Some(json!({"kind": "closed", "note": "Nghi le"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, quote) = call(&app, "POST", "/quote", "boss", Some(body)).await;
        let later: Date =
            serde_json::from_value(quote["data"]["options"][0]["eta"]["earliest_date"].clone()).unwrap();
        assert!(later > tomorrow, "{quote}");
        let (_, detail) = call(&app, "GET", &format!("/calendars/{calendar}"), "boss", None).await;
        assert_eq!(detail["data"]["days"][0]["kind"], "closed");
        assert_eq!(detail["data"]["working_days"], json!([1, 2, 3, 4, 5, 6, 7]));
        let _ = today.weekday();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn uoc_tinh_cong_khai_bang_bao_gia_mot_dong() {
    TestDatabase::run(async |test| {
        let (app, tree, _, _) = dated_shop(test).await;
        let variant = Uuid::from_u128(0xA1);
        let answer = app
            .get(&format!("/api/v1/shipping/public/estimate?variant_id={variant}&province_code=79&locale=vi"))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
        assert!(answer.header("cache-control").contains("max-age"));
        let estimate = answer.json();
        assert_eq!(estimate["data"]["estimate"], true);
        let option = &estimate["data"]["options"][0];
        assert_eq!(option["amount"]["amount"], "20000", "one item of 200 000 is under the free threshold");
        assert_eq!(option["free_over"]["amount"], "300000");
        // Hai món: 400 000 ≥ ngưỡng → miễn phí, như báo giá của giỏ.
        let answer = app
            .get(&format!("/api/v1/shipping/public/estimate?variant_id={variant}&quantity=2&province_code=79"))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.json()["data"]["options"][0]["amount"]["amount"], "0");
        // Bằng đúng báo giá của giỏ một dòng cùng tỉnh.
        let body = json!({"org_id": tree.a.as_uuid(),
            "destination": {"country": "VN", "province_code": "79", "commune_code": "26734", "line": "1"},
            "items": [{"variant_id": variant, "quantity": 1, "weight_grams": 300}], "order_value": vnd(200_000)});
        let (_, quote) = call(&app, "POST", "/quote", "boss", Some(body)).await;
        assert_eq!(quote["data"]["options"][0]["amount"], option["amount"]);
        assert_eq!(quote["data"]["options"][0]["eta"], option["eta"]);
        // Biến thể không có.
        let answer = app
            .get(&format!("/api/v1/shipping/public/estimate?variant_id={}&province_code=79", Uuid::now_v7()))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhan_tai_cua_hang_gio_san_sang_va_han_giu() {
    TestDatabase::run(async |test| {
        let (app, tree, _, calendar) = dated_shop(test).await;
        let id = method(
            &app,
            "boss",
            tree.chain,
            "pickup",
            json!({"kind": "pickup", "hold_days": 3, "calendar_id": calendar}),
        )
        .await;
        let points = json!({"pickup_points": [{"org_id": tree.a.as_uuid(), "prep_hours": 2}]});
        call(&app, "PUT", &format!("/methods/{id}/pickup-points"), "boss", Some(points)).await;
        let body = json!({"org_id": tree.a.as_uuid(), "items": [{"variant_id": Uuid::now_v7(), "quantity": 1}],
                          "order_value": vnd(1)});
        let (_, quote) = call(&app, "POST", "/quote", "boss", Some(body)).await;
        let option = &quote["data"]["options"][0];
        assert_eq!(option["kind"], "pickup", "{quote}");
        assert_eq!(option["hold_days"], 3);
        let ready: Timestamp = serde_json::from_value(option["ready_at"].clone()).unwrap();
        assert!(ready >= Timestamp::now() + SignedDuration::from_mins(119), "{option}");
    })
    .await
    .unwrap();
}
