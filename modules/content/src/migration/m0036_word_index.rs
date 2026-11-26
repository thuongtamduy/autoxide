//! Chỉ mục từ của bản công khai (PT-15, D-20261126-content-word-index):
//!
//! - `content_published.search_words`: các từ của `search_key` do Rust tách (`publishing::words::words`); dòng cũ
//!   được tính lại ngay trong migration.
//! - `content_published.search_rowid`: khoá số nguyên của dòng trong bảng FTS5 ở SQLite (trigger gán). Có trên cả hai
//!   database (Postgres để `NULL`) để schema hai bên giống nhau khi `autoxide copy`.
//! - Postgres: index GIN trên `to_tsvector('simple', search_words)`.
//! - SQLite: bảng ảo `content_published_fts` (FTS5) và ba trigger giữ nó đồng bộ với `content_published`.

use autoxide_kernel::migration::prelude::*;
use sea_orm::{DbBackend, Statement};

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0036_word_index"
    }
}

const TABLE: &str = "content_published";
/// Số dòng mỗi lượt tính lại `search_words`.
const BATCH: u64 = 500;

async fn execute(manager: &SchemaManager<'_>, sql: &str) -> Result<(), DbErr> {
    let connection = manager.get_connection();
    let backend = connection.get_database_backend();
    connection.execute_raw(Statement::from_string(backend, sql.to_owned())).await?;
    Ok(())
}

/// Ba trigger của SQLite. `search_rowid` của dòng mới = lớn nhất + 1 (index `content_published_search_rowid_idx`
/// trả lời `MAX` ngay), kể cả khi dòng được chép kèm giá trị cũ (`autoxide copy`): giá trị luôn do trigger đặt.
const SQLITE_TRIGGERS: [&str; 3] = [
    "CREATE TRIGGER content_published_fts_insert AFTER INSERT ON content_published BEGIN \
     UPDATE content_published SET search_rowid = \
     (SELECT COALESCE(MAX(search_rowid), 0) + 1 FROM content_published) WHERE rowid = new.rowid; \
     INSERT INTO content_published_fts(rowid, search_words) \
     SELECT search_rowid, search_words FROM content_published WHERE rowid = new.rowid; \
     END",
    "CREATE TRIGGER content_published_fts_delete AFTER DELETE ON content_published BEGIN \
     DELETE FROM content_published_fts WHERE rowid = old.search_rowid; \
     END",
    "CREATE TRIGGER content_published_fts_update AFTER UPDATE OF search_words ON content_published BEGIN \
     DELETE FROM content_published_fts WHERE rowid = old.search_rowid; \
     INSERT INTO content_published_fts(rowid, search_words) VALUES (new.search_rowid, new.search_words); \
     END",
];

/// Tính `search_words` cho mọi dòng có sẵn, từng lượt [`BATCH`] dòng theo khoá chính.
async fn backfill(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let connection = manager.get_connection();
    let mut after: Option<(uuid::Uuid, uuid::Uuid)> = None;
    loop {
        let mut select = Query::select();
        select
            .columns(["tenant_id", "entry_id", "search_key"])
            .from(TABLE)
            .order_by("tenant_id", Order::Asc)
            .order_by("entry_id", Order::Asc)
            .limit(BATCH);
        if let Some((tenant, entry)) = after {
            select.cond_where(
                Cond::any().add(Expr::col("tenant_id").gt(tenant)).add(
                    Cond::all()
                        .add(Expr::col("tenant_id").eq(tenant))
                        .add(Expr::col("entry_id").gt(entry)),
                ),
            );
        }
        let rows = connection.query_all(&select).await?;
        if rows.is_empty() {
            return Ok(());
        }
        for row in &rows {
            let tenant: uuid::Uuid = row.try_get("", "tenant_id")?;
            let entry: uuid::Uuid = row.try_get("", "entry_id")?;
            let key: String = row.try_get("", "search_key")?;
            let update = Query::update()
                .table(TABLE)
                .value("search_words", crate::publishing::words::words(&key))
                .and_where(Expr::col("tenant_id").eq(tenant))
                .and_where(Expr::col("entry_id").eq(entry))
                .to_owned();
            connection.execute(&update).await?;
            after = Some((tenant, entry));
        }
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(TABLE)
                    .add_column(schema::text("search_words").default(""))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(TABLE)
                    .add_column(schema::big_integer_null("search_rowid"))
                    .to_owned(),
            )
            .await?;
        backfill(manager).await?;
        match manager.get_database_backend() {
            DbBackend::Postgres => {
                execute(
                    manager,
                    "CREATE INDEX content_published_words_idx ON content_published \
                     USING GIN (to_tsvector('simple'::regconfig, search_words))",
                )
                .await?;
            }
            _ => {
                execute(manager, "UPDATE content_published SET search_rowid = rowid").await?;
            }
        }
        manager
            .create_index(
                Index::create()
                    .name("content_published_search_rowid_idx")
                    .table(TABLE)
                    .col("search_rowid")
                    .to_owned(),
            )
            .await?;
        if manager.get_database_backend() != DbBackend::Postgres {
            // `remove_diacritics 0`: dấu đã bỏ ở Rust; bộ tách chỉ còn cắt ở dấu cách.
            execute(
                manager,
                "CREATE VIRTUAL TABLE content_published_fts USING fts5(search_words, \
                 tokenize = 'unicode61 remove_diacritics 0')",
            )
            .await?;
            execute(
                manager,
                "INSERT INTO content_published_fts(rowid, search_words) \
                 SELECT search_rowid, search_words FROM content_published",
            )
            .await?;
            for trigger in SQLITE_TRIGGERS {
                execute(manager, trigger).await?;
            }
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager.get_database_backend() == DbBackend::Postgres {
            execute(manager, "DROP INDEX content_published_words_idx").await?;
        } else {
            for trigger in [
                "content_published_fts_insert",
                "content_published_fts_delete",
                "content_published_fts_update",
            ] {
                execute(manager, &format!("DROP TRIGGER {trigger}")).await?;
            }
            execute(manager, "DROP TABLE content_published_fts").await?;
        }
        manager
            .drop_index(
                Index::drop().name("content_published_search_rowid_idx").table(TABLE).to_owned(),
            )
            .await?;
        for column in ["search_rowid", "search_words"] {
            manager.alter_table(Table::alter().table(TABLE).drop_column(column).to_owned()).await?;
        }
        Ok(())
    }
}
