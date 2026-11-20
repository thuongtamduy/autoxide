//! Test của module `ledger` qua REST, MCP và `dyn Ledger` (cả SQLite và Postgres): bút toán cân và
//! cùng loại tiền, `reference` gửi lại, ghi đồng thời giữ đúng số dư và sàn, chuyển ngược chiều
//! không deadlock, đảo một lần, khoá kỳ đua với ghi, số dư tại ngày theo ảnh chụp kỳ, doctor phát
//! hiện lệch số dư và sửa / xoá bút toán đã niêm phong, chuỗi không băm `tenant_id`, không rò rỉ
//! sang nhánh anh em.

use std::sync::Arc;

use autoxide_kernel::access::Scope;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, TenantContext, root_of};
use autoxide_testkit::{TestApp, TestDatabase, tool};
use axum::http::{Method, StatusCode};
use chrono::NaiveDate;
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{AccountKey, Ledger, LineAccount, NewEntry, NewLine, NewReversal, OpenAccount};
use crate::chain;
use crate::entity::{balance, entry, head, line, period_balance, seal};
use crate::service::LedgerService;

const ALL: [&str; 5] = [
    "ledger.read",
    "ledger.post",
    "ledger.reverse",
    "ledger.accounts_manage",
    "ledger.periods_close",
];

struct Tree {
    x: OrgId,
    y: OrgId,
}

async fn app(test: &TestDatabase) -> (TestApp, Tree) {
    let (x, y) = (OrgId::from_uuid(Uuid::now_v7()), OrgId::from_uuid(Uuid::now_v7()));
    let app = TestApp::builder(test)
        .user("acct", 1, &ALL)
        .member("x-reader", 2, x, &[("ledger.read", Scope::Subtree)])
        .module(crate::module())
        .build()
        .await
        .unwrap();
    let write = app.database().begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, x, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, y, root, 8).await.unwrap();
    write.commit().await.unwrap();
    (app, Tree { x, y })
}

fn ledger(app: &TestApp) -> Arc<dyn Ledger> {
    app.api::<dyn Ledger>().unwrap()
}

fn root() -> OrgId {
    root_of(DEFAULT_TENANT)
}

fn vnd(minor: i64) -> Money {
    Money::new(minor, Currency::VND)
}

fn key(purpose: &str) -> AccountKey {
    AccountKey::new(root(), purpose, Currency::VND)
}

fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

/// Bút toán hai dòng: nợ `debit`, có `credit`.
fn transfer(debit: &str, credit: &str, amount: i64, reference: &str) -> NewEntry {
    NewEntry::new(
        root(),
        "demo",
        "transfer",
        reference,
        "Test transfer",
        vec![
            NewLine::debit(LineAccount::Key(key(debit)), vnd(amount)),
            NewLine::credit(LineAccount::Key(key(credit)), vnd(amount)),
        ],
    )
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

/// Số tiền (đơn vị nhỏ nhất, VND) của một `Money` JSON.
fn amount(value: &Value) -> i64 {
    value["amount"].as_str().unwrap().parse().unwrap()
}

async fn balance_of(app: &TestApp, tenant: &TenantContext, purpose: &str) -> i64 {
    ledger(app).balance(tenant, key(purpose), None).await.unwrap().minor()
}

async fn account_id(app: &TestApp, tenant: &TenantContext, purpose: &str) -> Uuid {
    ledger(app).ensure_account(tenant, key(purpose)).await.unwrap()
}

#[tokio::test]
async fn but_toan_phai_can_va_cung_loai_tien() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let tenant = app.default_tenant();
        let (cash, revenue) = (
            account_id(&app, &tenant, "cash.bank").await,
            account_id(&app, &tenant, "revenue.sales").await,
        );
        let body = |credit: i64, currency: &str| {
            json!({"description": "Sale", "reference": "S-1", "lines": [
                {"account_id": cash, "side": "debit", "amount": {"amount": "1000", "currency": "VND"}},
                {"account_id": revenue, "side": "credit", "amount": {"amount": credit.to_string(), "currency": currency}},
            ]})
        };
        let (status, answer) = call(&app, "POST", "/entries", "acct", Some(body(900, "VND"))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
        assert_eq!(answer["code"], "ledger.unbalanced");
        let (status, answer) = call(&app, "POST", "/entries", "acct", Some(body(1000, "USD"))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
        assert_eq!(answer["code"], "ledger.currency_mixed");
        let one_line = json!({"description": "Sale", "reference": "S-2", "lines": [
            {"account_id": cash, "side": "debit", "amount": {"amount": "1000", "currency": "VND"}}]});
        let (status, answer) = call(&app, "POST", "/entries", "acct", Some(one_line)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
        assert_eq!(answer["errors"][0]["field"], "lines");
        // Không có gì được ghi.
        assert_eq!(balance_of(&app, &tenant, "cash.bank").await, 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn reference_gui_lai_tra_ban_cu_va_dung_lai_bi_chan() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let tenant = app.default_tenant();
        let (cash, revenue) = (
            account_id(&app, &tenant, "cash.bank").await,
            account_id(&app, &tenant, "revenue.sales").await,
        );
        let body = |amount: &str| {
            json!({"description": "Sale", "reference": "S-1", "effective_date": "2026-03-02", "lines": [
                {"account_id": cash, "side": "debit", "amount": {"amount": amount, "currency": "VND"}},
                {"account_id": revenue, "side": "credit", "amount": {"amount": amount, "currency": "VND"}},
            ]})
        };
        let (status, first) = call(&app, "POST", "/entries", "acct", Some(body("1000"))).await;
        assert_eq!(status, StatusCode::CREATED, "{first}");
        assert_eq!(first["data"]["lines"].as_array().unwrap().len(), 2);
        let (status, again) = call(&app, "POST", "/entries", "acct", Some(body("1000"))).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(again["data"]["id"], first["data"]["id"]);
        let (status, reused) = call(&app, "POST", "/entries", "acct", Some(body("2000"))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{reused}");
        assert_eq!(reused["code"], "reference_reused");
        assert_eq!(balance_of(&app, &tenant, "cash.bank").await, 1000);
        assert_eq!(balance_of(&app, &tenant, "revenue.sales").await, 1000);
        // Lọc theo chứng từ nguồn.
        let (_, listed) = call(&app, "GET", "/entries?source_module=ledger", "acct", None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ghi_dong_thoi_mot_tai_khoan_so_du_chinh_xac() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let tenant = app.default_tenant();
        let tasks: Vec<_> = (0..40)
            .map(|index| {
                let (ledger, tenant) = (ledger(&app), tenant.clone());
                tokio::spawn(async move {
                    let entry =
                        transfer("cash.bank", "revenue.sales", 1000 + index, &format!("T-{index}"));
                    ledger.post(&tenant, Actor::system(), entry).await
                })
            })
            .collect();
        for task in tasks {
            assert!(matches!(task.await.unwrap().unwrap(), Submitted::Created(_)));
        }
        let expected: i64 = (0..40).map(|index| 1000 + index).sum();
        assert_eq!(balance_of(&app, &tenant, "cash.bank").await, expected);
        assert_eq!(balance_of(&app, &tenant, "revenue.sales").await, expected);
        assert!(LedgerService::drifted(app.database()).await.unwrap().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn san_khong_bao_gio_bi_thung_khi_ghi_dong_thoi() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let tenant = app.default_tenant();
        let ledger = ledger(&app);
        ledger.open_account(&tenant, OpenAccount::new(key("cash.on_hand")).floor(0)).await.unwrap();
        ledger
            .post(&tenant, Actor::system(), transfer("cash.on_hand", "equity.opening", 100, "OPEN"))
            .await
            .unwrap();
        let tasks: Vec<_> = (0..40)
            .map(|index| {
                let (ledger, tenant) = (ledger.clone(), tenant.clone());
                tokio::spawn(async move {
                    let entry = transfer(
                        "expense.gateway_fee",
                        "cash.on_hand",
                        10,
                        &format!("FEE-{index}"),
                    );
                    ledger.post(&tenant, Actor::system(), entry).await
                })
            })
            .collect();
        let mut ok = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(_) => ok += 1,
                Err(error) => assert_eq!(error.code(), "ledger.insufficient_balance"),
            }
        }
        assert_eq!(ok, 10);
        assert_eq!(balance_of(&app, &tenant, "cash.on_hand").await, 0);
        assert_eq!(balance_of(&app, &tenant, "expense.gateway_fee").await, 100);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chuyen_nguoc_chieu_khong_deadlock() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let tenant = app.default_tenant();
        let ledger = ledger(&app);
        let tasks: Vec<_> = (0..40)
            .map(|index| {
                let (ledger, tenant) = (ledger.clone(), tenant.clone());
                tokio::spawn(async move {
                    let (from, to) = if index % 2 == 0 {
                        ("cash.bank", "cash.on_hand")
                    } else {
                        ("cash.on_hand", "cash.bank")
                    };
                    ledger
                        .post(
                            &tenant,
                            Actor::system(),
                            transfer(to, from, 7, &format!("X-{index}")),
                        )
                        .await
                })
            })
            .collect();
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        assert_eq!(balance_of(&app, &tenant, "cash.bank").await, 0);
        assert_eq!(balance_of(&app, &tenant, "cash.on_hand").await, 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dao_dong_thoi_chi_mot_lan() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let tenant = app.default_tenant();
        let ledger = ledger(&app);
        let original = ledger
            .post(&tenant, Actor::system(), transfer("cash.bank", "revenue.sales", 500, "SALE"))
            .await
            .unwrap()
            .into_inner();
        let tasks: Vec<_> = (0..8)
            .map(|index| {
                let (ledger, tenant) = (ledger.clone(), tenant.clone());
                tokio::spawn(async move {
                    let reversal = NewReversal::new("demo", format!("REV-{index}"));
                    ledger.reverse(&tenant, Actor::system(), original.id, reversal).await
                })
            })
            .collect();
        let mut ok = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(_) => ok += 1,
                Err(error) => assert_eq!(error.code(), "ledger.already_reversed"),
            }
        }
        assert_eq!(ok, 1);
        assert_eq!(balance_of(&app, &tenant, "cash.bank").await, 0);
        let found = ledger.entry(&tenant, original.id).await.unwrap().unwrap();
        let reversal = found.reversed_by_entry_id.unwrap();
        // Đảo bút toán đảo thì được (ghi lại).
        let back = ledger
            .reverse(&tenant, Actor::system(), reversal, NewReversal::new("demo", "REV-BACK"))
            .await
            .unwrap();
        assert!(matches!(back, Submitted::Created(_)));
        assert_eq!(balance_of(&app, &tenant, "cash.bank").await, 500);
        // Qua REST: đảo lần nữa → 409.
        let (status, answer) = call(
            &app,
            "POST",
            &format!("/entries/{}/reverse", original.id),
            "acct",
            Some(json!({"reference": "REV-REST"})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{answer}");
        assert_eq!(answer["code"], "ledger.already_reversed");
    })
    .await
    .unwrap();
}

/// Kỳ (tháng) của bút toán `id`.
async fn period_of(app: &TestApp, tenant: &TenantContext, id: Uuid) -> Uuid {
    ledger(app).entry(tenant, id).await.unwrap().unwrap().period_id
}

#[tokio::test]
async fn khoa_ky_dua_voi_ghi_khong_but_toan_nao_lot_vao_ky_da_khoa() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let tenant = app.default_tenant();
        let ledger = ledger(&app);
        let first = ledger
            .post(
                &tenant,
                Actor::system(),
                transfer("cash.bank", "revenue.sales", 100, "JAN-0").on(day("2026-01-10")),
            )
            .await
            .unwrap()
            .into_inner();
        let period = period_of(&app, &tenant, first.id).await;
        let tasks: Vec<_> = (1..=20)
            .map(|index| {
                let (ledger, tenant) = (ledger.clone(), tenant.clone());
                tokio::spawn(async move {
                    let entry =
                        transfer("cash.bank", "revenue.sales", 100, &format!("JAN-{index}"))
                            .on(day("2026-01-20"));
                    ledger.post(&tenant, Actor::system(), entry).await
                })
            })
            .collect();
        let (status, closed) =
            call(&app, "POST", &format!("/periods/{period}/close"), "acct", None).await;
        assert_eq!(status, StatusCode::OK, "{closed}");
        assert_eq!(closed["data"]["status"], "closed");
        for task in tasks {
            if let Err(error) = task.await.unwrap() {
                assert_eq!(error.code(), "ledger.period_closed");
            }
        }
        // Mọi bút toán đều mang ngày trong tháng 1: số dư hiện tại phải bằng ảnh chụp lúc khoá.
        let cash = account_id(&app, &tenant, "cash.bank").await;
        let (_, at_close) =
            call(&app, "GET", &format!("/accounts/{cash}/balance?as_of=2026-01-31"), "acct", None)
                .await;
        assert_eq!(
            amount(&at_close["data"]["balance"]),
            balance_of(&app, &tenant, "cash.bank").await
        );
        // Sau khi khoá: ghi vào tháng 1 bị chặn, kể cả qua REST.
        let late = ledger
            .post(
                &tenant,
                Actor::system(),
                transfer("cash.bank", "revenue.sales", 1, "JAN-LATE").on(day("2026-01-31")),
            )
            .await
            .unwrap_err();
        assert_eq!(late.code(), "ledger.period_closed");
        // Khoá lại: trả kỳ đã khoá.
        let (status, _) =
            call(&app, "POST", &format!("/periods/{period}/close"), "acct", None).await;
        assert_eq!(status, StatusCode::OK);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khoa_ky_can_ky_truoc_da_khoa_va_ky_da_het() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let tenant = app.default_tenant();
        let ledger = ledger(&app);
        let post = |reference: &'static str, on: &'static str| {
            let (ledger, tenant) = (ledger.clone(), tenant.clone());
            async move {
                ledger
                    .post(
                        &tenant,
                        Actor::system(),
                        transfer("cash.bank", "revenue.sales", 10, reference).on(day(on)),
                    )
                    .await
                    .unwrap()
                    .into_inner()
            }
        };
        let jan = post("A", "2026-01-05").await;
        let feb = post("B", "2026-02-05").await;
        let (jan, feb) =
            (period_of(&app, &tenant, jan.id).await, period_of(&app, &tenant, feb.id).await);
        let (status, answer) =
            call(&app, "POST", &format!("/periods/{feb}/close"), "acct", None).await;
        assert_eq!(status, StatusCode::CONFLICT, "{answer}");
        assert_eq!(answer["code"], "ledger.period_order");
        let (status, _) = call(&app, "POST", &format!("/periods/{jan}/close"), "acct", None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = call(&app, "POST", &format!("/periods/{feb}/close"), "acct", None).await;
        assert_eq!(status, StatusCode::OK);
        // Kỳ chưa hết thì chưa khoá được.
        let today = chrono::Utc::now().date_naive();
        let now = ledger
            .post(
                &tenant,
                Actor::system(),
                transfer("cash.bank", "revenue.sales", 10, "C").on(today),
            )
            .await
            .unwrap()
            .into_inner();
        let current = period_of(&app, &tenant, now.id).await;
        let (status, answer) =
            call(&app, "POST", &format!("/periods/{current}/close"), "acct", None).await;
        assert_eq!(status, StatusCode::CONFLICT, "{answer}");
        assert_eq!(answer["code"], "ledger.period_not_ended");
        // Tháng hở (không có kỳ) trước kỳ đã khoá cũng bị chặn.
        let gap = ledger
            .post(
                &tenant,
                Actor::system(),
                transfer("cash.bank", "revenue.sales", 10, "GAP").on(day("2025-12-31")),
            )
            .await
            .unwrap_err();
        assert_eq!(gap.code(), "ledger.period_closed");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn so_du_tai_ngay_dung_anh_chup_ky_va_bang_can_doi_thu() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let tenant = app.default_tenant();
        let ledger = ledger(&app);
        for (reference, on, value) in
            [("A", "2026-01-05", 100), ("B", "2026-01-25", 50), ("C", "2026-02-03", 30)]
        {
            ledger
                .post(
                    &tenant,
                    Actor::system(),
                    transfer("cash.bank", "revenue.sales", value, reference).on(day(on)),
                )
                .await
                .unwrap();
        }
        let as_of = |on: &str| ledger.balance(&tenant, key("cash.bank"), Some(day(on)));
        assert_eq!(as_of("2026-01-10").await.unwrap().minor(), 100);
        assert_eq!(as_of("2026-01-31").await.unwrap().minor(), 150);
        assert_eq!(as_of("2026-02-28").await.unwrap().minor(), 180);
        let first = ledger.entry(&tenant, {
            let (_, listed) = call(&app, "GET", "/entries?order=asc", "acct", None).await;
            listed["data"][0]["id"].as_str().unwrap().parse().unwrap()
        });
        let period = first.await.unwrap().unwrap().period_id;
        let (status, _) =
            call(&app, "POST", &format!("/periods/{period}/close"), "acct", None).await;
        assert_eq!(status, StatusCode::OK);
        // Số dư sau kỳ khoá đọc từ ảnh chụp: sửa ảnh chụp thì số dư tại ngày đổi theo.
        let cash = account_id(&app, &tenant, "cash.bank").await;
        period_balance::Entity::update_many()
            .col_expr(
                period_balance::Column::ClosingMinor,
                Expr::col(period_balance::Column::ClosingMinor).add(1),
            )
            .filter(period_balance::Column::AccountId.eq(cash))
            .exec(app.database().write())
            .await
            .unwrap();
        assert_eq!(as_of("2026-02-28").await.unwrap().minor(), 181);
        assert_eq!(as_of("2026-01-10").await.unwrap().minor(), 100);
        // Bảng cân đối thử tháng 2: đầu kỳ từ ảnh chụp, phát sinh nợ = có.
        let (_, books) = call(&app, "GET", "/books", "acct", None).await;
        let book = books["data"][0]["id"].as_str().unwrap().to_owned();
        let (status, trial) = call(
            &app,
            "GET",
            &format!("/trial-balance?book_id={book}&from=2026-02-01&to=2026-02-28"),
            "acct",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{trial}");
        let totals = &trial["data"]["totals"][0];
        assert_eq!(amount(&totals["debit"]), amount(&totals["credit"]));
        let row = trial["data"]["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["purpose"] == "cash.bank")
            .unwrap()
            .clone();
        assert_eq!(
            (amount(&row["opening"]), amount(&row["debit"]), amount(&row["closing"])),
            (151, 30, 181)
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doctor_phat_hien_so_du_lech_voi_dong() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let tenant = app.default_tenant();
        ledger(&app)
            .post(&tenant, Actor::system(), transfer("cash.bank", "revenue.sales", 100, "A"))
            .await
            .unwrap();
        let probe = chain::BalancesProbe { databases: app.registry().clone() };
        use autoxide_kernel::doctor::Probe as _;
        let finding = format!("{:?}", probe.probe().await);
        assert!(finding.contains("Ok"), "{finding}");
        balance::Entity::update_many()
            .col_expr(
                balance::Column::BalanceMinor,
                Expr::col(balance::Column::BalanceMinor).add(5),
            )
            .exec(app.database().write())
            .await
            .unwrap();
        let drifted = LedgerService::drifted(app.database()).await.unwrap();
        assert_eq!(drifted.len(), 2);
        let finding = format!("{:?}", probe.probe().await);
        assert!(finding.contains("differ from their lines"), "{finding}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chuoi_bam_phat_hien_sua_va_xoa() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let tenant = app.default_tenant();
        let ledger = ledger(&app);
        let mut ids = Vec::new();
        for index in 0..3 {
            let posted = ledger
                .post(
                    &tenant,
                    Actor::system(),
                    transfer("cash.bank", "revenue.sales", 10, &format!("E-{index}")),
                )
                .await
                .unwrap()
                .into_inner();
            ids.push(posted.id);
        }
        assert_eq!(LedgerService::seal_entries(app.database()).await.unwrap(), 3);
        assert_eq!(LedgerService::seal_entries(app.database()).await.unwrap(), 0);
        let (checked, broken, stale) = chain::check_database(app.database()).await.unwrap();
        assert_eq!((checked, broken, stale), (3, None, 0));
        // Sửa một dòng của bút toán thứ hai.
        let bump = |delta: i64| {
            line::Entity::update_many()
                .col_expr(
                    line::Column::AmountMinor,
                    Expr::col(line::Column::AmountMinor).add(delta),
                )
                .filter(line::Column::EntryId.eq(ids[1]))
                .exec(app.database().write())
        };
        bump(1).await.unwrap();
        let (_, broken, _) = chain::check_database(app.database()).await.unwrap();
        let broken = broken.unwrap();
        assert!(broken.contains("#2") && broken.contains("changed"), "{broken}");
        bump(-1).await.unwrap();
        assert_eq!(chain::check_database(app.database()).await.unwrap().1, None);
        // Xoá bút toán cuối.
        line::Entity::delete_many()
            .filter(line::Column::EntryId.eq(ids[2]))
            .exec(app.database().write())
            .await
            .unwrap();
        entry::Entity::delete_by_id(ids[2]).exec(app.database().write()).await.unwrap();
        let (_, broken, _) = chain::check_database(app.database()).await.unwrap();
        assert!(broken.unwrap().contains("deleted"));
        // Bước kiểm của doctor báo hỏng.
        use autoxide_kernel::doctor::Probe as _;
        let probe = chain::ChainProbe { databases: app.registry().clone() };
        let finding = format!("{:?}", probe.probe().await);
        assert!(finding.contains("deleted"), "{finding}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chuoi_bam_khong_phu_thuoc_tenant_id() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let tenant = app.default_tenant();
        let ledger = ledger(&app);
        let posted = ledger
            .post(&tenant, Actor::system(), transfer("cash.bank", "revenue.sales", 10, "E"))
            .await
            .unwrap()
            .into_inner();
        LedgerService::seal_entries(app.database()).await.unwrap();
        // Chuyển sổ sang khách khác (như tách khách ở bước 38): chuỗi vẫn nguyên.
        let moved = Uuid::now_v7();
        let write = app.database().write();
        entry::Entity::update_many()
            .col_expr(entry::Column::TenantId, Expr::value(moved))
            .exec(write)
            .await
            .unwrap();
        line::Entity::update_many()
            .col_expr(line::Column::TenantId, Expr::value(moved))
            .exec(write)
            .await
            .unwrap();
        seal::Entity::update_many()
            .col_expr(seal::Column::TenantId, Expr::value(moved))
            .exec(write)
            .await
            .unwrap();
        head::Entity::update_many()
            .col_expr(head::Column::TenantId, Expr::value(moved))
            .exec(write)
            .await
            .unwrap();
        let verdict = chain::verify(app.database().read(), moved, posted.book_id).await.unwrap();
        assert_eq!(verdict, chain::Verdict::Intact(1));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_thay_so_cua_nhanh_anh_em() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let tenant = app.default_tenant();
        let ledger = ledger(&app);
        let in_y = |purpose: &str| AccountKey::new(tree.y, purpose, Currency::VND);
        let posted = ledger
            .post(
                &tenant,
                Actor::system(),
                NewEntry::new(
                    tree.y,
                    "demo",
                    "sale",
                    "Y-1",
                    "Sale",
                    vec![
                        NewLine::debit(LineAccount::Key(in_y("cash.bank")), vnd(10)),
                        NewLine::credit(LineAccount::Key(in_y("revenue.sales")), vnd(10)),
                    ],
                ),
            )
            .await
            .unwrap()
            .into_inner();
        // Người đọc nhánh X không thấy gì của Y.
        let (_, listed) = call(&app, "GET", "/entries", "x-reader", None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 0, "{listed}");
        let (status, _) =
            call(&app, "GET", &format!("/entries/{}", posted.id), "x-reader", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) =
            call(&app, "GET", &format!("/books/{}", posted.book_id), "x-reader", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let account = posted.lines[0].account_id;
        let (status, _) =
            call(&app, "GET", &format!("/accounts/{account}"), "x-reader", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (_, books) = call(&app, "GET", "/books", "x-reader", None).await;
        assert_eq!(books["data"].as_array().unwrap().len(), 0);
        // Ghi cần `ledger.post`.
        let (status, answer) = call(
            &app,
            "POST",
            &format!("/entries/{}/reverse", posted.id),
            "x-reader",
            Some(json!({"reference": "R"})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{answer}");
        // Tool MCP đi cùng đường.
        let client = app.mcp("x-reader").await.unwrap();
        let answer = client.call_tool(tool("ledger.entries", json!({}))).await.unwrap();
        let text = serde_json::to_string(&answer.structured_content).unwrap();
        assert!(!text.contains(&posted.id.to_string()), "{text}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn so_tai_khoan_va_tool_mcp() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let (status, created) =
            call(&app, "POST", "/books", "acct", Some(json!({"org_id": tree.x.as_uuid()}))).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        assert_eq!(created["data"]["base_currency"], "VND");
        let (status, again) =
            call(&app, "POST", "/books", "acct", Some(json!({"org_id": tree.x.as_uuid()}))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{again}");
        assert_eq!(again["code"], "ledger.book_exists");
        // Mục đích riêng của module cần bên thường.
        let custom = json!({"purpose": "demo.points", "currency": "VND"});
        let (status, answer) = call(&app, "POST", "/accounts", "acct", Some(custom)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
        let open = |purpose: &str| {
            json!({"purpose": purpose, "currency": "VND", "normal_side": "credit",
                   "counterparty_kind": "customer", "counterparty_id": "c-1"})
        };
        let (status, points) = call(&app, "POST", "/accounts", "acct", Some(open("demo.points"))).await;
        assert_eq!(status, StatusCode::CREATED, "{points}");
        let (status, same) = call(&app, "POST", "/accounts", "acct", Some(open("demo.points"))).await;
        assert_eq!(status, StatusCode::OK, "{same}");
        assert_eq!(same["data"]["id"], points["data"]["id"]);
        let points = points["data"]["id"].as_str().unwrap().to_owned();
        let (status, cash) = call(
            &app,
            "POST",
            "/accounts",
            "acct",
            Some(json!({"purpose": "cash.bank", "currency": "VND", "floor": {"amount": "0", "currency": "VND"}})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{cash}");
        let cash = cash["data"]["id"].as_str().unwrap().to_owned();
        // Ghi qua MCP.
        let client = app.mcp("acct").await.unwrap();
        let answer = client
            .call_tool(tool(
                "ledger.post_entry",
                json!({"description": "Points granted", "reference": "P-1", "currency": "VND", "lines": [
                    {"account_id": cash, "side": "debit", "amount": "2500"},
                    {"account_id": points, "side": "credit", "amount": "2500"}]}),
            ))
            .await
            .unwrap();
        assert_ne!(answer.is_error, Some(true), "{answer:?}");
        let answer = client
            .call_tool(tool("ledger.balance", json!({"account_id": points})))
            .await
            .unwrap();
        let text = serde_json::to_string(&answer.structured_content).unwrap();
        assert!(text.contains("\"2500\""), "{text}");
        let today = chrono::Utc::now().date_naive().format("%Y-%m-%d").to_string();
        let answer = client
            .call_tool(tool("ledger.trial_balance", json!({"from": today, "to": today})))
            .await
            .unwrap();
        assert_ne!(answer.is_error, Some(true), "{answer:?}");
        // Sàn không được cao hơn số dư; đóng tài khoản thì không ghi được nữa.
        let (status, answer) = call(
            &app,
            "PATCH",
            &format!("/accounts/{cash}"),
            "acct",
            Some(json!({"floor": {"amount": "3000", "currency": "VND"}})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
        let (status, _) =
            call(&app, "PATCH", &format!("/accounts/{cash}"), "acct", Some(json!({"status": "closed"}))).await;
        assert_eq!(status, StatusCode::OK);
        let answer = client
            .call_tool(tool(
                "ledger.post_entry",
                json!({"description": "More", "reference": "P-2", "currency": "VND", "lines": [
                    {"account_id": cash, "side": "debit", "amount": "1"},
                    {"account_id": points, "side": "credit", "amount": "1"}]}),
            ))
            .await
            .unwrap();
        assert_eq!(answer.is_error, Some(true));
        let (_, lines) = call(&app, "GET", &format!("/accounts/{points}/lines"), "acct", None).await;
        assert_eq!(lines["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn danh_sach_ky_kem_so_but_toan_va_so_da_niem_phong() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let tenant = app.default_tenant();
        let ledger = ledger(&app);
        let mut book = None;
        for (reference, on) in [("J-1", "2026-01-05"), ("J-2", "2026-01-20"), ("F-1", "2026-02-03")]
        {
            let posted = ledger
                .post(
                    &tenant,
                    Actor::system(),
                    transfer("cash.bank", "revenue.sales", 10, reference).on(day(on)),
                )
                .await
                .unwrap()
                .into_inner();
            book = Some(posted.book_id);
        }
        let uri = format!("/periods?book_id={}&order=asc", book.unwrap());
        let counts = |listed: &Value| -> Vec<(String, i64, i64)> {
            listed["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| {
                    (
                        row["starts_on"].as_str().unwrap().to_owned(),
                        row["entries"].as_i64().unwrap(),
                        row["sealed_entries"].as_i64().unwrap(),
                    )
                })
                .collect()
        };
        let (status, listed) = call(&app, "GET", &uri, "acct", None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert_eq!(
            counts(&listed),
            [("2026-01-01".to_owned(), 2, 0), ("2026-02-01".to_owned(), 1, 0)]
        );
        LedgerService::seal_entries(app.database()).await.unwrap();
        let (_, listed) = call(&app, "GET", &uri, "acct", None).await;
        assert_eq!(
            counts(&listed),
            [("2026-01-01".to_owned(), 2, 2), ("2026-02-01".to_owned(), 1, 1)]
        );
        // Khoá kỳ cũng trả số đếm.
        let january = listed["data"][0]["id"].as_str().unwrap().to_owned();
        let (status, closed) =
            call(&app, "POST", &format!("/periods/{january}/close"), "acct", None).await;
        assert_eq!(status, StatusCode::OK, "{closed}");
        assert_eq!(closed["data"]["entries"], 2);
        assert_eq!(closed["data"]["sealed_entries"], 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn kiem_chuoi_bam_cua_so_theo_yeu_cau() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let tenant = app.default_tenant();
        let ledger = ledger(&app);
        let mut ids = Vec::new();
        for index in 0..2 {
            let posted = ledger
                .post(
                    &tenant,
                    Actor::system(),
                    transfer("cash.bank", "revenue.sales", 10, &format!("I-{index}")),
                )
                .await
                .unwrap()
                .into_inner();
            ids.push((posted.id, posted.book_id));
        }
        LedgerService::seal_entries(app.database()).await.unwrap();
        let uri = format!("/books/{}/integrity", ids[0].1);
        let (status, check) = call(&app, "GET", &uri, "acct", None).await;
        assert_eq!(status, StatusCode::OK, "{check}");
        assert_eq!(check["data"]["intact"], true);
        assert_eq!(check["data"]["checked"], 2);
        assert_eq!(check["data"]["broken"], Value::Null);

        // Sửa một dòng ngoài API: phát hiện đúng bút toán.
        line::Entity::update_many()
            .col_expr(line::Column::AmountMinor, Expr::col(line::Column::AmountMinor).add(1))
            .filter(line::Column::EntryId.eq(ids[1].0))
            .exec(app.database().write())
            .await
            .unwrap();
        let (_, check) = call(&app, "GET", &uri, "acct", None).await;
        assert_eq!(check["data"]["intact"], false, "{check}");
        assert_eq!(check["data"]["broken"]["entry_id"], ids[1].0.to_string());
        assert_eq!(check["data"]["broken"]["sequence"], 2);

        let (status, _) =
            call(&app, "GET", &format!("/books/{}/integrity", Uuid::now_v7()), "acct", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nang_san_dong_thoi_voi_ghi_khong_xuong_duoi_san() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let tenant = app.default_tenant();
        let ledger = ledger(&app);
        ledger
            .post(&tenant, Actor::system(), transfer("cash.on_hand", "equity.opening", 100, "OPEN"))
            .await
            .unwrap();
        let cash = account_id(&app, &tenant, "cash.on_hand").await;
        // Nâng sàn lên đúng số dư trong lúc các khoản chi đang ghi. Thứ tự nào cũng được, nhưng số dư
        // không bao giờ dưới sàn: Postgres từng cho bút toán dùng sàn cũ (chưa có) sau khi sàn mới
        // đã chốt.
        let spends: Vec<_> = (0..10)
            .map(|index| {
                let (ledger, tenant) = (ledger.clone(), tenant.clone());
                tokio::spawn(async move {
                    let entry = transfer(
                        "expense.gateway_fee",
                        "cash.on_hand",
                        10,
                        &format!("FEE-{index}"),
                    );
                    ledger.post(&tenant, Actor::system(), entry).await
                })
            })
            .collect();
        let floor = json!({"floor": {"amount": "100", "currency": "VND"}});
        let (status, answer) =
            call(&app, "PATCH", &format!("/accounts/{cash}"), "acct", Some(floor)).await;
        for spend in spends {
            if let Err(error) = spend.await.unwrap() {
                assert_eq!(error.code(), "ledger.insufficient_balance");
            }
        }
        let balance = balance_of(&app, &tenant, "cash.on_hand").await;
        let (_, got) = call(&app, "GET", &format!("/accounts/{cash}"), "acct", None).await;
        if status == StatusCode::OK {
            assert_eq!(got["data"]["floor"]["amount"], "100", "{got}");
            assert!(balance >= 100, "balance {balance} below the floor");
        } else {
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
        }
    })
    .await
    .unwrap();
}
