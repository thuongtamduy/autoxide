//! Tìm theo ý nghĩa (D-109): vector của từng đoạn (cột nhị phân — `BLOB` / `BYTEA`, như nhau trên
//! SQLite và Postgres; không dùng `pgvector` hay `sqlite-vec`), và số phiên bản theo (`scope`,
//! `locale`) để các máy chủ biết khi nào nạp lại.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0006_embeddings"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("content_embeddings")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("entry_id"))
                    .col(schema::string_len("scope", 16))
                    .col(schema::string_len("locale", 35))
                    .col(schema::integer("chunk_index"))
                    .col(schema::string_len("content_hash", 64))
                    .col(schema::string_len("model", 200))
                    .col(schema::integer("dimensions"))
                    .col(schema::binary("vector"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_embeddings_chunk_unique")
                    .table("content_embeddings")
                    .col("entry_id")
                    .col("scope")
                    .col("chunk_index")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_embeddings_scope_idx")
                    .table("content_embeddings")
                    .col("scope")
                    .col("locale")
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("content_embedding_state")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("scope", 16))
                    .col(schema::string_len("locale", 35))
                    .col(schema::big_integer("version"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_embedding_state_unique")
                    .table("content_embedding_state")
                    .col("scope")
                    .col("locale")
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["content_embedding_state", "content_embeddings"] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
