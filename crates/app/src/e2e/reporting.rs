//! Báo cáo gộp (bước 38) với module thật: `ledger` ghi sổ ở khách cha và hai khách con (một con ở
//! database riêng), `reporting` đổ số liệu `ledger.revenue` về kho của khách gốc và quy đổi theo tỉ
//! giá của khách người xem; `invoices` khai số liệu của chứng từ. Chạy trên cả SQLite và Postgres.

use autoxide_kernel::event::{Actor, Delivered};
use autoxide_kernel::module::Module;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{OrgId, OrgTree, TenantContext};
use autoxide_ledger::api::{AccountKey, Ledger, LineAccount, NewEntry, NewLine};
use autoxide_testkit::{TestApp, TestDatabase};
use serde_json::{Value, json};

async fn record_rate(app: &TestApp, token: &str, base: &str, rate: &str) {
    let body = json!({"base": base, "quote": "VND", "rate": rate, "effective_date": "2026-01-01"});
    let response =
        app.post("/api/v1/ledger/exchange-rates").token(token).json(body).send().await.unwrap();
    assert_eq!(response.status(), 201, "{}", response.text());
}

/// Ghi doanh thu `amount` ở khách `tenant` (nút gốc).
async fn sell(ledger: &dyn Ledger, tenant: &TenantContext, reference: &str, amount: Money) {
    let write = tenant.database().begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, tenant.id()).await.unwrap();
    write.commit().await.unwrap();
    let key = |purpose: &str| LineAccount::Key(AccountKey::new(root, purpose, amount.currency()));
    let entry = NewEntry::new(
        root,
        "demo",
        "sale",
        reference,
        "Sale",
        vec![
            NewLine::debit(key("cash.bank"), amount),
            NewLine::credit(key("revenue.sales"), amount),
        ],
    );
    ledger.post(tenant, Actor::system(), entry).await.unwrap();
}

#[tokio::test]
async fn multi_currency_converted_with_viewer_rates_and_lists_missing() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .tenant("parent")
            .child_tenant("shop-a", "parent", true)
            .child_tenant("shop-b", "parent", false)
            .user_in("parent", "boss", 1, &["reporting.read", "ledger.settings_manage"])
            .user_in("shop-a", "shop-a", 2, &["ledger.settings_manage"])
            .user_in("shop-b", "shop-b", 3, &["ledger.settings_manage"])
            .module(autoxide_ledger::module())
            .module(autoxide_reporting::module())
            .build()
            .await
            .unwrap();
        let tenant = async |slug: &str| app.tenant(slug).await.unwrap();
        let (parent, shop_a, shop_b) =
            (tenant("parent").await, tenant("shop-a").await, tenant("shop-b").await);
        // Tỉ giá của người xem (cha): chỉ có USD. Con có tỉ giá riêng để ghi sổ ngoại tệ.
        record_rate(&app, "boss", "USD", "25000").await;
        record_rate(&app, "shop-a", "USD", "1").await;
        record_rate(&app, "shop-b", "EUR", "27000").await;
        let ledger = app.api::<dyn Ledger>().unwrap();
        sell(ledger.as_ref(), &shop_a, "S-1", Money::new(1000, Currency::USD)).await;
        sell(ledger.as_ref(), &parent, "S-2", Money::new(50_000, Currency::VND)).await;
        sell(ledger.as_ref(), &shop_b, "S-3", Money::new(500, Currency::EUR)).await;
        let dispatcher = app.modules().dispatcher().unwrap();
        while dispatcher.run_once().await.unwrap() > 0 {}
        // Lịch `reporting.forward` chạy trên từng database.
        app.modules().worker(4).unwrap().run_once().await.unwrap();

        let response = app
            .get("/api/v1/reporting/summary?kind=ledger.revenue&currency=VND&group_by=tenant")
            .token("boss")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{}", response.text());
        let body: Value = response.json();
        let totals = &body["data"]["totals"];
        // Theo từng tiền gốc: không cộng hai loại tiền với nhau.
        let currencies: Vec<&str> = totals["by_currency"]
            .as_array()
            .unwrap()
            .iter()
            .map(|total| total["currency"].as_str().unwrap())
            .collect();
        assert_eq!(currencies, ["EUR", "USD", "VND"]);
        // 10 USD × 25 000 (tỉ giá của cha, không phải của con) + 50 000 VND; EUR không có tỉ giá ở
        // cha: liệt kê, không cộng, không coi là 0.
        assert_eq!(totals["converted"], json!({"amount": "300000", "currency": "VND"}));
        assert_eq!(body["data"]["missing_rates"].as_array().unwrap().len(), 1);
        assert_eq!(body["data"]["missing_rates"][0]["currency"], "EUR");
        assert_eq!(body["data"]["groups"].as_array().unwrap().len(), 3);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn invoices_and_ledger_declare_their_facts() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).build().await.unwrap();
        let tenant = app.default_tenant();
        let org = uuid::Uuid::now_v7();
        let vnd = |minor| Money::new(minor, Currency::VND);
        let issued = Delivered::new(
            tenant.clone(),
            "invoices",
            "invoices.document_issued",
            json!({"org_id": org, "kind": "vat_invoice", "total": vnd(108_000)}),
        );
        let facts = autoxide_invoices::module().facts().remove(0).extract(&issued).unwrap();
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].amount, Some(vnd(108_000)));
        assert_eq!(facts[0].org_id, Some(OrgId::from_uuid(org)));
        assert_eq!(facts[0].dimensions["kind"], "vat_invoice");
        assert_eq!(facts[1].quantity, Some(1));

        let line = |purpose: &str, side: &str, amount: i64| {
            json!({"purpose": purpose, "side": side, "amount": vnd(amount)})
        };
        let entry = |name: &'static str, lines: Vec<Value>| {
            Delivered::new(tenant.clone(), "ledger", name, json!({"org_id": org, "lines": lines}))
        };
        let posted = entry(
            "ledger.entry_posted",
            vec![
                line("cash.bank", "debit", 108_000),
                line("revenue.sales", "credit", 100_000),
                line("tax.vat_output", "credit", 8_000),
            ],
        );
        let reversed = entry(
            "ledger.entry_reversed",
            vec![
                line("revenue.sales", "debit", 100_000),
                line("tax.vat_output", "debit", 8_000),
                line("cash.bank", "credit", 108_000),
            ],
        );
        let sources = autoxide_ledger::module().facts();
        let facts = sources[0].extract(&posted).unwrap();
        let amounts: Vec<(&str, i64)> =
            facts.iter().map(|fact| (fact.kind, fact.amount.unwrap().minor())).collect();
        assert_eq!(amounts, [("ledger.revenue", 100_000), ("ledger.tax_output", 8_000)]);
        assert_eq!(facts[0].dimensions["purpose"], "revenue.sales");
        // Đảo bút toán: số âm.
        let facts = sources[1].extract(&reversed).unwrap();
        assert_eq!(facts[0].amount.unwrap().minor(), -100_000);
    })
    .await
    .unwrap();
}
