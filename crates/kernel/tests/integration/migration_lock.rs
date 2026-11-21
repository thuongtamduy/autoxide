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
        let error =
            MigrationLock::acquire(&config, Duration::from_millis(600)).await.unwrap_err();
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
