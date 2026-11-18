//! Nhãn số đo không được theo khách hay người (bước 39, luật code về số đo): mỗi giá trị nhãn là
//! một chuỗi số đo sống mãi trong bộ nhớ của tiến trình và của Prometheus — nhãn `tenant_id` làm số
//! chuỗi tăng theo số khách. Tra theo khách đi qua log, trace, CLI và số đo dùng (`plans`).
//!
//! Luật `metric-labels`: trong lời gọi `metrics::counter!` / `gauge!` / `histogram!` (gồm cả các dòng
//! sau dòng mở, tới dấu `;`), không có khoá nhãn nào trong [`FORBIDDEN`].

use std::fs;
use std::path::Path;

use crate::boundaries::Violation;

/// Khoá nhãn bị cấm.
const FORBIDDEN: &[&str] = &["tenant", "tenant_id", "org_id", "user_id"];
/// Macro số đo.
const MACROS: &[&str] = &["counter!(", "gauge!(", "histogram!("];

pub fn check(root: &Path) -> Result<Vec<Violation>, String> {
    let mut violations = Vec::new();
    for file in crate::lengths::rust_files(root)? {
        if file.starts_with("xtask/") {
            continue;
        }
        let Ok(text) = fs::read_to_string(root.join(&file)) else {
            continue;
        };
        violations.extend(check_text(&file, &text));
    }
    Ok(violations)
}

/// Phần không đọc đĩa của [`check`], để test được.
fn check_text(file: &str, text: &str) -> Vec<Violation> {
    let lines: Vec<&str> = text.lines().collect();
    let mut violations = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !MACROS.iter().any(|name| line.contains(&format!("metrics::{name}"))) {
            continue;
        }
        // Lời gọi có thể xuống dòng: xét tới dòng có `;` (tối đa 8 dòng).
        let mut call = String::new();
        for next in lines.iter().skip(index).take(8) {
            call.push_str(next);
            if next.contains(';') {
                break;
            }
        }
        for key in FORBIDDEN {
            if call.contains(&format!("\"{key}\" =>")) || call.contains(&format!("(\"{key}\",")) {
                violations.push(Violation {
                    rule: "metric-labels",
                    location: format!("{file}:{}", index + 1),
                    message: format!(
                        "metric label `{key}`: one series per tenant or person grows without end — \
                         label only by small fixed sets (module, route, status); find a tenant \
                         through logs, traces or the usage figures instead"
                    ),
                });
            }
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nhan_so_do_khong_co_tenant() {
        let bad = "fn f() {\n    metrics::counter!(\"notes_created_total\",\n        \"tenant_id\" \
                   => id.to_string()).increment(1);\n}\n";
        let found = check_text("modules/notes/src/service.rs", bad);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].location, "modules/notes/src/service.rs:2");

        let good =
            "metrics::counter!(\"notes_created_total\", \"module\" => \"notes\").increment(1);\n";
        assert!(check_text("a.rs", good).is_empty());
        let tuple = "metrics::gauge!(\"x\", &[(\"org_id\", id)]).set(1.0);\n";
        assert_eq!(check_text("a.rs", tuple).len(), 1);
    }
}
