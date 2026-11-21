//! Test của module `tax` qua REST (cả SQLite và Postgres): nhập gói "vn", thuế suất theo ngày (8%
//! tới hết 2026 rồi 10%), tách thuế từ giá đã gồm khớp từng đồng, TTĐB rồi GTGT, khấu trừ thay của
//! sàn, hồ sơ thuế kế thừa theo cây và theo phạm vi, chế độ hộ kinh doanh, gói cũ bị từ chối, doctor.

use autoxide_kernel::access::Scope;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree};
use autoxide_kernel::time::{Date, Timestamp};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

struct Tree {
    root: OrgId,
    north: OrgId,
    store: OrgId,
    south: OrgId,
}

async fn app(test: &TestDatabase) -> (TestApp, Tree) {
    let org = || OrgId::from_uuid(uuid::Uuid::now_v7());
    let (north, store, south) = (org(), org(), org());
    let app = TestApp::builder(test)
        .user("ops", 1, &["tax.read", "tax.profiles_manage", "tax.packs_manage"])
        .member(
            "north-admin",
            2,
            north,
            &[("tax.read", Scope::Subtree), ("tax.profiles_manage", Scope::Subtree)],
        )
        .module(crate::module())
        .build()
        .await
        .unwrap();
    let write = app.database().begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, north, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, store, north, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, south, root, 8).await.unwrap();
    write.commit().await.unwrap();
    let pack: Value = serde_json::from_str(crate::pack::BUNDLED_VN).unwrap();
    let (status, body) = call(&app, "POST", "/packs", "ops", Some(json!({"pack": pack}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    (app, Tree { root, north, store, south })
}

async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/tax{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

fn line(id: &str, category: &str, amount: i64) -> Value {
    json!({"line_id": id, "category": category, "amount": {"amount": amount.to_string(), "currency": "VND"}})
}

async fn quote(app: &TestApp, who: &str, body: Value) -> Value {
    let (status, answer) = call(app, "POST", "/quote", who, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    answer["data"].clone()
}

/// Số tiền (đơn vị nhỏ nhất, VND) của một `Money`, hay của trường `amount` của một thành phần thuế.
fn amount(value: &Value) -> i64 {
    let money = if value["amount"].is_object() { &value["amount"] } else { value };
    money["amount"].as_str().unwrap().parse().unwrap()
}

#[tokio::test]
async fn vn_gtgt_8_toi_het_2026_roi_10() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let body = |day: &str| {
            json!({"jurisdiction": "VN", "tax_point": day, "price_mode": "exclusive", "currency": "VND",
                   "lines": [line("a", "vat.standard_reducible", 1_000_000)]})
        };
        let before = quote(&app, "ops", body("2026-12-31")).await;
        assert_eq!(before["lines"][0]["taxes"][0]["rate_percent"], "8");
        assert_eq!(amount(&before["tax"]), 80_000);
        assert_eq!(before["invoice_type"], "vat_invoice");
        let after = quote(&app, "ops", body("2027-01-01")).await;
        assert_eq!(after["lines"][0]["taxes"][0]["rate_percent"], "10");
        assert_eq!(amount(&after["gross"]), 1_100_000);
        assert!(after["snapshot"]["rates"][0]["rate_id"].is_string());
        // Trước khi có thuế suất: không đoán.
        let (status, missing) = call(&app, "POST", "/quote", "ops", Some(body("2020-01-01"))).await;
        assert_eq!((status, missing["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("tax.rate_missing")));
        let mut unknown = body("2026-01-01");
        unknown["lines"][0]["category"] = json!("vat.nope");
        let (_, refused) = call(&app, "POST", "/quote", "ops", Some(unknown)).await;
        assert_eq!(refused["code"], "tax.category_unknown");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tach_thue_gia_da_gom_va_ttdb_roi_gtgt() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let inclusive = quote(
            &app,
            "ops",
            json!({"jurisdiction": "VN", "tax_point": "2026-06-01", "price_mode": "inclusive", "currency": "VND",
                   "lines": [line("a", "vat.standard", 99_999), line("b", "vat.standard", 1), line("c", "vat.exempt", 5_000)]}),
        )
        .await;
        for found in inclusive["lines"].as_array().unwrap() {
            let taxes: i64 = found["taxes"].as_array().unwrap().iter().map(amount).sum();
            assert_eq!(amount(&found["net"]) + taxes, amount(&found["gross"]), "{found}");
        }
        assert_eq!(amount(&inclusive["gross"]), 105_000);
        assert!(inclusive["lines"][2]["taxes"][0]["exempt"].as_bool().unwrap());
        let beer = quote(
            &app,
            "ops",
            json!({"jurisdiction": "VN", "tax_point": "2026-06-01", "price_mode": "exclusive", "currency": "VND",
                   "lines": [line("beer", "excise.beer", 100_000)]}),
        )
        .await;
        let taxes: Vec<i64> = beer["lines"][0]["taxes"].as_array().unwrap().iter().map(amount).collect();
        assert_eq!(taxes, [65_000, 16_500]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khau_tru_thay_cua_san_chi_cho_ho_kinh_doanh() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let body = |regime: &str| {
            json!({"jurisdiction": "VN", "on": "2026-03-01", "payer_kind": "marketplace", "payee_regime": regime,
                   "business_line": "goods", "amount": {"amount": "1000000", "currency": "VND"}})
        };
        let (status, household) = call(&app, "POST", "/withholding", "ops", Some(body("household"))).await;
        assert_eq!(status, StatusCode::OK, "{household}");
        let parts: Vec<(String, i64)> = household["data"]["components"]
            .as_array()
            .unwrap()
            .iter()
            .map(|part| (part["tax_kind"].as_str().unwrap().to_owned(), amount(part)))
            .collect();
        assert_eq!(parts, [("pit".to_owned(), 5_000), ("vat".to_owned(), 10_000)]);
        let (_, business) = call(&app, "POST", "/withholding", "ops", Some(body("vat_credit"))).await;
        assert!(business["data"]["components"].as_array().unwrap().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ho_so_thue_ke_thua_theo_cay_va_pham_vi() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let household = json!({"jurisdiction": "VN", "tax_code": "0101234567", "regime": "household",
                               "business_lines": ["goods"], "annual_revenue": {"amount": "1500000000", "currency": "VND"}});
        let (status, body) =
            call(&app, "PUT", &format!("/profiles/{}", tree.north.as_uuid()), "north-admin", Some(household)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        // Cửa hàng dưới nhánh Bắc kế thừa.
        let (status, inherited) = call(&app, "GET", &format!("/profiles/{}", tree.store.as_uuid()), "north-admin", None).await;
        assert_eq!(status, StatusCode::OK, "{inherited}");
        assert_eq!((inherited["data"]["own"].as_bool(), inherited["data"]["regime"].as_str()), (Some(false), Some("household")));
        // Hộ kinh doanh: hoá đơn bán hàng, không tách GTGT.
        let sale = quote(
            &app,
            "north-admin",
            json!({"jurisdiction": "VN", "tax_point": "2026-06-01", "seller_org_id": tree.store.as_uuid(),
                   "price_mode": "exclusive", "currency": "VND", "lines": [line("a", "vat.standard", 100_000)]}),
        )
        .await;
        assert_eq!((sale["invoice_type"].as_str(), amount(&sale["tax"])), (Some("sales_invoice"), 0));
        // Ngoài phạm vi: nhánh Nam không thấy, không sửa được.
        let (status, hidden) = call(&app, "GET", &format!("/profiles/{}", tree.south.as_uuid()), "north-admin", None).await;
        assert_eq!((status, hidden["code"].as_str()), (StatusCode::NOT_FOUND, Some("tax.profile_not_found")));
        let other = json!({"jurisdiction": "VN", "regime": "vat_credit"});
        let (status, _) = call(&app, "PUT", &format!("/profiles/{}", tree.south.as_uuid()), "north-admin", Some(other)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // Sai dữ liệu: báo một lần.
        let bad = json!({"jurisdiction": "XX", "tax_code": "123", "regime": "nope", "business_lines": ["x"]});
        let (status, refused) = call(&app, "PUT", &format!("/profiles/{}", tree.root.as_uuid()), "ops", Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(refused["errors"].as_array().unwrap().len(), 4, "{refused}");
        // Yêu cầu hoá đơn: hộ có doanh thu từ 1 tỷ đ / năm dùng máy tính tiền.
        let engine = app.api::<dyn crate::api::TaxEngine>().unwrap();
        let tenant = app.default_tenant();
        let day = Date::new(2026, 6, 1).unwrap();
        let needs = engine.invoice_requirements(&tenant, tree.store, day).await.unwrap();
        assert!(needs.cash_register_required);
        let none = engine.invoice_requirements(&tenant, tree.south, day).await.unwrap();
        assert!(!none.cash_register_required);
        let (_, listed) = call(&app, "GET", "/profiles", "north-admin", None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn goi_cu_bi_tu_choi_goi_trung_khong_doi_va_doctor() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let mut pack: Value = serde_json::from_str(crate::pack::BUNDLED_VN).unwrap();
        // Cùng gói lần nữa: không đổi gì.
        let (status, _) =
            call(&app, "POST", "/packs", "ops", Some(json!({"pack": pack.clone()}))).await;
        assert_eq!(status, StatusCode::OK);
        let (_, packs) = call(&app, "GET", "/packs", "ops", None).await;
        assert_eq!(packs["data"].as_array().unwrap().len(), 1);
        // Khác nội dung, cùng phiên bản: từ chối.
        pack["source_note"] = json!("changed");
        let (status, refused) =
            call(&app, "POST", "/packs", "ops", Some(json!({"pack": pack.clone()}))).await;
        assert_eq!(
            (status, refused["code"].as_str()),
            (StatusCode::CONFLICT, Some("tax.pack_outdated"))
        );
        // Doctor: thuế suất hết hạn trong cửa sổ cảnh báo mà không có bản nối tiếp → cảnh báo.
        let soon = Timestamp::now()
            .date_utc()
            .checked_add_days(10)
            .unwrap()
            .format("%Y-%m-%d")
            .to_string();
        pack["version"] = json!(2);
        let rates: Vec<Value> = pack["rates"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|rate| {
                !(rate["category_code"] == "vat.standard_reducible"
                    && rate["effective_from"] == "2027-01-01")
            })
            .cloned()
            .map(|mut rate| {
                if rate["category_code"] == "vat.standard_reducible" {
                    rate["effective_to"] = json!(soon);
                }
                rate
            })
            .collect();
        pack["rates"] = json!(rates);
        let (status, body) = call(&app, "POST", "/packs", "ops", Some(json!({"pack": pack}))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["version"], 2);
        let checks = app.modules().checks();
        let (_, check) = checks.iter().find(|(module, _)| *module == "tax").unwrap();
        let text = format!("{:?}", check.run().await);
        assert!(text.contains("vat.standard_reducible"), "{text}");
        // Không phải nhân sự nền tảng thì không nhập gói được.
        let (status, _) =
            call(&app, "POST", "/packs", "north-admin", Some(json!({"pack": {}}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_lan_sua_cung_phien_ban_chi_mot_lan_qua() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let uri = format!("/api/v1/tax/profiles/{}", tree.north.as_uuid());
        let first = json!({"jurisdiction": "VN", "regime": "household"});
        let (status, body) = call(
            &app,
            "PUT",
            &format!("/profiles/{}", tree.north.as_uuid()),
            "north-admin",
            Some(first),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for round in 0..3 {
            let read = app.get(&uri).token("north-admin").send().await.unwrap();
            let etag = read.header("etag").to_owned();
            // Hai lần sửa cùng `If-Match`. Trước đây so phiên bản không khoá: Postgres cho cả hai
            // cùng qua, bản sau đè mất bản trước.
            let put = |regime: &'static str| {
                let etag = etag.clone();
                let app = &app;
                let uri = &uri;
                async move {
                    app.request(Method::PUT, uri)
                        .token("north-admin")
                        .header("If-Match", &etag)
                        .json(json!({"jurisdiction": "VN", "regime": regime}))
                        .send()
                        .await
                        .unwrap()
                        .status()
                }
            };
            let (one, two) = tokio::join!(put("vat_credit"), put("household"));
            let mut statuses = [one, two];
            statuses.sort();
            assert_eq!(
                statuses,
                [StatusCode::OK, StatusCode::PRECONDITION_FAILED],
                "round {round}"
            );
        }
    })
    .await
    .unwrap();
}
