//! Khoá ký của kernel: sinh một lần và lưu database khi không cấu hình — mọi máy chủ (kể cả
//! khởi động cùng lúc) dùng cùng một khoá; khoá theo mục đích khác nhau; cấu hình thì dùng
//! khoá cấu hình. Chạy trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
use autoxide_kernel::database::Database;
use autoxide_kernel::module::Modules;
use autoxide_kernel::secret::SecretKey;
use autoxide_testkit::TestDatabase;

/// Database đã migrate, cùng khoá mà module `kernel` công bố với cấu hình `pairs`.
async fn start(test: &TestDatabase, pairs: &[(&str, &str)]) -> (Database, SecretKey) {
    let mut all = vec![("DATABASE_URL", test.url())];
    all.extend_from_slice(pairs);
    let source = ConfigSource::from_pairs(all);
    let mut reader = ConfigReader::new(&source);
    let config = Config::read(&mut reader);
    let mut modules = Modules::new(Vec::new()).unwrap();
    let configured = modules.configure(&mut reader);
    let (config, _) = reader.finish(config.and_then(|config| configured.map(|_| config))).unwrap();
    let database = Database::connect(&config.database).await.unwrap();
    modules.migrator().unwrap().up(&database).await.unwrap();
    modules.start(&database).unwrap();
    let key = modules.api::<SecretKey>().unwrap();
    (database, (*key).clone())
}

#[tokio::test]
async fn khoa_sinh_mot_lan_dung_chung_moi_may_chu() {
    TestDatabase::run(async |test| {
        let (database, _) = start(test, &[]).await;
        // Tám "máy chủ" cùng khởi động lần đầu trên một database.
        let servers: Vec<SecretKey> =
            (0..8).map(|_| SecretKey::new(database.clone(), None)).collect();
        let keys =
            futures_util::future::join_all(servers.iter().map(|key| key.derive("test/a"))).await;
        let first = keys[0].as_ref().unwrap();
        assert!(keys.iter().all(|key| key.as_ref().unwrap() == first), "one stored key for all");
        // Khởi động lại: vẫn khoá đó.
        let again = SecretKey::new(database.clone(), None).derive("test/a").await.unwrap();
        assert_eq!(&again, first);
        // Mục đích khác: khoá khác.
        assert_ne!(&servers[0].derive("test/b").await.unwrap(), first);
        let rows = database.read();
        use sea_orm::ConnectionTrait as _;
        let backend = rows.get_database_backend();
        let count = rows
            .query_one_raw(sea_orm::Statement::from_string(
                backend,
                "SELECT COUNT(*) AS n FROM kernel_secrets",
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get_by_index::<i64>(0)
            .unwrap();
        assert_eq!(count, 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khoa_cau_hinh_thang_khoa_luu() {
    TestDatabase::run(async |test| {
        let configured = "a-configured-secret-that-is-long-enough-0123";
        let (database, key) = start(test, &[("AUTOXIDE_SECRET_KEY", configured)]).await;
        assert!(key.is_configured());
        let stored = SecretKey::new(database.clone(), None).derive("test/a").await.unwrap();
        let from_config = key.derive("test/a").await.unwrap();
        assert_ne!(from_config, stored);
        assert_eq!(
            SecretKey::new(database, Some(configured.as_bytes().to_vec()))
                .derive("test/a")
                .await
                .unwrap(),
            from_config
        );
    })
    .await
    .unwrap();
}

#[test]
fn khoa_ngan_bi_tu_choi_va_khong_bi_in_ra() {
    let source = ConfigSource::from_pairs([("AUTOXIDE_SECRET_KEY", "too-short-s3cr3t")]);
    let mut reader = ConfigReader::new(&source);
    let mut modules = Modules::new(Vec::new()).unwrap();
    let configured = modules.configure(&mut reader);
    let error = reader.finish(configured).unwrap_err().to_string();
    assert!(error.contains("AUTOXIDE_SECRET_KEY: must be at least 32"), "{error}");
    assert!(!error.contains("s3cr3t"), "{error}");
}

#[test]
fn ten_kernel_danh_rieng() {
    struct Fake;
    impl autoxide_kernel::module::Module for Fake {
        fn name(&self) -> &'static str {
            "kernel"
        }
    }
    let error = Modules::new(vec![Box::new(Fake)]).unwrap_err();
    assert!(error.message().contains("reserved"), "{}", error.message());
}

/// Băm IP: `bucket` đổi theo ngày, `fingerprint` cố định; cả hai khác nhau và không chứa IP.
#[tokio::test]
async fn bam_ip_theo_ngay_va_co_dinh() {
    use autoxide_kernel::ip_hash::{bucket, fingerprint};
    use chrono::{Duration, Utc};
    TestDatabase::run(async |test| {
        let (_, key) = start(test, &[]).await;
        let address: std::net::IpAddr = "203.0.113.9".parse().unwrap();
        let today = Utc::now();
        let a = bucket(&key, "forms:contact", address, today).await.unwrap();
        assert_eq!(
            a,
            bucket(&key, "forms:contact", "203.0.113.200".parse().unwrap(), today).await.unwrap()
        );
        assert_ne!(
            a,
            bucket(&key, "forms:contact", address, today + Duration::days(1)).await.unwrap()
        );
        let fixed = fingerprint(&key, "forms", address).await.unwrap();
        assert_ne!(fixed, a);
        assert_eq!(fixed, fingerprint(&key, "forms", address).await.unwrap());
        // Bản băm là chuỗi hex nên có thể tình cờ chứa "203"; điều cần kiểm là không lộ nguyên địa chỉ.
        assert!(!fixed.contains(&address.to_string()));
    })
    .await
    .unwrap();
}

/// Dấu IP (D-153): mỗi phạm vi một khoá — cùng người, hai module, hai dấu không nối được.
#[tokio::test]
async fn dau_ip_rieng_theo_pham_vi() {
    use autoxide_kernel::ip_hash::{bucket, fingerprint};
    TestDatabase::run(async |test| {
        let (_, key) = start(test, &[]).await;
        assert!(!key.is_configured());
        let visitor: std::net::IpAddr = "203.0.113.9".parse().unwrap();
        let forms = fingerprint(&key, "forms", visitor).await.unwrap();
        assert_ne!(forms, fingerprint(&key, "comments", visitor).await.unwrap());
        // Cùng phạm vi, cùng khối /24: cùng dấu.
        let neighbour = "203.0.113.200".parse().unwrap();
        assert_eq!(forms, fingerprint(&key, "forms", neighbour).await.unwrap());
        let now = chrono::Utc::now();
        let today = bucket(&key, "forms:contact", visitor, now).await.unwrap();
        assert_ne!(today, bucket(&key, "comments:post", visitor, now).await.unwrap());
        let tomorrow = now + chrono::Duration::days(1);
        assert_ne!(today, bucket(&key, "forms:contact", visitor, tomorrow).await.unwrap());
    })
    .await
    .unwrap();
}
