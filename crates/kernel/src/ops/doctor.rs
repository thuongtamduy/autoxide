//! Installation checks: the `autoxide doctor` command (D-166, borrowed from `cargo loco doctor` and Django's
//! system checks — D-164).
//!
//! A module declares its checks through [`Module::checks`](crate::module::Module::checks): whether the file
//! storage is writable, whether the mail server answers, whether an administrator exists yet… Each check
//! returns a [`Finding`] with a hint on how to fix it — a stranger installing by themselves and an AI agent
//! can both read it (`autoxide doctor --json`).

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

/// Each check gets at most this much time (a network call to a server that does not answer).
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// Result of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Passed.
    Ok,
    /// Works but should be fixed before running for real (e.g. no signing key set).
    Warning,
    /// Broken: features that depend on it will not work.
    Failed,
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Finding {
    pub status: Status,
    /// English sentence stating what was seen.
    pub message: String,
    /// What to do to fix it (command, setting), when the check does not pass.
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

    /// Adds a fix hint.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// The running part of a check. It must not change real data; anything it has to write as a trial (e.g. a
/// trial file in the storage) is cleaned up before it returns.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`.
#[async_trait::async_trait]
pub trait Probe: Send + Sync {
    async fn probe(&self) -> Finding;
}

/// A check declared by a module. The declared name is short (`storage`); the kernel adds the prefix `<module>.`.
#[derive(Clone)]
#[non_exhaustive]
pub struct Check {
    pub name: &'static str,
    /// English sentence stating what this check verifies.
    pub description: &'static str,
    probe: Arc<dyn Probe>,
    timeout: Duration,
}

impl Check {
    pub fn new(name: &'static str, description: &'static str, probe: Arc<dyn Probe>) -> Self {
        Check { name, description, probe, timeout: CHECK_TIMEOUT }
    }

    /// Gives the check more time than [`CHECK_TIMEOUT`] (e.g. re-reading a whole large table).
    pub fn with_timeout(mut self, limit: Duration) -> Self {
        self.timeout = limit;
        self
    }

    /// Runs the check, for at most [`CHECK_TIMEOUT`] (or the time set with
    /// [`with_timeout`](Self::with_timeout)).
    pub async fn run(&self) -> Finding {
        self.run_within(self.timeout).await
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
