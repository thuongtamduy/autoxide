//! `cargo xtask coverage-api [--run]`: endpoint nào trong `docs/openapi.json` và tool nào trong
//! `docs/mcp-tools.json` chưa được test nào gọi thành công (D-20261123-api-coverage).
//!
//! Testkit ghi lại khi biến `TEST_API_COVERAGE` trỏ tới một thư mục (`crates/testkit/src/coverage.rs`):
//! mỗi dòng `http <METHOD> <path template>` (trả lời dưới 400) hay `tool <tên>` (không `isError`).
//!
//! - `--run`: xoá thư mục `target/api-coverage`, chạy `cargo xtask test-all` với biến đó, rồi báo.
//! - Không `--run`: chỉ đọc kết quả của lần chạy trước.
//!
//! Đo trên SQLite là đủ: mọi test chạy trên SQLite cũng chạy trên Postgres ở gate đầy đủ (luật số một),
//! nên một endpoint được phủ ở đây là được phủ trên cả hai database.
//!
//! Chưa chặn gì (chỉ báo): đề xuất bật chặn endpoint mới không test khi đã phủ 100% (xem quyết định).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// Thư mục ghi, so với gốc repo.
pub const DIRECTORY: &str = "target/api-coverage";
/// Tên biến môi trường testkit đọc (`autoxide_testkit::TEST_API_COVERAGE`).
const VARIABLE: &str = "TEST_API_COVERAGE";
const METHODS: [&str; 5] = ["get", "put", "post", "patch", "delete"];

/// Một endpoint trong tài liệu.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Endpoint {
    pub method: String,
    pub path: String,
}

/// Kết quả so sánh.
#[derive(Debug, Default)]
pub struct Report {
    pub endpoints: usize,
    pub tools: usize,
    /// Nhóm (module) → endpoint chưa phủ.
    pub missing_endpoints: BTreeMap<String, Vec<Endpoint>>,
    /// Nhóm (module) → tool chưa phủ.
    pub missing_tools: BTreeMap<String, Vec<String>>,
}

impl Report {
    fn missing_endpoint_count(&self) -> usize {
        self.missing_endpoints.values().map(Vec::len).sum()
    }

    fn missing_tool_count(&self) -> usize {
        self.missing_tools.values().map(Vec::len).sum()
    }
}

/// Endpoint trong tài liệu OpenAPI.
pub fn endpoints(openapi: &Value) -> Vec<Endpoint> {
    let mut out = Vec::new();
    if let Some(paths) = openapi.get("paths").and_then(Value::as_object) {
        for (path, item) in paths {
            for method in METHODS {
                if item.get(method).is_some() {
                    out.push(Endpoint { method: method.to_ascii_uppercase(), path: path.clone() });
                }
            }
        }
    }
    out.sort();
    out
}

/// Tên tool trong `docs/mcp-tools.json`.
pub fn tools(manifest: &Value) -> Vec<String> {
    let mut out: Vec<String> = manifest
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Nhóm của một endpoint: đoạn sau `/api/v1/` (tên module), còn lại là `core`.
fn endpoint_group(path: &str) -> String {
    path.strip_prefix("/api/v1/")
        .and_then(|rest| rest.split('/').next())
        .filter(|segment| !segment.is_empty() && !segment.starts_with('{'))
        .unwrap_or("core")
        .to_owned()
}

/// Nhóm của một tool: phần trước dấu `.` đầu tiên (tên module).
fn tool_group(name: &str) -> String {
    name.split('.').next().unwrap_or(name).to_owned()
}

/// So các dòng đã ghi với tài liệu.
pub fn compare(endpoints: &[Endpoint], tools: &[String], recorded: &BTreeSet<String>) -> Report {
    let mut report = Report { endpoints: endpoints.len(), tools: tools.len(), ..Report::default() };
    for endpoint in endpoints {
        if !recorded.contains(&format!("http {} {}", endpoint.method, endpoint.path)) {
            report
                .missing_endpoints
                .entry(endpoint_group(&endpoint.path))
                .or_default()
                .push(endpoint.clone());
        }
    }
    for tool in tools {
        if !recorded.contains(&format!("tool {tool}")) {
            report.missing_tools.entry(tool_group(tool)).or_default().push(tool.clone());
        }
    }
    report
}

/// Đọc mọi dòng đã ghi trong `directory`.
fn recorded(directory: &Path) -> Result<BTreeSet<String>, String> {
    let entries = std::fs::read_dir(directory).map_err(|error| {
        format!(
            "cannot read {}: {error} — run `cargo xtask coverage-api --run` first",
            directory.display()
        )
    })?;
    let mut lines = BTreeSet::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "txt") {
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            lines.extend(text.lines().map(str::to_owned));
        }
    }
    Ok(lines)
}

fn percent(covered: usize, total: usize) -> f64 {
    if total == 0 { 100.0 } else { covered as f64 * 100.0 / total as f64 }
}

/// In báo cáo.
pub fn print(report: &Report) {
    let missing_endpoints = report.missing_endpoint_count();
    let missing_tools = report.missing_tool_count();
    for (group, endpoints) in &report.missing_endpoints {
        println!("\n{group}: {} endpoints not covered", endpoints.len());
        for endpoint in endpoints {
            println!("  {} {}", endpoint.method, endpoint.path);
        }
    }
    for (group, tools) in &report.missing_tools {
        println!("\n{group}: {} MCP tools not covered", tools.len());
        for tool in tools {
            println!("  {tool}");
        }
    }
    println!("\nby group (endpoints / tools not covered):");
    let groups: BTreeSet<&String> =
        report.missing_endpoints.keys().chain(report.missing_tools.keys()).collect();
    for group in groups {
        println!(
            "  {group:<16} {:>4} / {:>4}",
            report.missing_endpoints.get(group).map_or(0, Vec::len),
            report.missing_tools.get(group).map_or(0, Vec::len)
        );
    }
    let covered_endpoints = report.endpoints - missing_endpoints;
    let covered_tools = report.tools - missing_tools;
    println!(
        "\nendpoints: {covered_endpoints} / {} covered ({:.1}%)",
        report.endpoints,
        percent(covered_endpoints, report.endpoints)
    );
    println!(
        "MCP tools: {covered_tools} / {} covered ({:.1}%)",
        report.tools,
        percent(covered_tools, report.tools)
    );
}

/// Chạy lệnh. `run_tests`: chạy `cargo xtask test-all` trước.
pub fn run(root: &Path, run_tests: bool) -> Result<Report, String> {
    let directory = root.join(DIRECTORY);
    if run_tests {
        if directory.exists() {
            std::fs::remove_dir_all(&directory)
                .map_err(|error| format!("cannot clear {}: {error}", directory.display()))?;
        }
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        let status = Command::new(&cargo)
            .args(["xtask", "test-all"])
            .env(VARIABLE, &directory)
            .current_dir(root)
            .status()
            .map_err(|error| format!("cannot run `{cargo} xtask test-all`: {error}"))?;
        // Test đỏ vẫn báo độ phủ (phần đã chạy), nhưng nói rõ.
        if !status.success() {
            eprintln!(
                "coverage-api: some tests failed — the numbers below miss what they would have covered"
            );
        }
    }
    let read = |name: &str| -> Result<Value, String> {
        let path = root.join(name);
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        serde_json::from_str(&text).map_err(|error| format!("{name}: {error}"))
    };
    let openapi = read("docs/openapi.json")?;
    let manifest = read("docs/mcp-tools.json")?;
    Ok(compare(&endpoints(&openapi), &tools(&manifest), &recorded(&directory)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bao_dung_endpoint_va_tool_chua_phu() {
        let openapi = json!({"paths": {
            "/api/v1/notes": {"get": {}, "post": {}},
            "/api/v1/notes/{id}": {"get": {}},
            "/health": {"get": {}},
        }});
        let manifest = json!({"tools": [{"name": "notes.list"}, {"name": "notes.create"}]});
        let recorded: BTreeSet<String> =
            ["http GET /api/v1/notes", "http GET /api/v1/notes/{id}", "tool notes.list"]
                .into_iter()
                .map(str::to_owned)
                .collect();
        let report = compare(&endpoints(&openapi), &tools(&manifest), &recorded);
        assert_eq!(report.endpoints, 4);
        assert_eq!(
            report.missing_endpoints["notes"],
            vec![Endpoint { method: "POST".into(), path: "/api/v1/notes".into() }]
        );
        assert_eq!(report.missing_endpoints["core"][0].path, "/health");
        assert_eq!(report.missing_tools["notes"], vec!["notes.create".to_owned()]);
    }
}
