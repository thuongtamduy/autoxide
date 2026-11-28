//! Luật của hợp đồng module (D-20261126-module-manifest, D-20261127-transition-paths):
//!
//! - `module-contract-legacy`: số hàm khai báo cũ của trait `Module` (mọi hàm trừ `manifest`, `configure`, `start`,
//!   `dormant`) mà module còn cài, đếm theo file, theo [`BASELINE`] chỉ được giảm. Module mới khai bằng `Manifest`
//!   và đăng ký trong `start`.
//! - `module-contract-mixed`: một `impl Module` đã cài `manifest` thì không cài hàm cũ nào (kernel chỉ đọc một
//!   đường; hàm cũ cài thêm sẽ lặng lẽ thắng manifest). Không có danh sách chỗ cũ.

use std::fs;

use crate::boundaries::{Violation, rust_files};
use crate::workspace::{Role, Workspace};

/// Chỗ cũ chờ chuyển của `module-contract-legacy` (cùng định dạng `security-baseline.txt`).
pub const BASELINE: &str = "xtask/module-contract-legacy.txt";

/// Hàm của trait `Module` thuộc đường mới; mọi hàm khác là đường cũ.
const KEPT: &[&str] = &["manifest", "configure", "start", "dormant"];

/// Hàm cũ: `fn <tên>(` trong thân `impl Module for` (tên trong danh sách hàm của trait ở `crates/kernel`).
const LEGACY: &[&str] = &[
    "name",
    "description",
    "migrations",
    "upgrades",
    "tenant_settings",
    "org_settings",
    "seed_tenant",
    "sample_data",
    "retained_tables",
    "retention",
    "control_plane_tables",
    "features",
    "limits",
    "always_enabled",
    "stability",
    "essential",
    "records",
    "depends_on",
    "permissions",
    "events",
    "resource_events",
    "subscriptions",
    "jobs",
    "schedules",
    "deprecations",
    "tools",
    "mcp_resources",
    "prompts",
    "checks",
    "navigation",
    "reason_lists",
    "overridable_texts",
    "custom_field_targets",
    "tasks",
    "body_limits",
    "translations",
    "client_config",
    "routes",
    "endpoints",
    "facts",
    "relocation",
    "analytics_events",
    "funnels",
    "automation_templates",
    "sandbox_copy",
    "promotable",
    "sync_resources",
    "blocks",
];

/// Vi phạm của cả hai luật trong mã không phải test của mọi module (trừ ví dụ ngoài repo, có gate riêng).
pub fn check(workspace: &Workspace) -> Result<Vec<Violation>, String> {
    let mut violations = Vec::new();
    for package in workspace.packages.iter().filter(|package| package.role == Role::Module) {
        let src = workspace.root.join(&package.dir).join("src");
        for file in rust_files(&src)? {
            // Module giả trong test dựng nhanh bằng hàm cũ được: luật chỉ áp cho mã chạy thật.
            if file.rsplit('/').next().unwrap_or(&file).contains("test") || file.contains("/tests/")
            {
                continue;
            }
            let text = fs::read_to_string(workspace.root.join(&package.dir).join(&file))
                .map_err(|error| format!("{file}: {error}"))?;
            violations
                .extend(file_violations(&format!("{}/{file}", package.dir), &without_tests(&text)));
        }
    }
    Ok(violations)
}

/// Bỏ thân các `#[cfg(test)] mod x { … }` (giữ số dòng: dòng bị bỏ thành dòng trống).
fn without_tests(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut kept = Vec::with_capacity(lines.len());
    let mut index = 0;
    while index < lines.len() {
        let opens_test_module = lines[index].trim() == "#[cfg(test)]"
            && lines.get(index + 1).is_some_and(|next| {
                let next = next.trim_start();
                next.contains("mod ") && next.trim_end().ends_with('{')
            });
        if !opens_test_module {
            kept.push(lines[index]);
            index += 1;
            continue;
        }
        kept.push("");
        index += 1;
        let mut depth = 0;
        while index < lines.len() {
            depth += brace_delta(lines[index]);
            kept.push("");
            index += 1;
            if depth <= 0 {
                break;
            }
        }
    }
    kept.join("\n")
}

/// Vi phạm trong một file: từng `impl Module for` (thân đến dấu `}` cùng mức).
fn file_violations(path: &str, text: &str) -> Vec<Violation> {
    let lines: Vec<&str> = text.lines().collect();
    let mut violations = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if !is_module_impl(lines[index]) {
            index += 1;
            continue;
        }
        let mut depth = 0i32;
        let mut legacy = Vec::new();
        let mut has_manifest = false;
        let mut opened = false;
        while index < lines.len() {
            let line = lines[index];
            // Chỉ hàm ở mức thân impl (depth 1 trước dòng này) mới là hàm của trait.
            if depth == 1
                && let Some(name) = fn_name(line)
            {
                if name == "manifest" {
                    has_manifest = true;
                } else if LEGACY.contains(&name) && !KEPT.contains(&name) {
                    legacy.push((index, name));
                }
            }
            depth += brace_delta(line);
            opened |= depth > 0;
            index += 1;
            // Hết thân impl (kể cả `impl Module for X {}` trên một dòng).
            if depth <= 0 && (opened || line.contains('{')) {
                break;
            }
        }
        for (line, name) in legacy {
            let location = format!("{path}:{}", line + 1);
            violations.push(if has_manifest {
                Violation {
                    rule: "module-contract-mixed",
                    location,
                    message: format!(
                        "`Module::{name}` next to `Module::manifest`: declare it in the manifest (or register it on \
                         `StartContext` in `start`) and remove the old function"
                    ),
                }
            } else {
                Violation {
                    rule: "module-contract-legacy",
                    location,
                    message: format!(
                        "`Module::{name}` is the old way to declare a module: implement `Module::manifest` and \
                         register runtime parts on `StartContext` in `start` (docs/MODULE_GUIDE.md)"
                    ),
                }
            });
        }
    }
    violations
}

fn is_module_impl(line: &str) -> bool {
    let line = line.trim_start();
    !line.starts_with("//")
        && line.starts_with("impl ")
        && (line.contains(" Module for ") || line.contains("::Module for "))
}

/// Tên hàm nếu dòng mở một `fn` (bỏ `pub`, `async`).
fn fn_name(line: &str) -> Option<&str> {
    let line = line.trim_start();
    if line.starts_with("//") {
        return None;
    }
    let rest = line.strip_prefix("async fn ").or_else(|| line.strip_prefix("fn "))?;
    let end = rest.find(['(', '<'])?;
    Some(&rest[..end])
}

/// Số `{` trừ số `}` của dòng, bỏ phần chú thích `//` và chuỗi đơn giản.
fn brace_delta(line: &str) -> i32 {
    let code = line.split("//").next().unwrap_or_default();
    let mut delta = 0;
    let mut in_string = false;
    let mut previous = ' ';
    for character in code.chars() {
        match character {
            '"' if previous != '\\' => in_string = !in_string,
            '{' if !in_string => delta += 1,
            '}' if !in_string => delta -= 1,
            _ => {}
        }
        previous = character;
    }
    delta
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY_MODULE: &str = "\
impl Module for Old {
    fn name(&self) -> &'static str {
        \"old\"
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let inner = || { 1 };
        Ok(())
    }

    fn tools(&self) -> Vec<Tool> {
        Vec::new()
    }
}

impl Other for Old {
    fn name(&self) -> &'static str {
        \"not a module\"
    }
}
";

    #[test]
    fn ham_cu_duoc_dem_theo_dong() {
        let found = file_violations("modules/old/src/lib.rs", LEGACY_MODULE);
        let rules: Vec<(&str, &str)> =
            found.iter().map(|violation| (violation.rule, violation.location.as_str())).collect();
        assert_eq!(
            rules,
            [
                ("module-contract-legacy", "modules/old/src/lib.rs:2"),
                ("module-contract-legacy", "modules/old/src/lib.rs:11"),
            ]
        );
    }

    #[test]
    fn manifest_cung_ham_cu_la_tron_hai_duong() {
        let text = "\
impl autoxide_kernel::module::Module for New {
    fn manifest(&self) -> Manifest {
        Manifest::new(\"new\", \"New.\")
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![]
    }
}
";
        let found = file_violations("modules/new/src/lib.rs", text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].rule, "module-contract-mixed");
        assert_eq!(found[0].location, "modules/new/src/lib.rs:6");
    }

    #[test]
    fn module_gia_trong_mo_dun_test_duoc_bo_qua() {
        let text = format!(
            "#[cfg(test)]\nmod tests;\n{LEGACY_MODULE}\n#[cfg(test)]\nmod tests {{\n{LEGACY_MODULE}}}\n"
        );
        let found = file_violations("modules/old/src/lib.rs", &without_tests(&text));
        let lines: Vec<&str> = found.iter().map(|violation| violation.location.as_str()).collect();
        assert_eq!(lines, ["modules/old/src/lib.rs:4", "modules/old/src/lib.rs:13"]);
    }

    #[test]
    fn module_kieu_moi_khong_vi_pham() {
        let text = "\
impl Module for New {
    fn manifest(&self) -> Manifest {
        Manifest::new(\"new\", \"New.\")
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.tools(tools::all())?;
        Ok(())
    }
}
";
        assert!(file_violations("modules/new/src/lib.rs", text).is_empty());
    }
}
