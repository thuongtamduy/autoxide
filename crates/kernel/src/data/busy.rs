//! "Which of these tables have a row matching `condition`?" in ONE statement: `SELECT EXISTS(SELECT 1 FROM t0 WHERE …)
//! AS t0, EXISTS(…) AS t1, …`. The event dispatcher and the job worker poll every module's table each second; asking
//! first which tables have work turns an idle pass from one statement per module into one statement per database,
//! without delaying any delivery (the tables that have work are read in the same pass).

use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Condition, Expr, Query};

use crate::database::Database;
use crate::error::Error;

/// For each of `tables`, whether it has a row matching `condition`, in the same order.
pub(crate) async fn tables_with(
    database: &Database,
    tables: &[String],
    condition: &Condition,
) -> Result<Vec<bool>, Error> {
    if tables.is_empty() {
        return Ok(Vec::new());
    }
    let mut select = Query::select();
    for (index, table) in tables.iter().enumerate() {
        let probe = Query::select()
            .expr(Expr::val(1))
            .from(Alias::new(table.as_str()))
            .cond_where(condition.clone())
            .to_owned();
        select.expr_as(Expr::exists(probe), Alias::new(format!("t{index}")));
    }
    let Some(row) = database.write().query_one(&select).await? else {
        return Ok(vec![true; tables.len()]);
    };
    (0..tables.len())
        .map(|index| {
            let column = format!("t{index}");
            // SQLite trả số nguyên 0/1, Postgres trả `bool`: đọc thử `bool`, không được thì đọc số.
            row.try_get::<bool>("", &column)
                .or_else(|_| row.try_get::<i64>("", &column).map(|value| value != 0))
                .map_err(Error::from)
        })
        .collect()
}
