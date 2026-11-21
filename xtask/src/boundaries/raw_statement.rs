//! Luật `raw-statement`: câu SQL viết tay kèm tham số dựng bằng `autoxide_kernel::database::statement`, không
//! bằng `sea_orm::Statement::from_sql_and_values`.
//!
//! Vì sao: giá trị thời gian của kernel (`Timestamp`, `Date`) chỉ bind đúng kiểu trên Postgres khi câu lệnh mang
//! tên kiểu (`$1::"timestamptz"`); builder của SeaORM tự chèn, câu viết tay thì hàm của kernel chèn. Dựng thẳng
//! bằng SeaORM thì chạy được trên SQLite mà hỏng trên Postgres (lỗi 42804) — chỉ lộ ra ở gate đầy đủ. Áp cho mọi
//! crate trừ xtask, cả `src/` lẫn `tests/`.

use std::path::Path;

use super::{Violation, collect_rust_files, read};
use crate::workspace::{Package, Role};

/// Lời gọi bị chặn.
const RAW_CALL: &str = "from_sql_and_values(";
/// File duy nhất được gọi thẳng: hàm `statement` của kernel.
const ALLOWED_FILE: &str = "src/data/database.rs";

/// Kiểm `src/` và `tests/` của một crate.
pub(super) fn check(root: &Path, package: &Package) -> Result<Vec<Violation>, String> {
    // xtask chỉ nói VỀ lời gọi này (thông báo, test của luật), không chạy SQL.
    if package.role == Role::Tool {
        return Ok(Vec::new());
    }
    let crate_dir = root.join(&package.dir);
    let mut violations = Vec::new();
    for folder in ["src", "tests"] {
        let dir = crate_dir.join(folder);
        if !dir.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        collect_rust_files(&dir, folder, &mut files)?;
        for file in files {
            let text = read(&crate_dir.join(&file))?;
            violations.extend(check_source(package, &file, &text));
        }
    }
    Ok(violations)
}

/// Luật cho một file.
pub(super) fn check_source(package: &Package, file: &str, text: &str) -> Vec<Violation> {
    if package.role == Role::Kernel && file == ALLOWED_FILE {
        return Vec::new();
    }
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//") && line.contains(RAW_CALL))
        .map(|(index, _)| Violation {
            rule: "raw-statement",
            location: format!("{}/{}:{}", package.dir, file, index + 1),
            message: "build hand-written SQL with `autoxide_kernel::database::statement(backend, sql, \
                      values)`, not `Statement::from_sql_and_values`: it adds the type of time values \
                      that PostgreSQL needs"
                .to_owned(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Role;

    fn package(dir: &str) -> Package {
        Package {
            name: "x".to_owned(),
            license: None,
            dir: dir.to_owned(),
            role: Role::from_dir(dir),
            deps: Vec::new(),
        }
    }

    #[test]
    fn goi_thang_seaorm_bi_bao_tru_ham_cua_kernel() {
        let line = "    let s = Statement::from_sql_and_values(backend, sql, [id.into()]);";
        let found = check_source(&package("modules/notes"), "tests/integration/x.rs", line);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].location, "modules/notes/tests/integration/x.rs:1");
        assert!(check_source(&package("crates/kernel"), ALLOWED_FILE, line).is_empty());
        assert_eq!(check_source(&package("crates/kernel"), "src/job/claim.rs", line).len(), 1);
    }

    #[test]
    fn ham_cua_kernel_va_chu_thich_khong_bi_bao() {
        let text = "\
let s = autoxide_kernel::database::statement(backend, sql, [at.into()]);
// Statement::from_sql_and_values(…) không dùng được với thời gian
";
        assert!(check_source(&package("crates/app"), "src/e2e/x.rs", text).is_empty());
    }
}
