//! Hỏi đáp trên `comments` (D-202):
//!
//! - `comments_comments.badges`: mã nhãn của tác giả lúc viết (`seller,verified_buyer`), tính
//!   một lần lúc ghi để đường đọc không gọi sang module khác; rỗng là không nhãn.
//! - `comments_comments.match_key`: chữ và tên dạng `search::exact_key` — tầng "khớp đúng dấu"
//!   khi tìm, xếp trước tầng không dấu (`search_key`).
//! - `comments_targets.published_thread_count` / `pending_thread_count`: số bình luận GỐC (câu
//!   hỏi) đang hiện / chờ duyệt, tách khỏi số đếm mọi bình luận.
//!
//! Dòng có sẵn được tính lại trong Rust (khoá tìm) và bằng câu đếm theo nhóm (số đếm).

use std::collections::HashMap;

use autoxide_kernel::migration::prelude::*;
use uuid::Uuid;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0009_questions"
    }
}

/// Số dòng điền mỗi lượt đọc.
const BATCH: u64 = 500;

#[autoxide_kernel::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite chỉ thêm được một cột mỗi câu `ALTER TABLE`.
        for column in [schema::text("badges").default(""), schema::text("match_key").default("")] {
            manager
                .alter_table(
                    Table::alter()
                        .table("comments_comments")
                        .add_column(column.to_owned())
                        .to_owned(),
                )
                .await?;
        }
        for column in [
            schema::integer("published_thread_count").default(0),
            schema::integer("pending_thread_count").default(0),
        ] {
            manager
                .alter_table(
                    Table::alter()
                        .table("comments_targets")
                        .add_column(column.to_owned())
                        .to_owned(),
                )
                .await?;
        }
        fill_match_keys(manager).await?;
        fill_thread_counts(manager).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in ["badges", "match_key"] {
            manager
                .alter_table(
                    Table::alter().table("comments_comments").drop_column(column).to_owned(),
                )
                .await?;
        }
        for column in ["published_thread_count", "pending_thread_count"] {
            manager
                .alter_table(
                    Table::alter().table("comments_targets").drop_column(column).to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}

/// `match_key` của bình luận có sẵn, cùng công thức với lúc viết (`posting.rs`): chữ và tên
/// khách. Bình luận đã xoá giữ khoá rỗng như `search_key`.
async fn fill_match_keys(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let connection = manager.get_connection();
    let mut after: Option<Uuid> = None;
    loop {
        let mut select = Query::select()
            .columns(["id", "body", "author_name", "status"])
            .from("comments_comments")
            .order_by("id", Order::Asc)
            .limit(BATCH)
            .to_owned();
        if let Some(id) = after {
            select.and_where(Expr::col("id").gt(id));
        }
        let rows = connection.query_all(&select).await?;
        let Some(last) = rows.last() else {
            return Ok(());
        };
        after = Some(last.try_get("", "id")?);
        for row in &rows {
            let status: String = row.try_get("", "status")?;
            if status == "deleted" {
                continue;
            }
            let id: Uuid = row.try_get("", "id")?;
            let body: String = row.try_get("", "body")?;
            let name: Option<String> = row.try_get("", "author_name")?;
            let key = autoxide_kernel::search::exact_key(&format!(
                "{body} {}",
                name.as_deref().unwrap_or_default()
            ));
            let update = Query::update()
                .table("comments_comments")
                .value("match_key", key)
                .and_where(Expr::col("id").eq(id))
                .to_owned();
            connection.execute(&update).await?;
        }
    }
}

/// Số câu hỏi đang hiện / chờ duyệt của mỗi đích có sẵn: một câu đếm theo nhóm, rồi ghi từng đích.
async fn fill_thread_counts(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let connection = manager.get_connection();
    let select = Query::select()
        .columns(["target_type", "target_id", "status"])
        .expr_as(Expr::col("id").count(), "found")
        .from("comments_comments")
        .and_where(Expr::col("depth").eq(0))
        .and_where(Expr::col("status").is_in(["published", "pending"]))
        .group_by_columns(["target_type", "target_id", "status"])
        .to_owned();
    let mut counts: HashMap<(String, String), (i64, i64)> = HashMap::new();
    for row in connection.query_all(&select).await? {
        let kind: String = row.try_get("", "target_type")?;
        let id: String = row.try_get("", "target_id")?;
        let status: String = row.try_get("", "status")?;
        let found: i64 = row.try_get("", "found")?;
        let entry = counts.entry((kind, id)).or_default();
        if status == "published" {
            entry.0 = found;
        } else {
            entry.1 = found;
        }
    }
    for ((kind, id), (published, pending)) in counts {
        let update = Query::update()
            .table("comments_targets")
            .value("published_thread_count", published)
            .value("pending_thread_count", pending)
            .and_where(Expr::col("target_type").eq(kind))
            .and_where(Expr::col("target_id").eq(id))
            .to_owned();
        connection.execute(&update).await?;
    }
    Ok(())
}
