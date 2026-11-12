//! Hạn mức token mỗi ngày (UTC), đếm trong bảng `ai_usage` — dùng chung mọi tiến trình máy chủ,
//! như nhau trên SQLite và Postgres.

use autoxide_kernel::ai::Usage;
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler};
use chrono::{Duration, Utc};
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, SqlErr};
use uuid::Uuid;

use crate::errors;

pub(crate) mod entity {
    //! Bảng `ai_usage`: một dòng mỗi (ngày UTC, việc).

    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ai_usage")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// `YYYY-MM-DD` theo UTC (so sánh như chuỗi là so sánh ngày).
        pub day: String,
        /// `<module>.<việc>`, vd `content.translate`.
        pub purpose: String,
        pub input_tokens: i64,
        pub output_tokens: i64,
        pub requests: i64,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

use entity::{Column, Entity};

/// Sổ đếm token.
pub(crate) struct UsageBook {
    database: Database,
    /// `0`: không giới hạn.
    limit: u64,
}

pub(crate) fn today() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

impl UsageBook {
    pub(crate) fn new(database: Database, limit: u64) -> Self {
        UsageBook { database, limit }
    }

    pub(crate) fn limit(&self) -> u64 {
        self.limit
    }

    /// Tổng token (vào + ra) đã dùng hôm nay.
    pub(crate) async fn used_today(&self) -> Result<u64, Error> {
        let rows = Entity::find().filter(Column::Day.eq(today())).all(self.database.read()).await?;
        Ok(rows
            .iter()
            .map(|row| u64::try_from(row.input_tokens + row.output_tokens).unwrap_or(0))
            .sum())
    }

    /// Lỗi `ai.quota_exceeded` nếu hôm nay đã hết hạn mức.
    pub(crate) async fn check(&self) -> Result<(), Error> {
        if self.limit > 0 && self.used_today().await? >= self.limit {
            return Err(errors::quota_exceeded());
        }
        Ok(())
    }

    /// Cộng `usage` (và một lời gọi) vào dòng hôm nay của `purpose`. Cộng bằng một câu `UPDATE`
    /// (`cột = cột + n`) nên nhiều tiến trình cùng cộng không mất số; chưa có dòng thì thêm,
    /// thêm trùng (một tiến trình khác vừa thêm) thì cộng lại.
    pub(crate) async fn record(&self, purpose: &str, usage: Usage) -> Result<(), Error> {
        let day = today();
        let input = i64::try_from(usage.input_tokens).unwrap_or(i64::MAX);
        let output = i64::try_from(usage.output_tokens).unwrap_or(i64::MAX);
        for _ in 0..3 {
            if self.add(&day, purpose, input, output).await? {
                return Ok(());
            }
            let row = entity::ActiveModel {
                id: Set(Uuid::now_v7()),
                day: Set(day.clone()),
                purpose: Set(purpose.to_owned()),
                input_tokens: Set(input),
                output_tokens: Set(output),
                requests: Set(1),
                updated_at: Set(Utc::now()),
            };
            match row.insert(self.database.write()).await {
                Ok(_) => return Ok(()),
                Err(error)
                    if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Err(Error::internal("could not record AI token usage"))
    }

    async fn add(&self, day: &str, purpose: &str, input: i64, output: i64) -> Result<bool, Error> {
        let result = Entity::update_many()
            .col_expr(Column::InputTokens, Expr::col(Column::InputTokens).add(input))
            .col_expr(Column::OutputTokens, Expr::col(Column::OutputTokens).add(output))
            .col_expr(Column::Requests, Expr::col(Column::Requests).add(1))
            .col_expr(Column::UpdatedAt, Expr::value(Utc::now()))
            .filter(Column::Day.eq(day))
            .filter(Column::Purpose.eq(purpose))
            .exec(self.database.write())
            .await?;
        Ok(result.rows_affected > 0)
    }

    /// Xoá dòng cũ hơn `days` ngày.
    pub(crate) async fn prune(&self, days: u32) -> Result<u64, Error> {
        let cutoff = (Utc::now() - Duration::days(i64::from(days))).format("%Y-%m-%d").to_string();
        let result = Entity::delete_many()
            .filter(Column::Day.lt(cutoff))
            .exec(self.database.write())
            .await?;
        Ok(result.rows_affected)
    }
}

/// Job dọn dòng cũ của `ai_usage`.
pub(crate) const PRUNE: &str = "ai.prune_usage";

pub(crate) struct Prune {
    pub book: std::sync::Arc<UsageBook>,
    pub days: u32,
}

#[async_trait::async_trait]
impl JobHandler for Prune {
    async fn run(&self, _: &JobContext) -> Result<(), Error> {
        let removed = self.book.prune(self.days).await?;
        if removed > 0 {
            tracing::info!(removed, "old AI usage rows removed");
        }
        Ok(())
    }
}
