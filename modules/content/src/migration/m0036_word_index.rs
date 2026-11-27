//! Chỉ mục từ của bản công khai (PT-15, D-20261126-content-word-index): bảng thường `content_published_words` —
//! mỗi từ của một bài đã đăng một dòng `(tenant_id, entry_id, word, locale)` — giống nhau trên hai database. Dòng của
//! bài đã đăng từ trước được tính ngay trong migration, theo lô.

use autoxide_kernel::migration::prelude::*;
use sea_orm::DbBackend;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0036_word_index"
    }
}

const TABLE: &str = "content_published_words";
const PUBLISHED: &str = "content_published";
/// Số bài mỗi lượt tính từ.
const BATCH: u64 = 200;

/// Cột `word`: so sánh theo byte trên cả hai database (Postgres `COLLATE "C"`; SQLite mặc định `BINARY`), để khoảng
/// tiền tố `word >= w AND word < w_next` khớp đúng thứ tự điểm mã Unicode.
fn word(backend: DbBackend) -> ColumnDef {
    let mut column = schema::string_len("word", 64);
    if backend == DbBackend::Postgres {
        column.extra("COLLATE \"C\"");
    }
    column
}

/// Tính từ cho mọi bài đã đăng, từng lượt [`BATCH`] bài theo khoá chính.
async fn backfill(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let connection = manager.get_connection();
    let mut after: Option<(uuid::Uuid, uuid::Uuid)> = None;
    loop {
        let mut select = Query::select();
        select
            .columns(["tenant_id", "entry_id", "locale", "search_key"])
            .from(PUBLISHED)
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
        let mut insert = Query::insert();
        insert.into_table(TABLE).columns(["tenant_id", "entry_id", "word", "locale"]);
        let mut any = false;
        for row in &rows {
            let tenant: uuid::Uuid = row.try_get("", "tenant_id")?;
            let entry: uuid::Uuid = row.try_get("", "entry_id")?;
            let locale: String = row.try_get("", "locale")?;
            let key: String = row.try_get("", "search_key")?;
            for word in crate::publishing::words::words(&key) {
                insert.values_panic([
                    tenant.into(),
                    entry.into(),
                    word.into(),
                    locale.clone().into(),
                ]);
                any = true;
            }
            after = Some((tenant, entry));
        }
        if any {
            connection.execute(&insert).await?;
        }
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        manager
            .create_table(
                Table::create()
                    .table(TABLE)
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("entry_id"))
                    .col(word(backend))
                    .col(schema::string_len("locale", 35))
                    .primary_key(Index::create().col("tenant_id").col("entry_id").col("word"))
                    .to_owned(),
            )
            .await?;
        backfill(manager).await?;
        // Tìm: khách, ngôn ngữ, rồi khoảng từ; `entry_id` ở cuối để câu chỉ đọc index.
        manager
            .create_index(
                Index::create()
                    .name("content_published_words_word_idx")
                    .table(TABLE)
                    .col("tenant_id")
                    .col("locale")
                    .col("word")
                    .col("entry_id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(TABLE).to_owned()).await
    }
}
