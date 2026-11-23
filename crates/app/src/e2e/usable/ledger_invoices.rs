//! Sổ cái, thanh toán và chứng từ dùng được như kế toán thật (cả SQLite và Postgres): sổ của đơn vị,
//! bút toán tay và bút toán đảo, tài khoản ngân hàng và VietQR, khoản thu qua cổng giả rồi hoàn tiền,
//! đối soát, tỷ giá; dãy số chứng từ, mẫu in, xuất cho phần mềm kế toán.

use std::sync::Arc;

use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::payment::PaymentGateway;
use autoxide_testkit::{FakePaymentGateway, TestDatabase};
use serde_json::{Value, json};

use super::kit::{ADMIN, app_with, call_tool, drain, expect, get, id};

fn vnd(amount: i64) -> Value {
    json!({"amount": amount.to_string(), "currency": "VND"})
}

#[tokio::test]
async fn so_but_toan_dao_thu_qua_cong_hoan_tien_va_doi_soat() {
    TestDatabase::run(async |test| {
        let gateway = Arc::new(FakePaymentGateway::new());
        let app = app_with(test, |builder| builder.contribute::<dyn PaymentGateway>(gateway.clone())).await;

        let books = get(&app, "/api/v1/ledger/books").await;
        let book = match books["data"][0]["id"].as_str() {
            Some(book) => book.to_owned(),
            None => id(&expect(&app, "POST", "/api/v1/ledger/books", ADMIN, Some(json!({})), 201).await),
        };
        get(&app, &format!("/api/v1/ledger/books/{book}")).await;

        // Hai tài khoản của sổ: tiền mặt và vốn góp.
        let mut account = std::collections::HashMap::new();
        for purpose in ["cash.on_hand", "equity.opening"] {
            let body = json!({"currency": "VND", "purpose": purpose});
            let opened = expect(&app, "POST", "/api/v1/ledger/accounts", ADMIN, Some(body), 201).await;
            account.insert(purpose, json!(id(&opened)));
        }
        let account = |purpose: &str| account[purpose].clone();
        let entry = json!({"description": "Góp vốn", "reference": "manual-1", "lines": [
            {"account_id": account("cash.on_hand"), "amount": vnd(1_000_000), "side": "debit"},
            {"account_id": account("equity.opening"), "amount": vnd(1_000_000), "side": "credit"}]});
        let entry = id(&expect(&app, "POST", "/api/v1/ledger/entries", ADMIN, Some(entry), 201).await);
        get(&app, &format!("/api/v1/ledger/entries/{entry}")).await;
        let reverse = json!({"reference": "manual-1-reverse"});
        expect(&app, "POST", &format!("/api/v1/ledger/entries/{entry}/reverse"), ADMIN, Some(reverse), 201).await;

        let bank = json!({"account_name": "CONG TY AN", "account_number": "0123456789", "bank_bin": "970436"});
        expect(&app, "POST", "/api/v1/ledger/bank-accounts", ADMIN, Some(bank), 201).await;
        get(&app, "/api/v1/ledger/bank-accounts").await;
        let qr = json!({"amount": "250000", "description": "Thanh toan don 1", "reference": "qr-1"});
        call_tool(&app, ADMIN, "ledger.create_vietqr_payment", qr).await;

        // Thu qua cổng (giả), đối tác báo đã thu, rồi hoàn một phần.
        let payment = json!({"amount": vnd(300_000), "description": "Đơn 2", "method": "card",
                             "provider": "fake", "reference": "pay-2", "return_url": "https://shop.example/done"});
        let payment = id(&expect(&app, "POST", "/api/v1/ledger/payments", ADMIN, Some(payment), 201).await);
        let (headers, body) = gateway.paid(payment.parse().unwrap(), Money::new(300_000, Currency::VND), "evt-2");
        let mut request = app.post("/api/v1/ledger/webhooks/fake").bytes(body).header("content-type", "application/json");
        for (name, value) in &headers {
            request = request.header(name, value);
        }
        let answer = request.send().await.unwrap();
        assert!(answer.status().is_success(), "{}", answer.text());
        drain(&app).await;
        call_tool(&app, ADMIN, "ledger.payment", json!({"payment_id": payment})).await;
        let refund = json!({"amount": vnd(50_000), "reason": "Thiếu hàng", "reference": "refund-2"});
        expect(&app, "POST", &format!("/api/v1/ledger/payments/{payment}/refunds"), ADMIN, Some(refund), 201).await;

        let today = autoxide_kernel::time::Timestamp::now().date_utc().to_string();
        // Đối tác (giả) chưa có dòng nào trong bảng kê: khoản đã thu thành một chênh lệch chờ người xử lý.
        let run = json!({"date": today, "provider": "fake"});
        let run = expect(&app, "POST", "/api/v1/ledger/reconciliations", ADMIN, Some(run), 201).await;
        assert_eq!(run["data"]["exceptions"], 1, "{run}");
        get(&app, "/api/v1/ledger/reconciliations").await;
        let exceptions = call_tool(&app, ADMIN, "ledger.exceptions", json!({})).await;
        let exception = exceptions[0]["id"].as_str().unwrap_or_else(|| panic!("{exceptions}")).to_owned();
        let resolve = json!({"exception_id": exception, "action": "ignore", "note": "Đối tác gửi bảng kê muộn"});
        call_tool(&app, ADMIN, "ledger.resolve_exception", resolve).await;
        let rate = json!({"base": "USD", "quote": "VND", "rate": "25400", "effective_date": today});
        call_tool(&app, ADMIN, "ledger.record_exchange_rate", rate).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn day_so_mau_in_chung_tu_va_xuat_ke_toan() {
    TestDatabase::run(async |test| {
        let app = app_with(test, |builder| builder).await;
        let year = i32::from(autoxide_kernel::time::Timestamp::now().date_utc().year());
        let series = json!({"kind": "sales_invoice", "symbol": "2C26TAA", "year": year});
        let series = id(&expect(&app, "POST", "/api/v1/invoices/series", ADMIN, Some(series), 201).await);
        get(&app, "/api/v1/invoices/series").await;
        expect(&app, "PATCH", &format!("/api/v1/invoices/series/{series}"), ADMIN, Some(json!({"status": "inactive"})), 200).await;

        let defaults = get(&app, "/api/v1/invoices/template-defaults").await;
        let defaults = &defaults["data"];
        let template = json!({"kind": defaults["kinds"][0], "locale": "vi", "body": defaults["body"]});
        let template = id(&expect(&app, "POST", "/api/v1/invoices/templates", ADMIN, Some(template), 201).await);
        get(&app, "/api/v1/invoices/templates").await;
        get(&app, &format!("/api/v1/invoices/templates/{template}")).await;

        let documents = get(&app, "/api/v1/invoices/documents").await;
        if let Some(document) = documents["data"][0]["id"].as_str() {
            call_tool(&app, ADMIN, "invoices.document", json!({"document_id": document})).await;
        }
        get(&app, "/api/v1/invoices/archives").await;
        get(&app, "/api/v1/invoices/mandates").await;
        get(&app, "/api/v1/invoices/export-profiles").await;
        let export = json!({"format": "csv", "profile": "generic_journal", "from": "2026-01-01", "to": "2026-12-31"});
        let export = call_tool(&app, ADMIN, "invoices.create_export", export).await;
        let export = export["id"].as_str().unwrap_or_else(|| panic!("{export}")).to_owned();
        drain(&app).await;
        call_tool(&app, ADMIN, "invoices.export", json!({"export_id": export})).await;
        get(&app, "/api/v1/invoices/exports").await;
    })
    .await
    .unwrap();
}
