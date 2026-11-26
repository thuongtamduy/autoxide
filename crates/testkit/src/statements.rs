//! Counts the SQL statements a piece of work runs (D-20261120-step39-hot-paths): keeps the statement count of
//! a hot path from rising again after N+1 has been merged away. Used through
//! [`TestApp::statements`](crate::TestApp::statements).
//!
//! ```no_run
//! # async fn example(app: &autoxide_testkit::TestApp) {
//! let (answer, statements) = app.statements(app.get("/api/v1/notes").token("ada").send()).await;
//! assert_eq!(answer.unwrap().status(), 200);
//! assert!(statements.len() <= 2, "{statements:#?}");
//! # }
//! ```
//!
//! Only statements run by the measured work itself count: the work runs inside a task-local scope, and a
//! statement run by another task — a background task such as the request log writer, a job, a dispatcher — is
//! not counted even when it lands exactly in a measurement (it used to turn hot-path tests red at random on a
//! slow machine). Work the measured code hands to `tokio::spawn` is not counted either.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

tokio::task_local! {
    // Số của lần đo mà task hiện tại thuộc về. `task_local` gắn giá trị vào task (không phải thread): code chạy
    // trong `scope` thấy nó dù tokio chuyển task sang thread khác; task khác (việc nền) không thấy.
    static MEASURING: u64;
}

/// Numbers the measurements, so a nested one takes its statements from the outer one.
static NEXT_MEASUREMENT: AtomicU64 = AtomicU64::new(1);

/// SQL statement log: records only while a measurement is active.
#[derive(Default)]
pub(crate) struct StatementLog {
    // `Mutex<Option<…>>`: `None` = không đo (callback chỉ khoá rồi bỏ qua). Callback của SeaORM
    // chạy đồng bộ, không `await` bên trong nên Mutex của std là đủ.
    recording: Mutex<Option<(u64, Vec<String>)>>,
}

impl StatementLog {
    /// Function SeaORM calls after every SQL statement.
    pub(crate) fn observer(self: &Arc<Self>) -> Arc<dyn Fn(&str) + Send + Sync> {
        let log = Arc::clone(self);
        Arc::new(move |sql: &str| {
            // Một test panic giữa chừng làm Mutex "nhiễm độc"; dữ liệu bên trong vẫn dùng được.
            let mut recording = log.recording.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((measurement, statements)) = recording.as_mut()
                && MEASURING.try_with(|current| current == measurement).unwrap_or(false)
            {
                statements.push(sql.to_owned());
            }
        })
    }

    /// Runs `work` and returns the statements it ran itself (not those of other tasks).
    pub(crate) async fn measure<T>(&self, work: impl Future<Output = T>) -> (T, Vec<String>) {
        let measurement = NEXT_MEASUREMENT.fetch_add(1, Ordering::Relaxed);
        let previous = self
            .recording
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace((measurement, Vec::new()));
        let output = MEASURING.scope(measurement, work).await;
        let mut recording = self.recording.lock().unwrap_or_else(PoisonError::into_inner);
        let statements = recording.take().map(|(_, statements)| statements).unwrap_or_default();
        *recording = previous;
        (output, statements)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Câu của task khác (việc nền) chạy giữa lúc đo không được đếm; câu của chính việc đang đo thì đếm.
    #[tokio::test]
    async fn chi_dem_cau_cua_viec_dang_do() {
        let log = Arc::new(StatementLog::default());
        let observe = log.observer();
        let background = observe.clone();
        let ((), statements) = log
            .measure(async {
                observe(r#"SELECT "catalog_products"."id" FROM "catalog_products""#);
                tokio::spawn(async move {
                    background(r#"INSERT INTO "kernel_request_log" ("id") VALUES ($1)"#)
                })
                .await
                .unwrap();
            })
            .await;
        assert_eq!(statements.len(), 1);
        observe("SELECT 1");
        assert!(log.recording.lock().unwrap().is_none(), "ngoài lúc đo không ghi gì");
    }
}
