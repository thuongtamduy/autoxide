//! SPIKE: the hardest query shapes of ledger, inventory, reporting and catalog, rewritten on the prototype
//! `db` API, run on SQLite and Postgres (`TEST_DATABASE_URL`). Plus the tenant escape hatches the review named.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

use std::collections::HashMap;
use std::time::Duration;

use autoxide_kernel::config::DatabaseConfig;
use autoxide_kernel::database::Database;
use autoxide_kernel::tenancy::{MAIN_PLACEMENT, TenantContext, TenantId, TenantStatus};
use autoxide_kernel::time::Date;
use autoxide_spike_db::{
    Bounds, Col, Cond, Expr, Queries as _, TenantKey, count, global_db, max, record, sum, tenant_db,
};
use autoxide_testkit::TestDatabase;
use sea_orm::ConnectionTrait as _;
use uuid::Uuid;

// ── records (what `#[derive(Record)]` would generate) ──

record! {
    /// ledger: one journal line.
    struct Line in "ledger_line", scope Tenant {
        id: Uuid => ID,
        tenant_id: TenantKey => TENANT_ID,
        account_id: Uuid => ACCOUNT_ID,
        side: String => SIDE,
        amount_minor: i64 => AMOUNT,
        effective_date: Date => EFFECTIVE_DATE,
    }
}

record! {
    /// ledger: accounting period, locked shared by entries and exclusively by closing.
    struct Period in "ledger_period", scope Tenant {
        id: Uuid => ID,
        tenant_id: TenantKey => TENANT_ID,
        closed: bool => CLOSED,
    }
}

record! {
    /// inventory: reservation line.
    struct ResLine in "inventory_reservation_line", scope Tenant {
        id: Uuid => ID,
        tenant_id: TenantKey => TENANT_ID,
        reservation_id: Uuid => RESERVATION_ID,
        level_id: Uuid => LEVEL_ID,
        quantity: i64 => QUANTITY,
        fulfilled_quantity: i64 => FULFILLED,
        released_quantity: i64 => RELEASED,
    }
}

record! {
    /// inventory: stock level, adjusted with bounds.
    struct Level in "inventory_level", scope Tenant {
        id: Uuid => ID,
        tenant_id: TenantKey => TENANT_ID,
        code: String => CODE,
        on_hand: i64 => ON_HAND,
    }
}

record! {
    /// reporting: daily totals.
    struct Daily in "reporting_daily", scope Tenant {
        id: Uuid => ID,
        tenant_id: TenantKey => TENANT_ID,
        org_id: Uuid => ORG_ID,
        local_day: Date => LOCAL_DAY,
        currency: String => CURRENCY,
        amount_minor: i64 => AMOUNT,
        fact_count: i64 => FACTS,
    }
}

record! {
    /// catalog: storefront entry.
    struct Entry in "catalog_entry", scope Tenant {
        id: Uuid => ID,
        tenant_id: TenantKey => TENANT_ID,
        product_id: Uuid => PRODUCT_ID,
    }
}

record! {
    /// catalog: attribute value of a product.
    struct ProductAttr in "catalog_product_attribute", scope Tenant {
        id: Uuid => ID,
        tenant_id: TenantKey => TENANT_ID,
        product_id: Uuid => PRODUCT_ID,
        attribute_id: Uuid => ATTRIBUTE_ID,
        value_text: Option<String> => VALUE_TEXT,
        value_integer: Option<i64> => VALUE_INTEGER,
    }
}

record! {
    /// catalog: searchable translation.
    struct Translation in "catalog_translation", scope Tenant {
        id: Uuid => ID,
        tenant_id: TenantKey => TENANT_ID,
        product_id: Uuid => PRODUCT_ID,
        locale: String => LOCALE,
        search_key: String => SEARCH_KEY,
    }
}

record! {
    /// A table of another module, to check that joins across modules are refused.
    struct Foreign in "orders_order", scope Tenant {
        id: Uuid => ID,
        tenant_id: TenantKey => TENANT_ID,
        product_id: Uuid => PRODUCT_ID,
    }
}

record! {
    /// A control-plane table.
    struct Plan in "plans_plan", scope Global {
        id: Uuid => ID,
        code: String => CODE,
    }
}

const DDL: &[&str] = &[
    "CREATE TABLE ledger_line (id uuid PRIMARY KEY, tenant_id uuid NOT NULL, account_id uuid NOT NULL, side varchar(8) NOT NULL, amount_minor bigint NOT NULL, effective_date date NOT NULL)",
    "CREATE TABLE ledger_period (id uuid PRIMARY KEY, tenant_id uuid NOT NULL, closed boolean NOT NULL)",
    "CREATE TABLE inventory_reservation_line (id uuid PRIMARY KEY, tenant_id uuid NOT NULL, reservation_id uuid NOT NULL, level_id uuid NOT NULL, quantity bigint NOT NULL, fulfilled_quantity bigint NOT NULL, released_quantity bigint NOT NULL)",
    // Khoá unique KHÔNG có tenant_id: ca mà upsert dễ ghi đè dòng của khách khác.
    "CREATE TABLE inventory_level (id uuid PRIMARY KEY, tenant_id uuid NOT NULL, code varchar(40) NOT NULL, on_hand bigint NOT NULL)",
    "CREATE UNIQUE INDEX inventory_level_code ON inventory_level (code)",
    "CREATE TABLE reporting_daily (id uuid PRIMARY KEY, tenant_id uuid NOT NULL, org_id uuid NOT NULL, local_day date NOT NULL, currency varchar(3) NOT NULL, amount_minor bigint NOT NULL, fact_count bigint NOT NULL)",
    "CREATE TABLE catalog_entry (id uuid PRIMARY KEY, tenant_id uuid NOT NULL, product_id uuid NOT NULL)",
    "CREATE TABLE catalog_product_attribute (id uuid PRIMARY KEY, tenant_id uuid NOT NULL, product_id uuid NOT NULL, attribute_id uuid NOT NULL, value_text text NULL, value_integer bigint NULL)",
    "CREATE TABLE catalog_translation (id uuid PRIMARY KEY, tenant_id uuid NOT NULL, product_id uuid NOT NULL, locale varchar(10) NOT NULL, search_key text NOT NULL)",
    "CREATE TABLE orders_order (id uuid PRIMARY KEY, tenant_id uuid NOT NULL, product_id uuid NOT NULL)",
    "CREATE TABLE plans_plan (id uuid PRIMARY KEY, code varchar(20) NOT NULL)",
];

async fn open(test: &TestDatabase) -> Database {
    let config = DatabaseConfig {
        url: test.url().parse().unwrap(),
        read_url: None,
        write_connections: 4,
        read_connections: 4,
        wait: Duration::from_secs(5),
    };
    let database = Database::connect(&config).await.unwrap();
    for ddl in DDL {
        database.write().execute_unprepared(ddl).await.unwrap();
    }
    database
}

fn tenant(database: &Database, n: u128) -> TenantContext {
    TenantContext::new(
        TenantId::from_uuid(Uuid::from_u128(n)),
        format!("t{n}"),
        format!("Tenant {n}"),
        TenantStatus::Active,
        MAIN_PLACEMENT,
        database.clone(),
    )
}

fn day(text: &str) -> Date {
    Date::parse_from_str(text, "%Y-%m-%d").unwrap()
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

#[tokio::test]
async fn bon_truy_van_kho_nhat_va_cach_ly_khach() {
    TestDatabase::run(async |test| {
        let database = open(test).await;
        let a = tenant(&database, 0xA);
        let b = tenant(&database, 0xB);
        let (ta, tb) = (TenantKey(a.id().as_uuid()), TenantKey(b.id().as_uuid()));

        // ── dữ liệu mẫu: khách A và khách B có dữ liệu giống hình dạng ──
        for (ctx, key, base) in [(&a, ta, 0x1000_u128), (&b, tb, 0x2000_u128)] {
            let tx = tenant_db(ctx).begin_write().await.unwrap();
            let acc = id(base + 1);
            for (n, side, amount, date) in [
                (10, "debit", 500, "2026-01-05"),
                (11, "credit", 200, "2026-01-06"),
                (12, "debit", 1_000, "2026-02-01"),
            ] {
                let line = Line { id: id(base + n), tenant_id: key, account_id: acc, side: side.into(), amount_minor: amount, effective_date: day(date) };
                tx.insert(line).exec().await.unwrap();
            }
            tx.insert(Period { id: id(base + 20), tenant_id: key, closed: false }).exec().await.unwrap();
            for (n, reservation, level, q, f, r) in [(30, 31, 40, 10, 3, 2), (32, 33, 40, 4, 0, 0), (34, 35, 41, 7, 7, 0)] {
                let row = ResLine { id: id(base + n), tenant_id: key, reservation_id: id(base + reservation), level_id: id(base + level), quantity: q, fulfilled_quantity: f, released_quantity: r };
                tx.insert(row).exec().await.unwrap();
            }
            tx.insert(Level { id: id(base + 40), tenant_id: key, code: format!("L-{base:x}"), on_hand: 5 }).exec().await.unwrap();
            for (n, org, date, cur, amount, facts) in [(50, 1, "2026-03-01", "VND", 100, 1), (51, 1, "2026-03-01", "VND", 250, 2), (52, 2, "2026-03-02", "USD", 7, 1)] {
                let row = Daily { id: id(base + n), tenant_id: key, org_id: id(base + org), local_day: day(date), currency: cur.into(), amount_minor: amount, fact_count: facts };
                tx.insert(row).exec().await.unwrap();
            }
            for (n, product) in [(60, 70), (61, 71), (62, 72)] {
                tx.insert(Entry { id: id(base + n), tenant_id: key, product_id: id(base + product) }).exec().await.unwrap();
            }
            let colour = id(0xC0);
            for (n, product, text) in [(80, 70, "red"), (81, 71, "blue"), (82, 72, "red")] {
                let row = ProductAttr { id: id(base + n), tenant_id: key, product_id: id(base + product), attribute_id: colour, value_text: Some(text.into()), value_integer: None };
                tx.insert(row).exec().await.unwrap();
            }
            for (n, product, text) in [(90, 70, "ao thun 100%_cotton"), (91, 71, "ao khoac"), (92, 72, "quan jean")] {
                let row = Translation { id: id(base + n), tenant_id: key, product_id: id(base + product), locale: "vi".into(), search_key: text.into() };
                tx.insert(row).exec().await.unwrap();
            }
            tx.commit().await.unwrap();
        }

        let read = tenant_db(&a).read();

        // ── (1) ledger: tổng có dấu theo bên nợ, theo tài khoản, trong khoảng ngày (CASE WHEN trong SUM) ──
        let signed = Expr::case_when(
            Line::SIDE.eq("debit".into()),
            Line::AMOUNT.expr(),
            -Line::AMOUNT.expr(),
        );
        let sums: Vec<(Uuid, i64)> = read
            .find::<Line>()
            .filter(Line::EFFECTIVE_DATE.gt(day("2026-01-01")))
            .filter(Line::EFFECTIVE_DATE.lte(day("2026-01-31")))
            .select((Line::ACCOUNT_ID, sum(signed)))
            .group_by(Line::ACCOUNT_ID)
            .all()
            .await
            .unwrap();
        assert_eq!(sums, vec![(id(0x1001), 300)], "chỉ khách A, tháng 1: 500 − 200");

        // ── (2) inventory: lượng đang giữ = SUM(q − f − r) theo kho, lọc theo danh sách (chunk IN) ──
        let held: Vec<(Uuid, i64)> = read
            .find::<ResLine>()
            .filter(ResLine::RESERVATION_ID.is_in([id(0x1031), id(0x1033), id(0x1035), id(0x2031)]))
            .select((ResLine::LEVEL_ID, sum(ResLine::QUANTITY.expr() - ResLine::FULFILLED.expr() - ResLine::RELEASED.expr())))
            .group_by(ResLine::LEVEL_ID)
            .order_asc(ResLine::LEVEL_ID)
            .all()
            .await
            .unwrap();
        assert_eq!(held, vec![(id(0x1040), 9), (id(0x1041), 0)]);

        // ── (3) reporting: nhóm theo (nút, ngày, tiền tệ), SUM ép BIGINT, lọc IN + khoảng ngày ──
        let rows: Vec<(Uuid, Date, String, i64, i64)> = read
            .find::<Daily>()
            .filter(Daily::LOCAL_DAY.gte(day("2026-03-01")))
            .filter(Daily::LOCAL_DAY.lte(day("2026-03-31")))
            .filter(Daily::ORG_ID.is_in([id(0x1001), id(0x1002)]))
            .select((Daily::ORG_ID, Daily::LOCAL_DAY, Daily::CURRENCY, sum(Daily::AMOUNT), sum(Daily::FACTS)))
            .group_by(Daily::ORG_ID)
            .group_by(Daily::LOCAL_DAY)
            .group_by(Daily::CURRENCY)
            .order_asc(Daily::LOCAL_DAY)
            .all()
            .await
            .unwrap();
        assert_eq!(
            rows,
            vec![
                (id(0x1001), day("2026-03-01"), "VND".to_owned(), 350, 3),
                (id(0x1002), day("2026-03-02"), "USD".to_owned(), 7, 1),
            ]
        );
        let (latest,): (Option<Date>,) = read.find::<Daily>().select((max(Daily::LOCAL_DAY),)).all().await.unwrap()[0].clone();
        assert_eq!(latest, Some(day("2026-03-02")));

        // ── (4) catalog: bộ lọc thuộc tính động (OR) qua subquery + tìm chữ có ký tự đặc biệt ──
        let mut any = Cond::any();
        for value in ["red", "green"] {
            any = any.add(ProductAttr::VALUE_TEXT.eq(Some(value.to_owned())));
        }
        let matching = read.sub(ProductAttr::PRODUCT_ID).filter(ProductAttr::ATTRIBUTE_ID.eq(id(0xC0))).filter(any);
        let texts = read
            .sub(Translation::PRODUCT_ID)
            .filter(Translation::LOCALE.eq("vi".into()))
            .filter(Cond::any().add(Translation::SEARCH_KEY.contains("100%_c")));
        let found: Vec<Entry> = read
            .find::<Entry>()
            .filter(Entry::PRODUCT_ID.in_sub(matching))
            .filter(Entry::PRODUCT_ID.in_sub(texts))
            .all()
            .await
            .unwrap();
        assert_eq!(found.iter().map(|e| e.product_id).collect::<Vec<_>>(), vec![id(0x1070)]);
        // `%` và `_` là chữ thường, không phải ký tự đại diện.
        let none = read.find::<Translation>().filter(Translation::SEARCH_KEY.contains("100%x")).count().await.unwrap();
        assert_eq!(none, 0);

        // join trong cùng module: dòng của B không lọt qua bảng được nối.
        let joined = read
            .find::<Entry>()
            .join(Entry::PRODUCT_ID.eq_col(ProductAttr::PRODUCT_ID))
            .count()
            .await;
        assert_eq!(joined.unwrap(), 3);
        // join sang module khác: bị từ chối.
        let cross = read.find::<Entry>().join(Entry::PRODUCT_ID.eq_col(Foreign::PRODUCT_ID)).all().await;
        assert!(cross.is_err());

        // ── (5) khoá: chia sẻ trên kỳ, độc quyền trên mức tồn, khoá theo tên, trừ tồn có cận ──
        let tx = tenant_db(&a).begin_write().await.unwrap();
        let period = tx.find::<Period>().filter(Period::ID.eq(id(0x1020))).for_share().one().await.unwrap();
        assert!(period.is_some_and(|p| !p.closed));
        tx.lock_key("ledger.close").await.unwrap();
        let level = tx.find::<Level>().filter(Level::ID.eq(id(0x1040))).for_update().one().await.unwrap().unwrap();
        assert_eq!(level.on_hand, 5);
        let ok = tx.update::<Level>().adjust(Level::ON_HAND, -5, Bounds::NON_NEGATIVE).filter(Level::ID.eq(id(0x1040))).exec().await.unwrap();
        let too_much = tx.update::<Level>().adjust(Level::ON_HAND, -1, Bounds::NON_NEGATIVE).filter(Level::ID.eq(id(0x1040))).exec().await.unwrap();
        let overflow = tx.update::<Level>().adjust(Level::ON_HAND, i64::MAX, Bounds { floor: 0, ceiling: i64::MAX }).filter(Level::ID.eq(id(0x1040))).exec().await.unwrap();
        assert_eq!((ok, too_much, overflow), (1, 0, 1), "0 + MAX vẫn trong cận");
        let overflow = tx.update::<Level>().adjust(Level::ON_HAND, 1, Bounds { floor: 0, ceiling: i64::MAX }).filter(Level::ID.eq(id(0x1040))).exec().await.unwrap();
        assert_eq!(overflow, 0, "MAX + 1 tràn: không đổi, không lỗi, cả hai database");
        // B không sửa được dòng của A dù biết ID.
        let tx_b = tenant_db(&b).begin_write().await;
        drop(tx_b);
        tx.commit().await.unwrap();
        let tx_b = tenant_db(&b).begin_write().await.unwrap();
        let touched = tx_b.update::<Level>().set(Level::ON_HAND, 99).filter(Level::ID.eq(id(0x1040))).exec().await.unwrap();
        let deleted = tx_b.delete::<Level>().filter(Level::ID.eq(id(0x1040))).exec().await.unwrap();
        assert_eq!((touched, deleted), (0, 0));

        // ── (6) upsert trên khoá unique không có tenant_id: B không ghi đè được dòng của A ──
        let clash = Level { id: id(0x2099), tenant_id: tb, code: "L-1000".into(), on_hand: 1 };
        let result = tx_b.insert(clash).on_conflict_update([Level::CODE.any()], [Level::ON_HAND.any()]).exec().await;
        assert!(result.is_err(), "xung đột với dòng của khách khác thành lỗi, không ghi đè");
        tx_b.commit().await.ok();
        let still = tenant_db(&a).read().find::<Level>().filter(Level::CODE.eq("L-1000".into())).one().await.unwrap().unwrap();
        assert_eq!(still.on_hand, i64::MAX);
        // upsert của chính A vẫn cập nhật.
        let tx = tenant_db(&a).begin_write().await.unwrap();
        let own = Level { id: id(0x1099), tenant_id: ta, code: "L-1000".into(), on_hand: 3 };
        assert_eq!(tx.insert(own).on_conflict_update([Level::CODE.any()], [Level::ON_HAND.any()]).exec().await.unwrap(), 1);
        // insert dòng mang tenant_id của khách khác: từ chối trước khi gửi câu.
        let wrong = Level { id: id(0x1098), tenant_id: tb, code: "X".into(), on_hand: 0 };
        assert!(tx.insert(wrong).exec().await.is_err());
        tx.commit().await.unwrap();

        // ── (7) SQL viết tay: bắt buộc `:tenant` trên handle của khách ──
        let raw: Vec<(String, i64)> = read
            .sql("SELECT currency, CAST(SUM(amount_minor) AS BIGINT) FROM reporting_daily WHERE tenant_id = :tenant AND local_day >= :from GROUP BY currency ORDER BY currency")
            .bind("from", day("2026-03-01"))
            .all()
            .await
            .unwrap();
        assert_eq!(raw, vec![("USD".to_owned(), 7), ("VND".to_owned(), 350)]);
        let unscoped = read.sql("SELECT currency, amount_minor FROM reporting_daily").all::<(String, i64)>().await;
        assert!(unscoped.is_err());

        // ── (8) handle không có khách: bảng của khách bị từ chối; bảng control-plane chạy ──
        let global = global_db(&database);
        assert!(global.find::<Line>().all().await.is_err());
        assert_eq!(global.find::<Plan>().count().await.unwrap(), 0);

        // Đếm tổng thể: mỗi khách thấy đúng phần mình.
        let mut per_tenant = HashMap::new();
        for ctx in [&a, &b] {
            per_tenant.insert(ctx.id().as_uuid(), tenant_db(ctx).read().find::<Line>().count().await.unwrap());
        }
        assert!(per_tenant.values().all(|n| *n == 3));
        let _ = count::<Line>;
        let _: Col<Line, TenantKey> = Line::TENANT_ID;
    })
    .await
    .unwrap();
}
