//! `autoxide copy` với chỉ mục tìm chữ của module (PT-15): bảng ảo FTS5 của SQLite và các bảng bóng của nó không được
//! chép (dữ liệu dẫn xuất); trigger của module dựng lại chỉ mục ở đích khi bảng gốc được chép. Chép SQLite → database
//! gate đang chọn → SQLite, rồi tìm được trên mọi đích, đúng số dòng, và `VACUUM` / kiểm toàn vẹn FTS5 vẫn qua.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::{Database, statement};
use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::migration::{Migrator, ModuleMigrations};
use autoxide_kernel::transfer::{CopiedTable, copy_modules};
use autoxide_testkit::TestDatabase;
use sea_orm::{ConnectionTrait, DatabaseBackend};
use uuid::Uuid;

struct CreateDocs;

impl MigrationName for CreateDocs {
    fn name(&self) -> &str {
        "m0001_create_docs"
    }
}

/// Như `m0036_word_index` của content: cột từ, khoá số của dòng trong FTS5, trigger giữ FTS5 đồng bộ (SQLite), index
/// GIN trên `tsvector` (Postgres).
const SQLITE: [&str; 4] = [
    "CREATE VIRTUAL TABLE note_fts USING fts5(words, tokenize = 'unicode61 remove_diacritics 0')",
    "CREATE TRIGGER note_fts_insert AFTER INSERT ON note_doc BEGIN \
     UPDATE note_doc SET search_rowid = (SELECT COALESCE(MAX(search_rowid), 0) + 1 FROM note_doc) \
     WHERE rowid = new.rowid; \
     INSERT INTO note_fts(rowid, words) SELECT search_rowid, words FROM note_doc WHERE rowid = new.rowid; \
     END",
    "CREATE TRIGGER note_fts_delete AFTER DELETE ON note_doc BEGIN \
     DELETE FROM note_fts WHERE rowid = old.search_rowid; END",
    "CREATE INDEX note_doc_search_rowid_idx ON note_doc (search_rowid)",
];

#[async_trait::async_trait]
impl MigrationTrait for CreateDocs {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("note_doc")
                    .col(schema::pk_uuid("id"))
                    .col(schema::text("words"))
                    .col(schema::big_integer_null("search_rowid"))
                    .to_owned(),
            )
            .await?;
        let connection = manager.get_connection();
        let backend = connection.get_database_backend();
        let sql: Vec<&str> = match backend {
            DatabaseBackend::Postgres => vec![
                "CREATE INDEX note_doc_words_idx ON note_doc \
                 USING GIN (to_tsvector('simple'::regconfig, words))",
            ],
            _ => SQLITE.to_vec(),
        };
        for sql in sql {
            connection.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager.get_database_backend() == DatabaseBackend::Sqlite {
            manager.get_connection().execute_unprepared("DROP TABLE note_fts").await?;
        }
        manager.drop_table(Table::drop().table("note_doc").to_owned()).await
    }
}

async fn connect(url: &str) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", url)]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    let migrator =
        Migrator::new(vec![ModuleMigrations::new("note", vec![Box::new(CreateDocs)])]).unwrap();
    migrator.up(&database).await.unwrap();
    database
}

async fn sqlite(directory: &std::path::Path, name: &str) -> Database {
    connect(&format!("sqlite://{}?mode=rwc", directory.join(name).display())).await
}

const DOCS: u32 = 300;

/// `DOCS` dòng; dòng thứ `n` có từ `w<n>`, dòng chia hết cho 10 có thêm từ `pho`.
async fn fill(database: &Database) {
    for n in 0..DOCS {
        let words = if n % 10 == 0 { format!("w{n} pho") } else { format!("w{n} bun") };
        let backend = database.write().get_database_backend();
        let sql = match backend {
            DatabaseBackend::Postgres => "INSERT INTO note_doc (id, words) VALUES ($1, $2)",
            _ => "INSERT INTO note_doc (id, words) VALUES (?, ?)",
        };
        database
            .write()
            .execute_raw(statement(
                backend,
                sql.to_owned(),
                vec![Uuid::now_v7().into(), words.into()],
            ))
            .await
            .unwrap();
    }
}

/// Số dòng khớp từ `word` qua chỉ mục của database.
async fn found(database: &Database, word: &str) -> i64 {
    let backend = database.read().get_database_backend();
    let sql = match backend {
        DatabaseBackend::Postgres => {
            "SELECT COUNT(*) FROM note_doc \
             WHERE to_tsvector('simple'::regconfig, words) @@ to_tsquery('simple'::regconfig, $1)"
        }
        _ => {
            "SELECT COUNT(*) FROM note_doc \
             WHERE search_rowid IN (SELECT rowid FROM note_fts WHERE note_fts MATCH ?)"
        }
    };
    let row = database
        .read()
        .query_one_raw(statement(backend, sql.to_owned(), vec![word.to_owned().into()]))
        .await
        .unwrap()
        .unwrap();
    row.try_get_by_index::<i64>(0).unwrap()
}

/// SQLite: `VACUUM` rồi lệnh kiểm toàn vẹn của FTS5 (lỗi nếu chỉ mục lệch bảng gốc).
async fn vacuum_and_check(database: &Database) {
    if database.backend() != autoxide_kernel::config::DatabaseBackend::Sqlite {
        return;
    }
    let write = database.write();
    write.execute_unprepared("VACUUM").await.unwrap();
    write
        .execute_unprepared("INSERT INTO note_fts(note_fts) VALUES ('integrity-check')")
        .await
        .unwrap();
}

#[tokio::test]
async fn chep_bang_co_chi_muc_tim_chu_va_tim_duoc_o_dich() {
    TestDatabase::run(async |test| {
        let directory =
            std::env::temp_dir().join(format!("autoxide-transfer-fts-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&directory).unwrap();
        let first = sqlite(&directory, "first.sqlite").await;
        fill(&first).await;
        let middle = connect(test.url()).await;
        let last = sqlite(&directory, "last.sqlite").await;

        // Chỉ bảng gốc được chép: không bảng ảo, không bảng bóng `note_fts_*`.
        let copied = copy_modules(&first, &middle, &["note"]).await.unwrap();
        assert_eq!(copied, [CopiedTable::new("note_doc", u64::from(DOCS))]);
        let copied = copy_modules(&middle, &last, &["note"]).await.unwrap();
        assert_eq!(copied, [CopiedTable::new("note_doc", u64::from(DOCS))]);

        for database in [&first, &middle, &last] {
            assert_eq!(found(database, "pho").await, i64::from(DOCS / 10));
            assert_eq!(found(database, "w7").await, 1);
            vacuum_and_check(database).await;
            assert_eq!(found(database, "bun").await, i64::from(DOCS - DOCS / 10));
        }

        // Chép lại vào đích đã có dữ liệu: chỉ mục không nhân đôi.
        copy_modules(&first, &last, &["note"]).await.unwrap();
        assert_eq!(found(&last, "pho").await, i64::from(DOCS / 10));
        vacuum_and_check(&last).await;

        for database in [first, middle, last] {
            database.close().await.unwrap();
        }
        std::fs::remove_dir_all(&directory).unwrap();
    })
    .await
    .unwrap();
}
