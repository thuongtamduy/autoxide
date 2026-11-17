//! Test tỉ giá và bút toán ngoại tệ (bước 36 mục 10, D-352) qua REST, MCP và `dyn Ledger`, trên cả
//! SQLite và Postgres: tỉ giá theo ngày hiệu lực (nhập tay thắng nguồn cùng ngày), không sửa tỉ giá đã
//! ghi, quyền kiểm trước khi đọc đầu vào, lấy từ nguồn giả, bút toán ngoại tệ cân ở cả tiền sổ và giữ
//! ảnh chụp tỉ giá, đảo bằng đúng tỉ giá gốc, sổ nhật ký chung có số hiệu tài khoản.

use std::sync::Arc;

use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::payment::ExchangeRateSource;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgTree, root_of};
use autoxide_testkit::{FakeExchangeRateSource, TestApp, TestDatabase, tool};
use axum::http::{Method, StatusCode};
use chrono::NaiveDate;
use serde_json::{Value, json};

use crate::api::{AccountKey, JournalQuery, Ledger, LineAccount, NewEntry, NewLine, NewReversal};

fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

async fn app(test: &TestDatabase, source: Arc<FakeExchangeRateSource>) -> TestApp {
    let app = TestApp::builder(test)
        .user(
            "acct",
            1,
            &["ledger.read", "ledger.post", "ledger.reverse", "ledger.settings_manage"],
        )
        .user("reader", 2, &["ledger.read"])
        .contribute::<dyn ExchangeRateSource>(source)
        .module(crate::module())
        .build()
        .await
        .unwrap();
    let write = app.database().begin_write().await.unwrap();
    let _ = OrgTree::insert_root(&write, DEFAULT_TENANT).await;
    write.commit().await.unwrap();
    app
}

async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/ledger{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

fn rate(base: &str, quote: &str, rate: &str, on: &str) -> Value {
    json!({"base": base, "quote": quote, "rate": rate, "effective_date": on})
}

#[tokio::test]
async fn ti_gia_theo_ngay_hieu_luc_nhap_tay_thang_nguon() {
    TestDatabase::run(async |test| {
        let source = Arc::new(FakeExchangeRateSource::new());
        let app = app(test, source.clone()).await;
        // Quyền kiểm trước đầu vào: người chỉ đọc gửi thân sai vẫn nhận 403.
        let (status, answer) =
            call(&app, "POST", "/exchange-rates", "reader", Some(rate("USD", "USD", "-1", "2026-01-01"))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{answer}");
        // Lỗi đầu vào gom một lần.
        let (status, answer) =
            call(&app, "POST", "/exchange-rates", "acct", Some(rate("USD", "USD", "0", "2026-01-01"))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
        assert_eq!(answer["errors"].as_array().unwrap().len(), 2, "{answer}");
        let (status, first) =
            call(&app, "POST", "/exchange-rates", "acct", Some(rate("USD", "VND", "25000", "2026-03-01"))).await;
        assert_eq!(status, StatusCode::CREATED, "{first}");
        assert_eq!(first["data"]["rate"], "25000");
        let (status, again) =
            call(&app, "POST", "/exchange-rates", "acct", Some(rate("usd", "vnd", "25000.00", "2026-03-01"))).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(again["data"]["id"], first["data"]["id"]);
        let (status, other) =
            call(&app, "POST", "/exchange-rates", "acct", Some(rate("USD", "VND", "26000", "2026-03-01"))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{other}");
        assert_eq!(other["code"], "ledger.exchange_rate_exists");
        // Nguồn giả báo tỉ giá ngày 10/3; cùng ngày có tỉ giá tay thì tay thắng.
        source.set(Currency::USD, Currency::VND, "25500.5");
        let (status, fetched) = call(
            &app,
            "POST",
            "/exchange-rates/fetch",
            "acct",
            Some(json!({"base": "USD", "on": "2026-03-10"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{fetched}");
        assert_eq!(fetched["data"][0]["source"], "provider:fake");
        let effective = async |on: &str| {
            call(&app, "GET", &format!("/exchange-rates/effective?base=USD&quote=VND&on={on}"), "reader", None).await
        };
        let (status, answer) = effective("2026-02-28").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{answer}");
        assert_eq!(answer["code"], "ledger.exchange_rate_not_found");
        assert_eq!(effective("2026-03-09").await.1["data"]["rate"], "25000");
        assert_eq!(effective("2026-03-10").await.1["data"]["rate"], "25500.5");
        call(&app, "POST", "/exchange-rates", "acct", Some(rate("USD", "VND", "25400", "2026-03-10"))).await;
        let (_, answer) = effective("2026-03-20").await;
        assert_eq!(answer["data"]["rate"], "25400");
        assert_eq!(answer["data"]["source"], "manual");
        // Nguồn không trả lời → 503, không ghi gì.
        source.fail(true);
        let (status, _) = call(
            &app,
            "POST",
            "/exchange-rates/fetch",
            "acct",
            Some(json!({"base": "USD", "on": "2026-03-11"})),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        let (_, listed) = call(&app, "GET", "/exchange-rates?base=USD", "reader", None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 3);
        // Tool MCP đi cùng đường.
        let client = app.mcp("reader").await.unwrap();
        let answer = client
            .call_tool(tool("ledger.exchange_rate", json!({"base": "USD", "quote": "VND", "on": "2026-03-05"})))
            .await
            .unwrap();
        let text = serde_json::to_string(&answer.structured_content).unwrap();
        assert!(text.contains("\"25000\""), "{text}");
        let answer = client
            .call_tool(tool(
                "ledger.record_exchange_rate",
                json!({"base": "EUR", "quote": "VND", "rate": "27000", "effective_date": "2026-03-01"}),
            ))
            .await;
        assert!(answer.is_err() || answer.unwrap().is_error == Some(true));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn but_toan_ngoai_te_can_o_tien_so_va_giu_anh_chup_ti_gia() {
    TestDatabase::run(async |test| {
        let app = app(test, Arc::new(FakeExchangeRateSource::new())).await;
        let tenant = app.default_tenant();
        let ledger = app.api::<dyn Ledger>().unwrap();
        let root = root_of(DEFAULT_TENANT);
        let usd = |minor| Money::new(minor, Currency::USD);
        let key = |purpose: &str| LineAccount::Key(AccountKey::new(root, purpose, Currency::USD));
        let entry = |reference: &str| {
            NewEntry::new(
                root,
                "demo",
                "sale",
                reference,
                "Foreign sale",
                vec![
                    NewLine::debit(key("receivable.customer"), usd(1001)),
                    NewLine::debit(key("cash.bank"), usd(1000)),
                    NewLine::credit(key("revenue.sales"), usd(2001)),
                ],
            )
            .on(day("2026-04-15"))
        };
        // Chưa có tỉ giá: từ chối, nói rõ cặp và ngày.
        let error = ledger.post(&tenant, Actor::system(), entry("F-0")).await.unwrap_err();
        assert_eq!(error.code(), "ledger.exchange_rate_missing");
        assert!(error.message().contains("USD/VND"), "{}", error.message());
        call(
            &app,
            "POST",
            "/exchange-rates",
            "acct",
            Some(rate("USD", "VND", "25345.5", "2026-04-01")),
        )
        .await;
        let posted =
            ledger.post(&tenant, Actor::system(), entry("F-1")).await.unwrap().into_inner();
        let base_total = |side: crate::api::Side| -> i64 {
            posted
                .lines
                .iter()
                .filter(|line| line.side == side)
                .map(|line| line.base_amount.minor())
                .sum()
        };
        // 20,01 USD × 25345,5 = 507163,4 → 507163 VND (HalfUp), chia về hai dòng nợ theo tỉ lệ.
        assert_eq!(base_total(crate::api::Side::Debit), 507_163);
        assert_eq!(base_total(crate::api::Side::Credit), 507_163);
        assert!(posted.lines.iter().all(|line| line.exchange_rate.as_deref() == Some("25345.5")));
        assert!(posted.lines.iter().all(|line| line.base_amount.currency() == Currency::VND));
        // Tỉ giá mới sau đó không đổi bút toán đã ghi; đảo bút toán dùng đúng tỉ giá gốc.
        call(
            &app,
            "POST",
            "/exchange-rates",
            "acct",
            Some(rate("USD", "VND", "26000", "2026-04-20")),
        )
        .await;
        let reversed = ledger
            .reverse(
                &tenant,
                Actor::system(),
                posted.id,
                NewReversal::new("demo", "F-1:rev").on(day("2026-04-25")),
            )
            .await
            .unwrap()
            .into_inner();
        assert!(reversed.lines.iter().all(|line| line.exchange_rate.as_deref() == Some("25345.5")));
        let reversed_total: i64 = reversed
            .lines
            .iter()
            .filter(|line| line.side == crate::api::Side::Credit)
            .map(|line| line.base_amount.minor())
            .sum();
        assert_eq!(reversed_total, 507_163);
        // Tỉ giá do bên gọi chụp sẵn (chứng từ) thắng tỉ giá trong bảng.
        let explicit =
            entry("F-2").exchange_rate(autoxide_kernel::decimal::parse("24000").unwrap());
        let posted = ledger.post(&tenant, Actor::system(), explicit).await.unwrap().into_inner();
        assert!(posted.lines.iter().all(|line| line.exchange_rate.as_deref() == Some("24000")));
        // Sổ nhật ký chung: dòng mang số hiệu tài khoản theo ánh xạ "vn", dòng nợ = dòng có ở tiền sổ.
        let journal = ledger
            .journal(&tenant, JournalQuery::new(day("2026-04-01"), day("2026-04-30")))
            .await
            .unwrap();
        assert_eq!(journal.data.len(), 9);
        assert!(
            journal
                .data
                .iter()
                .any(|line| line.purpose == "revenue.sales" && line.account_code == "5111")
        );
        let signed: i64 = journal
            .data
            .iter()
            .map(|line| match line.side {
                crate::api::Side::Debit => line.base_amount.minor(),
                crate::api::Side::Credit => -line.base_amount.minor(),
            })
            .sum();
        assert_eq!(signed, 0);
        let empty = ledger
            .journal(&tenant, JournalQuery::new(day("2026-05-01"), day("2026-05-31")))
            .await
            .unwrap();
        assert!(empty.data.is_empty());
        assert_eq!(ledger.book_currency(&tenant, root).await.unwrap(), Currency::VND);
    })
    .await
    .unwrap();
}
