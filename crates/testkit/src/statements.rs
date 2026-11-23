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
//! batches by a background task, which can land exactly in a measurement (random red on Postgres) and is not
//! part of the request's processing path — statements writing to `kernel_request_log` are not counted.

use std::sync::{Arc, Mutex, PoisonError};

/// Statement that the background task writes to the request log (see the top of the file) — skipped when counting.
const REQUEST_LOG_INSERT: &str = "INSERT INTO \"kernel_request_log\"";

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
                && !sql.starts_with(REQUEST_LOG_INSERT)
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
