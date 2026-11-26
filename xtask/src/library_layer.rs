//! Luật của lớp kernel thay thư viện (D-20261126-library-isolation, D-20261126-kernel-data-layer):
//!
//! - `wrapped-libraries`: module không khai thư viện đã có lớp của kernel (danh sách ở [`LIBRARIES`]). Module chưa
//!   chuyển xong nằm trong [`BASELINE`], chỉ được giảm.
//! - `private-paths`: mã viết tay không gọi `autoxide_kernel::__private` — đường đó chỉ cho mã do macro của kernel
//!   sinh ra, không thuộc hợp đồng.
//! - `raw-sql`: số chỗ gọi `.sql(` (SQL viết tay của lớp `db`) trong mã không phải test của module, theo
//!   [`BASELINE`] chỉ được giảm: kiểm `:tenant` của nó chỉ là rào phụ, nên mỗi chỗ mới là một quyết định có lý do.

use std::fs;
use std::path::Path;

use crate::boundaries::{Violation, rust_files};
use crate::workspace::{Role, Workspace};

/// Thư viện đã có (hay sẽ có) lớp của kernel; module không khai trực tiếp.
pub const LIBRARIES: &str = "xtask/wrapped-libraries.txt";
/// Chỗ cũ chờ chuyển của `wrapped-libraries` và `raw-sql` (cùng định dạng `security-baseline.txt`).
pub const BASELINE: &str = "xtask/library-baseline.txt";

/// Thư viện trong [`LIBRARIES`]: một tên mỗi dòng, `#` là chú thích.
fn libraries(root: &Path) -> Result<Vec<String>, String> {
    let text = fs::read_to_string(root.join(LIBRARIES))
        .map_err(|error| format!("{LIBRARIES}: {error}"))?;
    Ok(text
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect())
}

/// Vi phạm của `wrapped-libraries` và `raw-sql` (trước khi trừ [`BASELINE`]).
pub fn check(workspace: &Workspace) -> Result<Vec<Violation>, String> {
    let libraries = libraries(&workspace.root)?;
    let mut violations = Vec::new();
    for package in workspace.packages.iter().filter(|package| package.role == Role::Module) {
        let manifest = format!("{}/Cargo.toml", package.dir);
        for dep in package.deps.iter().filter(|dep| libraries.contains(&dep.name)) {
            violations.push(Violation {
                rule: "wrapped-libraries",
                location: manifest.clone(),
                message: format!(
                    "module depends on `{}`, which the kernel wraps: use the kernel layer instead (`autoxide_kernel::db` \
                     for queries; see docs/research/kernel-contract-1-0.md) and remove the dependency",
                    dep.name
                ),
            });
        }
        let src = workspace.root.join(&package.dir).join("src");
        for file in rust_files(&src)? {
            if is_test_file(&file) {
                continue;
            }
            let text = fs::read_to_string(workspace.root.join(&package.dir).join(&file))
                .map_err(|error| format!("{file}: {error}"))?;
            for (index, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if line.contains(".sql(") {
                    violations.push(Violation {
                        rule: "raw-sql",
                        location: format!("{}/{file}:{}", package.dir, index + 1),
                        message: "hand-written SQL: express it with the `db` query builder; if it is measured to \
                                  be slow, record the reason (skill `record-decision`) and add a tenant-isolation \
                                  test"
                            .to_owned(),
                    });
                }
            }
        }
    }
    Ok(violations)
}

/// `private-paths`: `__private` trong mã viết tay của mọi crate trừ kernel (nơi khai đường đó) và crate macro
/// (nơi sinh mã gọi nó). Không có danh sách chỗ cũ.
pub fn check_private_paths(workspace: &Workspace) -> Result<Vec<Violation>, String> {
    let mut violations = Vec::new();
    for package in &workspace.packages {
        if matches!(package.role, Role::Kernel | Role::Macros | Role::Tool) {
            continue;
        }
        let dir = workspace.root.join(&package.dir);
        for sub in ["src", "tests", "benches"] {
            let base = dir.join(sub);
            if !base.is_dir() {
                continue;
            }
            for file in rust_files(&base)? {
                // `rust_files` trả đường dẫn dạng `src/…` dù thư mục gốc là gì.
                let path = base.join(file.trim_start_matches("src/"));
                let text = fs::read_to_string(&path)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                for (index, line) in text.lines().enumerate() {
                    if line.contains("__private") {
                        violations.push(Violation {
                            rule: "private-paths",
                            location: format!("{}/{sub}/{}:{}", package.dir, file.trim_start_matches("src/"), index + 1),
                            message: "`autoxide_kernel::__private` is for code the kernel's macros generate, not part \
                                      of the contract: use the public path"
                                .to_owned(),
                        });
                    }
                }
            }
        }
    }
    Ok(violations)
}

/// File test của module: `tests.rs`, `*_tests.rs`, hay nằm trong thư mục `tests/`.
fn is_test_file(file: &str) -> bool {
    let name = file.rsplit('/').next().unwrap_or(file);
    name == "tests.rs" || name.ends_with("_tests.rs") || file.contains("/tests/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nhan_ra_file_test() {
        assert!(is_test_file("src/tests.rs"));
        assert!(is_test_file("src/sync_tests.rs"));
        assert!(is_test_file("src/http/tests/list.rs"));
        assert!(!is_test_file("src/service.rs"));
    }
}
