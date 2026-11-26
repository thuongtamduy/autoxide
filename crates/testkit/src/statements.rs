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
//! The test app does not run background work (event dispatcher, jobs). Only the request log is written in
//! batches by a background task — an `INSERT` per batch, and after the first batch of a tenant a `DELETE` of
//! expired rows — which can land exactly in a measurement (random red on Postgres, more likely on a slow
//! machine) and is not part of the request's processing path: no statement on `kernel_request_log` is counted.

use std::sync::{Arc, Mutex, PoisonError};

/// Table the background task of the request log writes and prunes (see the top of the file) — statements on it
/// are skipped when counting.
const REQUEST_LOG_TABLE: &str = "\"kernel_request_log\"";

/// SQL statement log: records only while a measurement is active.
#[derive(Default)]
pub(crate) struct StatementLog {
    // `Mutex<Option<…>>`: `None` = không đo (callback chỉ khoá rồi bỏ qua). Callback của SeaORM
    // chạy đồng bộ, không `await` bên trong nên Mutex của std là đủ.
    recording: Mutex<Option<Vec<String>>>,
}

impl StatementLog {
    /// Function SeaORM calls after every SQL statement.
    pub(crate) fn observer(self: &Arc<Self>) -> Arc<dyn Fn(&str) + Send + Sync> {
        let log = Arc::clone(self);
        Arc::new(move |sql: &str| {
            // Một test panic giữa chừng làm Mutex "nhiễm độc"; dữ liệu bên trong vẫn dùng được.
            let mut recording = log.recording.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(statements) = recording.as_mut()
                && !sql.contains(REQUEST_LOG_TABLE)
            {
                statements.push(sql.to_owned());
            }
        })
    }

    pub(crate) fn start(&self) {
        *self.recording.lock().unwrap_or_else(PoisonError::into_inner) = Some(Vec::new());
    }

    pub(crate) fn stop(&self) -> Vec<String> {
        self.recording.lock().unwrap_or_else(PoisonError::into_inner).take().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Câu của tác vụ nền ghi nhật ký request (cả `INSERT` lẫn `DELETE` dọn dòng hết hạn) không được đếm; câu khác thì đếm.
    #[test]
    fn bo_qua_moi_cau_cua_nhat_ky_request() {
        let log = Arc::new(StatementLog::default());
        let observe = log.observer();
        log.start();
        observe(r#"INSERT INTO "kernel_request_log" ("id") VALUES ($1)"#);
        observe(r#"DELETE FROM "kernel_request_log" WHERE "kernel_request_log"."expires_at" < $1"#);
        observe(r#"SELECT "catalog_products"."id" FROM "catalog_products""#);
        assert_eq!(log.stop().len(), 1);
    }
}
