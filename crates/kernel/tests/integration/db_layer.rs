//! Lớp `db` (D-20261126-kernel-data-layer): những hình dạng truy vấn khó nhất của module (sổ cái, tồn kho, báo
//! cáo, bộ lọc thuộc tính của catalog), khoá, upsert, SQL viết tay — và cách ly khách — chạy giống nhau trên
//! SQLite và Postgres. Bảng tên `kernel_…` vì crate đang biên dịch là `autoxide-kernel` (derive kiểm tiền tố).

use std::collections::HashMap;

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::db::{
    Bounds, Cond, Expr, Queries as _, Record, TenantKey, all_tenants_for_tests,
    is_unique_violation, max, register_control_plane_for_tests, sum,
};
use autoxide_kernel::pagination::{Order, PageRequest};
use autoxide_kernel::tenancy::{MAIN_PLACEMENT, TenantContext, TenantId, TenantStatus};
use autoxide_kernel::time::Date;
use autoxide_testkit::TestDatabase;
use uuid::Uuid;

#[derive(Record, Debug, Clone, PartialEq)]
#[record(table = "kernel_t_line", scope = tenant)]
struct Line {
    #[record(primary_key)]
    id: Uuid,
    tenant_id: TenantKey,
    account_id: Uuid,
    side: String,
    amount_minor: i64,
    effective_date: Date,
}

#[derive(Record, Debug, Clone, PartialEq)]
#[record(table = "kernel_t_period", scope = tenant)]
struct Period {
    id: Uuid,
    tenant_id: TenantKey,
    closed: bool,
}

#[derive(Record, Debug, Clone, PartialEq)]
#[record(table = "kernel_t_reservation_line", scope = tenant)]
struct ResLine {
    id: Uuid,
    tenant_id: TenantKey,
    reservation_id: Uuid,
    level_id: Uuid,
    quantity: i64,
    fulfilled_quantity: i64,
    released_quantity: i64,
}

#[derive(Record, Debug, Clone, PartialEq)]
#[record(table = "kernel_t_level", scope = tenant)]
struct Level {
    id: Uuid,
    tenant_id: TenantKey,
    code: String,
    on_hand: i64,
}

#[derive(Record, Debug, Clone, PartialEq)]
#[record(table = "kernel_t_daily", scope = tenant)]
struct Daily {
    id: Uuid,
    tenant_id: TenantKey,
    org_id: Uuid,
    local_day: Date,
    currency: String,
    amount_minor: i64,
    fact_count: i64,
}

#[derive(Record, Debug, Clone, PartialEq)]
#[record(table = "kernel_t_entry", scope = tenant)]
struct Entry {
    id: Uuid,
    tenant_id: TenantKey,
    product_id: Uuid,
}

#[derive(Record, Debug, Clone, PartialEq)]
#[record(table = "kernel_t_product_attribute", scope = tenant)]
struct ProductAttr {
    id: Uuid,
    tenant_id: TenantKey,
    product_id: Uuid,
    attribute_id: Uuid,
    value_text: Option<String>,
}

#[derive(Record, Debug, Clone, PartialEq)]
#[record(table = "kernel_t_translation", scope = tenant)]
struct Translation {
    id: Uuid,
    tenant_id: TenantKey,
    product_id: Uuid,
    locale: String,
    search_key: String,
}

#[derive(Record, Debug, Clone, PartialEq)]
#[record(table = "kernel_t_plan", scope = global)]
struct Plan {
    id: Uuid,
    code: String,
}

const DDL: &[&str] = &[
    "CREATE TABLE kernel_t_line (id uuid PRIMARY KEY, tenant_id uuid NOT NULL, account_id uuid NOT NULL, side varchar(8) NOT NULL, amount_minor bigint NOT NULL, effective_date date NOT NULL)",
    "CREATE TABLE kernel_t_period (id uuid PRIMARY KEY, tenant_id uuid NOT NULL, closed boolean NOT NULL)",
    "CREATE TABLE kernel_t_reservation_line (id uuid PRIMARY KEY, tenant_id uuid NOT NULL, reservation_id uuid NOT NULL, level_id uuid NOT NULL, quantity bigint NOT NULL, fulfilled_quantity bigint NOT NULL, released_quantity bigint NOT NULL)",
    // Khoá unique KHÔNG có tenant_id: ca mà upsert dễ ghi đè dòng của khách khác.
    "CREATE TABLE kernel_t_level (id uuid PRIMARY KEY, tenant_id uuid NOT NULL, code varchar(40) NOT NULL, on_hand bigint NOT NULL)",
    "CREATE UNIQUE INDEX kernel_t_level_code ON kernel_t_level (code)",
    "CREATE TABLE kernel_t_daily (id uuid PRIMARY KEY, tenant_id uuid NOT NULL, org_id uuid NOT NULL, local_day date NOT NULL, currency varchar(3) NOT NULL, amount_minor bigint NOT NULL, fact_count bigint NOT NULL)",
    "CREATE TABLE kernel_t_entry (id uuid PRIMARY KEY, tenant_id uuid NOT NULL, product_id uuid NOT NULL)",
    "CREATE TABLE kernel_t_product_attribute (id uuid PRIMARY KEY, tenant_id uuid NOT NULL, product_id uuid NOT NULL, attribute_id uuid NOT NULL, value_text text NULL)",
    "CREATE TABLE kernel_t_translation (id uuid PRIMARY KEY, tenant_id uuid NOT NULL, product_id uuid NOT NULL, locale varchar(10) NOT NULL, search_key text NOT NULL)",
    "CREATE TABLE kernel_t_plan (id uuid PRIMARY KEY, code varchar(20) NOT NULL)",
];

async fn open(test: &TestDatabase) -> Database {
    let source = ConfigSource::from_pairs(vec![("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    // Bảng control-plane của file này (trong app thật: `Module::control_plane_tables`).
    register_control_plane_for_tests(&["kernel_t_plan"]);
    let tx = all_tenants_for_tests(&database).begin_write().await.unwrap();
    for ddl in DDL {
        tx.sql(ddl).execute().await.unwrap();
    }
    tx.commit().await.unwrap();
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

/// Hai khách có dữ liệu cùng hình dạng: khách A ở `0x1000 + n`, khách B ở `0x2000 + n`.
async fn seed(a: &TenantContext, b: &TenantContext) {
    for (ctx, base) in [(a, 0x1000_u128), (b, 0x2000_u128)] {
        let key = TenantKey::new(ctx.id());
        let tx = ctx.db().begin_write().await.unwrap();
        let account = id(base + 0x1);
        let lines = [
            (0x10, "debit", 500, "2026-01-05"),
            (0x11, "credit", 200, "2026-01-06"),
            (0x12, "debit", 1_000, "2026-02-01"),
        ]
        .map(|(n, side, amount, date)| Line {
            id: id(base + n),
            tenant_id: key,
            account_id: account,
            side: side.into(),
            amount_minor: amount,
            effective_date: day(date),
        });
        assert_eq!(tx.insert_many(lines).exec().await.unwrap(), 3);
        tx.insert(Period { id: id(base + 0x20), tenant_id: key, closed: false })
            .exec()
            .await
            .unwrap();
        for (n, reservation, level, q, f, r) in
            [(0x30, 0x31, 0x40, 10, 3, 2), (0x32, 0x33, 0x40, 4, 0, 0), (0x34, 0x35, 0x41, 7, 7, 0)]
        {
            let row = ResLine {
                id: id(base + n),
                tenant_id: key,
                reservation_id: id(base + reservation),
                level_id: id(base + level),
                quantity: q,
                fulfilled_quantity: f,
                released_quantity: r,
            };
            tx.insert(row).exec().await.unwrap();
        }
        tx.insert(Level {
            id: id(base + 0x40),
            tenant_id: key,
            code: format!("L-{base:x}"),
            on_hand: 5,
        })
        .exec()
        .await
        .unwrap();
        for (n, org, date, currency, amount, facts) in [
            (0x50, 0x1, "2026-03-01", "VND", 100, 1),
            (0x51, 0x1, "2026-03-01", "VND", 250, 2),
            (0x52, 0x2, "2026-03-02", "USD", 7, 1),
        ] {
            let row = Daily {
                id: id(base + n),
                tenant_id: key,
                org_id: id(base + org),
                local_day: day(date),
                currency: currency.into(),
                amount_minor: amount,
                fact_count: facts,
            };
            tx.insert(row).exec().await.unwrap();
        }
        let colour = id(0xC0);
        for (n, product, text, search) in [
            (0x60, 0x70, "red", "ao thun 100%_cotton"),
            (0x61, 0x71, "blue", "ao khoac"),
            (0x62, 0x72, "red", "quan jean"),
        ] {
            tx.insert(Entry { id: id(base + n), tenant_id: key, product_id: id(base + product) })
                .exec()
                .await
                .unwrap();
            let attribute = ProductAttr {
                id: id(base + n + 0x20),
                tenant_id: key,
                product_id: id(base + product),
                attribute_id: colour,
                value_text: Some(text.into()),
            };
            tx.insert(attribute).exec().await.unwrap();
            let translation = Translation {
                id: id(base + n + 0x30),
                tenant_id: key,
                product_id: id(base + product),
                locale: "vi".into(),
                search_key: search.into(),
            };
            tx.insert(translation).exec().await.unwrap();
        }
        tx.commit().await.unwrap();
    }
}

#[tokio::test]
async fn truy_van_kho_nhat_cua_module_chay_dung_tren_ca_hai_database() {
    TestDatabase::run(async |test| {
        let database = open(test).await;
        let (a, b) = (tenant(&database, 0xA), tenant(&database, 0xB));
        seed(&a, &b).await;
        let read = a.db().read();

        // Sổ cái: tổng có dấu theo bên nợ, theo tài khoản, trong khoảng ngày (CASE WHEN trong SUM).
        let signed = Expr::case_when(
            Line::SIDE.eq("debit".into()),
            Line::AMOUNT_MINOR.expr(),
            -Line::AMOUNT_MINOR.expr(),
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

        // Tồn kho: lượng đang giữ = SUM(q − f − r) theo kho, lọc IN.
        let held: Vec<(Uuid, i64)> = read
            .find::<ResLine>()
            .filter(ResLine::RESERVATION_ID.is_in([id(0x1031), id(0x1033), id(0x1035), id(0x2031)]))
            .select((
                ResLine::LEVEL_ID,
                sum(ResLine::QUANTITY.expr()
                    - ResLine::FULFILLED_QUANTITY.expr()
                    - ResLine::RELEASED_QUANTITY.expr()),
            ))
            .group_by(ResLine::LEVEL_ID)
            .order_asc(ResLine::LEVEL_ID)
            .all()
            .await
            .unwrap();
        assert_eq!(held, vec![(id(0x1040), 9), (id(0x1041), 0)]);

        // Báo cáo: gom theo (nút, ngày, tiền tệ), SUM ép BIGINT; MAX trả Option.
        let rows: Vec<(Uuid, Date, String, i64, i64)> = read
            .find::<Daily>()
            .filter(Daily::LOCAL_DAY.gte(day("2026-03-01")))
            .filter(Daily::ORG_ID.is_in([id(0x1001), id(0x1002)]))
            .select((
                Daily::ORG_ID,
                Daily::LOCAL_DAY,
                Daily::CURRENCY,
                sum(Daily::AMOUNT_MINOR),
                sum(Daily::FACT_COUNT),
            ))
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
        let latest: Vec<(Option<Date>,)> =
            read.find::<Daily>().select((max(Daily::LOCAL_DAY),)).all().await.unwrap();
        assert_eq!(latest, vec![(Some(day("2026-03-02")),)]);
        let empty: Vec<(Option<Date>, i64)> = read
            .find::<Daily>()
            .filter(Daily::CURRENCY.eq("EUR".into()))
            .select((max(Daily::LOCAL_DAY), sum(Daily::AMOUNT_MINOR)))
            .all()
            .await
            .unwrap();
        assert_eq!(
            empty,
            vec![(None, 0)],
            "không có dòng: MAX là NULL, SUM là 0 trên cả hai database"
        );

        // Catalog: bộ lọc thuộc tính dựng động (OR) qua subquery, cộng tìm chữ có `%` `_` là chữ thường.
        let mut any = Cond::any();
        for value in ["red", "green"] {
            any = any.with(ProductAttr::VALUE_TEXT.eq(Some(value.to_owned())));
        }
        let matching = read
            .sub(ProductAttr::PRODUCT_ID)
            .filter(ProductAttr::ATTRIBUTE_ID.eq(id(0xC0)))
            .filter(any);
        let texts = read
            .sub(Translation::PRODUCT_ID)
            .filter(Translation::LOCALE.eq("vi".into()))
            .filter(Translation::SEARCH_KEY.contains("100%_c"));
        let found: Vec<Entry> = read
            .find::<Entry>()
            .filter(Entry::PRODUCT_ID.in_sub(matching))
            .filter(Entry::PRODUCT_ID.in_sub(texts))
            .all()
            .await
            .unwrap();
        assert_eq!(
            found.iter().map(|entry| entry.product_id).collect::<Vec<_>>(),
            vec![id(0x1070)]
        );
        assert_eq!(
            read.find::<Translation>()
                .filter(Translation::SEARCH_KEY.contains("100%x"))
                .count()
                .await
                .unwrap(),
            0
        );

        // Nối trong cùng module: bảng được nối cũng lọc theo khách.
        let joined = read
            .find::<Entry>()
            .join(Entry::PRODUCT_ID.eq_col(ProductAttr::PRODUCT_ID))
            .count()
            .await
            .unwrap();
        assert_eq!(joined, 3);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khoa_va_adjust_co_can_giong_nhau_hai_database() {
    TestDatabase::run(async |test| {
        let database = open(test).await;
        let (a, b) = (tenant(&database, 0xA), tenant(&database, 0xB));
        seed(&a, &b).await;
        let tx = a.db().begin_write().await.unwrap();
        let period =
            tx.find::<Period>().filter(Period::ID.eq(id(0x1020))).for_share().one().await.unwrap();
        assert!(period.is_some_and(|period| !period.closed));
        tx.lock_key("ledger.close").await.unwrap();
        let level = tx
            .find::<Level>()
            .filter(Level::ID.eq(id(0x1040)))
            .for_update()
            .one()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(level.on_hand, 5);
        let by = |delta, bounds| {
            tx.update::<Level>()
                .adjust(Level::ON_HAND, delta, bounds)
                .filter(Level::ID.eq(id(0x1040)))
        };
        assert_eq!(by(-5, Bounds::NON_NEGATIVE).exec().await.unwrap(), 1);
        assert_eq!(by(-1, Bounds::NON_NEGATIVE).exec().await.unwrap(), 0, "không đủ: 0 dòng");
        assert_eq!(by(i64::MAX, Bounds::NON_NEGATIVE).exec().await.unwrap(), 1);
        assert_eq!(
            by(1, Bounds::NON_NEGATIVE).exec().await.unwrap(),
            0,
            "MAX + 1 tràn: 0 dòng, không lỗi"
        );
        tx.commit().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khach_khong_cham_duoc_dong_cua_khach_khac() {
    TestDatabase::run(async |test| {
        let database = open(test).await;
        let (a, b) = (tenant(&database, 0xA), tenant(&database, 0xB));
        seed(&a, &b).await;

        // Đọc, sửa, xoá theo ID của khách kia: không thấy, 0 dòng.
        assert!(
            b.db()
                .read()
                .find::<Level>()
                .filter(Level::ID.eq(id(0x1040)))
                .one()
                .await
                .unwrap()
                .is_none()
        );
        let tx = b.db().begin_write().await.unwrap();
        assert_eq!(
            tx.update::<Level>()
                .set(Level::ON_HAND, 99)
                .filter(Level::ID.eq(id(0x1040)))
                .exec()
                .await
                .unwrap(),
            0
        );
        assert_eq!(tx.delete::<Level>().filter(Level::ID.eq(id(0x1040))).exec().await.unwrap(), 0);

        // Upsert trên khoá unique không có tenant_id: đụng dòng của A thành lỗi unique, dòng của A không đổi.
        let clash = Level {
            id: id(0x2099),
            tenant_id: TenantKey::new(b.id()),
            code: "L-1000".into(),
            on_hand: 1,
        };
        let error = tx
            .insert(clash)
            .on_conflict_update([Level::CODE.key()], [Level::ON_HAND.updated()])
            .exec()
            .await
            .unwrap_err();
        assert!(is_unique_violation(&error), "{error:?}");
        // Insert dòng mang tenant_id của khách khác: từ chối trước khi gửi câu.
        let foreign = Level {
            id: id(0x2098),
            tenant_id: TenantKey::new(a.id()),
            code: "X".into(),
            on_hand: 0,
        };
        assert!(tx.insert(foreign).exec().await.is_err());
        tx.rollback().await.unwrap();
        let still = a
            .db()
            .read()
            .find::<Level>()
            .filter(Level::CODE.eq("L-1000".into()))
            .one()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(still.on_hand, 5);

        // Upsert của chính A vẫn cập nhật; insert trùng thường là lỗi unique.
        let tx = a.db().begin_write().await.unwrap();
        let own = Level {
            id: id(0x1099),
            tenant_id: TenantKey::new(a.id()),
            code: "L-1000".into(),
            on_hand: 3,
        };
        assert_eq!(
            tx.insert(own.clone())
                .on_conflict_update([Level::CODE.key()], [Level::ON_HAND.updated()])
                .exec()
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            tx.insert(own.clone())
                .on_conflict_do_nothing([Level::CODE.key()])
                .exec()
                .await
                .unwrap(),
            0
        );
        assert!(is_unique_violation(&tx.insert(own).exec().await.unwrap_err()));
        tx.rollback().await.unwrap();

        // `returning` đọc lại dòng vừa đổi, chỉ của khách mình.
        let tx = a.db().begin_write().await.unwrap();
        let changed = tx
            .update::<Level>()
            .set(Level::ON_HAND, 7)
            .filter(Level::CODE.starts_with("L-"))
            .returning()
            .await
            .unwrap();
        assert_eq!(changed.iter().map(|level| level.id).collect::<Vec<_>>(), vec![id(0x1040)]);
        tx.commit().await.unwrap();

        // Mỗi khách đếm đúng phần mình; handle "mọi khách" thấy cả hai.
        let mut per_tenant = HashMap::new();
        for ctx in [&a, &b] {
            per_tenant.insert(ctx.id(), ctx.db().read().find::<Line>().count().await.unwrap());
        }
        assert!(per_tenant.values().all(|n| *n == 3));
        let everyone = all_tenants_for_tests(&database);
        assert_eq!(everyone.read().find::<Line>().count().await.unwrap(), 6);
        assert_eq!(everyone.read().find::<Plan>().count().await.unwrap(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn sql_viet_tay_va_trang_keyset() {
    TestDatabase::run(async |test| {
        let database = open(test).await;
        let (a, b) = (tenant(&database, 0xA), tenant(&database, 0xB));
        seed(&a, &b).await;
        let read = a.db().read();

        let raw: Vec<(String, i64)> = read
            .sql("SELECT currency, CAST(SUM(amount_minor) AS BIGINT) FROM kernel_t_daily WHERE tenant_id = :tenant AND local_day >= :from GROUP BY currency ORDER BY currency")
            .bind("from", day("2026-03-01"))
            .all()
            .await
            .unwrap();
        assert_eq!(raw, vec![("USD".to_owned(), 7), ("VND".to_owned(), 350)]);
        assert!(read.sql("SELECT currency, amount_minor FROM kernel_t_daily").all::<(String, i64)>().await.is_err());

        // Trang keyset theo id, mới nhất trước, có tổng ở trang đầu; con trỏ của khách A không lộ dòng của B.
        let first = read.find::<Line>().page(Line::ID, &PageRequest::first(2, Order::Desc).counted()).await.unwrap();
        assert_eq!(first.data.iter().map(|line| line.id).collect::<Vec<_>>(), vec![id(0x1012), id(0x1011)]);
        assert_eq!(first.pagination.total, Some(3));
        let cursor = first.pagination.next_cursor.clone().unwrap();
        let next = PageRequest::first(2, Order::Desc).after(cursor.parse().unwrap());
        let second = read.find::<Line>().page(Line::ID, &next).await.unwrap();
        assert_eq!(second.data.iter().map(|line| line.id).collect::<Vec<_>>(), vec![id(0x1010)]);
        assert_eq!(second.pagination.next_cursor, None);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn loi_thoat_da_dong_upsert_nhieu_dong_subquery_la_hoa_thuong() {
    TestDatabase::run(async |test| {
        let database = open(test).await;
        let (a, b) = (tenant(&database, 0xA), tenant(&database, 0xB));
        seed(&a, &b).await;

        // Upsert nhiều dòng của B, một dòng va khoá unique của A: cả lô là lỗi (không lặng lẽ bỏ dòng).
        let key = TenantKey::new(b.id());
        let rows = [
            Level { id: id(0x2201), tenant_id: key, code: "B-NEW".into(), on_hand: 1 },
            Level { id: id(0x2202), tenant_id: key, code: "L-1000".into(), on_hand: 1 },
        ];
        let tx = b.db().begin_write().await.unwrap();
        let error = tx
            .insert_many(rows)
            .on_conflict_update([Level::CODE.key()], [Level::ON_HAND.updated()])
            .exec()
            .await
            .unwrap_err();
        assert!(is_unique_violation(&error), "{error:?}");
        tx.rollback().await.unwrap();

        // Subquery dựng từ handle của khách B (hay handle "mọi khách") không dùng được trong query của khách A.
        let read_a = a.db().read();
        let read_b = b.db().read();
        let of_b = read_b.sub(ProductAttr::PRODUCT_ID);
        let error =
            read_a.find::<Entry>().filter(Entry::PRODUCT_ID.in_sub(of_b)).all().await.unwrap_err();
        assert!(error.to_string().contains("another handle"), "{error:?}");
        let everyone = all_tenants_for_tests(&database);
        let all_reads = everyone.read();
        let of_all = all_reads.sub(ProductAttr::PRODUCT_ID);
        assert!(
            read_a.find::<Entry>().filter(Entry::PRODUCT_ID.in_sub(of_all)).count().await.is_err()
        );
        let tx = a.db().begin_write().await.unwrap();
        let of_b = read_b.sub(ProductAttr::PRODUCT_ID);
        assert!(tx.delete::<Entry>().filter(Entry::PRODUCT_ID.in_sub(of_b)).exec().await.is_err());
        tx.rollback().await.unwrap();

        // `contains` không phân biệt hoa thường, giống nhau trên hai database.
        assert_eq!(
            read_a
                .find::<Translation>()
                .filter(Translation::SEARCH_KEY.contains("AO THUN"))
                .count()
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            read_a.find::<Level>().filter(Level::CODE.starts_with("l-1")).count().await.unwrap(),
            1
        );
    })
    .await
    .unwrap();
}

/// Record nhẹ khai `global` trên bảng của khách (`kernel_t_level` có `tenant_id`, không phải bảng control-plane).
#[derive(Record, Debug, Clone)]
#[record(table = "kernel_t_level", scope = global)]
struct LevelAsGlobal {
    id: Uuid,
    code: String,
}

#[tokio::test]
async fn record_global_tren_bang_cua_khach_bi_tu_choi_luc_chay() {
    TestDatabase::run(async |test| {
        let database = open(test).await;
        let (a, b) = (tenant(&database, 0xA), tenant(&database, 0xB));
        seed(&a, &b).await;
        // Mọi đường đều lỗi, dù lint có bỏ sót record này: đọc, đếm, subquery, sửa, xoá.
        let read = a.db().read();
        assert!(read.find::<LevelAsGlobal>().all().await.is_err());
        assert!(read.find::<LevelAsGlobal>().count().await.is_err());
        let sub = read.sub(LevelAsGlobal::ID);
        assert!(read.find::<Level>().filter(Level::ID.in_sub(sub)).all().await.is_err());
        let everyone = all_tenants_for_tests(&database);
        assert!(everyone.read().find::<LevelAsGlobal>().all().await.is_err());
        let tx = a.db().begin_write().await.unwrap();
        assert!(
            tx.update::<LevelAsGlobal>().set(LevelAsGlobal::CODE, "x".into()).exec().await.is_err()
        );
        assert!(tx.delete::<LevelAsGlobal>().exec().await.is_err());
        tx.rollback().await.unwrap();
        // Dữ liệu không đổi.
        assert_eq!(read.find::<Level>().count().await.unwrap(), 1);
    })
    .await
    .unwrap();
}

/// `Find::within` / `Grouped::within`: câu của bộ dựng câu chạy dưới trần thời gian cho cùng kết quả như không có trần
/// (mọi kiểu giá trị bind, cả `LIMIT`), vẫn lọc theo khách; trên transaction ghi thì là lỗi, không âm thầm bỏ qua.
#[tokio::test]
async fn doc_co_tran_tren_bo_dung_cau() {
    TestDatabase::run(async |test| {
        let database = open(test).await;
        let (a, b) = (tenant(&database, 0xA), tenant(&database, 0xB));
        seed(&a, &b).await;
        let read = a.db().read();
        let limit = std::time::Duration::from_secs(5);
        let query = || {
            read.find::<Line>()
                .filter(Line::AMOUNT_MINOR.gte(0))
                .filter(Line::EFFECTIVE_DATE.gte(day("2000-01-01")))
                .order_desc(Line::ID)
                .limit(2)
        };
        let plain: Vec<Uuid> = query().all().await.unwrap().iter().map(|line| line.id).collect();
        let bounded: Vec<Uuid> =
            query().within(limit).all().await.unwrap().iter().map(|line| line.id).collect();
        assert_eq!(bounded, plain);
        assert_eq!(bounded, vec![id(0x1012), id(0x1011)], "tenant A only");
        let ids: Vec<(Uuid,)> = query().within(limit).select((Line::ID,)).all().await.unwrap();
        assert_eq!(ids, vec![(id(0x1012),), (id(0x1011),)]);

        // `page` cũng đi qua trần (cả câu đếm lẫn câu lấy dòng), không bỏ qua lặng lẽ.
        let request = PageRequest::first(2, Order::Desc).counted();
        let page = read.find::<Line>().within(limit).page(Line::ID, &request).await.unwrap();
        assert_eq!(page.data.iter().map(|line| line.id).collect::<Vec<_>>(), plain);

        // Trên transaction ghi, `within` là lỗi ở mọi cách đọc — bằng chứng cách đọc đó đi qua trần.
        let tx = a.db().begin_write().await.unwrap();
        assert!(tx.find::<Line>().within(limit).all().await.is_err());
        assert!(tx.find::<Line>().within(limit).count().await.is_err());
        let uncounted = PageRequest::first(2, Order::Desc);
        let rows = tx.find::<Line>().within(limit).page(Line::ID, &uncounted);
        assert!(rows.await.is_err(), "page honours within");
        let counted = tx.find::<Line>().within(limit).page(Line::ID, &request);
        assert!(counted.await.is_err(), "the page count honours within");
        tx.rollback().await.unwrap();
    })
    .await
    .unwrap();
}

// Trang theo một cột khác id (`page_by`): thứ tự (cột, id), giá trị trùng không lặp không sót qua các trang, cả hai
// chiều; con trỏ của cột khác hay giá trị sai kiểu là 422 trên `cursor`; dòng của khách khác không lộ.
#[tokio::test]
async fn trang_theo_cot_tu_nhien() {
    use autoxide_kernel::pagination::NaturalOrderPageQuery;
    TestDatabase::run(async |test| {
        let database = open(test).await;
        let (a, b) = (tenant(&database, 0xA), tenant(&database, 0xB));
        let stock = [(0x3001, 3), (0x3002, 1), (0x3003, 3), (0x3004, 2), (0x3005, 3)];
        let tx = a.db().begin_write().await.unwrap();
        for (n, on_hand) in stock {
            let row = Level {
                id: id(n),
                tenant_id: TenantKey::new(a.id()),
                code: format!("P-{n:x}"),
                on_hand,
            };
            tx.insert(row).exec().await.unwrap();
        }
        tx.commit().await.unwrap();
        let tx = b.db().begin_write().await.unwrap();
        let row = Level {
            id: id(0x3999),
            tenant_id: TenantKey::new(b.id()),
            code: "P-B".into(),
            on_hand: 0,
        };
        tx.insert(row).exec().await.unwrap();
        tx.commit().await.unwrap();

        let read = a.db().read();
        let walk = async |order: &str| {
            let mut seen = Vec::new();
            let mut cursor = None;
            loop {
                let query =
                    NaturalOrderPageQuery::new(Some("2".into()), cursor, Some(order.into()));
                let request = PageRequest::try_from(query).unwrap();
                let page = read
                    .find::<Level>()
                    .page_by(Level::ON_HAND, |row| row.on_hand, Level::ID, &request)
                    .await
                    .unwrap();
                assert!(page.data.len() <= 2);
                seen.extend(page.data.iter().map(|row| row.id));
                match page.pagination.next_cursor {
                    Some(next) => cursor = Some(next),
                    None => return seen,
                }
            }
        };
        let ascending = vec![id(0x3002), id(0x3004), id(0x3001), id(0x3003), id(0x3005)];
        assert_eq!(walk("asc").await, ascending);
        let mut descending = ascending.clone();
        descending.reverse();
        assert_eq!(walk("desc").await, descending);

        // Có tổng ở trang đầu như `page`.
        let counted = PageRequest::try_from(
            NaturalOrderPageQuery::new(Some("2".into()), None, None).with_count(true),
        )
        .unwrap();
        let first = read
            .find::<Level>()
            .page_by(Level::CODE, |row| row.code.clone(), Level::ID, &counted)
            .await
            .unwrap();
        assert_eq!(first.pagination.total, Some(5));
        let by_code = first.pagination.next_cursor.unwrap();

        // Con trỏ của danh sách theo `code` dùng cho danh sách theo `on_hand`: 422 trên `cursor`.
        let wrong =
            PageRequest::try_from(NaturalOrderPageQuery::new(None, Some(by_code), None)).unwrap();
        let error = read
            .find::<Level>()
            .page_by(Level::ON_HAND, |row| row.on_hand, Level::ID, &wrong)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "validation_failed");
        // Con trỏ theo id (UUID) cho danh sách theo cột: cũng 422.
        let id_cursor = PageRequest::first(2, Order::Asc).after(id(0x3001));
        let error = read
            .find::<Level>()
            .page_by(Level::ON_HAND, |row| row.on_hand, Level::ID, &id_cursor)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "validation_failed");
        // Đúng tên cột nhưng giá trị là chữ (con trỏ bị sửa): 422, không tới database.
        let tampered = autoxide_kernel::pagination::text_key_cursor("on_hand", "x");
        let tampered =
            PageRequest::try_from(NaturalOrderPageQuery::new(None, Some(tampered), None)).unwrap();
        let error = read
            .find::<Level>()
            .page_by(Level::ON_HAND, |row| row.on_hand, Level::ID, &tampered)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "validation_failed");
    })
    .await
    .unwrap();
}
