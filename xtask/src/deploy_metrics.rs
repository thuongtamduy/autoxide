//! Dashboard và luật cảnh báo trong `deploy/` khớp code (bước 39, A4).
//!
//! Luật `deploy-metrics`:
//! - mọi tên số đo dùng trong biểu thức (`expr`) của `deploy/prometheus/*.yml` và
//!   `deploy/grafana/*.json` phải được code ghi bằng `metrics::counter!` / `gauge!` / `histogram!`
//!   (histogram được thêm đuôi `_bucket`, `_sum`, `_count`) — đổi tên số đo mà quên sửa dashboard thì
//!   gate đỏ thay vì bảng trống lúc có sự cố;
//! - mỗi luật cảnh báo (`- alert:`) có `action:` (câu "làm gì").

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::boundaries::Violation;

const RULES_DIR: &str = "deploy/prometheus";
const DASHBOARDS_DIR: &str = "deploy/grafana";
const MACROS: &[&str] = &["counter!(\"", "gauge!(\"", "histogram!(\""];

pub fn check(root: &Path) -> Result<Vec<Violation>, String> {
    let mut known = BTreeSet::new();
    for file in crate::lengths::rust_files(root)? {
        if let Ok(text) = fs::read_to_string(root.join(&file)) {
            known.extend(metric_names(&text));
        }
    }
    let mut violations = Vec::new();
    for (dir, extension) in [(RULES_DIR, "yml"), (DASHBOARDS_DIR, "json")] {
        let Ok(entries) = fs::read_dir(root.join(dir)) else {
            continue;
        };
        let mut files: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|found| found == extension))
            .collect();
        files.sort();
        for path in files {
            let text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
            let name = format!("{dir}/{}", path.file_name().unwrap_or_default().to_string_lossy());
            let expressions = if extension == "yml" {
                violations.extend(check_actions(&name, &text));
                yaml_expressions(&text)
            } else {
                json_expressions(&text).map_err(|error| format!("{name}: {error}"))?
            };
            violations.extend(check_expressions(&name, &expressions, &known));
        }
    }
    Ok(violations)
}

/// Tên số đo ghi trong một file nguồn: chuỗi ngay sau `metrics::<macro>!("`.
fn metric_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for macro_name in MACROS {
        let pattern = format!("metrics::{macro_name}");
        for (position, _) in text.match_indices(&pattern) {
            let rest = &text[position + pattern.len()..];
            if let Some(end) = rest.find('"') {
                names.push(rest[..end].to_owned());
            }
        }
    }
    names
}

/// Biểu thức của file luật cảnh báo: nội dung từ `expr:` tới khoá kế tiếp của luật.
fn yaml_expressions(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("expr:") {
            if let Some(done) = current.take() {
                found.push(done);
            }
            current = Some(rest.trim_start_matches([' ', '|', '>']).to_owned());
        } else if let Some(expression) = current.as_mut() {
            let is_key = trimmed.split_once(':').is_some_and(|(key, _)| {
                !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            }) || trimmed.starts_with("- ");
            if is_key {
                found.push(current.take().unwrap_or_default());
            } else {
                expression.push(' ');
                expression.push_str(trimmed);
            }
        }
    }
    found.extend(current);
    found
}

/// Biểu thức của dashboard: mọi giá trị `expr` trong JSON.
fn json_expressions(text: &str) -> Result<Vec<String>, String> {
    fn walk(value: &serde_json::Value, found: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, item) in map {
                    match (key.as_str(), item) {
                        ("expr", serde_json::Value::String(expression)) => {
                            found.push(expression.clone());
                        }
                        _ => walk(item, found),
                    }
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|item| walk(item, found)),
            _ => {}
        }
    }
    let value: serde_json::Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let mut found = Vec::new();
    walk(&value, &mut found);
    Ok(found)
}

/// Tên số đo trong một biểu thức PromQL: định danh có dấu `_`, không phải hàm (theo sau là `(`),
/// sau khi bỏ bộ chọn nhãn `{…}`, `by (…)`, `without(…)` và khoảng `[…]`.
fn selectors(expression: &str) -> Vec<String> {
    let mut text = String::with_capacity(expression.len());
    let mut depth = 0usize;
    for c in expression.chars() {
        match c {
            '{' | '[' => depth += 1,
            '}' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => text.push(c),
            _ => {}
        }
    }
    for keyword in ["without", "by"] {
        while let Some(start) =
            text.find(&format!("{keyword}(")).or_else(|| text.find(&format!("{keyword} (")))
        {
            let Some(close) = text[start..].find(')') else {
                break;
            };
            text.replace_range(start..start + close + 1, " ");
        }
    }
    let mut names = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index].is_ascii_lowercase() || chars[index] == '_' {
            let start = index;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || chars[index] == '_')
            {
                index += 1;
            }
            let word: String = chars[start..index].iter().collect();
            let after = chars[index..].iter().find(|c| !c.is_whitespace());
            let preceded_by_dollar = start > 0 && chars[start - 1] == '$';
            if word.contains('_') && after != Some(&'(') && !preceded_by_dollar {
                names.push(word);
            }
        } else {
            index += 1;
        }
    }
    names
}

fn check_expressions(
    file: &str,
    expressions: &[String],
    known: &BTreeSet<String>,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut seen = BTreeSet::new();
    for expression in expressions {
        for name in selectors(expression) {
            let base = ["_bucket", "_sum", "_count"]
                .iter()
                .find_map(|suffix| name.strip_suffix(suffix))
                .unwrap_or(&name);
            if !known.contains(&name) && !known.contains(base) && seen.insert(name.clone()) {
                violations.push(Violation {
                    rule: "deploy-metrics",
                    location: file.to_owned(),
                    message: format!(
                        "metric `{name}` is not recorded anywhere in the code (renamed or removed?) — \
                         fix the expression"
                    ),
                });
            }
        }
    }
    violations
}

fn check_actions(file: &str, text: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut alert: Option<String> = None;
    let mut has_action = true;
    let finish = |alert: Option<String>, has_action: bool, violations: &mut Vec<Violation>| {
        if let Some(name) = alert
            && !has_action
        {
            violations.push(Violation {
                rule: "deploy-metrics",
                location: file.to_owned(),
                message: format!("alert `{name}` has no `action:` annotation saying what to do"),
            });
        }
    };
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(name) = trimmed.strip_prefix("- alert:") {
            finish(alert.take(), has_action, &mut violations);
            alert = Some(name.trim().to_owned());
            has_action = false;
        } else if trimmed.starts_with("action:") {
            has_action = true;
        }
    }
    finish(alert, has_action, &mut violations);
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canh_bao_va_dashboard_chi_dung_so_do_co_that() {
        let known: BTreeSet<String> =
            metric_names("metrics::counter!(\"http_requests_total\", &labels);\nmetrics::histogram!(\"http_request_duration_seconds\")")
                .into_iter()
                .collect();
        let rules = "groups:\n  - name: a\n    rules:\n      - alert: A\n        expr: |\n          \
                     sum without(instance, status) (rate(http_requests_total{status=~\"5..\"}[10m]))\n            \
                     > 0.02\n        for: 10m\n        annotations:\n          action: \"do it\"\n      \
                     - alert: B\n        expr: histogram_quantile(0.95, rate(http_request_duration_seconds_bucket[5m])) + jobs_gone\n";
        let found = check_expressions("rules.yml", &yaml_expressions(rules), &known);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].message.contains("`jobs_gone`"));
        let actions = check_actions("rules.yml", rules);
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert!(actions[0].message.contains("`B`"));

        let dashboard = r#"{"panels":[{"targets":[{"expr":"sum by (le, route) (rate(http_request_duration_seconds_bucket{database=~\"$database\"}[5m]))"}]}]}"#;
        let expressions = json_expressions(dashboard).unwrap();
        assert!(check_expressions("d.json", &expressions, &known).is_empty());
    }
}
