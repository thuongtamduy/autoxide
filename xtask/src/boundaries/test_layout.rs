//! Luật `test-binary` (D-20261118-build-size): test tích hợp của một crate là **một** chương trình,
//! `tests/integration/` với `main.rs` khai `mod` cho từng file.

use std::collections::BTreeSet;
use std::path::Path;

use super::{Violation, read};
use crate::workspace::Package;

/// Kiểm thư mục `tests/` của một crate.
pub(super) fn check(root: &Path, package: &Package) -> Vec<Violation> {
    let tests = root.join(&package.dir).join("tests");
    let mut violations = check_test_layout(package, &names(&tests));
    let integration = tests.join("integration");
    if integration.is_dir() {
        let main = read(&integration.join("main.rs")).unwrap_or_default();
        violations.extend(check_integration_mods(package, &names(&integration), &main));
    }
    violations
}

/// Tên các mục trực tiếp trong một thư mục; không có thư mục thì rỗng.
fn names(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Luật `test-binary` (D-20261118-build-size): test tích hợp của một crate nằm trong **một** chương trình,
/// `tests/integration/` (khai `mod` ở `tests/integration/main.rs`). Mỗi file `tests/*.rs` là một
/// chương trình riêng link lại toàn bộ thư viện, tốn đĩa và thời gian link. `entries` là tên các
/// mục trực tiếp trong `tests/`.
fn check_test_layout(package: &Package, entries: &[String]) -> Vec<Violation> {
    entries
        .iter()
        .filter(|entry| entry.as_str() != "integration")
        .map(|entry| Violation {
            rule: "test-binary",
            location: format!("{}/tests/{entry}", package.dir),
            message: "integration tests of a crate live in one test program: move this into \
                      tests/integration/ and declare it with `mod` in tests/integration/main.rs"
                .to_owned(),
        })
        .collect()
}

/// Luật `test-binary`, phần hai: mọi file trong `tests/integration/` phải được khai `mod` ở
/// `main.rs`; file không khai không được biên dịch, test trong đó âm thầm không bao giờ chạy.
fn check_integration_mods(package: &Package, files: &[String], main: &str) -> Vec<Violation> {
    let declared: BTreeSet<&str> = main
        .lines()
        .filter_map(|line| line.trim().strip_prefix("mod ")?.strip_suffix(';'))
        .collect();
    files
        .iter()
        .filter_map(|file| file.strip_suffix(".rs"))
        .filter(|name| *name != "main" && !declared.contains(name))
        .map(|name| Violation {
            rule: "test-binary",
            location: format!("{}/tests/integration/{name}.rs", package.dir),
            message: format!("not compiled: add `mod {name};` to tests/integration/main.rs"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Role;

    #[test]
    fn test_tich_hop_chi_trong_mot_chuong_trinh() {
        let package = Package {
            name: "autoxide-kernel".to_owned(),
            license: None,
            dir: "crates/kernel".to_owned(),
            role: Role::Kernel,
            deps: Vec::new(),
        };
        assert_eq!(check_test_layout(&package, &["integration".to_owned()]), Vec::new());
        let violations =
            check_test_layout(&package, &["integration".to_owned(), "money.rs".to_owned()]);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule, "test-binary");
        assert_eq!(violations[0].location, "crates/kernel/tests/money.rs");

        let files = ["main.rs".to_owned(), "money.rs".to_owned(), "job.rs".to_owned()];
        let violations = check_integration_mods(&package, &files, "//! x\n\nmod money;\n");
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].location, "crates/kernel/tests/integration/job.rs");
    }
}
