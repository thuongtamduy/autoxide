//! Luật `rust-free-paths`: danh sách cho phép `scripts/rust-free-paths.txt` — file mà pull request chỉ đổi chúng thì
//! CI bỏ biên dịch và test Rust (D-20261125-ci-skip-rust) — không được chứa file nào mà Rust đọc.
//!
//! `scripts/rust-changed.sh` (bash, chạy trên CI trước khi có toolchain) mới là nơi quyết định; ở đây là bản Rust của
//! cùng luật khớp để kiểm danh sách: mọi file trong repo khớp danh sách phải không kéo test của crate nào theo đúng mô
//! hình của `cargo xtask test-filter` ([`affected`]): không nằm trong thư mục crate, không phải file dùng chung
//! (`Cargo.lock`…), không bị `include_str!` / `include_bytes!`, không nằm trong danh sách file test đọc lúc chạy. Luật
//! chạy trong `cargo xtask check` — tức chạy cả ở pull request bị bỏ phần Rust — nên ai thêm một mục sai vào danh sách
//! (hay thêm `include_str!` tới một file đã có trong danh sách) thì gate đỏ ngay.

use std::path::Path;
use std::process::Command;

use crate::affected::{Affected, affected};
use crate::boundaries::Violation;
use crate::workspace::Workspace;

/// Danh sách cho phép, tính từ gốc repo.
pub const RULES_FILE: &str = "scripts/rust-free-paths.txt";

/// Một dòng của danh sách: mẫu đường dẫn, `negated` khi dòng bắt đầu bằng `!` (ngoại lệ: chạm Rust).
#[derive(Debug, PartialEq, Eq)]
pub struct Rule {
    pub negated: bool,
    pub pattern: String,
}

/// Đọc danh sách: bỏ dòng trống và dòng bắt đầu bằng `#`.
pub fn parse(text: &str) -> Vec<Rule> {
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| match line.strip_prefix('!') {
            Some(pattern) => Rule { negated: true, pattern: pattern.to_owned() },
            None => Rule { negated: false, pattern: line.to_owned() },
        })
        .collect()
}

/// `*` khớp mọi chuỗi (kể cả `/`), ký tự khác khớp đúng nó — như `[[ $path == $pattern ]]` của bash khi mẫu chỉ có
/// `*`.
fn glob(pattern: &str, path: &str) -> bool {
    let mut parts = pattern.split('*');
    // `split` luôn trả ít nhất một phần.
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = path.strip_prefix(first) else { return false };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        // Không có `*`: phải khớp trọn.
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

/// File có trong danh sách cho phép không: dòng đầu tiên khớp quyết định; không dòng nào khớp thì không.
pub fn rust_free(rules: &[Rule], path: &str) -> bool {
    rules.iter().find(|rule| glob(&rule.pattern, path)).is_some_and(|rule| !rule.negated)
}

pub fn check(workspace: &Workspace) -> Result<Vec<Violation>, String> {
    let text = std::fs::read_to_string(workspace.root.join(RULES_FILE))
        .map_err(|error| format!("cannot read {RULES_FILE}: {error}"))?;
    let rules = parse(&text);
    let mut violations = Vec::new();
    for rule in &rules {
        // Bash hiểu `?`, `[…]` là ký tự đại diện, bản Rust ở đây thì không: cấm để hai bản không lệch nhau.
        if rule.pattern.contains(['?', '[', ']', '\\']) || rule.pattern.is_empty() {
            violations.push(Violation {
                rule: "rust-free-paths",
                location: RULES_FILE.to_owned(),
                message: format!(
                    "pattern `{}`: only `*` is a wildcard here; write the path literally",
                    rule.pattern
                ),
            });
        }
    }
    let files = tracked_files(&workspace.root)?;
    let free: Vec<String> = files.into_iter().filter(|file| rust_free(&rules, file)).collect();
    if affected(workspace, &free) == Affected::Crates(Default::default()) {
        return Ok(violations);
    }
    // Hiếm: tìm đúng file gây ra (mỗi lần gọi `affected` quét lại mọi file `.rs`).
    for file in &free {
        let readers = match affected(workspace, std::slice::from_ref(file)) {
            Affected::All => "every crate".to_owned(),
            Affected::Crates(crates) if crates.is_empty() => continue,
            Affected::Crates(crates) => crates.into_iter().collect::<Vec<_>>().join(", "),
        };
        violations.push(Violation {
            rule: "rust-free-paths",
            location: file.clone(),
            message: format!(
                "matches {RULES_FILE}, so CI skips the Rust build and tests when only it changes, but it \
                 affects the tests of {readers}; add a `!` exception for it above the matching line \
                 (D-20261125-ci-skip-rust)"
            ),
        });
    }
    Ok(violations)
}

/// Mọi file git theo dõi, đường dẫn tính từ gốc repo.
fn tracked_files(root: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .map_err(|error| format!("cannot run `git ls-files`: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "`git ls-files` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|file| !file.is_empty())
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
    }

    fn real_rules() -> Vec<Rule> {
        parse(&std::fs::read_to_string(repo().join(RULES_FILE)).unwrap())
    }

    /// Kết quả của `scripts/rust-changed.sh --stdin` cho một danh sách file.
    fn script(files: &[&str]) -> bool {
        use std::io::Write;
        let mut child = Command::new("bash")
            .arg(repo().join("scripts/rust-changed.sh"))
            .arg("--stdin")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(files.join("\n").as_bytes()).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        match String::from_utf8(output.stdout).unwrap().trim() {
            "true" => true,
            "false" => false,
            other => panic!("unexpected output {other:?}"),
        }
    }

    /// Như script: có ít nhất một file không nằm trong danh sách.
    fn rust_changed(rules: &[Rule], files: &[&str]) -> bool {
        files.iter().any(|file| !rust_free(rules, file))
    }

    #[test]
    fn glob_chi_co_dau_sao() {
        assert!(glob("README.md", "README.md"));
        assert!(!glob("README.md", "README.md.bak"));
        assert!(!glob("README.md", "docs/README.md"));
        assert!(glob("sdk/typescript/*", "sdk/typescript/src/schema.ts"));
        assert!(!glob("sdk/typescript/*", "sdk/typescriptx"));
        assert!(glob("docs/*.md", "docs/DEPLOY.md"));
        assert!(glob("docs/*.md", "docs/guides/x.md"), "`*` khớp cả `/`, như bash");
        assert!(!glob("docs/*.md", "docs/openapi.json"));
        assert!(glob("docs/MODULES*", "docs/MODULES.vi.md"));
        assert!(glob("a*b*c", "abc"));
        assert!(glob("a*b*c", "axxbyyc"));
        assert!(!glob("a*b*c", "axxcyyb"));
        assert!(!glob("ab*ba", "aba"), "đầu và đuôi không được chồng lên nhau");
    }

    #[test]
    fn dong_dau_tien_khop_quyet_dinh() {
        let rules = parse("# chú thích\n\n!docs/guides/*\ndocs/*.md\n");
        assert_eq!(rules.len(), 2);
        assert!(rust_free(&rules, "docs/DEPLOY.md"));
        assert!(!rust_free(&rules, "docs/guides/notes.md"), "ngoại lệ đứng trước");
        assert!(!rust_free(&rules, "src/main.rs"), "không khớp dòng nào: chạm Rust");
    }

    /// File của hai pull request thật đã chạy trọn CI Rust dù không chạm Rust (#460 của Dependabot, #497).
    #[test]
    fn pull_request_that_460_va_497() {
        let rules = real_rules();
        let pr_460 = ["sdk/typescript/package-lock.json", "sdk/typescript/package.json"];
        let pr_497_without_workflow = [
            "changelog.d/sdk-typescript-7.changed.md",
            "docs/decisions/D-20261125-typescript-range.md",
            "docs/decisions/INDEX.md",
            "sdk/typescript/README.md",
            "sdk/typescript/README.vi.md",
            "sdk/typescript/package-lock.json",
            "sdk/typescript/package.json",
        ];
        let mut pr_497 = pr_497_without_workflow.to_vec();
        pr_497.push(".github/workflows/ci.yml");
        for (files, expected) in [
            (&pr_460[..], false),
            (&pr_497_without_workflow[..], false),
            // Đổi workflow thì chạy đủ: chính CI có thể đã đổi cách build hay test.
            (&pr_497[..], true),
        ] {
            assert_eq!(rust_changed(&rules, files), expected, "{files:?}");
            assert_eq!(script(files), expected, "script: {files:?}");
        }
    }

    /// Bản bash (`scripts/rust-changed.sh`) và bản Rust cho cùng kết quả trên từng đường dẫn — gồm cả những file nhìn
    /// như tài liệu mà Rust đọc.
    #[test]
    fn script_va_ban_rust_cung_ket_qua() {
        let rules = real_rules();
        let cases = [
            ("README.md", false),
            ("docs/DEPLOY.md", false),
            ("docs/ROADMAP.md", false),
            ("docs/steps/README.md", false),
            ("changelog.d/x.added.md", false),
            ("sdk/typescript/src/client.ts", false),
            (".claude/settings.json", false),
            (".claude/skills/next-step/SKILL.md", false),
            ("CHANGELOG.md", true),
            ("docs/MODULES.md", true),
            ("docs/MODULES.vi.md", true),
            ("docs/guides/notes.md", true),
            ("docs/guides/files/photo.png", true),
            ("docs/openapi.json", true),
            ("docs/mcp-tools.json", true),
            (".env.example", true),
            ("Cargo.toml", true),
            ("Cargo.lock", true),
            ("rust-toolchain.toml", true),
            ("deny.toml", true),
            ("clippy.toml", true),
            ("rustfmt.toml", true),
            ("gate.sh", true),
            ("scripts/rust-changed.sh", true),
            ("scripts/rust-free-paths.txt", true),
            (".github/workflows/ci.yml", true),
            (".github/workflows/gate.yml", true),
            ("examples/external-module/README.md", true),
            ("xtask/src/main.rs", true),
            ("modules/notes/README.md", true),
            ("modules/notes/locales/vi.json", true),
            ("crates/kernel/src/lib.rs", true),
            ("Dockerfile", true),
        ];
        for (file, changed) in cases {
            assert_eq!(rust_changed(&rules, &[file]), changed, "{file}");
            assert_eq!(script(&[file]), changed, "script: {file}");
        }
        assert!(!script(&[]), "không file nào đổi: không có gì phải chạy");
    }
}
