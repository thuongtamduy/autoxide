//! Bước 36 mục 7, 10, 11, 12 trên chương trình thật (`invoices` cùng `tax`, `ledger`, `files`), cả
//! SQLite và Postgres: chứng từ ngoại tệ chụp tỉ giá, mẫu in HTML / PDF và mẫu của khách, xuất kế toán
//! CSV / XLSX cân nợ có, bản lưu trữ có băm khớp, dọn chứng từ hết hạn lưu (giữ chứng từ còn hạn, kể cả
//! sau khi xoá hẳn khách).

use autoxide_kernel::time::{Date, Timestamp};
use std::sync::Arc;

use autoxide_kernel::einvoice::EInvoiceProvider;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::storage::Storage;
use autoxide_kernel::tenancy::{
    DEFAULT_TENANT, OrgTree, TenantAdmin, TenantChanges, TenantContext, root_of,
};
use autoxide_testkit::{FakeEInvoiceProvider, TestApp, TestDatabase, tool};
use axum::http::{Method, StatusCode};
use futures_util::TryStreamExt as _;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use autoxide_invoices::api::{Buyer, Document, Invoices, NewDocument, NewDocumentLine};

const PERMISSIONS: [&str; 14] = [
    "invoices.read",
    "invoices.write",
    "invoices.issue",
    "invoices.series_manage",
    "invoices.templates_manage",
    "invoices.export",
    "invoices.archive",
    "tax.read",
    "tax.packs_manage",
    "ledger.read",
    "ledger.settings_manage",
    "kernel.tenants.read",
    "kernel.tenants.manage",
    "kernel.tenants.delete",
];

async fn app(test: &TestDatabase) -> TestApp {
    let dir = std::env::temp_dir().join(format!("autoxide-invoices-{}", uuid::Uuid::now_v7()));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_FILES_DIR", &dir.display().to_string())
        .user("clerk", 1, &PERMISSIONS)
        .user("reader", 2, &["invoices.read"])
        .tenant("acme")
        .user_in("acme", "acme", 3, &PERMISSIONS)
        .contribute::<dyn EInvoiceProvider>(Arc::new(FakeEInvoiceProvider::new()));
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    let app = builder.build().await.unwrap();
    for slug in ["default", "acme"] {
        let tenant =
            if slug == "default" { app.default_tenant() } else { app.tenant(slug).await.unwrap() };
        let write = tenant.database().begin_write().await.unwrap();
        let _ = OrgTree::insert_root(&write, tenant.id()).await;
        write.commit().await.unwrap();
    }
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
    app
}

async fn call(
    app: &TestApp,
    who: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, uri).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = serde_json::from_slice(answer.body()).unwrap_or(Value::Null);
    (answer.status(), body)
}

async fn series(app: &TestApp, who: &str, kind: &str) {
    let body =
        json!({"kind": kind, "symbol": "1C26TAA", "year": Timestamp::now().date_utc().year()});
    let (status, created) = call(app, who, "POST", "/api/v1/invoices/series", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
}

fn sale(
    tenant: &TenantContext,
    currency: Currency,
    price: i64,
    buyer: Buyer,
    reference: &str,
) -> NewDocument {
    let line = NewDocumentLine::new(
        "Tư vấn <b>đặc biệt</b>",
        rust_decimal::Decimal::ONE,
        Money::new(price, currency),
    )
    .service()
    .tax_category("vat.standard")
    .unit("giờ");
    NewDocument::new(
        root_of(tenant.id()),
        "vat_invoice",
        currency,
        buyer,
        vec![line],
        "demo",
        reference,
    )
}

async fn issued(app: &TestApp, tenant: &TenantContext, document: NewDocument) -> Document {
    let invoices = app.api::<dyn Invoices>().unwrap();
    let draft =
        invoices.create_document(tenant, Actor::system(), document).await.unwrap().into_inner();
    invoices.issue(tenant, Actor::system(), draft.id).await.unwrap()
}

async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    for _ in 0..6 {
        if worker.run_once().await.unwrap() == 0 {
            break;
        }
    }
}

#[tokio::test]
async fn chung_tu_ngoai_te_chup_ti_gia_khi_phat_hanh() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let tenant = app.default_tenant();
        series(&app, "clerk", "vat_invoice").await;
        let invoices = app.api::<dyn Invoices>().unwrap();
        let business = Buyer::business("Foreign Buyer Ltd", "0101234567");
        // Chưa có tỉ giá EUR: phát hành bị từ chối, số không bị tiêu.
        let draft = invoices
            .create_document(&tenant, Actor::system(), sale(&tenant, Currency::EUR, 10_000, business.clone(), "E-1"))
            .await
            .unwrap()
            .into_inner();
        let error = invoices.issue(&tenant, Actor::system(), draft.id).await.unwrap_err();
        assert_eq!(error.code(), "invoices.rate_missing");
        let today = Timestamp::now().date_utc();
        let rate = json!({"base": "USD", "quote": "VND", "rate": "25000", "effective_date": today.pred().unwrap()});
        let (status, answer) = call(&app, "clerk", "POST", "/api/v1/ledger/exchange-rates", Some(rate)).await;
        assert_eq!(status, StatusCode::CREATED, "{answer}");
        let document = issued(&app, &tenant, sale(&tenant, Currency::USD, 10_000, business, "U-1")).await;
        assert_eq!(document.number, Some(1));
        assert_eq!(document.exchange_rate.as_deref(), Some("25000"));
        let base = document.base_total.unwrap();
        assert_eq!(base.currency(), Currency::VND);
        // total USD (đơn vị cent) × 25000 / 100.
        assert_eq!(base.minor(), document.total.minor() * 250);
        // Tỉ giá mới không đổi chứng từ đã phát hành; bút toán ghi cùng tỉ giá.
        let newer = json!({"base": "USD", "quote": "VND", "rate": "26000", "effective_date": today});
        call(&app, "clerk", "POST", "/api/v1/ledger/exchange-rates", Some(newer)).await;
        let again = invoices.document(&tenant, document.id).await.unwrap().unwrap();
        assert_eq!(again.exchange_rate.as_deref(), Some("25000"));
        let (_, entries) = call(
            &app,
            "clerk",
            "GET",
            &format!("/api/v1/ledger/entries?source_module=invoices&source_id={}", document.id),
            None,
        )
        .await;
        let lines = entries["data"][0]["lines"].as_array().unwrap().clone();
        assert!(lines.iter().all(|line| line["exchange_rate"] == "25000"), "{entries}");
        let debit: i64 = lines
            .iter()
            .filter(|line| line["side"] == "debit")
            .map(|line| line["base_amount"]["amount"].as_str().unwrap().parse::<i64>().unwrap())
            .sum();
        assert_eq!(debit, base.minor());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mau_in_html_pdf_va_mau_cua_khach() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let tenant = app.default_tenant();
        series(&app, "clerk", "vat_invoice").await;
        let buyer = Buyer::business("Công ty <script>alert(1)</script>", "0101234567").address("Hà Nội");
        let document = issued(&app, &tenant, sale(&tenant, Currency::VND, 1_000_000, buyer, "P-1")).await;
        let render = format!("/api/v1/invoices/documents/{}/render", document.id);
        let answer = app.get(&format!("{render}?locale=vi")).token("reader").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK);
        assert!(answer.header("content-type").starts_with("text/html"));
        let html = answer.text();
        assert!(html.contains("Hoá đơn giá trị gia tăng"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "dữ liệu người mua phải được thoát HTML");
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("1.000.000 ₫"), "{html}");
        let pdf = app.get(&format!("{render}?format=pdf&locale=vi")).token("reader").send().await.unwrap();
        assert_eq!(pdf.status(), StatusCode::OK);
        assert_eq!(pdf.header("content-type"), "application/pdf");
        assert!(pdf.body().starts_with(b"%PDF"));
        assert!(pdf.body().len() > 1000);
        // Ngôn ngữ, định dạng sai: lỗi gom một lần; quyền kiểm trước.
        let (status, answer) = call(&app, "reader", "GET", &format!("{render}?format=doc&locale=fr"), None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
        assert_eq!(answer["errors"].as_array().unwrap().len(), 2);
        // Mẫu sai: mọi lỗi (biến lạ, thiếu biến bắt buộc) báo một lần kèm dòng.
        let bad = json!({"kind": "vat_invoice", "locale": "vi", "body": "<p>{{ secret }}</p>\n{{ env.HOME }}"});
        let (status, answer) = call(&app, "reader", "POST", "/api/v1/invoices/templates", Some(bad.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "người không có quyền nhận 403 trước khi mẫu bị kiểm");
        let (status, answer2) = call(&app, "clerk", "POST", "/api/v1/invoices/templates", Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer2}");
        let codes: Vec<&str> = answer2["errors"].as_array().unwrap().iter().map(|e| e["code"].as_str().unwrap()).collect();
        assert_eq!(codes.iter().filter(|code| **code == "template.unknown_variable").count(), 2, "{answer2}");
        assert!(codes.contains(&"template.required_variable_missing"), "{answer2}");
        let _ = answer;
        let body = "<h1>MẪU RIÊNG {{ document.number }}</h1><p>{{ buyer.name }}</p><p>{{ totals.total|money }}</p>";
        let good = json!({"kind": "vat_invoice", "locale": "vi", "body": body});
        let (status, saved) = call(&app, "clerk", "POST", "/api/v1/invoices/templates", Some(good)).await;
        assert_eq!(status, StatusCode::CREATED, "{saved}");
        let html = app.get(&format!("{render}?locale=vi")).token("reader").send().await.unwrap().text();
        assert!(html.starts_with("<h1>MẪU RIÊNG 1</h1>"), "{html}");
        assert!(html.contains("&lt;script&gt;"));
        // Tiếng Anh vẫn dùng mẫu mặc định.
        let html = app.get(&render).token("reader").send().await.unwrap().text();
        assert!(html.contains("VAT invoice"), "{html}");
        let id = saved["data"]["id"].as_str().unwrap();
        let (status, changed) = call(
            &app,
            "clerk",
            "PUT",
            &format!("/api/v1/invoices/templates/{id}"),
            Some(json!({"body": "v2 {{ document.number }}"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{changed}");
        assert_eq!(changed["data"]["version"], 2);
        let (status, preview) = call(
            &app,
            "clerk",
            "POST",
            "/api/v1/invoices/templates/preview",
            Some(json!({"kind": "delivery_note", "locale": "en", "body": "{{ labels.title[document.kind] }} {{ document.number }}"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{preview}");
        assert_eq!(preview["data"]["html"], "Delivery note 123");
        let (status, defaults) = call(&app, "reader", "GET", "/api/v1/invoices/template-defaults", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(defaults["data"]["variables"].as_array().unwrap().iter().any(|v| v["name"] == "document.number" && v["required"] == true));
        let (status, _) = call(&app, "clerk", "DELETE", &format!("/api/v1/invoices/templates/{id}"), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        // Phiếu giao hàng (không phải chứng từ bán hàng), bản nháp: PDF có chữ "bản nháp".
        let invoices = app.api::<dyn Invoices>().unwrap();
        let note = NewDocument::new(
            root_of(DEFAULT_TENANT),
            "delivery_note",
            Currency::VND,
            Buyer::consumer(Some("Nguyễn Văn A".to_owned())),
            vec![NewDocumentLine::new("Áo thun", rust_decimal::Decimal::TWO, Money::new(150_000, Currency::VND))],
            "demo",
            "N-1",
        );
        let note = invoices.create_document(&tenant, Actor::system(), note).await.unwrap().into_inner();
        let rendered = invoices
            .render(&tenant, note.id, autoxide_invoices::api::RenderFormat::Pdf, "vi")
            .await
            .unwrap();
        assert!(rendered.body.starts_with(b"%PDF"));
        // Tool MCP đi cùng đường.
        let client = app.mcp("reader").await.unwrap();
        let answer = client
            .call_tool(tool("invoices.print", json!({"document_id": document.id, "locale": "vi"})))
            .await
            .unwrap();
        let text = serde_json::to_string(&answer.structured_content).unwrap();
        assert!(text.contains("Hoá đơn giá trị gia tăng"), "{text}");
    })
    .await
    .unwrap();
}

/// Đọc CSV (RFC 4180, ô trong ngoặc kép, CRLF) đã bỏ BOM.
fn read_csv(bytes: &[u8]) -> Vec<Vec<String>> {
    let text = std::str::from_utf8(bytes).unwrap().strip_prefix('\u{feff}').unwrap();
    let mut rows = Vec::new();
    for line in text.split("\r\n").filter(|line| !line.is_empty()) {
        let mut cells = Vec::new();
        let mut cell = String::new();
        let mut quoted = false;
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            match (c, quoted) {
                ('"', true) if chars.peek() == Some(&'"') => {
                    chars.next();
                    cell.push('"');
                }
                ('"', _) => quoted = !quoted,
                (',', false) => cells.push(std::mem::take(&mut cell)),
                (other, _) => cell.push(other),
            }
        }
        cells.push(cell);
        rows.push(cells);
    }
    rows
}

async fn export(app: &TestApp, profile: &str, format: &str) -> Vec<u8> {
    let today = Timestamp::now().date_utc();
    let body =
        json!({"profile": profile, "format": format, "from": today.first_of_month(), "to": today});
    let (status, created) =
        call(app, "clerk", "POST", "/api/v1/invoices/exports", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["data"]["id"].as_str().unwrap().to_owned();
    let (status, early) =
        call(app, "clerk", "GET", &format!("/api/v1/invoices/exports/{id}/download"), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{early}");
    assert_eq!(early["code"], "invoices.export_not_ready");
    drain(app).await;
    let (_, done) =
        call(app, "clerk", "GET", &format!("/api/v1/invoices/exports/{id}"), None).await;
    assert_eq!(done["data"]["status"], "ready", "{done}");
    let file = app
        .get(&format!("/api/v1/invoices/exports/{id}/download"))
        .token("clerk")
        .send()
        .await
        .unwrap();
    assert_eq!(file.status(), StatusCode::OK);
    file.body().to_vec()
}

fn number(text: &str) -> i64 {
    if text.is_empty() {
        0
    } else {
        text.parse::<rust_decimal::Decimal>().unwrap().trunc().to_string().parse().unwrap()
    }
}

#[tokio::test]
async fn xuat_ke_toan_csv_xlsx_can_no_co() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let tenant = app.default_tenant();
        series(&app, "clerk", "vat_invoice").await;
        let mut totals = 0;
        for (index, price) in [1_000_000, 2_500_000].iter().enumerate() {
            let buyer = Buyer::business("=HYPERLINK(\"http://evil\")", "0101234567");
            let doc = issued(
                &app,
                &tenant,
                sale(&tenant, Currency::VND, *price, buyer, &format!("X-{index}")),
            )
            .await;
            totals += doc.total.minor();
        }
        // Người chỉ đọc không xuất được.
        let (status, _) =
            call(&app, "reader", "POST", "/api/v1/invoices/exports", Some(json!({}))).await;
        assert!(status == StatusCode::FORBIDDEN || status == StatusCode::UNPROCESSABLE_ENTITY);
        let rows = read_csv(&export(&app, "generic_journal", "csv").await);
        assert_eq!(rows[0].len(), 15);
        assert!(rows[0][6].starts_with("Nợ") && rows[0][7].starts_with("Có"), "{:?}", rows[0]);
        let body = &rows[1..];
        assert_eq!(body.len(), 6, "hai hoá đơn, mỗi hoá đơn ba dòng (131, 5111, 33311)");
        let debit: i64 = body.iter().map(|row| number(&row[6])).sum();
        let credit: i64 = body.iter().map(|row| number(&row[7])).sum();
        assert_eq!(debit, credit);
        assert_eq!(debit, totals);
        assert!(body.iter().any(|row| row[5] == "5111") && body.iter().any(|row| row[5] == "131"));
        assert!(
            body.iter().all(|row| row[3].starts_with("1C26TAA-")),
            "số chứng từ nguồn: {body:?}"
        );
        let pairs = read_csv(&export(&app, "vn_double_entry", "csv").await);
        assert_eq!(pairs[0][4], "TK Nợ");
        assert_eq!(pairs[0][5], "TK Có");
        let paired: i64 = pairs[1..].iter().map(|row| number(&row[6])).sum();
        assert_eq!(paired, totals);
        assert!(pairs[1..].iter().all(|row| row[4] == "131"));
        let documents = export(&app, "generic_documents", "csv").await;
        let text = String::from_utf8(documents.clone()).unwrap();
        assert!(
            text.contains("\"'=HYPERLINK(\"\"http://evil\"\")\""),
            "chặn CSV injection: {text}"
        );
        let xlsx = export(&app, "generic_journal", "xlsx").await;
        assert_eq!(&xlsx[..2], b"PK");
        // Hồ sơ sai: lỗi gom một lần.
        let bad =
            json!({"profile": "misa", "format": "pdf", "from": "2026-02-01", "to": "2026-01-01"});
        let (status, answer) =
            call(&app, "clerk", "POST", "/api/v1/invoices/exports", Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(answer["errors"].as_array().unwrap().len(), 3, "{answer}");
    })
    .await
    .unwrap();
}

async fn file(storage: &Arc<dyn Storage>, key: &str) -> Vec<u8> {
    let found = storage.get(key).await.unwrap().unwrap_or_else(|| panic!("missing {key}"));
    let chunks: Vec<bytes::Bytes> = found.content.try_collect().await.unwrap();
    chunks.concat()
}

async fn set_retention(tenant: &TenantContext, id: uuid::Uuid, until: Date) {
    let write = tenant.database().begin_write().await.unwrap();
    let backend = write.get_database_backend();
    let sql = "UPDATE invoices_documents SET retention_until = $1 WHERE id = $2";
    write
        .execute_raw(autoxide_kernel::database::statement(backend, sql, [until.into(), id.into()]))
        .await
        .unwrap();
    write.commit().await.unwrap();
}

/// Đếm dòng của khách `tenant` trong `table`.
async fn rows_of(tenant: &TenantContext, table: &str) -> i64 {
    let read = tenant.database().read();
    let backend = read.get_database_backend();
    let row = read
        .query_one_raw(autoxide_kernel::database::statement(
            backend,
            format!("SELECT COUNT(*) AS n FROM {table} WHERE tenant_id = $1"),
            [tenant.id().as_uuid().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get::<i64>("", "n").unwrap()
}

#[tokio::test]
async fn luu_tru_va_don_chung_tu_het_han() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let storage = app.api::<dyn Storage>().unwrap();
        let tenant = app.default_tenant();
        series(&app, "clerk", "vat_invoice").await;
        let consumer = || Buyer::consumer(Some("Nguyễn Văn A".to_owned())).email("a@example.com");
        let old =
            issued(&app, &tenant, sale(&tenant, Currency::VND, 100_000, consumer(), "R-old")).await;
        let kept =
            issued(&app, &tenant, sale(&tenant, Currency::VND, 200_000, consumer(), "R-kept"))
                .await;
        // Bản lưu trữ: băm trong manifest khớp từng file.
        let (status, archive) =
            call(&app, "clerk", "POST", "/api/v1/invoices/archives", None).await;
        assert_eq!(status, StatusCode::CREATED, "{archive}");
        let prefix = archive["data"]["storage_prefix"].as_str().unwrap().to_owned();
        assert!(prefix.starts_with("archives/") && !prefix.starts_with("tenants/"));
        let manifest_bytes = file(&storage, &format!("{prefix}manifest.json")).await;
        assert_eq!(
            hex(&Sha256::digest(&manifest_bytes)),
            archive["data"]["manifest_sha256"].as_str().unwrap()
        );
        let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
        for entry in manifest["files"].as_array().unwrap() {
            let bytes =
                file(&storage, &format!("{prefix}{}", entry["path"].as_str().unwrap())).await;
            assert_eq!(hex(&Sha256::digest(&bytes)), entry["sha256"].as_str().unwrap());
        }
        let documents = file(&storage, &format!("{prefix}documents.jsonl")).await;
        assert_eq!(
            documents.split(|byte| *byte == b'\n').filter(|line| !line.is_empty()).count(),
            2
        );
        // Khách acme: một chứng từ quá hạn, một còn hạn, một nháp; rồi xoá hẳn khách.
        let acme = app.tenant("acme").await.unwrap();
        series(&app, "acme", "vat_invoice").await;
        let acme_old =
            issued(&app, &acme, sale(&acme, Currency::VND, 100_000, consumer(), "A-old")).await;
        let acme_kept =
            issued(&app, &acme, sale(&acme, Currency::VND, 300_000, consumer(), "A-kept")).await;
        let invoices = app.api::<dyn Invoices>().unwrap();
        invoices
            .create_document(
                &acme,
                Actor::system(),
                sale(&acme, Currency::VND, 5_000, consumer(), "A-draft"),
            )
            .await
            .unwrap();
        let yesterday = Timestamp::now().date_utc().pred().unwrap();
        set_retention(&tenant, old.id, yesterday).await;
        set_retention(&acme, acme_old.id, yesterday).await;
        let admin = app.api::<TenantAdmin>().unwrap();
        let changes = TenantChanges {
            status: Some("suspended".to_owned()),
            reason: Some("closing".to_owned()),
            ..TenantChanges::default()
        };
        admin.update(Actor::system(), acme.id().as_uuid(), changes, None).await.unwrap();
        let erased = admin.erase(Actor::system(), "acme", "closed for good").await.unwrap();
        assert!(erased.retained_tables.iter().any(|table| table == "invoices_documents"));
        drain(&app).await;
        // Khách nền tảng: người mua cá nhân của chứng từ quá hạn được ẩn danh; chứng từ còn hạn giữ nguyên.
        let old = invoices.document(&tenant, old.id).await.unwrap().unwrap();
        assert_eq!(old.buyer.name, None);
        assert_eq!(old.buyer.email, None);
        assert_eq!(old.number, Some(1), "chứng từ vẫn còn, chỉ ẩn danh người mua");
        let kept = invoices.document(&tenant, kept.id).await.unwrap().unwrap();
        assert_eq!(kept.buyer.name.as_deref(), Some("Nguyễn Văn A"));
        // Khách đã xoá: có bản lưu trữ, chứng từ quá hạn và nháp bị xoá, chứng từ còn hạn giữ lại.
        let acme = app.registry().tenant(acme.id()).await.unwrap();
        assert_eq!(rows_of(&acme, "invoices_archives").await, 1);
        assert_eq!(rows_of(&acme, "invoices_documents").await, 1);
        assert!(invoices.document(&acme, acme_kept.id).await.unwrap().is_some());
        assert!(invoices.document(&acme, acme_old.id).await.unwrap().is_none());
        assert!(rows_of(&acme, "ledger_entries").await >= 2, "sổ cái giữ nguyên");
    })
    .await
    .unwrap();
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[tokio::test]
async fn lap_lai_ban_xuat_bi_loi() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let today = Timestamp::now().date_utc();
        let body = json!({"profile": "generic_journal", "format": "csv",
                          "from": today.first_of_month(), "to": today});
        let (status, created) =
            call(&app, "clerk", "POST", "/api/v1/invoices/exports", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["data"]["id"].as_str().unwrap().to_owned();
        let retry = format!("/api/v1/invoices/exports/{id}/retry");

        // Đang chờ: lập lại trả nguyên, không xếp thêm job.
        let (status, same) = call(&app, "clerk", "POST", &retry, None).await;
        assert_eq!(status, StatusCode::OK, "{same}");
        assert_eq!(same["data"]["status"], "pending");
        drain(&app).await;

        // Đã xong: không lập lại.
        let (status, problem) = call(&app, "clerk", "POST", &retry, None).await;
        assert_eq!(status, StatusCode::CONFLICT, "{problem}");
        assert_eq!(problem["code"], "invoices.export_not_retryable");

        // Giả như job đã hỏng.
        let tenant = app.default_tenant();
        let write = tenant.database().begin_write().await.unwrap();
        let backend = write.get_database_backend();
        let sql = "UPDATE invoices_exports SET status = 'failed', error = 'boom', file_key = NULL \
                   WHERE id = $1";
        let uuid: uuid::Uuid = id.parse().unwrap();
        write
            .execute_raw(autoxide_kernel::database::statement(backend, sql, [uuid.into()]))
            .await
            .unwrap();
        write.commit().await.unwrap();

        let (status, again) = call(&app, "clerk", "POST", &retry, None).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(again["data"]["status"], "pending");
        assert_eq!(again["data"]["error"], Value::Null);
        let (status, _) = call(&app, "clerk", "POST", &retry, None).await;
        assert_eq!(status, StatusCode::OK);
        drain(&app).await;
        let (_, done) =
            call(&app, "clerk", "GET", &format!("/api/v1/invoices/exports/{id}"), None).await;
        assert_eq!(done["data"]["status"], "ready", "{done}");

        let (status, _) = call(&app, "reader", "POST", &retry, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let missing = format!("/api/v1/invoices/exports/{}/retry", uuid::Uuid::now_v7());
        let (status, problem) = call(&app, "clerk", "POST", &missing, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(problem["code"], "invoices.export_not_found");
    })
    .await
    .unwrap();
}
