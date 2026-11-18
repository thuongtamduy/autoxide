//! Lõi backup của kernel (bước 39, A1) trên database gate đang chọn: ghi bản backup vào nơi lưu
//! file, không chép chính các bản backup theo lịch (`kernel/backups/`).

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use autoxide_kernel::backup::{
    BACKUP_PREFIX, BackupManifest, DATABASE_FILE, MANIFEST, StorageSink, open_sqlite, write_backup,
};
use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::migration::{Migrator, ModuleMigrations};
use autoxide_kernel::storage::Storage;
use autoxide_testkit::{MemoryStorage, TestDatabase};
use futures_util::{StreamExt as _, TryStreamExt as _};
use sea_orm::sea_query::{Alias, Query};
use uuid::Uuid;

struct CreateItems;

impl MigrationName for CreateItems {
    fn name(&self) -> &str {
        "m0001_create_items"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateItems {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("shop_item")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("name", 100))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("shop_item").to_owned()).await
    }
}

fn migrator() -> Migrator {
    Migrator::new(vec![ModuleMigrations::new("shop", vec![Box::new(CreateItems)])]).unwrap()
}

async fn connect(url: &str) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", url)]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    Database::connect(&reader.finish(config).unwrap().0).await.unwrap()
}

async fn put(storage: &MemoryStorage, key: &str, text: &str) {
    let chunks = vec![Ok(bytes::Bytes::from(text.to_owned()))];
    storage.put(key, futures_util::stream::iter(chunks).boxed()).await.unwrap();
}

async fn read(storage: &MemoryStorage, key: &str) -> Vec<u8> {
    let stored = storage.get(key).await.unwrap().unwrap();
    let chunks: Vec<bytes::Bytes> = stored.content.try_collect().await.unwrap();
    chunks.concat()
}

#[tokio::test]
async fn backup_khong_chep_thu_muc_backup() {
    TestDatabase::run(async |test| {
        let migrator = migrator();
        let database = connect(test.url()).await;
        migrator.up(&database).await.unwrap();
        for name in ["chảo gang", "thớt gỗ"] {
            let insert = Query::insert()
                .into_table(Alias::new("shop_item"))
                .columns([Alias::new("id"), Alias::new("name")])
                .values_panic([Uuid::now_v7().into(), name.into()])
                .to_owned();
            sea_orm::ConnectionTrait::execute(database.write(), &insert).await.unwrap();
        }
        let storage = Arc::new(MemoryStorage::new());
        put(&storage, "media/a.jpg", "ảnh").await;
        // Một bản backup theo lịch có từ trước: không được chép vào bản mới.
        put(&storage, &format!("{BACKUP_PREFIX}main/old/{DATABASE_FILE}"), "cũ").await;

        let prefix = format!("{BACKUP_PREFIX}main/new/");
        let sink = StorageSink::new(storage.clone(), prefix.clone());
        let scratch = std::env::temp_dir().join(format!("autoxide-test-{}", Uuid::now_v7()));
        let manifest = write_backup(
            &database,
            &migrator,
            &["shop"],
            Some(storage.clone() as Arc<dyn Storage>),
            &sink,
            &scratch,
        )
        .await
        .unwrap();
        assert!(!scratch.exists(), "the scratch directory is removed");
        assert_eq!((manifest.files, manifest.file_bytes), (1, "ảnh".len() as u64));
        assert_eq!(manifest.rows(), 2);

        let mut keys = storage.keys().await.unwrap();
        keys.sort();
        let new: Vec<_> = keys.iter().filter(|key| key.starts_with(&prefix)).cloned().collect();
        assert_eq!(
            new,
            [
                format!("{prefix}{MANIFEST}"),
                format!("{prefix}{DATABASE_FILE}"),
                format!("{prefix}files/media/a.jpg"),
            ]
        );
        let written: BackupManifest =
            serde_json::from_slice(&read(&storage, &format!("{prefix}{MANIFEST}")).await).unwrap();
        assert_eq!(written, manifest);

        // File SQLite của bản backup mở được và có đủ dữ liệu.
        let file = std::env::temp_dir().join(format!("autoxide-test-{}.sqlite", Uuid::now_v7()));
        std::fs::write(&file, read(&storage, &format!("{prefix}{DATABASE_FILE}")).await).unwrap();
        let copy = open_sqlite(&file, &migrator).await.unwrap();
        let count = Query::select()
            .expr(sea_orm::sea_query::Func::count(sea_orm::sea_query::Expr::col(Alias::new("id"))))
            .from(Alias::new("shop_item"))
            .to_owned();
        let row = sea_orm::ConnectionTrait::query_one(copy.read(), &count).await.unwrap().unwrap();
        assert_eq!(row.try_get_by_index::<i64>(0).unwrap(), 2);
        copy.close().await.unwrap();
        let _ = std::fs::remove_file(&file);
        database.close().await.unwrap();
    })
    .await
    .unwrap();
}
