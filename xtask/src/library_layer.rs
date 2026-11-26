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

/// Vi phạm có danh sách chỗ cũ ([`BASELINE`]): `wrapped-libraries`, `raw-sql`, `raw-database`.
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
        for (file, text) in module_files(workspace, package)? {
            if is_test_file(&file) {
                continue;
            }
            for (index, line) in code_lines(&text) {
                let location = format!("{}/{file}:{}", package.dir, index + 1);
                if line.contains(".sql(") {
                    violations.push(Violation {
                        rule: "raw-sql",
                        location: location.clone(),
                        message: "hand-written SQL: express it with the `db` query builder; if it is measured to \
                                  be slow, record the reason (skill `record-decision`) and add a tenant-isolation \
                                  test"
                            .to_owned(),
                    });
                }
                for _ in 0..line.matches(".database()").count() {
                    violations.push(Violation {
                        rule: "raw-database",
                        location: location.clone(),
                        message: "the raw database (`TenantContext::database()`) skips the tenant filter: query \
                                  through `tenant.db()` (control-plane tables: `context.control_plane()`)"
                            .to_owned(),
                    });
                }
            }
        }
    }
    Ok(violations)
}

/// Luật không có danh sách chỗ cũ, cho module đã chuyển và cho mọi crate:
///
/// - `record-registered`: mọi `#[derive(Record)]` trong mã không phải test của module có mặt trong
///   `Module::records()` (`RecordInfo::of::<Tên>()`), để bước đối chiếu lúc khởi động thấy nó.
/// - `wrapped-paths`: module đã bỏ hết thư viện bọc (không còn dòng `wrapped-libraries`) chỉ được gọi `sea_orm` và
///   `migration::prelude` trong `src/migration/` — cầu `ConnectionTrait` tạm thời của handle không thành lối đi vòng.
/// - `testing-feature`: chỉ `crates/testkit` bật feature `testing` của kernel.
pub fn check_strict(workspace: &Workspace) -> Result<Vec<Violation>, String> {
    let libraries = libraries(&workspace.root)?;
    let mut violations = Vec::new();
    for package in &workspace.packages {
        if package.role != Role::Testkit {
            let manifest = format!("{}/Cargo.toml", package.dir);
            let text = fs::read_to_string(workspace.root.join(&manifest))
                .map_err(|error| format!("{manifest}: {error}"))?;
            for (index, line) in text.lines().enumerate() {
                if line.contains("autoxide-kernel") && line.contains("\"testing\"") {
                    violations.push(Violation {
                        rule: "testing-feature",
                        location: format!("{manifest}:{}", index + 1),
                        message: "only crates/testkit turns on the kernel feature `testing` (direct table access for \
                                  tests); depend on `autoxide-testkit` under [dev-dependencies] instead"
                            .to_owned(),
                    });
                }
            }
        }
        if package.role != Role::Module {
            continue;
        }
        let converted = !package.deps.iter().any(|dep| libraries.contains(&dep.name));
        let files = module_files(workspace, package)?;
        let registered = registered_records(files.iter().map(|(_, text)| text.as_str()));
        for (file, text) in &files {
            let in_migrations = file.starts_with("src/migration/") || file == "src/migration.rs";
            for (index, line) in code_lines(text) {
                let location = format!("{}/{file}:{}", package.dir, index + 1);
                if converted
                    && !in_migrations
                    && (line.contains("sea_orm") || line.contains("migration::prelude"))
                {
                    violations.push(Violation {
                        rule: "wrapped-paths",
                        location: location.clone(),
                        message: "this module no longer depends on SeaORM: query through `autoxide_kernel::db`; \
                                  `sea_orm` and `migration::prelude` stay only in src/migration/"
                            .to_owned(),
                    });
                }
            }
            if is_test_file(file) {
                continue;
            }
            for name in derived_records(text) {
                if !registered.contains(&name) {
                    violations.push(Violation {
                        rule: "record-registered",
                        location: format!("{}/{file}", package.dir),
                        message: format!(
                            "record `{name}` is not declared in `Module::records()`: add \
                             `RecordInfo::of::<{name}>()` so the startup check compares it with the table"
                        ),
                    });
                }
            }
        }
    }
    Ok(violations)
}

/// Các file `.rs` dưới `src/` của crate, kèm nội dung.
fn module_files(
    workspace: &Workspace,
    package: &crate::workspace::Package,
) -> Result<Vec<(String, String)>, String> {
    let src = workspace.root.join(&package.dir).join("src");
    let mut files = Vec::new();
    for file in rust_files(&src)? {
        let text = fs::read_to_string(workspace.root.join(&package.dir).join(&file))
            .map_err(|error| format!("{file}: {error}"))?;
        files.push((file, text));
    }
    Ok(files)
}

/// Dòng mã (bỏ dòng chú thích `//`), kèm số thứ tự.
fn code_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines().enumerate().filter(|(_, line)| !line.trim_start().starts_with("//"))
}

/// Tên (đoạn cuối của đường dẫn) các record trong `RecordInfo::of::<…>()` của module.
fn registered_records<'a>(texts: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut names = Vec::new();
    for text in texts {
        for part in text.split("RecordInfo::of::<").skip(1) {
            let path = part.split('>').next().unwrap_or_default();
            names.push(path.rsplit("::").next().unwrap_or_default().trim().to_owned());
        }
    }
    names
}

/// Tên các struct có `#[derive(…Record…)]` trong `text`.
fn derived_records(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut pending = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with("#[derive(")
            && line.split(|c: char| !c.is_alphanumeric() && c != '_').any(|word| word == "Record")
        {
            pending = true;
            continue;
        }
        if pending
            && let Some(rest) = line
                .strip_prefix("pub(crate) struct ")
                .or_else(|| line.strip_prefix("pub struct "))
                .or_else(|| line.strip_prefix("struct "))
        {
            names.push(
                rest.split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
            );
            pending = false;
        }
    }
    names
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
    fn tim_struct_derive_record() {
        let text = "#[derive(Record, Clone, Debug)]\n#[record(table = \"x_y\", scope = tenant)]\npub(crate) struct Note {\n}\n#[derive(Debug)]\nstruct Other;\n#[derive(autoxide_kernel::db::Record)]\nstruct Two {}\n";
        assert_eq!(derived_records(text), ["Note", "Two"]);
    }

    #[test]
    fn doc_record_da_dang_ky() {
        let text = "vec![RecordInfo::of::<entity::Note>(), RecordInfo::of::<Plain>()]";
        assert_eq!(registered_records(std::iter::once(text)), ["Note", "Plain"]);
    }

    #[test]
    fn nhan_ra_file_test() {
        assert!(is_test_file("src/tests.rs"));
        assert!(is_test_file("src/sync_tests.rs"));
        assert!(is_test_file("src/http/tests/list.rs"));
        assert!(!is_test_file("src/service.rs"));
    }
}
