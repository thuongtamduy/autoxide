//! Đếm câu SQL một việc chạy (D-20261120-step39-hot-paths): giữ số câu của đường nóng không tăng
//! lại sau khi đã gộp N+1. Dùng qua [`TestApp::statements`](crate::TestApp::statements).
//!
//! ```no_run
//! # async fn example(app: &autoxide_testkit::TestApp) {
//! let (answer, statements) = app.statements(app.get("/api/v1/notes").token("ada").send()).await;
//! assert_eq!(answer.unwrap().status(), 200);
//! assert!(statements.len() <= 2, "{statements:#?}");
//! # }
//! ```
//!
//! App của test không chạy việc nền (bộ phát event, job) nên mọi câu ghi lại là của chính việc
//! được đo.

use std::sync::{Arc, Mutex, PoisonError};

/// Sổ ghi câu SQL: chỉ ghi khi đang có một lượt đo.
#[derive(Default)]
pub(crate) struct StatementLog {
    // `Mutex<Option<…>>`: `None` = không đo (callback chỉ khoá rồi bỏ qua). Callback của SeaORM
    // chạy đồng bộ, không `await` bên trong nên Mutex của std là đủ.
    recording: Mutex<Option<Vec<String>>>,
}

impl StatementLog {
    /// Hàm SeaORM gọi sau mỗi câu SQL.
    pub(crate) fn observer(self: &Arc<Self>) -> Arc<dyn Fn(&str) + Send + Sync> {
        let log = Arc::clone(self);
        Arc::new(move |sql: &str| {
            // Một test panic giữa chừng làm Mutex "nhiễm độc"; dữ liệu bên trong vẫn dùng được.
            let mut recording = log.recording.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(statements) = recording.as_mut() {
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
