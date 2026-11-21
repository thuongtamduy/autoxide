//! Bộ đo tính khuyến mại cho một giỏ (D-222): giỏ 50 dòng, 500 chương trình đang chạy, mục tiêu
//! p99 < 5 ms. Đo qua `dyn Promotions::evaluate` — đúng lời gọi của `orders` — nên số gồm cả phần
//! tính trong bộ nhớ lẫn vài câu đọc nhỏ (bản sửa của chương trình, cài đặt cửa hàng); catalog là
//! bản giả trong bộ nhớ để chỉ đo `promotions`.
//!
//! ```text
//! cargo bench -p autoxide-promotions                                   # SQLite
//! TEST_DATABASE_URL=postgres://… cargo bench -p autoxide-promotions    # Postgres
//! ```
//!
//! Số gốc để so: `docs/BENCHMARKS.md`.

// Bộ đo là code công cụ: hỏng thì dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use autoxide_catalog::api::{
    BundleComponent, Catalog, PriceFloor, PriceQuery, PriceSnapshot, VariantSnapshot,
};
use autoxide_kernel::access::Scope;
use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, TenantContext, root_of};
use autoxide_promotions::api::{Cart, CartLine, Promotions};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

/// Số chương trình đang chạy.
const PROGRAMS: usize = 500;
/// Số dòng của giỏ.
const LINES: usize = 50;
/// Mỗi chương trình nhắm bấy nhiêu biến thể; mỗi biến thể thuộc hai chương trình.
const TARGETS_PER_PROGRAM: usize = 10;
/// Số lần đo (sau khi chạy nóng).
const ITERATIONS: usize = 2000;
/// Giá niêm yết của mọi biến thể (đồng).
const PRICE: i64 = 200_000;

const STAFF: [&str; 4] =
    ["promotions.read", "promotions.write", "promotions.publish", "promotions.compliance"];

/// `dyn Catalog` giả: mọi biến thể cùng một nút bán, cùng giá từ 30 ngày trước.
struct FakeCatalog {
    org: OrgId,
    currency: Currency,
    since: Timestamp,
}

impl FakeCatalog {
    fn snapshot(&self, id: Uuid) -> VariantSnapshot {
        // `VariantSnapshot` là `#[non_exhaustive]` ở crate khác: dựng qua JSON.
        serde_json::from_value(json!({
            "variant_id": id, "product_id": id, "org_id": self.org, "kind": "physical",
            "status": "active", "variant_active": true, "sku": id.to_string(),
            "barcode": null, "name": "Item", "variant_label": "", "locale": "vi",
            "tax_category": "vat.standard", "category_ids": [], "weight_grams": null,
            "length_mm": null, "width_mm": null, "height_mm": null, "requires_shipping": true,
            "track_inventory": true, "sale_mode": "in_stock", "preorder_available_on": null,
            "preorder_limit": null, "preorder_ends_at": null, "allow_backorder": false,
            "lot_tracked": false, "serial_tracked": false, "warranty_months": null,
            "image_media_id": null, "updated_at": self.since,
        }))
        .unwrap()
    }

    fn price(&self, id: Uuid) -> PriceSnapshot {
        serde_json::from_value(json!({
            "variant_id": id, "price_list_id": Uuid::nil(), "price_id": Uuid::nil(),
            "amount": Money::new(PRICE, self.currency), "tax_inclusive": true, "compare_at": null,
            "valid_from": self.since, "valid_to": null,
        }))
        .unwrap()
    }
}

#[async_trait::async_trait]
impl Catalog for FakeCatalog {
    async fn variants(
        &self,
        _tenant: &TenantContext,
        ids: &[Uuid],
        _locale: Option<&str>,
    ) -> Result<Vec<VariantSnapshot>, Error> {
        Ok(ids.iter().map(|id| self.snapshot(*id)).collect())
    }

    async fn variant_by_code(
        &self,
        _tenant: &TenantContext,
        _code: &str,
        _locale: Option<&str>,
    ) -> Result<Option<VariantSnapshot>, Error> {
        Ok(None)
    }

    async fn prices(
        &self,
        _tenant: &TenantContext,
        query: &PriceQuery,
    ) -> Result<Vec<PriceSnapshot>, Error> {
        Ok(query.variant_ids.iter().map(|id| self.price(*id)).collect())
    }

    async fn reference_prices(
        &self,
        _tenant: &TenantContext,
        query: &PriceQuery,
    ) -> Result<Vec<PriceSnapshot>, Error> {
        Ok(query.variant_ids.iter().map(|id| self.price(*id)).collect())
    }

    async fn price_floors(
        &self,
        _tenant: &TenantContext,
        _org_id: OrgId,
        _currency: Currency,
        _variant_ids: &[Uuid],
    ) -> Result<Vec<PriceFloor>, Error> {
        Ok(Vec::new())
    }

    async fn bundle_components(
        &self,
        _tenant: &TenantContext,
        _variant_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<BundleComponent>>, Error> {
        Ok(HashMap::new())
    }
}

/// Gọi `/api/v1/promotions{uri}` bằng `boss`; dừng nếu trạng thái khác `expected`.
async fn call(app: &TestApp, method: Method, uri: &str, body: Option<Value>) -> Value {
    let mut request = app.request(method, &format!("/api/v1/promotions{uri}")).token("boss");
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let status = answer.status();
    let text = answer.text();
    assert!(status.is_success(), "{uri}: {status} {text}");
    if status == StatusCode::NO_CONTENT || text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap()
    }
}

/// Tạo, gắn mục tiêu, ghi nhận thông báo và bật một chương trình giảm 10% cho `targets`.
async fn running(app: &TestApp, org: OrgId, index: usize, targets: &[Uuid]) {
    let body = json!({
        "org_id": org, "kind": "percent_off", "currency": "VND", "value_bp": 1000,
        "starts_at": Timestamp::now() - SignedDuration::from_hours(1),
        "ends_at": Timestamp::now() + SignedDuration::from_hours(24 * 7),
        "priority": index % 7, "locale": "vi", "title": format!("Chương trình {index}"),
        "terms": "Áp dụng cho sản phẩm được chọn",
    });
    let created = call(app, Method::POST, "/promotions", Some(body)).await;
    let id = created["data"]["id"].as_str().unwrap().to_owned();
    let targets: Vec<Value> =
        targets.iter().map(|id| json!({"target_type": "variant", "target_id": id})).collect();
    call(app, Method::PUT, &format!("/promotions/{id}/targets"), Some(json!({"targets": targets})))
        .await;
    let checks = call(app, Method::POST, &format!("/promotions/{id}/activation-check"), None).await;
    for check in checks["data"]["checks"].as_array().cloned().unwrap_or_default() {
        if check["code"] == "promotions.notice_required" {
            let authority = check["params"]["authority"].as_str().unwrap();
            call(
                app,
                Method::PATCH,
                &format!("/promotions/{id}/notices/{authority}"),
                Some(json!({"status": "submitted", "reference": "TB-1"})),
            )
            .await;
        }
    }
    call(app, Method::POST, &format!("/promotions/{id}/activate"), None).await;
}

fn report(name: &str, mut samples: Vec<Duration>, elapsed: Duration) {
    samples.sort();
    let at = |fraction: f64| {
        let index = ((samples.len() as f64 - 1.0) * fraction).round() as usize;
        samples[index].as_secs_f64() * 1000.0
    };
    let per_second = samples.len() as f64 / elapsed.as_secs_f64();
    println!(
        "{name:<28} {per_second:>9.0}/s   p50 {:>7.3} ms   p95 {:>7.3} ms   p99 {:>7.3} ms",
        at(0.50),
        at(0.95),
        at(0.99)
    );
}

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        TestDatabase::run(async |test| {
            let backend = if test.url().starts_with("postgres") { "PostgreSQL" } else { "SQLite" };
            let store = OrgId::from_uuid(Uuid::now_v7());
            let currency = Currency::parse("VND").unwrap();
            let catalog = Arc::new(FakeCatalog {
                org: store,
                currency,
                since: Timestamp::now() - SignedDuration::from_hours(24 * 30),
            });
            let staff = STAFF.map(|name| (name, Scope::Tenant));
            let app = TestApp::builder(test)
                .member("boss", 1, root_of(DEFAULT_TENANT), &staff)
                .provide::<dyn Catalog>(catalog.clone())
                .module(autoxide_promotions::module())
                .build()
                .await
                .unwrap();
            let write = app.database().begin_write().await.unwrap();
            let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
            OrgTree::insert(&write, DEFAULT_TENANT, store, root, 8).await.unwrap();
            write.commit().await.unwrap();
            let tasks = app.modules().tasks().unwrap();
            let (_, task) =
                tasks.into_iter().find(|(name, _)| name == "promotions.load_legal_data").unwrap();
            task.run(&task.parse(&[]).unwrap()).await.unwrap();

            // Biến thể k thuộc chương trình k % 500 và (k + 250) % 500: mỗi dòng có hai ứng viên,
            // cộng 20% — dưới trần 50%, nên mọi chương trình bật được.
            let variants: Vec<Uuid> =
                (0..PROGRAMS * TARGETS_PER_PROGRAM / 2).map(|_| Uuid::now_v7()).collect();
            let started = Instant::now();
            for index in 0..PROGRAMS {
                let targets: Vec<Uuid> = variants
                    .iter()
                    .enumerate()
                    .filter(|(k, _)| {
                        k % PROGRAMS == index || (k + PROGRAMS / 2) % PROGRAMS == index
                    })
                    .map(|(_, id)| *id)
                    .collect();
                running(&app, store, index, &targets).await;
            }
            println!(
                "Autoxide promotions benchmark — {backend}, {PROGRAMS} running programs, \
                 {LINES}-line cart, {ITERATIONS} evaluations (setup {:.1}s)",
                started.elapsed().as_secs_f64()
            );

            let promotions = app.api::<dyn Promotions>().unwrap();
            let tenant = app.default_tenant();
            // Giỏ thứ i lấy 50 biến thể liên tiếp, xoay vòng: chạm nhiều chương trình khác nhau.
            let cart = |round: usize| {
                let mut cart = Cart::new(store, currency, Timestamp::now());
                for line in 0..LINES {
                    let variant = variants[(round * LINES + line) % variants.len()];
                    let price = Money::new(PRICE, currency);
                    cart.lines.push(CartLine::new(format!("l{line}"), variant, 1, price));
                }
                cart
            };
            let first = promotions.evaluate(&tenant, &cart(0)).await.unwrap();
            assert!(first.lines.len() >= LINES, "mỗi dòng có ít nhất một khoản giảm");
            assert!(first.total_discount.minor() > 0);
            for round in 0..ITERATIONS / 10 {
                promotions.evaluate(&tenant, &cart(round)).await.unwrap();
            }
            let mut samples = Vec::with_capacity(ITERATIONS);
            let started = Instant::now();
            for round in 0..ITERATIONS {
                let cart = cart(round);
                let start = Instant::now();
                promotions.evaluate(&tenant, &cart).await.unwrap();
                samples.push(start.elapsed());
            }
            report("evaluate (50 lines)", samples, started.elapsed());
        })
        .await
        .unwrap();
    });
}
