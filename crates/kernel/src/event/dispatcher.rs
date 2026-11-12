//! Bộ phát event: đọc event chưa giao trong bảng `<module>_events` của các module và
//! giao cho bên nhận.
//!
//! Nhiều tiến trình chạy bộ phát cùng lúc vẫn đúng: mỗi event được "nhận việc"
//! (`claimed_until`) bằng một câu `UPDATE` có điều kiện — chỉ một tiến trình nhận được,
//! các tiến trình khác bỏ qua. Câu lệnh chạy giống nhau trên SQLite và Postgres. Tiến
//! trình nhận việc rồi chết giữa chừng thì hết hạn nhận việc, tiến trình khác giao lại.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, Order, Query};
use sea_orm::{ConnectionTrait, QueryResult};
use tokio::sync::Notify;
use uuid::Uuid;

use super::{Actor, Delivered, Subscription, table_name};
use crate::database::Database;
use crate::error::Error;

/// Số lần giao tối đa của một event; quá thì đánh dấu hỏng (`failed_at`) — trừ event xoá
/// tài khoản ([`USER_DELETED`](crate::privacy::USER_DELETED)), thử lại mãi.
pub const MAX_ATTEMPTS: i32 = 10;
/// Số event mỗi bảng xử lý trong một lượt.
const BATCH: u64 = 100;
/// Một tiến trình giữ việc giao một event tối đa chừng này.
const CLAIM_SECONDS: i64 = 60;
/// Không có event mới thì hỏi lại sau chừng này.
const IDLE_POLL: Duration = Duration::from_secs(1);
/// Độ dài tối đa của lỗi lưu lại.
const MAX_ERROR_LENGTH: usize = 2000;
/// Giữ event đã giao chừng này rồi xoá (audit log đã có bản của nó).
const KEEP_DELIVERED: chrono::Duration = chrono::Duration::days(7);
/// Dọn event đã giao mỗi chừng này.
const CLEANUP_EVERY: Duration = Duration::from_secs(3600);

/// Bộ phát event của chương trình.
pub struct Dispatcher {
    database: Database,
    /// Các module có bảng event.
    modules: Vec<&'static str>,
    /// (module nhận, đăng ký).
    subscriptions: Vec<(&'static str, Subscription)>,
    wake: Arc<Notify>,
    last_cleanup: std::sync::Mutex<Option<std::time::Instant>>,
}

impl Dispatcher {
    pub(crate) fn new(
        database: Database,
        modules: Vec<&'static str>,
        subscriptions: Vec<(&'static str, Subscription)>,
        wake: Arc<Notify>,
    ) -> Self {
        Dispatcher {
            database,
            modules,
            subscriptions,
            wake,
            last_cleanup: std::sync::Mutex::new(None),
        }
    }

    /// Giao một lượt mọi event đang chờ; trả về số event đã xử lý (giao xong hoặc
    /// hẹn lại).
    pub async fn run_once(&self) -> Result<usize, Error> {
        self.cleanup().await?;
        let mut handled = 0;
        for module in &self.modules {
            handled += self.run_table(module).await?;
        }
        Ok(handled)
    }

    /// Giao event tới khi `shutdown` hoàn thành. Lỗi database thì ghi log rồi thử lại
    /// ở lượt sau — bộ phát không được chết vì một lần database chập chờn.
    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        tokio::pin!(shutdown);
        loop {
            let handled = match self.run_once().await {
                Ok(handled) => handled,
                Err(error) => {
                    tracing::error!(error = %error, "event dispatcher failed; retrying");
                    0
                }
            };
            if handled > 0 {
                continue;
            }
            tokio::select! {
                () = &mut shutdown => return,
                () = self.wake.notified() => {}
                () = tokio::time::sleep(IDLE_POLL) => {}
            }
        }
    }

    /// Xoá event đã giao hơn [`KEEP_DELIVERED`], mỗi giờ một lần. Event hỏng hẳn được
    /// giữ lại để người vận hành xem.
    async fn cleanup(&self) -> Result<(), Error> {
        {
            let mut last = self.last_cleanup.lock().map_err(|_| Error::internal("cleanup lock"))?;
            if last.is_some_and(|at| at.elapsed() < CLEANUP_EVERY) {
                return Ok(());
            }
            *last = Some(std::time::Instant::now());
        }
        let before = now() - KEEP_DELIVERED;
        for module in &self.modules {
            let mut delete = Query::delete();
            delete
                .from_table(Alias::new(table_name(module)))
                .and_where(Expr::col(Alias::new("delivered_at")).lt(before));
            self.database.write().execute(&delete).await?;
        }
        Ok(())
    }

    async fn run_table(&self, module: &'static str) -> Result<usize, Error> {
        let table = Alias::new(table_name(module));
        let now = now();
        let mut select = Query::select();
        select
            .columns([
                "id",
                "name",
                "actor_user_id",
                "actor_api_key_id",
                "actor_kind",
                "subject",
                "payload",
                "occurred_at",
                "attempts",
            ])
            .from(table.clone())
            .cond_where(pending(now))
            .order_by(Alias::new("id"), Order::Asc)
            .limit(BATCH);
        let rows = self.database.write().query_all(&select).await?;
        let mut handled = 0;
        for row in rows {
            let event = read_event(module, &row)?;
            let attempts: i32 = row.try_get("", "attempts")?;
            if !self.claim(&table, event.id, now).await? {
                continue; // tiến trình khác vừa nhận việc giao event này
            }
            handled += 1;
            match self.deliver(&event).await {
                Ok(()) => self.mark_delivered(&table, event.id).await?,
                Err(error) => self.mark_failed(&table, &event, attempts + 1, &error).await?,
            }
        }
        Ok(handled)
    }

    /// Nhận việc giao event `id`. `false` nếu tiến trình khác đã nhận.
    async fn claim(&self, table: &Alias, id: Uuid, now: DateTime<Utc>) -> Result<bool, Error> {
        let mut update = Query::update();
        update
            .table(table.clone())
            .value(Alias::new("claimed_until"), now + chrono::Duration::seconds(CLAIM_SECONDS))
            .cond_where(Condition::all().add(Expr::col(Alias::new("id")).eq(id)).add(pending(now)));
        Ok(self.database.write().execute(&update).await?.rows_affected() == 1)
    }

    /// Giao cho mọi bên nhận khớp tên event, lần lượt; bên đầu tiên lỗi thì dừng.
    async fn deliver(&self, event: &Delivered) -> Result<(), Error> {
        for (subscriber, subscription) in &self.subscriptions {
            if subscription.matches(&event.name) {
                subscription.handler.handle(event).await.map_err(|error| {
                    Error::internal(format!("module `{subscriber}`: {}", describe(&error)))
                })?;
            }
        }
        Ok(())
    }

    async fn mark_delivered(&self, table: &Alias, id: Uuid) -> Result<(), Error> {
        let mut update = Query::update();
        update
            .table(table.clone())
            .value(Alias::new("delivered_at"), now())
            .value(Alias::new("claimed_until"), Option::<DateTime<Utc>>::None)
            .and_where(Expr::col(Alias::new("id")).eq(id));
        self.database.write().execute(&update).await?;
        metrics::counter!("events_delivered_total").increment(1);
        Ok(())
    }

    /// Ghi một lần giao hỏng: hẹn lần sau theo backoff, hoặc đánh dấu hỏng hẳn.
    async fn mark_failed(
        &self,
        table: &Alias,
        event: &Delivered,
        attempts: i32,
        error: &Error,
    ) -> Result<(), Error> {
        let now = now();
        let message: String = describe(error).chars().take(MAX_ERROR_LENGTH).collect();
        let mut update = Query::update();
        update
            .table(table.clone())
            .value(Alias::new("attempts"), attempts)
            .value(Alias::new("next_attempt_at"), now + backoff(attempts))
            .value(Alias::new("claimed_until"), Option::<DateTime<Utc>>::None)
            .value(Alias::new("last_error"), message.clone())
            .and_where(Expr::col(Alias::new("id")).eq(event.id));
        // Xoá dữ liệu của tài khoản đã xoá là nghĩa vụ, không phải việc phụ: không bỏ cuộc,
        // cứ thử lại (tối đa mỗi giờ một lần) tới khi mọi module xong (D-149). Tài khoản đã
        // mất nên người dùng không tự xin xoá lại được.
        let never_give_up = event.name == crate::privacy::USER_DELETED;
        if attempts >= MAX_ATTEMPTS && !never_give_up {
            update.value(Alias::new("failed_at"), now);
            tracing::error!(
                event_id = %event.id,
                event = %event.name,
                attempts,
                error = %message,
                "event delivery failed for good; it will not be retried"
            );
            metrics::counter!("events_failed_total").increment(1);
        } else if attempts >= MAX_ATTEMPTS {
            tracing::error!(
                event_id = %event.id,
                event = %event.name,
                attempts,
                error = %message,
                "erasing a deleted user's data keeps failing; it is retried every hour"
            );
        } else {
            tracing::warn!(
                event_id = %event.id,
                event = %event.name,
                attempts,
                error = %message,
                "event delivery failed; will retry"
            );
        }
        self.database.write().execute(&update).await?;
        Ok(())
    }
}

/// Điều kiện "đang chờ giao": chưa giao, chưa hỏng hẳn, tới giờ thử, không ai đang giữ.
fn pending(now: DateTime<Utc>) -> Condition {
    Condition::all()
        .add(Expr::col(Alias::new("delivered_at")).is_null())
        .add(Expr::col(Alias::new("failed_at")).is_null())
        .add(Expr::col(Alias::new("next_attempt_at")).lte(now))
        .add(
            Condition::any()
                .add(Expr::col(Alias::new("claimed_until")).is_null())
                .add(Expr::col(Alias::new("claimed_until")).lt(now)),
        )
}

/// Chờ trước lần giao thứ `attempts + 1`: 2, 4, 8… giây, tối đa một giờ.
fn backoff(attempts: i32) -> chrono::Duration {
    let exponent = u32::try_from(attempts.clamp(1, 12)).unwrap_or(12);
    chrono::Duration::seconds(std::cmp::min(i64::from(2u32.pow(exponent)), 3600))
}

fn read_event(module: &'static str, row: &QueryResult) -> Result<Delivered, Error> {
    let payload: String = row.try_get("", "payload")?;
    Ok(Delivered {
        id: row.try_get("", "id")?,
        module,
        name: row.try_get("", "name")?,
        actor: Actor {
            user_id: row.try_get("", "actor_user_id")?,
            api_key_id: row.try_get("", "actor_api_key_id")?,
            public: row.try_get::<Option<String>>("", "actor_kind")?.as_deref() == Some("public"),
        },
        subject: row.try_get("", "subject")?,
        payload: serde_json::from_str(&payload).map_err(Error::internal)?,
        occurred_at: row.try_get("", "occurred_at")?,
    })
}

/// Lỗi kèm nguyên nhân gốc (lỗi nội bộ chỉ có câu chung; nguyên nhân mới có ích để sửa).
fn describe(error: &Error) -> String {
    match std::error::Error::source(error) {
        Some(source) => format!("{}: {source}", error.code()),
        None => format!("{}: {}", error.code(), error.message()),
    }
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_tang_dan_va_co_tran() {
        assert_eq!(backoff(1), chrono::Duration::seconds(2));
        assert_eq!(backoff(3), chrono::Duration::seconds(8));
        assert_eq!(backoff(30), chrono::Duration::seconds(3600));
    }
}
