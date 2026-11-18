//! `cargo xtask api-diff [--base <ref>]`: chặn thay đổi phá vỡ của `/api/v1` (D-059, đặc tả 31 A1).
//!
//! So `docs/openapi.json` trên đĩa với bản ở tag phát hành gần nhất (tổ tiên của `HEAD`). Mỗi thay
//! đổi phá vỡ có một mã ổn định (`operation.removed`, `response.property_removed`…) và một câu nói
//! cách làm đúng. Thay đổi không phá vỡ in `note:`; thay đổi có thể làm client chặt vỡ in `warning:`.
//!
//! Cách so:
//! - Operation ghép theo (method, đường dẫn với tên tham số thay bằng `{}`): đổi tên `{id}` không
//!   phá client.
//! - Schema so theo **hình dạng**, giải `$ref` hai bên: utoipa đổi tên component không phá client.
//!   `$ref` vòng lặp dừng nhờ tập cặp (ref cũ, ref mới, chiều) đã thăm.
//! - Hai chiều: *request* (bản mới phải nhận mọi thứ bản cũ nhận) và *response* (bản mới chỉ trả thứ
//!   bản cũ đã hứa).
//! - Nullable hai cách viết (`type: [X, "null"]`, `oneOf: [X, {type: null}]`) là một; `allOf` gộp.
//! - Lỗi (4xx, 5xx, `default`): `application/json` và `application/problem+json` là một (B4).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;
use std::path::Path;
use std::process::Command;

use chrono::NaiveDate;
use serde_json::Value;

mod schema;
mod tools;
use schema::*;
pub use tools::*;

/// File ngoại lệ có chủ đích, ở gốc `xtask/`.
const ALLOW_FILE: &str = "xtask/api-breaking-allow.json";

const METHODS: [&str; 8] = ["get", "put", "post", "delete", "options", "head", "patch", "trace"];

/// Mức của một phát hiện.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Breaking,
    Warning,
    Note,
}

/// Một thay đổi tìm thấy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub level: Level,
    /// Mã ổn định, vd `response.property_removed`; rỗng với `note`.
    pub rule: &'static str,
    /// Chỗ thay đổi: `GET /api/v1/x 200 $.data.slug`.
    pub at: String,
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.level {
            Level::Breaking => write!(
                f,
                "[api-breaking] {}: {}\n  {}",
                self.rule,
                self.at,
                hint(self.rule, &self.message)
            ),
            Level::Warning => write!(f, "warning: {}: {} — {}", self.rule, self.at, self.message),
            Level::Note => write!(f, "note: {} — {}", self.at, self.message),
        }
    }
}

/// Câu "làm đúng thế nào" cho từng loại phá vỡ.
fn hint(rule: &str, detail: &str) -> String {
    let fix = match rule {
        "operation.removed" => {
            "Keep it working; declare it in Module::deprecations with a sunset date and add the \
             replacement as a new endpoint (CLAUDE.md, architecture rule 6; D-059). It may be \
             removed after the sunset date."
        }
        "parameter.added_required" | "parameter.became_required" => {
            "Clients built for the old version do not send it. Make it optional with a default, or \
             add a new endpoint."
        }
        "parameter.removed" => {
            "The server would silently ignore what clients send. Keep accepting it, or add a new \
             endpoint and deprecate this one."
        }
        "request.body_required" | "request.property_required" => {
            "Clients built for the old version do not send it. Make it optional, or add a new \
             endpoint."
        }
        "request.property_removed" => {
            "The body refuses unknown fields, so clients that send it now fail. Keep accepting it \
             (and ignore it), or add a new endpoint."
        }
        "request.media_type_removed" | "response.media_type_removed" => {
            "Keep serving this media type, or add a new endpoint."
        }
        "request.type_narrowed" | "request.constraint_tightened" => {
            "Values the old version accepted are now refused. Keep accepting them, or add a new \
             endpoint."
        }
        "response.status_removed" => {
            "Clients check this status. Keep returning it, or add a new endpoint."
        }
        "response.property_removed" => {
            "Clients read this field. Keep returning it (mark it deprecated in its description) \
             and add the new field."
        }
        "response.property_optional" => {
            "Clients rely on this field always being there and not null. Keep it required, or add \
             a new field."
        }
        "response.type_changed" => {
            "Clients parse this value as before. Keep its type and format, or add a new field."
        }
        "enum.value_removed" => {
            "Clients send or expect this value. Keep it (mark it deprecated in the description)."
        }
        "schema.composite_changed" => {
            "The variants of a oneOf / anyOf changed in a way that cannot be shown to be safe. \
             Keep the old variants, or add a new endpoint."
        }
        "tool.removed" => {
            "Keep it working; declare it in Module::deprecations (Deprecation::tool) with a sunset \
             date and add the replacement as a new tool, e.g. `<name>_v2` (D-233, D-240). It may be \
             removed after the sunset date."
        }
        "tool.parameter_added_required" | "tool.parameter_became_required" => {
            "Agents built for the old version do not send it. Make it optional, or add a new tool."
        }
        "tool.parameter_removed" => {
            "Tool inputs refuse unknown fields, so agents that send it now fail. Keep accepting it, \
             or add a new tool."
        }
        "tool.parameter_type_changed" | "tool.constraint_tightened" => {
            "Values the old version accepted are now refused. Keep accepting them, or add a new tool."
        }
        "tool.no_longer_read_only" => {
            "Clients call read-only tools without asking the user. Keep it read-only and add a new \
             tool that writes."
        }
        "tool.approval_added" => {
            "Agents running unattended now get `approval_required`. Add a new tool that needs \
             approval instead."
        }
        "tool.result_property_removed" | "tool.result_changed" => {
            "Agents read this result as before. Keep it, or add a new tool."
        }
        _ => "See docs/steps/31-api-contract.md (A1, A6).",
    };
    format!("{detail}. {fix}")
}

/// Kết quả so hai bản.
#[derive(Debug, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn breaking(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|finding| finding.level == Level::Breaking)
    }

    fn push(
        &mut self,
        level: Level,
        rule: &'static str,
        at: impl Into<String>,
        message: impl Into<String>,
    ) {
        let finding = Finding { level, rule, at: at.into(), message: message.into() };
        if !self.findings.contains(&finding) {
            self.findings.push(finding);
        }
    }

    fn breaking_at(
        &mut self,
        rule: &'static str,
        at: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.push(Level::Breaking, rule, at, message);
    }
}

/// Chiều so schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Direction {
    /// Bản mới phải nhận mọi thứ bản cũ nhận.
    Request,
    /// Bản mới chỉ trả thứ bản cũ đã hứa.
    Response,
}

/// So hai tài liệu OpenAPI. `today`: ngày (UTC) để xét sunset.
pub fn diff(old: &Value, new: &Value, today: NaiveDate) -> Report {
    let mut report = Report::default();
    let old_ops = operations(old);
    let new_ops = operations(new);
    for (key, old_op) in &old_ops {
        let label = format!("{} {}", key.0.to_uppercase(), old_op.path);
        match new_ops.get(key) {
            None => {
                if let Some(sunset) = sunset_of(old_op.operation)
                    && sunset <= today
                {
                    report.push(
                        Level::Note,
                        "",
                        label,
                        format!("removed after its sunset date {sunset}"),
                    );
                } else {
                    report.breaking_at(
                        "operation.removed",
                        label,
                        "This operation of the base version is gone",
                    );
                }
            }
            Some(new_op) => {
                let mut compare = Comparison {
                    old_doc: old,
                    new_doc: new,
                    report: &mut report,
                    visited: HashSet::new(),
                };
                compare.operation(&label, old_op.operation, new_op.operation);
            }
        }
    }
    for (key, new_op) in &new_ops {
        if !old_ops.contains_key(key) {
            report.push(
                Level::Note,
                "",
                format!("{} {}", key.0.to_uppercase(), new_op.path),
                "new operation",
            );
        }
    }
    report
}

struct Op<'a> {
    path: &'a str,
    operation: &'a Value,
}

/// Operation trong phạm vi so (`/api/v1/…`, `/health/…`), theo (method, đường dẫn chuẩn hoá).
fn operations(doc: &Value) -> BTreeMap<(String, String), Op<'_>> {
    let mut found = BTreeMap::new();
    let Some(paths) = doc.get("paths").and_then(Value::as_object) else {
        return found;
    };
    for (path, item) in paths {
        if !(path.starts_with("/api/v1/") || path == "/api/v1" || path.starts_with("/health/")) {
            continue;
        }
        for method in METHODS {
            if let Some(operation) = item.get(method) {
                found.insert((method.to_owned(), normalize_path(path)), Op { path, operation });
            }
        }
    }
    found
}

/// `/a/{id}/b/{name}` → `/a/{}/b/{}`.
fn normalize_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut inside = false;
    for c in path.chars() {
        match c {
            '{' => {
                inside = true;
                out.push_str("{}");
            }
            '}' => inside = false,
            _ if inside => {}
            _ => out.push(c),
        }
    }
    out
}

/// Ngày sunset của operation deprecated: `x-sunset`, hay câu "It will be removed on YYYY-MM-DD."
/// trong `description` (bản cũ chưa có extension).
fn sunset_of(operation: &Value) -> Option<NaiveDate> {
    if operation.get("deprecated").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    if let Some(date) = operation.get("x-sunset").and_then(Value::as_str) {
        return NaiveDate::parse_from_str(date, "%Y-%m-%d").ok();
    }
    let description = operation.get("description").and_then(Value::as_str)?;
    let (_, rest) = description.split_once("removed on ")?;
    NaiveDate::parse_from_str(rest.get(..10)?, "%Y-%m-%d").ok()
}

/// Một ngoại lệ có chủ đích (`xtask/api-breaking-allow.json`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allowed {
    pub base: String,
    pub rule: String,
    pub at: String,
    pub decision: Option<String>,
}

/// Bỏ phát hiện phá vỡ có ngoại lệ cho đúng `base`; lỗi khi ngoại lệ thiếu `decision`.
pub fn apply_allowlist(
    report: &mut Report,
    allowed: &[Allowed],
    base: &str,
) -> Result<usize, String> {
    if let Some(entry) =
        allowed.iter().find(|entry| entry.decision.as_deref().is_none_or(str::is_empty))
    {
        return Err(format!(
            "{ALLOW_FILE}: the entry for `{} {}` has no `decision` (D-NNN explaining why the break is intended)",
            entry.rule, entry.at
        ));
    }
    let before = report.findings.len();
    report.findings.retain(|finding| {
        !(finding.level == Level::Breaking
            && allowed.iter().any(|entry| {
                entry.base == base && entry.rule == finding.rule && entry.at == finding.at
            }))
    });
    Ok(before - report.findings.len())
}

/// Lệnh `cargo xtask api-diff`.
pub fn run(root: &Path, base: Option<&str>, today: NaiveDate) -> Result<bool, String> {
    let base = match base {
        Some(base) => base.to_owned(),
        None => match last_release_tag(root)? {
            Some(tag) => tag,
            None => {
                println!("api-diff: no release tag found; nothing to compare");
                return Ok(true);
            }
        },
    };
    let Ok(old_text) = git(root, &["show", &format!("{base}:docs/openapi.json")]) else {
        println!("api-diff: no docs/openapi.json in {base}; nothing to compare");
        return Ok(true);
    };
    let old: Value = serde_json::from_str(&old_text)
        .map_err(|error| format!("docs/openapi.json in {base} is not JSON: {error}"))?;
    let new_text = std::fs::read_to_string(root.join("docs/openapi.json"))
        .map_err(|error| format!("cannot read docs/openapi.json: {error}"))?;
    let new: Value = serde_json::from_str(&new_text)
        .map_err(|error| format!("docs/openapi.json is not JSON: {error}"))?;
    let mut report = diff(&old, &new, today);
    match git(root, &["show", &format!("{base}:docs/mcp-tools.json")]) {
        Err(_) => println!("api-diff: no MCP tool manifest in {base}; nothing to compare"),
        Ok(old_text) => {
            let old: Value = serde_json::from_str(&old_text)
                .map_err(|error| format!("docs/mcp-tools.json in {base} is not JSON: {error}"))?;
            let new_text = std::fs::read_to_string(root.join("docs/mcp-tools.json"))
                .map_err(|error| format!("cannot read docs/mcp-tools.json: {error}"))?;
            let new: Value = serde_json::from_str(&new_text)
                .map_err(|error| format!("docs/mcp-tools.json is not JSON: {error}"))?;
            report.findings.extend(diff_tools(&old, &new, today).findings);
        }
    }
    let allowed = read_allowlist(root)?;
    let skipped = apply_allowlist(&mut report, &allowed, &base)?;
    for finding in report.findings.iter().filter(|finding| finding.level != Level::Breaking) {
        println!("{finding}");
    }
    let breaking: Vec<&Finding> = report.breaking().collect();
    for finding in &breaking {
        eprintln!("{finding}");
    }
    if breaking.is_empty() {
        println!(
            "api-diff: no breaking change to /api/v1 or the MCP tools since {base}{}",
            if skipped > 0 {
                format!(" ({skipped} allowed in {ALLOW_FILE})")
            } else {
                String::new()
            }
        );
        Ok(true)
    } else {
        eprintln!(
            "api-diff: {} breaking change(s) to /api/v1 or the MCP tools since {base}. An intended \
             break needs an entry in {ALLOW_FILE} with the decision that explains it.",
            breaking.len()
        );
        Ok(false)
    }
}

/// Tag phát hành gần nhất là tổ tiên của `HEAD` (`git describe`, không phải tag mới nhất của repo:
/// tag gắn ở commit `release:`, có thể không nằm trên nhánh đang làm). `None` khi chưa có tag nào;
/// lỗi khi repo là shallow clone (không tìm đúng được).
pub fn last_release_tag(root: &Path) -> Result<Option<String>, String> {
    let shallow = git(root, &["rev-parse", "--is-shallow-repository"])?;
    if shallow.trim() == "true" {
        return Err("the repository is a shallow clone, so the last release tag cannot be found; \
                    fetch the full history (`git fetch --unshallow --tags`, or `fetch-depth: 0` in \
                    actions/checkout)"
            .to_owned());
    }
    Ok(git(root, &["describe", "--tags", "--abbrev=0", "--match", "v[0-9]*.[0-9]*.[0-9]*", "HEAD"])
        .ok()
        .map(|tag| tag.trim().to_owned()))
}

fn read_allowlist(root: &Path) -> Result<Vec<Allowed>, String> {
    let path = root.join(ALLOW_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {ALLOW_FILE}: {error}"))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("{ALLOW_FILE} is not a list of entries: {error}"))
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|error| format!("git printed non-UTF-8: {error}"))
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

#[cfg(test)]
mod tests;
