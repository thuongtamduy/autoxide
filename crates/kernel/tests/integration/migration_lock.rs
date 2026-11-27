//! Khoá migration (D-20261121-migrate-on-start) trên database đang chọn: gate chạy cả SQLite (khoá
//! file cạnh file database) và Postgres (advisory lock trên kết nối riêng).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::migration::MigrationLock;
use autoxide_testkit::TestDatabase;

fn config_for(url: &str) -> DatabaseConfig {
    let source = ConfigSource::from_pairs([("DATABASE_URL", url)]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    reader.finish(config).unwrap().0
}

#[tokio::test]
async fn chi_mot_ben_giu_khoa_ben_kia_cho_toi_khi_nha() {
    TestDatabase::run(async |database| {
        let config = config_for(database.url());
        let first = MigrationLock::acquire(&config, Duration::from_secs(5)).await.unwrap();

        // Bên thứ hai chờ: đang có người giữ thì không lấy được trong 600 ms.
        let started = Instant::now();
        let error = MigrationLock::acquire(&config, Duration::from_millis(600)).await.unwrap_err();
        assert!(started.elapsed() >= Duration::from_millis(600), "phải chờ đủ hạn");
        assert!(error.to_string().contains("another process has been applying migrations"));

        // Nhả trong lúc bên kia đang chờ: bên kia lấy được ngay sau đó.
        let config_second = config.clone();
        let waiting = tokio::spawn(async move {
            MigrationLock::acquire(&config_second, Duration::from_secs(10)).await
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!waiting.is_finished(), "chưa nhả thì bên kia còn chờ");
        first.release().await.unwrap();
        let second = waiting.await.unwrap().unwrap();
        second.release().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nha_bang_drop_cung_duoc() {
    TestDatabase::run(async |database| {
        let config = config_for(database.url());
        let first = MigrationLock::acquire(&config, Duration::from_secs(5)).await.unwrap();
        // Như tiến trình chết giữa chừng: không gọi `release`, kết nối / file bị đóng.
        drop(first);

        let deadline = Instant::now() + Duration::from_secs(10);
        let second = MigrationLock::acquire(&config, Duration::from_secs(10)).await.unwrap();
        assert!(Instant::now() < deadline);
        second.release().await.unwrap();
    })
    .await
    .unwrap();
}

/// Khoá của placement tách khỏi khoá chính (D-20261126-placement-migration-exclusive): lượt chạy nền giữ khoá
/// placement thì máy chủ khởi động (chỉ cần khoá chính) không phải chờ; hai bên khoá placement vẫn loại trừ nhau.
#[tokio::test]
async fn khoa_placement_tach_khoi_khoa_chinh() {
    TestDatabase::run(async |database| {
        let config = config_for(database.url());
        let placements =
            MigrationLock::acquire_placements(&config, Duration::from_secs(5)).await.unwrap();
        let main = MigrationLock::acquire(&config, Duration::from_millis(500)).await.unwrap();
        MigrationLock::acquire_placements(&config, Duration::from_millis(300)).await.unwrap_err();
        main.release().await.unwrap();
        placements.release().await.unwrap();
    })
    .await
    .unwrap();
}

/// `verify`: còn giữ thì đạt; trên Postgres, kết nối giữ khoá bị cắt (proxy, failover) thì báo mất khoá để lượt dừng.
#[tokio::test]
async fn verify_bao_khi_mat_khoa() {
    TestDatabase::run(async |database| {
        let config = config_for(database.url());
        let mut lock =
            MigrationLock::acquire_placements(&config, Duration::from_secs(5)).await.unwrap();
        lock.verify().await.unwrap();
        if database.url().starts_with("postgres") {
            let other = autoxide_kernel::database::Database::connect(&config).await.unwrap();
            let write = other.write();
            let backend = sea_orm::ConnectionTrait::get_database_backend(write);
            let key = i64::from_be_bytes(*b"autoxplc");
            let sql = "SELECT pg_terminate_backend(pid) FROM pg_locks WHERE locktype = 'advisory' \
                       AND classid::bigint = $1 AND objid::bigint = $2 AND objsubid = 1 \
                       AND database = (SELECT oid FROM pg_database WHERE datname = current_database())";
            let statement = autoxide_kernel::database::statement(
                backend,
                sql,
                [i64::from((key >> 32) as u32).into(), i64::from(key as u32).into()],
            );
            sea_orm::ConnectionTrait::execute_raw(write, statement).await.unwrap();
            let error = lock.verify().await.unwrap_err();
            assert!(error.to_string().contains("migration lock was lost"), "{error}");
            other.close().await.unwrap();
        }
        let _ = lock.release().await;
    })
    .await
    .unwrap();
}
