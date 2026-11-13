//! `cargo xtask test-affected [--base <ref>]`: chạy test của các crate bị sửa so với `main` và
//! của mọi crate phụ thuộc vào chúng, trên SQLite. Đây là phần test của gate nhanh
//! (`./gate.sh --quick`, D-173); gate đầy đủ vẫn chạy toàn bộ test trên cả hai database.
//!
//! Vì sao không dùng `cargo test -p <crate>`: chỉ chọn vài crate thì cargo hợp feature của thư
//! viện theo riêng các crate đó, khác lúc build cả workspace, nên phải build lại sqlx, sea-orm…
//! thành bộ file thứ hai (đo được 87 s chỉ cho `content`). Cách gom feature cả workspace
//! (`resolver.feature-unification`) còn là tính năng nightly. Vì vậy ở đây build cả workspace —
//! dùng chung cache với gate đầy đủ, cargo chỉ build lại phần đã sửa — rồi chỉ chạy các file
//! test của crate bị ảnh hưởng, giống cách `cargo test` chạy chúng: thư mục làm việc là thư mục
//! crate, có `CARGO_MANIFEST_DIR`.
//!
//! Doc-test không chạy ở đây (cần `rustdoc`); gate đầy đủ chạy chúng.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

use crate::workspace::Workspace;

/// File ở gốc repo mà đổi thì mọi crate có thể build hoặc chạy khác đi.
const GLOBAL_FILES: &[&str] =
    &["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "clippy.toml", "rustfmt.toml"];
const GLOBAL_DIRS: &[&str] = &[".cargo/"];

/// File nằm ngoài thư mục crate nhưng có test của một crate đọc (bản sinh từ code phải khớp
/// bản đã commit): đổi file thì chạy test của crate đó.
const CHECKED_FILES: &[(&str, &str)] =
    &[("docs/openapi.json", "autoxide"), (".env.example", "autoxide")];

#[derive(Debug, PartialEq, Eq)]
pub enum Affected {
    /// Đổi thứ dùng chung (manifest gốc, `Cargo.lock`, toolchain…): chạy mọi test.
    All,
    /// Tên các crate cần chạy test; rỗng khi chỉ sửa tài liệu.
    Crates(BTreeSet<String>),
}

/// Crate bị ảnh hưởng bởi các file `changed` (đường dẫn tương đối so với gốc repo, dùng `/`).
pub fn affected(workspace: &Workspace, changed: &[String]) -> Affected {
    let mut crates = BTreeSet::new();
    for file in changed {
        if GLOBAL_FILES.contains(&file.as_str())
            || GLOBAL_DIRS.iter().any(|dir| file.starts_with(dir))
        {
            return Affected::All;
        }
        if let Some((_, package)) = CHECKED_FILES.iter().find(|(path, _)| path == file) {
            crates.insert((*package).to_owned());
            continue;
        }
        // Crate có thư mục dài nhất chứa file; file ngoài mọi crate (tài liệu…) không kéo
        // theo test nào.
        let owner = workspace
            .packages
            .iter()
            .filter(|package| {
                !package.dir.is_empty() && file.starts_with(&format!("{}/", package.dir))
            })
            .max_by_key(|package| package.dir.len());
        if let Some(package) = owner {
            crates.insert(package.name.clone());
        }
    }
    // Thêm crate phụ thuộc, lặp tới khi không thêm được nữa. Tính mọi loại dependency: crate
    // dùng `testkit` ở `[dev-dependencies]` vẫn phải chạy lại test khi `testkit` đổi.
    loop {
        let before = crates.len();
        for package in &workspace.packages {
            if !crates.contains(&package.name)
                && package.deps.iter().any(|dep| crates.contains(&dep.name))
            {
                crates.insert(package.name.clone());
            }
        }
        if crates.len() == before {
            return Affected::Crates(crates);
        }
    }
}

/// Lệnh `test-affected`. `Err` là lỗi chạy công cụ (git, cargo); test đỏ cũng là `Err`.
pub fn run(workspace: &Workspace, base: Option<&str>) -> Result<(), String> {
    let base = match base {
        Some(base) => base.to_owned(),
        None => default_base(&workspace.root),
    };
    let changed = changed_files(&workspace.root, &base)?;
    let selected = affected(workspace, &changed);
    let dirs: Vec<(String, PathBuf)> = match &selected {
        Affected::All => {
            println!("test-affected: shared build files changed since {base}, running every crate");
            workspace
                .packages
                .iter()
                .map(|package| (package.name.clone(), workspace.root.join(&package.dir)))
                .collect()
        }
        Affected::Crates(crates) if crates.is_empty() => {
            println!("test-affected: no crate changed since {base}, nothing to test");
            return Ok(());
        }
        Affected::Crates(crates) => {
            println!(
                "test-affected: {} crate(s) changed since {base} or depend on one: {}",
                crates.len(),
                crates.iter().cloned().collect::<Vec<_>>().join(", ")
            );
            workspace
                .packages
                .iter()
                .filter(|package| crates.contains(&package.name))
                .map(|package| (package.name.clone(), workspace.root.join(&package.dir)))
                .collect()
        }
    };

    let mut failed = Vec::new();
    let mut ran = 0;
    for test in build_tests(&workspace.root)? {
        let Some((name, dir)) = dirs.iter().find(|(_, dir)| *dir == test.dir) else {
            continue;
        };
        let display = test.executable.strip_prefix(&workspace.root).unwrap_or(&test.executable);
        println!("\n     Running {} ({name})", display.display());
        ran += 1;
        let status = without_cargo_run_env(&mut Command::new(&test.executable))
            .current_dir(dir)
            .env("CARGO_MANIFEST_DIR", dir)
            .env("CARGO_PKG_NAME", name)
            // Gate nhanh chỉ chạy SQLite, lưu file trên đĩa (D-173): bỏ biến nếu lỡ đặt sẵn.
            .env_remove("TEST_DATABASE_URL")
            .env_remove("TEST_S3_ENDPOINT")
            .status()
            .map_err(|error| format!("cannot run {}: {error}", display.display()))?;
        if !status.success() {
            failed.push(display.display().to_string());
        }
    }
    if failed.is_empty() {
        println!("\ntest-affected: {ran} test binaries passed");
        Ok(())
    } else {
        Err(format!("{} test binaries failed: {}", failed.len(), failed.join(", ")))
    }
}

/// Mặc định so với `origin/main` (bản đã lên GitHub), không có thì `main`.
fn default_base(root: &Path) -> String {
    let remote = git(root, &["rev-parse", "--verify", "--quiet", "origin/main"]);
    if remote.is_ok() { "origin/main".to_owned() } else { "main".to_owned() }
}

/// File đã đổi kể từ chỗ rẽ nhánh với `base`: đã commit, đã stage, chưa stage và file mới.
fn changed_files(root: &Path, base: &str) -> Result<Vec<String>, String> {
    let merge_base = git(root, &["merge-base", "HEAD", base])?;
    let mut files: Vec<String> = git(root, &["diff", "--name-only", merge_base.trim()])?
        .lines()
        .map(str::to_owned)
        .collect();
    files.extend(
        git(root, &["ls-files", "--others", "--exclude-standard"])?.lines().map(str::to_owned),
    );
    Ok(files)
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "`git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| format!("git output is not UTF-8: {error}"))
}

/// Bỏ các biến `cargo run` đặt cho chính xtask (`CARGO_MANIFEST_DIR`, `CARGO_PKG_*`…).
///
/// Vì sao: build script của vài thư viện (vd `ring`) khai `rerun-if-env-changed` cho các biến
/// này, và cargo so giá trị trong môi trường của chính nó. Để nguyên thì cargo gọi từ xtask
/// thấy chúng khác với cargo gọi thẳng từ `gate.sh`, nên build lại `ring` cùng mọi thứ phụ
/// thuộc vào nó (khoảng 36 crate) mỗi lần đổi qua lại.
fn without_cargo_run_env(command: &mut Command) -> &mut Command {
    for (name, _) in std::env::vars_os() {
        let Some(name) = name.to_str() else { continue };
        if name.starts_with("CARGO_PKG_")
            || name.starts_with("CARGO_MANIFEST_")
            || matches!(
                name,
                "CARGO_CRATE_NAME" | "CARGO_BIN_NAME" | "CARGO_PRIMARY_PACKAGE" | "OUT_DIR"
            )
        {
            command.env_remove(name);
        }
    }
    command
}

struct TestBinary {
    executable: PathBuf,
    /// Thư mục crate chứa test này.
    dir: PathBuf,
}

// Đúng những trường cần của từng dòng JSON mà `cargo --message-format=json` in ra.
#[derive(Deserialize)]
struct Message {
    reason: String,
    manifest_path: Option<PathBuf>,
    executable: Option<PathBuf>,
    profile: Option<Profile>,
}

#[derive(Deserialize)]
struct Profile {
    test: bool,
}

/// Build mọi file test của workspace (như gate đầy đủ, để dùng chung cache) và trả danh sách.
fn build_tests(root: &Path) -> Result<Vec<TestBinary>, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = without_cargo_run_env(&mut Command::new(&cargo))
        .args([
            "test",
            "--workspace",
            "--locked",
            "--no-run",
            // Lỗi biên dịch vẫn in ra stderr như thường; stdout chỉ còn JSON để đọc.
            "--message-format=json-render-diagnostics",
        ])
        .current_dir(root)
        .stderr(Stdio::inherit())
        .output()
        .map_err(|error| format!("cannot run `{cargo} test`: {error}"))?;
    if !output.status.success() {
        return Err("building the tests failed (see the errors above)".to_owned());
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("cargo output is not UTF-8: {error}"))?;
    Ok(stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Message>(line).ok())
        .filter(|message| message.reason == "compiler-artifact")
        .filter(|message| message.profile.as_ref().is_some_and(|profile| profile.test))
        .filter_map(|message| {
            let dir = message.manifest_path?.parent()?.to_path_buf();
            Some(TestBinary { executable: message.executable?, dir })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{Dep, DepKind, Package, Role};

    fn package(name: &str, dir: &str, deps: &[(&str, DepKind)]) -> Package {
        Package {
            name: name.to_owned(),
            license: None,
            dir: dir.to_owned(),
            role: Role::from_dir(dir),
            deps: deps
                .iter()
                .map(|(name, kind)| Dep { name: (*name).to_owned(), kind: *kind })
                .collect(),
        }
    }

    /// kernel ← testkit (dev) ← notes, content ← autoxide; xtask đứng riêng.
    fn workspace() -> Workspace {
        use DepKind::{Dev, Normal};
        Workspace {
            root: "/repo".into(),
            packages: vec![
                package("autoxide-kernel", "crates/kernel", &[("serde", Normal)]),
                package("autoxide-testkit", "crates/testkit", &[("autoxide-kernel", Normal)]),
                package(
                    "autoxide-notes",
                    "modules/notes",
                    &[("autoxide-kernel", Normal), ("autoxide-testkit", Dev)],
                ),
                package("autoxide-content", "modules/content", &[("autoxide-kernel", Normal)]),
                package(
                    "autoxide",
                    "crates/app",
                    &[("autoxide-notes", Normal), ("autoxide-content", Normal)],
                ),
                package("xtask", "xtask", &[("serde", Normal)]),
            ],
        }
    }

    fn crates(names: &[&str]) -> Affected {
        Affected::Crates(names.iter().map(|name| (*name).to_owned()).collect())
    }

    fn changed(files: &[&str]) -> Affected {
        let files: Vec<String> = files.iter().map(|file| (*file).to_owned()).collect();
        affected(&workspace(), &files)
    }

    #[test]
    fn sua_module_chay_module_va_binary() {
        assert_eq!(
            changed(&["modules/content/src/body.rs"]),
            crates(&["autoxide", "autoxide-content"])
        );
    }

    #[test]
    fn sua_kernel_chay_moi_crate_dua_vao_no() {
        assert_eq!(
            changed(&["crates/kernel/src/lib.rs"]),
            crates(&[
                "autoxide",
                "autoxide-content",
                "autoxide-kernel",
                "autoxide-notes",
                "autoxide-testkit",
            ])
        );
    }

    #[test]
    fn dev_dependency_cung_keo_theo() {
        assert_eq!(
            changed(&["crates/testkit/src/app.rs"]),
            crates(&["autoxide", "autoxide-notes", "autoxide-testkit"])
        );
    }

    #[test]
    fn chi_sua_tai_lieu_thi_khong_chay_gi() {
        assert_eq!(
            changed(&["docs/ROADMAP.md", "README.md", ".claude/skills/x/SKILL.md"]),
            crates(&[])
        );
        assert_eq!(changed(&[]), crates(&[]));
    }

    #[test]
    fn file_dung_chung_thi_chay_het() {
        for file in ["Cargo.lock", "Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml"] {
            assert_eq!(changed(&["docs/ROADMAP.md", file]), Affected::All, "{file}");
        }
        // Manifest của một crate không phải file dùng chung.
        assert_eq!(changed(&["xtask/Cargo.toml"]), crates(&["xtask"]));
    }

    #[test]
    fn file_sinh_tu_code_chay_test_cua_binary() {
        assert_eq!(changed(&["docs/openapi.json"]), crates(&["autoxide"]));
        assert_eq!(changed(&[".env.example"]), crates(&["autoxide"]));
    }

    #[test]
    fn thu_muc_trung_tien_to_khong_bi_nham() {
        // `modules/notes-extra` không thuộc `modules/notes`.
        assert_eq!(changed(&["modules/notes-extra/README.md"]), crates(&[]));
    }
}
