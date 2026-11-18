//! Mã hoá secret khi lưu (bước 33 A6): xoay khoá, khoá gốc trong database vẫn mở được sau khi đặt
//! `AUTOXIDE_SECRET_KEY`, upgrade dữ liệu chạy một lần và còn chờ thì chương trình không phục vụ.
//! Chạy trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::migration::{DataUpgrade, UpgradeContext};
use autoxide_kernel::module::{Module, Modules};
use autoxide_kernel::secret::{SecretKey, is_sealed};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, TenantId};
use autoxide_testkit::TestDatabase;
use sea_orm::{ConnectionTrait, Statement};

const KEY_A: &str = "first-key-first-key-first-key-first-key";
const KEY_B: &str = "second-key-second-key-second-key-second";

async fn database(test: &TestDatabase) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    // Bảng `kernel_secrets` (khoá gốc giữ trong database).
    Modules::new(Vec::new()).unwrap().migrator().unwrap().up(&database).await.unwrap();
    database
}

#[tokio::test]
async fn seal_open_va_xoay_khoa() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let old = SecretKey::new(database.clone(), Some(KEY_A.into())).sealer().await.unwrap();
        let sealed = old.seal(DEFAULT_TENANT, "demo.secret", "row-1", b"s3cret").unwrap();
        let rotated = SecretKey::new(database.clone(), Some(KEY_B.into()))
            .with_previous(vec![KEY_A.into()])
            .sealer()
            .await
            .unwrap();
        assert_ne!(old.current_key_id(), rotated.current_key_id());
        let opened = rotated.open(DEFAULT_TENANT, "demo.secret", "row-1", &sealed).unwrap();
        assert_eq!(opened.as_slice(), b"s3cret");
        assert!(rotated.needs_reseal(&sealed));
        let resealed = rotated.seal(DEFAULT_TENANT, "demo.secret", "row-1", &opened).unwrap();
        assert!(!rotated.needs_reseal(&resealed));
        // Khoá cũ đã bỏ: mã hoá lại rồi thì vẫn đọc được bằng khoá mới.
        let only_new = SecretKey::new(database.clone(), Some(KEY_B.into())).sealer().await.unwrap();
        assert!(only_new.open(DEFAULT_TENANT, "demo.secret", "row-1", &sealed).is_err());
        assert!(only_new.open(DEFAULT_TENANT, "demo.secret", "row-1", &resealed).is_ok());
        // Khác khách, khác dòng: không mở được.
        let other = TenantId::from_uuid(uuid::Uuid::now_v7());
        assert!(rotated.open(other, "demo.secret", "row-1", &resealed).is_err());
        assert!(rotated.open(DEFAULT_TENANT, "demo.secret", "row-2", &resealed).is_err());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khoa_trong_database_van_mo_duoc_sau_khi_dat_bien_moi_truong() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let stored = SecretKey::new(database.clone(), None).sealer().await.unwrap();
        let sealed = stored.seal(DEFAULT_TENANT, "demo.secret", "row", b"before").unwrap();
        let configured =
            SecretKey::new(database.clone(), Some(KEY_A.into())).sealer().await.unwrap();
        assert_ne!(stored.current_key_id(), configured.current_key_id());
        let opened = configured.open(DEFAULT_TENANT, "demo.secret", "row", &sealed).unwrap();
        assert_eq!(opened.as_slice(), b"before");
        assert!(configured.needs_reseal(&sealed));
    })
    .await
    .unwrap();
}

/// Bảng `vault_items` giữ secret dạng chữ thường (bản cũ); upgrade `u0001_seal_values` mã hoá.
struct CreateVault;

impl MigrationName for CreateVault {
    fn name(&self) -> &str {
        "m0001_create_vault"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateVault {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("vault_items")
                    .col(schema::string_len("id", 20).primary_key())
                    .col(schema::text("value"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("vault_items").to_owned()).await
    }
}

struct SealValues(Arc<AtomicUsize>);

async fn rows(database: &Database) -> Vec<(String, String)> {
    let backend = database.read().get_database_backend();
    let select = Statement::from_string(backend, "SELECT id, value FROM vault_items ORDER BY id");
    let rows = database.read().query_all_raw(select).await.unwrap();
    rows.iter()
        .map(|row| (row.try_get_by_index(0).unwrap(), row.try_get_by_index(1).unwrap()))
        .collect()
}

async fn set(database: &Database, id: &str, value: &str) -> Result<(), Error> {
    let backend = database.write().get_database_backend();
    let sql = match backend {
        sea_orm::DbBackend::Postgres => "UPDATE vault_items SET value = $1 WHERE id = $2",
        _ => "UPDATE vault_items SET value = ? WHERE id = ?",
    };
    let statement = Statement::from_sql_and_values(backend, sql, [value.into(), id.into()]);
    database.write().execute_raw(statement).await?;
    Ok(())
}

#[async_trait::async_trait]
impl DataUpgrade for SealValues {
    fn name(&self) -> &'static str {
        "u0001_seal_values"
    }

    fn after(&self) -> &'static str {
        "m0001_create_vault"
    }

    async fn up(&self, context: &UpgradeContext) -> Result<(), Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let sealer = context.secret().sealer().await?;
        for (id, value) in rows(context.database()).await {
            if !is_sealed(&value) {
                let sealed = sealer.seal(DEFAULT_TENANT, "vault.value", &id, value.as_bytes())?;
                set(context.database(), &id, &sealed).await?;
            }
        }
        Ok(())
    }

    async fn down(&self, context: &UpgradeContext) -> Result<(), Error> {
        let sealer = context.secret().sealer().await?;
        for (id, value) in rows(context.database()).await {
            let plain = sealer.open(DEFAULT_TENANT, "vault.value", &id, &value)?;
            set(context.database(), &id, &String::from_utf8_lossy(&plain)).await?;
        }
        Ok(())
    }
}

struct Vault {
    upgraded: bool,
    runs: Arc<AtomicUsize>,
}

impl Module for Vault {
    fn name(&self) -> &'static str {
        "vault"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateVault)]
    }

    fn upgrades(&self) -> Vec<Box<dyn DataUpgrade>> {
        if self.upgraded { vec![Box::new(SealValues(self.runs.clone()))] } else { Vec::new() }
    }
}

#[tokio::test]
async fn upgrade_du_lieu_chay_mot_lan_va_serve_tu_choi_khi_con_cho() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let runs = Arc::new(AtomicUsize::new(0));
        // Bản cũ: chưa có upgrade; secret lưu chữ thường.
        let old = Modules::new(vec![Box::new(Vault { upgraded: false, runs: runs.clone() })]);
        old.unwrap().migrator().unwrap().up(&database).await.unwrap();
        let backend = database.write().get_database_backend();
        let insert = "INSERT INTO vault_items (id, value) VALUES ('a', 'alpha'), ('b', 'beta')";
        database.write().execute_raw(Statement::from_string(backend, insert)).await.unwrap();

        // Bản mới: upgrade chờ — như migration chờ, chương trình không phục vụ (`serve` từ chối).
        let new = Modules::new(vec![Box::new(Vault { upgraded: true, runs: runs.clone() })]);
        let migrator = new.unwrap().migrator().unwrap();
        let pending = migrator.pending(&database).await.unwrap();
        let names: Vec<&str> = pending.iter().map(|state| state.name.as_str()).collect();
        assert_eq!(names, ["u0001_seal_values"]);

        migrator.up(&database).await.unwrap();
        migrator.up(&database).await.unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 1, "runs once");
        assert!(migrator.pending(&database).await.unwrap().is_empty());
        let sealer = SecretKey::new(database.clone(), None).sealer().await.unwrap();
        for (id, value) in rows(&database).await {
            assert!(is_sealed(&value), "{id} is sealed");
            let plain = sealer.open(DEFAULT_TENANT, "vault.value", &id, &value).unwrap();
            assert_eq!(plain.as_slice(), if id == "a" { b"alpha".as_slice() } else { b"beta" });
        }

        // Gỡ upgrade: dữ liệu về chữ thường, bản cũ đọc được; lên lại được.
        migrator.down(&database, "vault", 1).await.unwrap();
        assert_eq!(
            rows(&database).await,
            [("a".to_owned(), "alpha".to_owned()), ("b".to_owned(), "beta".to_owned())]
        );
        migrator.up(&database).await.unwrap();
        assert!(rows(&database).await.iter().all(|(_, value)| is_sealed(value)));

        // Khai upgrade sau một migration không có: lỗi khai báo, báo ngay.
        struct Lost;
        #[async_trait::async_trait]
        impl DataUpgrade for Lost {
            fn name(&self) -> &'static str {
                "u0002_lost"
            }
            fn after(&self) -> &'static str {
                "m0009_missing"
            }
            async fn up(&self, _: &UpgradeContext) -> Result<(), Error> {
                Ok(())
            }
            async fn down(&self, _: &UpgradeContext) -> Result<(), Error> {
                Ok(())
            }
        }
        let error = autoxide_kernel::migration::Migrator::new(vec![
            autoxide_kernel::migration::ModuleMigrations::new("vault", vec![Box::new(CreateVault)])
                .with_upgrades(vec![Box::new(Lost)]),
        ])
        .unwrap_err();
        assert!(error.to_string().contains("m0009_missing"), "{error}");
    })
    .await
    .unwrap();
}
