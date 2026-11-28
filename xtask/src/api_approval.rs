//! Lệnh rủi ro phải chờ người duyệt khi AI agent gọi (bước 71 G4, D-20261128-agent-approval-audit).
//!
//! Hai luật, kiểm trên file sinh:
//!
//! - `api-approval` (`docs/openapi.json`): mọi `DELETE`, và mọi `POST` / `PUT` / `PATCH` mà đoạn cuối của đường dẫn là
//!   một động từ chạm tiền hay không hoàn tác được ([`RISKY`]: `refunds`, `payouts`, `void`, `erase`, `issue`…), phải
//!   mang `x-approval: "agent"` (`approval = "agent"` trên `#[endpoint]`). Endpoint công khai (`x-public`) và SCIM
//!   (IdP gọi bằng token SCIM, không phải agent) đứng ngoài.
//! - `mcp-approval` (`docs/mcp-tools.json`): tool khai `destructive` phải khai `requires_approval_for_agent`.
//!
//! Ngoại lệ có chủ đích ghi ở [`ALLOW_FILE`], mỗi dòng `<luật> <METHOD|TOOL> <đường dẫn|tên tool> # lý do` — lý do
//! bắt buộc. Dòng không còn khớp vi phạm nào chỉ được cảnh báo (nhắc xoá).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::boundaries::Violation;

const OPENAPI_FILE: &str = "docs/openapi.json";
const MCP_FILE: &str = "docs/mcp-tools.json";
pub const ALLOW_FILE: &str = "xtask/approval-allow.txt";
const SCIM_PREFIX: &str = "/api/v1/auth/scim/v2/";
const RULES: &[&str] = &["api-approval", "mcp-approval"];

/// Đoạn cuối đường dẫn của lệnh ghi (không phải `DELETE`) bị coi là rủi ro: chạm tiền, xoá hay không hoàn tác được.
const RISKY: &[&str] = &[
    "adjust",
    "adjustments",
    "cancel",
    "close",
    "delete",
    "erase",
    "fulfil",
    "issue",
    "mark-paid",
    "merge",
    "move",
    "pay",
    "payments",
    "payouts",
    "purge",
    "refunds",
    "relocations",
    "replace",
    "retire",
    "reverse",
    "send",
    "suspend",
    "void",
];

pub struct Report {
    pub violations: Vec<Violation>,
    pub warnings: Vec<String>,
}

type Key = (&'static str, String, String);

pub fn check(root: &Path) -> Result<Report, String> {
    let openapi = read_json(root, OPENAPI_FILE)?;
    let tools = read_json(root, MCP_FILE)?;
    let allow_text = fs::read_to_string(root.join(ALLOW_FILE)).unwrap_or_default();
    let allowed = parse_allow(&allow_text)?;
    let mut found = operations(&openapi);
    found.extend(destructive_tools(&tools));
    let mut used = BTreeSet::new();
    let mut violations = Vec::new();
    for (key, message) in found {
        if allowed.contains(&key) {
            used.insert(key);
            continue;
        }
        let (rule, method, path) = key;
        let location = if method == "TOOL" {
            format!("{MCP_FILE} {path}")
        } else {
            format!("{OPENAPI_FILE} {method} {path}")
        };
        violations.push(Violation { rule, location, message });
    }
    let warnings = allowed
        .difference(&used)
        .map(|(rule, method, path)| {
            format!(
                "warning: {ALLOW_FILE}: `{rule} {method} {path}` no longer matches a violation; remove the line"
            )
        })
        .collect();
    Ok(Report { violations, warnings })
}

fn read_json(root: &Path, file: &str) -> Result<Value, String> {
    let text = fs::read_to_string(root.join(file)).map_err(|error| format!("{file}: {error}"))?;
    serde_json::from_str(&text).map_err(|error| format!("{file}: {error}"))
}

fn parse_allow(text: &str) -> Result<BTreeSet<Key>, String> {
    let mut entries = BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        let content = line.split('#').next().unwrap_or("").trim();
        if content.is_empty() {
            continue;
        }
        let fields: Vec<&str> = content.split_whitespace().collect();
        let [rule, method, path] = fields.as_slice() else {
            return Err(format!(
                "{ALLOW_FILE}:{}: expected `<rule> <METHOD|TOOL> <path|tool> # reason`",
                index + 1
            ));
        };
        let Some(rule) = RULES.iter().find(|known| *known == rule) else {
            return Err(format!("{ALLOW_FILE}:{}: unknown rule `{rule}`", index + 1));
        };
        let reason = line.split_once('#').map_or("", |(_, reason)| reason.trim());
        if reason.is_empty() {
            return Err(format!("{ALLOW_FILE}:{}: give the reason after `#`", index + 1));
        }
        entries.insert((*rule, (*method).to_owned(), (*path).to_owned()));
    }
    Ok(entries)
}

/// Lệnh rủi ro của REST chưa mang `x-approval: "agent"`.
fn operations(document: &Value) -> Vec<(Key, String)> {
    let Some(paths) = document.get("paths").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (path, item) in paths {
        if !path.starts_with("/api/v1/") || path.starts_with(SCIM_PREFIX) {
            continue;
        }
        let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        for method in ["post", "put", "patch", "delete"] {
            let Some(operation) = item.get(method) else { continue };
            if method != "delete" && !RISKY.contains(&last) {
                continue;
            }
            if operation.get("x-public").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            if operation.get("x-approval").and_then(Value::as_str) == Some("agent") {
                continue;
            }
            found.push((
                ("api-approval", method.to_uppercase(), path.clone()),
                "deletes, touches money or cannot be undone, but an AI agent's call runs without a person's \
                 approval; add `approval = \"agent\"` (or `(\"x-approval\" = json!(\"agent\"))` in \
                 `extensions`), or list it in xtask/approval-allow.txt with the reason"
                    .to_owned(),
            ));
        }
    }
    found
}

/// Tool `destructive` chưa khai `requires_approval_for_agent`.
fn destructive_tools(document: &Value) -> Vec<(Key, String)> {
    let Some(tools) = document.get("tools").and_then(Value::as_array) else {
        return Vec::new();
    };
    let flag = |tool: &Value, name: &str| {
        tool.pointer(&format!("/annotations/{name}")).and_then(Value::as_bool) == Some(true)
    };
    tools
        .iter()
        .filter(|tool| flag(tool, "destructive") && !flag(tool, "requires_approval_for_agent"))
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .map(|name| {
            (
                ("mcp-approval", "TOOL".to_owned(), name.to_owned()),
                "is destructive but an AI agent's call runs without a person's approval; add \
                 `.requires_approval_for_agent()` (and `approval = \"agent\"` on its REST endpoint), or list \
                 it in xtask/approval-allow.txt with the reason"
                    .to_owned(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn delete_va_dong_tu_rui_ro_can_duyet() {
        let document = json!({"paths": {
            "/api/v1/x/{id}": {"delete": {}, "patch": {}},
            "/api/v1/x/{id}/refunds": {"post": {}},
            "/api/v1/x/{id}/approved": {"delete": {"x-approval": "agent"}},
            "/api/v1/x/public/{id}/cancel": {"post": {"x-public": true}},
            "/api/v1/auth/scim/v2/Users/{id}": {"delete": {}},
        }});
        let keys: Vec<_> = operations(&document).into_iter().map(|(key, _)| key).collect();
        assert_eq!(
            keys,
            [
                ("api-approval", "DELETE".to_owned(), "/api/v1/x/{id}".to_owned()),
                ("api-approval", "POST".to_owned(), "/api/v1/x/{id}/refunds".to_owned()),
            ]
        );
    }

    #[test]
    fn tool_destructive_can_duyet() {
        let document = json!({"tools": [
            {"name": "x.a", "annotations": {"destructive": true, "requires_approval_for_agent": false}},
            {"name": "x.b", "annotations": {"destructive": true, "requires_approval_for_agent": true}},
            {"name": "x.c", "annotations": {"destructive": false, "requires_approval_for_agent": false}},
        ]});
        let names: Vec<_> =
            destructive_tools(&document).into_iter().map(|((_, _, name), _)| name).collect();
        assert_eq!(names, ["x.a"]);
    }

    #[test]
    fn dong_cho_phep_phai_co_ly_do() {
        assert!(parse_allow("api-approval DELETE /api/v1/x # ").is_err());
        assert!(parse_allow("api-approval DELETE /api/v1/x").is_err());
        assert!(parse_allow("nope DELETE /api/v1/x # r").is_err());
        assert_eq!(parse_allow("mcp-approval TOOL x.a # vì sao").unwrap().len(), 1);
    }
}
