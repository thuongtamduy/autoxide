//! Chuỗi băm của audit log (D-183): cột `sequence`, `payload_hash`, `chain_hash` cho
//! `audit_entry`, bảng `audit_chain` giữ đầu chuỗi, và điền chuỗi cho các dòng có sẵn theo thứ tự
//! ID (UUIDv7 = thứ tự xảy ra).
//!
//! Migration này gọi cách băm ở `crate::chain` thay vì chép lại: cách băm `v1` là định dạng lưu
//! trữ, không bao giờ được sửa (bản mới là `v2` riêng), nên migration không đổi theo code.

use autoxide_kernel::migration::prelude::*;
use chrono::{DateTime, Utc};
use sea_orm::ConnectionTrait;
use uuid::Uuid;

use crate::chain::{self, Link};

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0004_add_hash_chain"
    }
}

const INDEX: &str = "audit_entry_sequence_idx";
/// Số dòng điền mỗi lượt đọc.
const BATCH: u64 = 500;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite chỉ thêm được một cột mỗi câu `ALTER TABLE`.
        for column in [
            schema::big_integer_null("sequence"),
            schema::string_len_null("payload_hash", 64),
            schema::string_len_null("chain_hash", 64),
        ] {
            manager
                .alter_table(
                    Table::alter().table("audit_entry").add_column(column.to_owned()).to_owned(),
                )
                .await?;
        }
        manager
            .create_table(
                Table::create()
                    .table("audit_chain")
                    .col(schema::string_len("name", 32).primary_key())
                    .col(schema::big_integer("sequence"))
                    .col(schema::string_len("hash", 64))
                    .to_owned(),
            )
            .await?;

        let connection = manager.get_connection();
        let (mut sequence, mut previous) = (0i64, String::new());
        let mut after: Option<Uuid> = None;
        loop {
            let mut select = Query::select()
                .columns([
                    "id",
                    "module",
                    "action",
                    "actor_user_id",
                    "actor_api_key_id",
                    "actor_kind",
                    "subject",
                    "before",
                    "after",
                    "details",
                    "occurred_at",
                    "recorded_at",
                ])
                .from("audit_entry")
                .order_by("id", Order::Asc)
                .limit(BATCH)
                .to_owned();
            if let Some(last) = after {
                select.and_where(Expr::col("id").gt(last));
            }
            let rows = connection.query_all(&select).await?;
            for row in &rows {
                let id: Uuid = row.try_get("", "id")?;
                let module: String = row.try_get("", "module")?;
                let action: String = row.try_get("", "action")?;
                let actor_kind: Option<String> = row.try_get("", "actor_kind")?;
                let subject: Option<String> = row.try_get("", "subject")?;
                let before: Option<String> = row.try_get("", "before")?;
                let data_after: Option<String> = row.try_get("", "after")?;
                let details: Option<String> = row.try_get("", "details")?;
                let occurred_at: DateTime<Utc> = row.try_get("", "occurred_at")?;
                let recorded_at: DateTime<Utc> = row.try_get("", "recorded_at")?;
                let payload_hash = chain::payload_hash(
                    before.as_deref(),
                    data_after.as_deref(),
                    details.as_deref(),
                );
                sequence += 1;
                let hash = chain::chain_hash(
                    &previous,
                    &Link {
                        sequence,
                        id,
                        module: &module,
                        action: &action,
                        actor_user_id: row.try_get("", "actor_user_id")?,
                        actor_api_key_id: row.try_get("", "actor_api_key_id")?,
                        actor_kind: actor_kind.as_deref(),
                        subject: subject.as_deref(),
                        occurred_at,
                        recorded_at,
                        payload_hash: &payload_hash,
                    },
                );
                let update = Query::update()
                    .table("audit_entry")
                    .value("sequence", sequence)
                    .value("payload_hash", payload_hash)
                    .value("chain_hash", hash.clone())
                    .and_where(Expr::col("id").eq(id))
                    .to_owned();
                connection.execute(&update).await?;
                previous = hash;
                after = Some(id);
            }
            if (rows.len() as u64) < BATCH {
                break;
            }
        }
        let mut head = Query::insert();
        head.into_table("audit_chain").columns(["name", "sequence", "hash"]);
        head.values([chain::HEAD.into(), sequence.into(), previous.into()])
            .map_err(|error| DbErr::Custom(error.to_string()))?;
        connection.execute(&head).await?;
        // Unique: hai dòng không thể cùng một vị trí trong chuỗi.
        manager
            .create_index(
                Index::create()
                    .name(INDEX)
                    .table("audit_entry")
                    .col("sequence")
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_index(Index::drop().name(INDEX).table("audit_entry").to_owned()).await?;
        manager.drop_table(Table::drop().table("audit_chain").to_owned()).await?;
        for column in ["chain_hash", "payload_hash", "sequence"] {
            manager
                .alter_table(Table::alter().table("audit_entry").drop_column(column).to_owned())
                .await?;
        }
        Ok(())
    }
}
