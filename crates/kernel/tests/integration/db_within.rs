//! Đọc có trần thời gian (`Sql::within`): database dừng câu khi quá trần và khi người gọi thôi chờ; lỗi nhận ra
//! bằng mã của database (không bằng đồng hồ); mọi kết nối của pool dùng lại được ngay sau đó. Cả SQLite và Postgres.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use autoxide_kernel::config::DatabaseBackend;
use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::db::{Queries as _, all_tenants_for_tests, is_query_timeout};
use autoxide_kernel::tenancy::{MAIN_PLACEMENT, TenantContext, TenantId, TenantStatus};
use autoxide_testkit::TestDatabase;
use uuid::Uuid;

async fn open(test: &TestDatabase) -> (Database, TenantContext) {
    let source = ConfigSource::from_pairs(vec![("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    let tenant = TenantContext::new(
        TenantId::from_uuid(Uuid::from_u128(0xA)),
        "ta".to_owned(),
        "Tenant A".to_owned(),
        TenantStatus::Active,
        MAIN_PLACEMENT,
        database.clone(),
    );
    (database, tenant)
}

/// Câu chạy rất lâu nếu không bị dừng (hàng phút), có `:tenant` như mọi câu viết tay trên handle của khách.
fn slow(backend: DatabaseBackend, marker: &str) -> String {
    match backend {
        DatabaseBackend::Postgres => {
            format!("SELECT :tenant::uuid FROM pg_sleep(120) /* {marker} */")
        }
        _ => format!(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 4000000000) \
             SELECT :tenant FROM c WHERE x = 3999999999 /* {marker} */"
        ),
    }
}

/// Câu nhanh qua handle của khách: đúng khách, đúng kết quả.
async fn quick(tenant: &TenantContext) {
    let rows: Vec<(Uuid,)> = tenant
        .db()
        .read()
        .sql("SELECT :tenant")
        .within(Duration::from_secs(5))
        .all()
        .await
        .unwrap();
    assert_eq!(rows, vec![(tenant.id().as_uuid(),)]);
}

#[tokio::test]
async fn cau_qua_tran_bi_database_dung_va_pool_dung_lai_duoc() {
    TestDatabase::run(async |test| {
        let (database, tenant) = open(test).await;
        let backend = database.backend();
        quick(&tenant).await;

        let started = Instant::now();
        let error = tenant
            .db()
            .read()
            .sql(&slow(backend, "kernel-within-limit"))
            .within(Duration::from_millis(300))
            .all::<(Uuid,)>()
            .await
            .unwrap_err();
        assert!(is_query_timeout(&error), "{error:?}");
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());

        // Sau khi bị dừng: nhiều câu bình thường (đi qua mọi kết nối của pool) vẫn đúng, không câu nào bị dừng nhầm
        // (SQLite: handler đã gỡ; Postgres: `statement_timeout` chỉ trong transaction).
        for _ in 0..12 {
            quick(&tenant).await;
            let count: Vec<(i64,)> = tenant
                .db()
                .read()
                .sql("SELECT CAST(COUNT(*) AS BIGINT) FROM (SELECT :tenant) t")
                .all()
                .await
                .unwrap();
            assert_eq!(count, vec![(1,)]);
        }

        // Câu ghi không có trần: `within` trên transaction ghi là lỗi, không âm thầm bỏ qua.
        let tx = tenant.db().begin_write().await.unwrap();
        assert!(
            tx.sql("SELECT :tenant").within(Duration::from_secs(1)).all::<(Uuid,)>().await.is_err()
        );
        tx.rollback().await.unwrap();
    })
    .await
    .unwrap();
}

/// Người gọi thôi chờ (future bị drop, như request bị huỷ): câu dừng thật ở database, không chạy tiếp tới trần.
#[tokio::test]
async fn nguoi_goi_thoi_cho_thi_cau_dung_that() {
    TestDatabase::run(async |test| {
        let (database, tenant) = open(test).await;
        let backend = database.backend();
        quick(&tenant).await;
        let marker = "kernel-within-dropped";
        // Số kết nối đang mở của pool đọc SQLite trước khi chạy (câu dùng lại một kết nối rảnh).
        let sqlite_open = (backend == DatabaseBackend::Sqlite)
            .then(|| database.read().get_sqlite_connection_pool().size());
        let read = tenant.db().read();
        let running =
            read.sql(&slow(backend, marker)).within(Duration::from_secs(600)).all::<(Uuid,)>();
        assert!(tokio::time::timeout(Duration::from_millis(500), running).await.is_err());

        // Dưới 5 giây: sqlx bỏ chờ đóng kết nối sau 5 giây (`CLOSE_ON_DROP_TIMEOUT`) dù câu còn chạy, nên chỉ câu đã
        // dừng thật mới làm kết nối đóng trước trần này.
        let deadline = Instant::now() + Duration::from_secs(3);
        match backend {
            DatabaseBackend::Postgres => {
                // Không còn tiến trình nào chạy câu mang dấu (dấu ghép lúc chạy để câu đếm không tự khớp chính nó).
                let watch = all_tenants_for_tests(&database);
                loop {
                    let active: Vec<(i64,)> = watch
                        .read()
                        .sql(
                            "SELECT CAST(COUNT(*) AS BIGINT) FROM pg_stat_activity \
                             WHERE state = 'active' AND query LIKE '%' || :marker || '%' \
                             AND pid <> pg_backend_pid()",
                        )
                        .bind("marker", marker.to_owned())
                        .all()
                        .await
                        .unwrap();
                    if active == vec![(0,)] {
                        break;
                    }
                    assert!(Instant::now() < deadline, "the dropped query still runs");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
            _ => {
                // Kết nối của câu bị huỷ được đóng (không trả về pool); sqlx chỉ giảm số kết nối của pool khi đóng
                // xong — tức là khi SQLite đã dừng câu. Câu không dừng thì kết nối kẹt hàng phút.
                let pool = database.read().get_sqlite_connection_pool();
                let open_before = sqlite_open.unwrap();
                loop {
                    if pool.size() < open_before {
                        break;
                    }
                    assert!(Instant::now() < deadline, "the dropped query still runs");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
        for _ in 0..12 {
            quick(&tenant).await;
        }
    })
    .await
    .unwrap();
}

/// Pool đọc đầy (mọi kết nối đang bận): đọc có trần không chờ hết thời gian chờ của pool mà trả quá hạn đúng trần —
/// chờ kết nối cũng tính vào trần. Trả kết nối xong thì đọc lại được.
#[tokio::test]
async fn pool_day_thi_cho_ket_noi_tinh_vao_tran() {
    TestDatabase::run(async |test| {
        use sea_orm::TransactionTrait as _;
        let (database, tenant) = open(test).await;
        let size = database.read_pool_size();
        assert!(size >= 1);
        // Giữ mọi kết nối của pool đọc (mỗi transaction đọc giữ một kết nối).
        let mut held = Vec::new();
        for _ in 0..size {
            held.push(database.read().begin().await.unwrap());
        }
        let started = Instant::now();
        let refused = tenant
            .db()
            .read()
            .sql("SELECT :tenant")
            .within(Duration::from_millis(300))
            .all::<(Uuid,)>()
            .await
            .unwrap_err();
        assert!(is_query_timeout(&refused), "{refused:?}");
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        for transaction in held {
            transaction.rollback().await.unwrap();
        }
        quick(&tenant).await;
    })
    .await
    .unwrap();
}
