//! Records which endpoints and MCP tools the tests actually used successfully (D-20261123-api-coverage).
//!
//! Off unless the environment variable [`TEST_API_COVERAGE`] names a directory: then every
//! [`TestRequest::send`](crate::TestRequest::send) that gets a non-error answer (status below 400) appends
//! `http <METHOD> <path template>` and every successful [`McpClient::call_tool`](crate::mcp::McpClient::call_tool)
//! appends `tool <name>` to `<directory>/<process id>.txt`. `cargo xtask coverage-api` sets the variable,
//! runs the tests and compares the files with `docs/openapi.json` and `docs/mcp-tools.json`.
//!
//! Tests that call a router without [`TestRequest`](crate::TestRequest) — the binary's real runtime, a stream,
//! a WebSocket over a real port — record the same way with [`record_response`].
//!
//! Each line is written once per process (a set in memory remembers what was written), so the cost while
//! recording is one small file append per distinct endpoint; when the variable is unset, the cost is one
//! lookup of a cached `Option`.

use std::collections::HashSet;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Environment variable: directory to record coverage into. Unset: nothing is recorded.
pub const TEST_API_COVERAGE: &str = "TEST_API_COVERAGE";

// `OnceLock`: đọc biến môi trường một lần cho cả process. `Mutex<HashSet>`: nhiều test chạy song
// song trên nhiều thread cùng ghi, mỗi dòng chỉ ghi một lần.
struct Recorder {
    file: PathBuf,
    seen: Mutex<HashSet<String>>,
}

fn recorder() -> Option<&'static Recorder> {
    static RECORDER: OnceLock<Option<Recorder>> = OnceLock::new();
    RECORDER
        .get_or_init(|| {
            let directory =
                std::env::var_os(TEST_API_COVERAGE).filter(|value| !value.is_empty())?;
            let directory = PathBuf::from(directory);
            std::fs::create_dir_all(&directory).ok()?;
            Some(Recorder {
                file: directory.join(format!("{}.txt", std::process::id())),
                seen: Mutex::new(HashSet::new()),
            })
        })
        .as_ref()
}

fn record(line: String) {
    let Some(recorder) = recorder() else {
        return;
    };
    // Mutex bị "poison" (một test panic khi đang giữ) vẫn dùng tiếp được: tập chỉ thêm, không hỏng.
    let mut seen = recorder.seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !seen.insert(line.clone()) {
        return;
    }
    // Ghi lỗi thì bỏ qua: đo độ phủ không được làm test đỏ.
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&recorder.file)
    {
        let _ = writeln!(file, "{line}");
    }
}

/// An HTTP call answered below 400, on the documented path `template`.
pub(crate) fn http(method: &str, template: &str) {
    if recorder().is_some() {
        record(format!("http {} {template}", method.to_ascii_uppercase()));
    }
}

/// An MCP tool call answered without `isError`.
pub(crate) fn tool(name: &str) {
    if recorder().is_some() {
        record(format!("tool {name}"));
    }
}

/// Records one answer of a call made without [`TestRequest`](crate::TestRequest) (a real runtime's router, a
/// stream, a WebSocket over a real port), like `TestRequest::send` does: `path` is the concrete path (no
/// query); it counts when `status` is below 400 and `path` matches an operation of `openapi`. Does nothing
/// unless [`TEST_API_COVERAGE`] is set.
pub fn record_response(openapi: &utoipa::openapi::OpenApi, method: &str, path: &str, status: u16) {
    if status >= 400 || recorder().is_none() {
        return;
    }
    let path = path.split('?').next().unwrap_or(path);
    // Đổi hợp đồng sang JSON chỉ khi đang đo: tốn vài mili giây mỗi lần, không ảnh hưởng gate.
    let Ok(document) = serde_json::to_value(openapi) else {
        return;
    };
    if let Some(template) = crate::contract::template(&document, method, path) {
        http(method, template);
    }
}
