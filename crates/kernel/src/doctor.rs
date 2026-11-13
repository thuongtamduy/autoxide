//! Kiểm bản cài đặt: lệnh `autoxide doctor` (D-166, mượn `cargo loco doctor` và system checks
//! của Django — D-164).
//!
//! Module khai các bước kiểm của mình qua [`Module::checks`](crate::module::Module::checks):
//! nơi lưu file có ghi được không, máy chủ mail có trả lời không, đã có quản trị viên chưa…
//! Mỗi bước trả một [`Finding`] kèm gợi ý sửa — người lạ tự cài và AI agent đều đọc được
//! (`autoxide doctor --json`).

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

/// Mỗi bước kiểm có tối đa chừng này thời gian (gọi mạng tới máy chủ không trả lời).
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// Kết quả một bước kiểm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Đạt.
    Ok,
    /// Chạy được nhưng nên sửa trước khi chạy thật (vd chưa đặt khoá ký).
    Warning,
    /// Hỏng: tính năng phụ thuộc vào nó sẽ không chạy.
    Failed,
}

/// Điều một bước kiểm tìm thấy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Finding {
    pub status: Status,
    /// Câu tiếng Anh nói điều đã thấy.
    pub message: String,
    /// Việc cần làm để sửa (lệnh, biến cấu hình), khi không đạt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl Finding {
    pub fn ok(message: impl Into<String>) -> Self {
        Finding { status: Status::Ok, message: message.into(), hint: None }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Finding { status: Status::Warning, message: message.into(), hint: None }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Finding { status: Status::Failed, message: message.into(), hint: None }
    }

    /// Thêm gợi ý sửa.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// Phần chạy của một bước kiểm. Không được thay đổi dữ liệu thật; thứ phải ghi thử (vd một file
/// thử trong nơi lưu) thì dọn trước khi trả.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`.
#[async_trait::async_trait]
pub trait Probe: Send + Sync {
    async fn probe(&self) -> Finding;
}

/// Một bước kiểm module khai. Tên khai ngắn (`storage`); kernel thêm tiền tố `<module>.`.
#[derive(Clone)]
#[non_exhaustive]
pub struct Check {
    pub name: &'static str,
    /// Câu tiếng Anh nói bước này kiểm gì.
    pub description: &'static str,
    probe: Arc<dyn Probe>,
}

impl Check {
    pub fn new(name: &'static str, description: &'static str, probe: Arc<dyn Probe>) -> Self {
        Check { name, description, probe }
    }

    /// Chạy bước kiểm, tối đa [`CHECK_TIMEOUT`].
    pub async fn run(&self) -> Finding {
        self.run_within(CHECK_TIMEOUT).await
    }

    async fn run_within(&self, limit: Duration) -> Finding {
        match tokio::time::timeout(limit, self.probe.probe()).await {
            Ok(finding) => finding,
            Err(_) => Finding::failed(format!("did not finish within {} ms", limit.as_millis()))
                .with_hint("Check that the service it talks to is reachable from this server."),
        }
    }
}

impl std::fmt::Debug for Check {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Check").field("name", &self.name).finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Slow;

    #[async_trait::async_trait]
    impl Probe for Slow {
        async fn probe(&self) -> Finding {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Finding::ok("never")
        }
    }

    #[tokio::test]
    async fn buoc_kiem_qua_lau_thi_hong() {
        let check = Check::new("slow", "Never answers.", Arc::new(Slow));
        let finding = check.run_within(Duration::from_millis(20)).await;
        assert_eq!(finding.status, Status::Failed);
        assert!(finding.message.contains("20 ms"), "{}", finding.message);
    }

    #[test]
    fn json_cho_ai_agent() {
        let finding = Finding::warning("No key.").with_hint("Set AUTOXIDE_SECRET_KEY.");
        assert_eq!(
            serde_json::to_value(&finding).unwrap(),
            serde_json::json!({"status": "warning", "message": "No key.", "hint": "Set AUTOXIDE_SECRET_KEY."})
        );
        assert!(serde_json::to_value(Finding::ok("Fine.")).unwrap().get("hint").is_none());
    }
}
