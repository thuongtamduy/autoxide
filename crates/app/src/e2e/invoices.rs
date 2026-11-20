//! Chứng từ (bước 36) trên chương trình thật — `invoices` cùng `tax` và `ledger` — qua REST, MCP và
//! `dyn Invoices` (cả SQLite và Postgres): phát hành đồng
//! thời không nhảy số, phát hành lại cùng số, nháp không số và xoá được, đã phát hành không xoá được,
//! bị từ chối gửi lại cùng số, lập thay cần uỷ nhiệm, hoá đơn máy tính tiền, ghi sổ và huỷ đảo sổ,
//! điều chỉnh / thay thế.

use std::sync::Arc;

use autoxide_kernel::einvoice::EInvoiceProvider;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, TenantContext, root_of};
use autoxide_ledger::api::{AccountKey, Ledger};
use autoxide_testkit::{FakeEInvoiceProvider, TestApp, TestDatabase, tool};
use axum::http::{Method, StatusCode};
use chrono::{Datelike, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use autoxide_invoices::api::{Buyer, Document, Invoices, NewDocument, NewDocumentLine};

const PERMISSIONS: [&str; 9] = [
    "invoices.read",
    "invoices.write",
    "invoices.issue",
    "invoices.correct",
    "invoices.series_manage",
    "invoices.mandates_manage",
    "tax.read",
    "tax.packs_manage",
    "ledger.read",
];

struct Setup {
    app: TestApp,
    provider: Arc<FakeEInvoiceProvider>,
    seller: OrgId,
    market: OrgId,
}

async fn setup(test: &TestDatabase) -> Setup {
    let provider = Arc::new(FakeEInvoiceProvider::new());
    let mut builder = TestApp::builder(test)
        .user("clerk", 1, &PERMISSIONS)
        .contribute::<dyn EInvoiceProvider>(provider.clone());
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    let app = builder.build().await.unwrap();
    let (seller, market) = (OrgId::from_uuid(Uuid::now_v7()), OrgId::from_uuid(Uuid::now_v7()));
    let write = app.database().begin_write().await.unwrap();
    // Module `organizations` có thể đã tạo nút gốc.
    let _ = OrgTree::insert_root(&write, DEFAULT_TENANT).await;
    let root = root_of(DEFAULT_TENANT);
    OrgTree::insert(&write, DEFAULT_TENANT, seller, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, market, root, 8).await.unwrap();
    write.commit().await.unwrap();
    let pack: Value =
        serde_json::from_str(include_str!("../../../../modules/tax/packs/vn.json")).unwrap();
    let answer = app
        .post("/api/v1/tax/packs")
        .token("clerk")
        .json(json!({"pack": pack}))
        .send()
        .await
        .unwrap();
    assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
    Setup { app, provider, seller, market }
}

async fn call(app: &TestApp, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/invoices{uri}")).token("clerk");
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

fn year() -> i32 {
    Utc::now().year()
}

async fn series(app: &TestApp, kind: &str, symbol: &str, provider: Option<&str>) -> String {
    let (status, created) = call(
        app,
        "POST",
        "/series",
        Some(
            json!({"kind": kind, "symbol": symbol, "year": year(), "einvoice_provider": provider}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["data"]["id"].as_str().unwrap().to_owned()
}

fn invoices(app: &TestApp) -> Arc<dyn Invoices> {
    app.api::<dyn Invoices>().unwrap()
}

fn vnd(minor: i64) -> Money {
    Money::new(minor, Currency::VND)
}

fn sale(reference: &str, price: i64) -> NewDocument {
    let line = NewDocumentLine::new("Service fee", rust_decimal::Decimal::ONE, vnd(price))
        .service()
        .tax_category("vat.standard");
    NewDocument::new(
        root_of(DEFAULT_TENANT),
        "vat_invoice",
        Currency::VND,
        Buyer::business("Buyer Co", "0101234567"),
        vec![line],
        "demo",
        reference,
    )
}

async fn draft(app: &TestApp, reference: &str, price: i64) -> Document {
    let tenant = app.default_tenant();
    invoices(app)
        .create_document(&tenant, Actor::system(), sale(reference, price))
        .await
        .unwrap()
        .into_inner()
}

async fn balance(app: &TestApp, tenant: &TenantContext, purpose: &str) -> i64 {
    let ledger = app.api::<dyn Ledger>().unwrap();
    let key = AccountKey::new(root_of(DEFAULT_TENANT), purpose, Currency::VND);
    ledger.balance(tenant, key, None).await.unwrap().minor()
}

/// Chạy hết job đang chờ (gửi hoá đơn điện tử…).
async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    for _ in 0..5 {
        if worker.run_once().await.unwrap() == 0 {
            break;
        }
    }
}

#[tokio::test]
async fn phat_hanh_dong_thoi_khong_nhay_so() {
    TestDatabase::run(async |test| {
        let Setup { app, .. } = setup(test).await;
        series(&app, "vat_invoice", "1C26TAA", None).await;
        let mut ids = Vec::new();
        for index in 0..20 {
            ids.push(draft(&app, &format!("D-{index}"), 100_000).await.id);
        }
        let tenant = app.default_tenant();
        let tasks: Vec<_> = ids
            .iter()
            .map(|id| {
                let (service, tenant, id) = (invoices(&app), tenant.clone(), *id);
                tokio::spawn(async move { service.issue(&tenant, Actor::system(), id).await })
            })
            .collect();
        let mut numbers = Vec::new();
        for task in tasks {
            numbers.push(task.await.unwrap().unwrap().number.unwrap());
        }
        numbers.sort_unstable();
        assert_eq!(numbers, (1..=20).collect::<Vec<i64>>());
        // Phát hành lại: cùng số, không ghi sổ thêm.
        let again = invoices(&app).issue(&tenant, Actor::system(), ids[0]).await.unwrap();
        let first = invoices(&app).document(&tenant, ids[0]).await.unwrap().unwrap();
        assert_eq!(again.number, first.number);
        // 20 hoá đơn 100.000 đ + GTGT 10% (hoặc 8% theo ngày): phải thu = tổng các hoá đơn.
        let total: i64 = 20 * first.total.minor();
        assert_eq!(balance(&app, &tenant, "receivable.customer").await, total);
        assert_eq!(balance(&app, &tenant, "revenue.sales").await, 20 * 100_000);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhap_khong_so_xoa_duoc_da_phat_hanh_thi_khong() {
    TestDatabase::run(async |test| {
        let Setup { app, .. } = setup(test).await;
        series(&app, "vat_invoice", "1C26TAA", None).await;
        let body = json!({"kind": "vat_invoice", "currency": "VND", "reference": "R-1",
            "buyer": {"kind": "business", "name": "Buyer Co", "tax_code": "0101234567"},
            "lines": [{"description": "Consulting", "kind": "service", "quantity": "2",
                       "unit_price": {"amount": "500000", "currency": "VND"}, "tax_category": "vat.standard"}]});
        let (status, created) = call(&app, "POST", "/documents", Some(body.clone())).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        assert_eq!(created["data"]["number"], Value::Null);
        assert_eq!(created["data"]["subtotal"]["amount"], "1000000");
        let (status, again) = call(&app, "POST", "/documents", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(again["data"]["id"], created["data"]["id"]);
        let id = created["data"]["id"].as_str().unwrap();
        let (status, _) = call(&app, "DELETE", &format!("/documents/{id}"), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        // Đã phát hành: giữ ít nhất 10 năm.
        let issued = draft(&app, "R-2", 100_000).await;
        let (status, done) = call(&app, "POST", &format!("/documents/{}/issue", issued.id), None).await;
        assert_eq!(status, StatusCode::OK, "{done}");
        assert_eq!(done["data"]["number"], 1);
        let until = done["data"]["retention_until"].as_str().unwrap();
        assert_eq!(until, format!("{}-12-31", year() + 10));
        let (status, kept) = call(&app, "DELETE", &format!("/documents/{}", issued.id), None).await;
        assert_eq!(status, StatusCode::CONFLICT, "{kept}");
        assert_eq!(kept["code"], "invoices.retention_hold");
        let (status, _) = call(&app, "PATCH", &format!("/documents/{}", issued.id), Some(json!({"lines": []}))).await;
        assert_eq!(status, StatusCode::CONFLICT);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bi_tu_choi_gui_lai_cung_so_roi_duoc_chap_nhan() {
    TestDatabase::run(async |test| {
        let Setup { app, provider, .. } = setup(test).await;
        series(&app, "vat_invoice", "1C26TAA", Some("fake")).await;
        let found = draft(&app, "R-1", 200_000).await;
        provider.reject_next("buyer tax code does not exist");
        let (status, issued) =
            call(&app, "POST", &format!("/documents/{}/issue", found.id), None).await;
        assert_eq!(status, StatusCode::OK, "{issued}");
        drain(&app).await;
        let (_, rejected) = call(&app, "GET", &format!("/documents/{}", found.id), None).await;
        assert_eq!(rejected["data"]["status"], "rejected", "{rejected}");
        assert_eq!(rejected["data"]["rejection_reason"], "buyer tax code does not exist");
        // Chỉ sửa được người mua.
        let (status, _) = call(
            &app,
            "PATCH",
            &format!("/documents/{}", found.id),
            Some(json!({"issue_date": null})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        let fixed =
            json!({"buyer": {"kind": "business", "name": "Buyer Co", "tax_code": "0109999999"}});
        let (status, _) =
            call(&app, "PATCH", &format!("/documents/{}", found.id), Some(fixed)).await;
        assert_eq!(status, StatusCode::OK);
        let (status, again) =
            call(&app, "POST", &format!("/documents/{}/issue", found.id), None).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        drain(&app).await;
        let (_, accepted) = call(&app, "GET", &format!("/documents/{}", found.id), None).await;
        assert_eq!(accepted["data"]["status"], "accepted", "{accepted}");
        assert_eq!(accepted["data"]["number"], 1);
        assert!(accepted["data"]["authority_code"].as_str().is_some());
        let sent = provider.issued.lock().unwrap().clone();
        assert_eq!(sent.len(), 2);
        assert!(sent.iter().all(|invoice| invoice.number == Some(1)));
        assert_eq!(sent[1].buyer.tax_code.as_deref(), Some("0109999999"));
        let answer = app
            .get(&format!("/api/v1/invoices/documents/{}/einvoice?format=xml", found.id))
            .token("clerk")
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::OK);
        assert!(answer.text().starts_with("<invoice"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lap_thay_can_uy_nhiem_va_hoa_don_may_tinh_tien() {
    TestDatabase::run(async |test| {
        let Setup { app, provider, seller, market } = setup(test).await;
        let tenant = app.default_tenant();
        // Sàn lập hoá đơn máy tính tiền thay người bán.
        let (status, _) = call(
            &app,
            "POST",
            "/series",
            Some(json!({"org_id": market.as_uuid(), "kind": "cash_register_invoice", "symbol": "1C26MTT",
                        "year": year(), "einvoice_provider": "fake"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let line = NewDocumentLine::new("Coffee", rust_decimal::Decimal::from(2), vnd(30_000)).tax_category("vat.standard");
        let draft = NewDocument::new(market, "cash_register_invoice", Currency::VND, Buyer::consumer(None), vec![line], "demo", "POS-1")
            .on_behalf_of(seller)
            .prices_include_tax();
        let created = invoices(&app).create_document(&tenant, Actor::system(), draft).await.unwrap().into_inner();
        assert_eq!(created.total.minor(), 60_000);
        let refused = invoices(&app).issue(&tenant, Actor::system(), created.id).await.unwrap_err();
        assert_eq!(refused.code(), "invoices.mandate_missing");
        let (status, mandate) = call(
            &app,
            "POST",
            "/mandates",
            Some(json!({"principal_org_id": seller.as_uuid(), "agent_org_id": market.as_uuid(),
                        "kinds": ["cash_register_invoice"], "valid_from": "2020-01-01"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{mandate}");
        let issued = invoices(&app).issue(&tenant, Actor::system(), created.id).await.unwrap();
        assert_eq!(issued.number, Some(1));
        drain(&app).await;
        let sent = provider.issued.lock().unwrap().clone();
        assert_eq!(sent[0].kind, autoxide_kernel::einvoice::EInvoiceKind::CashRegister);
        assert!(sent[0].on_behalf_of.is_some());
        // Doanh thu vào sổ của người bán, không phải sổ của sàn.
        let ledger = app.api::<dyn Ledger>().unwrap();
        let in_seller = ledger.balance(&tenant, AccountKey::new(seller, "receivable.customer", Currency::VND), None).await.unwrap();
        assert_eq!(in_seller.minor(), 60_000);
        let in_market = ledger.balance(&tenant, AccountKey::new(market, "receivable.customer", Currency::VND), None).await.unwrap();
        assert_eq!(in_market.minor(), 0);
        // Thu hồi uỷ nhiệm thì không lập thay được nữa.
        let id = mandate["data"]["id"].as_str().unwrap();
        let (status, _) = call(&app, "POST", &format!("/mandates/{id}/revoke"), None).await;
        assert_eq!(status, StatusCode::OK);
        let line = NewDocumentLine::new("Tea", rust_decimal::Decimal::ONE, vnd(20_000)).tax_category("vat.standard");
        let next = NewDocument::new(market, "cash_register_invoice", Currency::VND, Buyer::consumer(None), vec![line], "demo", "POS-2")
            .on_behalf_of(seller);
        let next = invoices(&app).create_document(&tenant, Actor::system(), next).await.unwrap().into_inner();
        let refused = invoices(&app).issue(&tenant, Actor::system(), next.id).await.unwrap_err();
        assert_eq!(refused.code(), "invoices.mandate_missing");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn huy_dao_so_thay_the_va_dieu_chinh() {
    TestDatabase::run(async |test| {
        let Setup { app, provider, .. } = setup(test).await;
        series(&app, "vat_invoice", "1C26TAA", Some("fake")).await;
        let tenant = app.default_tenant();
        let first = draft(&app, "R-1", 100_000).await;
        let issued = invoices(&app).issue(&tenant, Actor::system(), first.id).await.unwrap();
        drain(&app).await;
        let total = issued.total.minor();
        assert_eq!(balance(&app, &tenant, "receivable.customer").await, total);
        // Thay thế: hoá đơn gốc `replaced`, bút toán gốc bị đảo, bút toán mới theo số mới.
        let (status, replacement) = call(
            &app,
            "POST",
            &format!("/documents/{}/replace", first.id),
            Some(json!({"reference": "REPL-1", "lines": [{"description": "Service fee", "kind": "service",
                "unit_price": {"amount": "150000", "currency": "VND"}, "tax_category": "vat.standard"}]})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{replacement}");
        let replacement_id = replacement["data"]["id"].as_str().unwrap().parse().unwrap();
        let replaced = invoices(&app).issue(&tenant, Actor::system(), replacement_id).await.unwrap();
        assert_eq!(replaced.number, Some(2));
        let original = invoices(&app).document(&tenant, first.id).await.unwrap().unwrap();
        assert_eq!(original.status, "replaced");
        assert_eq!(balance(&app, &tenant, "receivable.customer").await, replaced.total.minor());
        drain(&app).await;
        assert_eq!(provider.issued.lock().unwrap().last().unwrap().related.as_ref().unwrap().number, 1);
        // Điều chỉnh giảm 50.000 đ trước thuế.
        let (status, adjustment) = call(
            &app,
            "POST",
            &format!("/documents/{replacement_id}/adjust"),
            Some(json!({"reference": "ADJ-1", "direction": "decrease", "lines": [{"description": "Discount",
                "kind": "service", "unit_price": {"amount": "50000", "currency": "VND"}, "tax_category": "vat.standard"}]})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{adjustment}");
        let adjustment_id = adjustment["data"]["id"].as_str().unwrap().parse().unwrap();
        let adjusted = invoices(&app).issue(&tenant, Actor::system(), adjustment_id).await.unwrap();
        drain(&app).await;
        assert_eq!(
            balance(&app, &tenant, "receivable.customer").await,
            replaced.total.minor() - adjusted.total.minor()
        );
        // Huỷ hoá đơn điều chỉnh: đảo bút toán của nó.
        let (status, cancelled) = call(
            &app,
            "POST",
            &format!("/documents/{adjustment_id}/cancel"),
            Some(json!({"reason": "Wrong amount"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{cancelled}");
        assert_eq!(cancelled["data"]["status"], "cancelled");
        assert_eq!(balance(&app, &tenant, "receivable.customer").await, replaced.total.minor());
        assert_eq!(provider.cancelled.lock().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tool_mcp_lap_nhap_va_phat_hanh() {
    TestDatabase::run(async |test| {
        let Setup { app, .. } = setup(test).await;
        series(&app, "sales_invoice", "2C26TBB", None).await;
        let client = app.mcp("clerk").await.unwrap();
        let answer = client
            .call_tool(tool(
                "invoices.create_draft",
                json!({"kind": "sales_invoice", "currency": "VND", "reference": "M-1",
                       "lines": [{"description": "Book", "unit_price": "120000", "tax_category": "vat.reduced_5"}]}),
            ))
            .await
            .unwrap();
        assert_ne!(answer.is_error, Some(true), "{answer:?}");
        let created = answer.structured_content.clone().unwrap_or_else(|| panic!("{answer:?}"));
        let id = created["data"]["id"].as_str().unwrap_or_else(|| panic!("{created}"));
        let answer = client.call_tool(tool("invoices.issue", json!({"document_id": id}))).await.unwrap();
        assert_ne!(answer.is_error, Some(true), "{answer:?}");
        assert_eq!(answer.structured_content.unwrap()["data"]["number"], 1);
        let answer = client.call_tool(tool("invoices.documents", json!({"status": "issued"}))).await.unwrap();
        let text = serde_json::to_string(&answer.structured_content).unwrap();
        assert!(text.contains(id), "{text}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hoi_lai_nha_cung_cap_hoa_don_ngay() {
    use sea_orm::ConnectionTrait as _;
    TestDatabase::run(async |test| {
        let Setup { app, provider, .. } = setup(test).await;
        series(&app, "vat_invoice", "1C26TAA", Some("fake")).await;
        let found = draft(&app, "R-1", 200_000).await;
        let refresh = format!("/documents/{}/refresh", found.id);
        // Nháp: không có gì để hỏi, trả nguyên.
        let (status, same) = call(&app, "POST", &refresh, None).await;
        assert_eq!(status, StatusCode::OK, "{same}");
        assert_eq!(same["data"]["status"], "draft");

        let (status, _) = call(&app, "POST", &format!("/documents/{}/issue", found.id), None).await;
        assert_eq!(status, StatusCode::OK);
        drain(&app).await;
        // Giả như lần hỏi định kỳ chưa tới: chứng từ còn chờ cơ quan thuế.
        let write = app.database().write();
        let backend = write.get_database_backend();
        write
            .execute_raw(sea_orm::Statement::from_sql_and_values(
                backend,
                "UPDATE invoices_documents SET status = 'submitting', authority_code = NULL \
                 WHERE id = $1",
                [found.id.into()],
            ))
            .await
            .unwrap();

        provider.fail_next(1);
        let (status, problem) = call(&app, "POST", &refresh, None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{problem}");
        assert_eq!(problem["code"], "einvoice.provider_unavailable");
        let (status, accepted) = call(&app, "POST", &refresh, None).await;
        assert_eq!(status, StatusCode::OK, "{accepted}");
        assert_eq!(accepted["data"]["status"], "accepted");
        assert!(accepted["data"]["authority_code"].as_str().is_some());

        let missing = format!("/documents/{}/refresh", Uuid::now_v7());
        let (status, _) = call(&app, "POST", &missing, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tong_hop_gtgt_dau_ra_theo_ky() {
    TestDatabase::run(async |test| {
        let Setup { app, .. } = setup(test).await;
        let tenant = app.default_tenant();
        series(&app, "vat_invoice", "1C26TAA", Some("fake")).await;
        let kept = draft(&app, "V-1", 200_000).await;
        let kept = invoices(&app).issue(&tenant, Actor::system(), kept.id).await.unwrap();
        let dropped = draft(&app, "V-2", 100_000).await;
        invoices(&app).issue(&tenant, Actor::system(), dropped.id).await.unwrap();
        drain(&app).await;
        let (status, _) = call(
            &app,
            "POST",
            &format!("/documents/{}/cancel", dropped.id),
            Some(json!({"reason": "Wrong buyer"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // Điều chỉnh giảm 50.000 đ trước thuế cho chứng từ còn lại.
        let (status, adjustment) = call(
            &app,
            "POST",
            &format!("/documents/{}/adjust", kept.id),
            Some(json!({"reference": "ADJ-V", "direction": "decrease", "lines": [{"description": "Discount",
                "kind": "service", "unit_price": {"amount": "50000", "currency": "VND"}, "tax_category": "vat.standard"}]})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{adjustment}");
        let adjustment_id = adjustment["data"]["id"].as_str().unwrap().parse().unwrap();
        let adjusted = invoices(&app).issue(&tenant, Actor::system(), adjustment_id).await.unwrap();
        drain(&app).await;

        let today = Utc::now().date_naive();
        let uri = format!("/vat-summary?from={}&to={today}", today.with_day(1).unwrap());
        let (status, summary) = call(&app, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{summary}");
        let rows = summary["data"]["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{summary}");
        let row = &rows[0];
        assert_eq!(row["kind"], "vat_invoice");
        assert_eq!(row["rate_percent"], "10");
        let net = kept.subtotal.minor() - adjusted.subtotal.minor();
        let tax = kept.tax.minor() - adjusted.tax.minor();
        assert_eq!(row["net"], json!({"amount": net.to_string(), "currency": "VND"}));
        assert_eq!(row["tax"], json!({"amount": tax.to_string(), "currency": "VND"}));
        assert_eq!(row["documents"], 2);

        let (status, problem) =
            call(&app, "GET", "/vat-summary?from=2026-01-01&to=2025-01-01", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        let (status, _) = call(&app, "GET", "/vat-summary", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ben_lap_tu_ghi_so_thi_phat_hanh_khong_ghi_lan_hai_va_dieu_chinh_qua_trait() {
    TestDatabase::run(async |test| {
        let Setup { app, .. } = setup(test).await;
        series(&app, "vat_invoice", "1C26TAA", None).await;
        let tenant = app.default_tenant();
        // `orders` đã ghi doanh thu khi giao: hoá đơn của nó mang `booked_by_source`.
        let draft = sale("ORD-1", 200_000).booked_by_source();
        let created = invoices(&app)
            .create_document(&tenant, Actor::system(), draft)
            .await
            .unwrap()
            .into_inner();
        let issued = invoices(&app).issue(&tenant, Actor::system(), created.id).await.unwrap();
        assert_eq!(issued.status, "issued");
        assert_eq!(balance(&app, &tenant, "receivable.customer").await, 0);
        assert_eq!(balance(&app, &tenant, "revenue.sales").await, 0);
        // Hoá đơn điều chỉnh qua `dyn Invoices::adjust` (trả hàng): kế thừa cờ, không ghi sổ.
        let line = NewDocumentLine::new("Service fee", rust_decimal::Decimal::ONE, vnd(50_000))
            .service()
            .tax_category("vat.standard");
        let adjustment = invoices(&app)
            .adjust(
                &tenant,
                Actor::system(),
                created.id,
                "decrease",
                vec![line.clone()],
                "ORD-1-R1".to_owned(),
            )
            .await
            .unwrap();
        assert!(matches!(adjustment, autoxide_kernel::reference::Submitted::Created(_)));
        let adjustment = adjustment.into_inner();
        assert_eq!(adjustment.relation.as_deref(), Some("adjustment"));
        assert_eq!(adjustment.direction, "decrease");
        // Gọi lại cùng mã: chứng từ cũ.
        let again = invoices(&app)
            .adjust(
                &tenant,
                Actor::system(),
                created.id,
                "decrease",
                vec![line],
                "ORD-1-R1".to_owned(),
            )
            .await
            .unwrap();
        assert_eq!(again.into_inner().id, adjustment.id);
        invoices(&app).issue(&tenant, Actor::system(), adjustment.id).await.unwrap();
        let original = invoices(&app).document(&tenant, created.id).await.unwrap().unwrap();
        assert_eq!(original.status, "adjusted");
        assert_eq!(balance(&app, &tenant, "receivable.customer").await, 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_nhap_thay_the_cung_mot_hoa_don_chi_phat_hanh_mot() {
    TestDatabase::run(async |test| {
        let Setup { app, .. } = setup(test).await;
        series(&app, "vat_invoice", "1C26TAA", Some("fake")).await;
        let tenant = app.default_tenant();
        let first = draft(&app, "R-1", 100_000).await;
        invoices(&app).issue(&tenant, Actor::system(), first.id).await.unwrap();
        drain(&app).await;
        let mut drafts = Vec::new();
        for reference in ["REPL-1", "REPL-2", "REPL-3"] {
            let (status, replacement) = call(
                &app,
                "POST",
                &format!("/documents/{}/replace", first.id),
                Some(json!({"reference": reference, "lines": [{"description": "Service fee", "kind": "service",
                    "unit_price": {"amount": "150000", "currency": "VND"}, "tax_category": "vat.standard"}]})),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{replacement}");
            drafts.push(replacement["data"]["id"].as_str().unwrap().parse::<uuid::Uuid>().unwrap());
        }
        // Hai nháp thay thế phát hành cùng lúc: chỉ một được (Postgres từng cho cả hai, doanh thu
        // ghi hai lần).
        let service = invoices(&app);
        let (one, two) = tokio::join!(
            service.issue(&tenant, Actor::system(), drafts[0]),
            service.issue(&tenant, Actor::system(), drafts[1]),
        );
        let (issued, refused) = match (one, two) {
            (Ok(issued), Err(error)) | (Err(error), Ok(issued)) => (issued, error),
            (one, two) => panic!("exactly one replacement must issue: {one:?} / {two:?}"),
        };
        assert_eq!(refused.code(), "invoices.not_correctable");
        // Nháp thứ ba, phát hành sau: cũng bị từ chối.
        let refused = service.issue(&tenant, Actor::system(), drafts[2]).await.unwrap_err();
        assert_eq!(refused.code(), "invoices.not_correctable");
        drain(&app).await;
        // Bút toán gốc đảo một lần, chỉ bút toán của một hoá đơn thay thế còn lại.
        assert_eq!(balance(&app, &tenant, "receivable.customer").await, issued.total.minor());
        let current = service.document(&tenant, first.id).await.unwrap().unwrap();
        assert_eq!(current.status, "replaced");
    })
    .await
    .unwrap();
}
