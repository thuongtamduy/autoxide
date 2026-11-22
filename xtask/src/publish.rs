//! Sẵn sàng cho crates.io (D-20261121-external-modules, giai đoạn 2). Hai phần:
//!
//! - Luật `publish-ready` của `cargo xtask check` ([`check`]): rẻ, đọc manifest — mỗi crate sẽ đăng có đủ
//!   metadata crates.io đòi, và phụ thuộc nội bộ ở `[workspace.dependencies]` ghim đúng phiên bản của
//!   workspace (đổi `version` mà quên các dòng đó thì gate đỏ, không phải tới lúc đăng mới biết).
//! - `cargo xtask publish-check [--no-verify]` ([`run`]): `cargo package` cho mọi crate sẽ đăng — đóng gói
//!   đúng như `cargo publish`, rồi (trừ khi `--no-verify`) build từng gói từ bản đã đóng, tức là thứ người
//!   dùng crates.io nhận được. Build lại cả workspace nên lâu: chạy lúc phát hành, không trong gate.
//!
//! Không lệnh nào ở đây đăng gì lên crates.io.

use std::path::Path;
use std::process::Command;

use crate::boundaries::Violation;
use crate::workspace::Workspace;

/// Crate không bao giờ đăng: công cụ nội bộ và module mẫu.
pub const UNPUBLISHED: [&str; 2] = ["xtask", "autoxide-notes"];

/// Dòng mỗi crate sẽ đăng phải có trong `Cargo.toml` (thừa hưởng từ `[workspace.package]`).
const INHERITED: [&str; 5] = ["repository", "homepage", "readme", "keywords", "categories"];

/// Luật `publish-ready` trên workspace ở đĩa.
pub fn check(workspace: &Workspace) -> Result<Vec<Violation>, String> {
    let root_manifest = read(&workspace.root.join("Cargo.toml"))?;
    let version = crate::semver::workspace_version(&workspace.root)?;
    let mut violations = check_internal_versions(&root_manifest, &version);
    for package in &workspace.packages {
        if UNPUBLISHED.contains(&package.name.as_str()) {
            continue;
        }
        let manifest = read(&workspace.root.join(&package.dir).join("Cargo.toml"))?;
        violations.extend(check_metadata(&package.dir, &manifest));
    }
    Ok(violations)
}

/// Mỗi crate nội bộ ở `[workspace.dependencies]` (trừ crate không đăng) có `version = "=<phiên bản
/// workspace>"` cạnh `path`.
pub fn check_internal_versions(root_manifest: &str, version: &str) -> Vec<Violation> {
    let expected = format!("version = \"={version}\"");
    let mut violations = Vec::new();
    for (index, line) in root_manifest.lines().enumerate() {
        let Some((name, value)) = line.split_once(" = ") else {
            continue;
        };
        if !name.starts_with("autoxide-") || !value.contains("path = ") {
            continue;
        }
        if UNPUBLISHED.contains(&name) {
            if value.contains("version") {
                violations.push(Violation {
                    rule: "publish-ready",
                    location: format!("Cargo.toml:{}", index + 1),
                    message: format!(
                        "`{name}` is never published: its workspace dependency must have no \
                         `version`, so that cargo drops it from published manifests"
                    ),
                });
            }
        } else if !value.contains(&expected) {
            violations.push(Violation {
                rule: "publish-ready",
                location: format!("Cargo.toml:{}", index + 1),
                message: format!(
                    "`{name}` must be `{{ path = \"…\", {expected} }}`: crates.io needs a version \
                     on every dependency, and the crates are released together"
                ),
            });
        }
    }
    violations
}

/// Metadata crates.io đòi hay hiện: mô tả riêng của crate và các dòng thừa hưởng từ workspace.
pub fn check_metadata(dir: &str, manifest: &str) -> Vec<Violation> {
    let mut missing: Vec<String> = INHERITED
        .iter()
        .filter(|key| !manifest.contains(&format!("{key}.workspace = true")))
        .map(|key| format!("`{key}.workspace = true`"))
        .collect();
    if !manifest.lines().any(|line| line.starts_with("description = \"")) {
        missing.insert(0, "`description = \"…\"`".to_owned());
    }
    if missing.is_empty() {
        return Vec::new();
    }
    vec![Violation {
        rule: "publish-ready",
        location: format!("{dir}/Cargo.toml"),
        message: format!("add {} (metadata crates.io shows)", missing.join(", ")),
    }]
}

/// `cargo package` cho mọi crate sẽ đăng. `verify`: build từng gói từ bản đã đóng.
///
/// Chạy trên một BẢN SAO của mã nguồn (mọi file git theo dõi, lấy từ thư mục làm việc) ở
/// `target/publish-check/source`, trong đó `publish = false` của workspace đổi thành `true`: với `publish =
/// false`, cargo không cho các crate của workspace thấy nhau lúc đóng gói ("no matching package named
/// `autoxide-kernel`"), nên không kiểm được. Bản thật giữ nguyên `publish = false` — lệnh gõ nhầm
/// `cargo publish` ở repo vẫn bị từ chối.
pub fn run(root: &Path, verify: bool) -> Result<(), String> {
    let copy = root.join("target/publish-check/source");
    copy_tracked_files(root, &copy)?;
    let manifest = copy.join("Cargo.toml");
    let text = read(&manifest)?;
    let allowed = allow_publish(&text)
        .ok_or("Cargo.toml: no `publish = false` line under [workspace.package] to lift")?;
    std::fs::write(&manifest, allowed)
        .map_err(|error| format!("cannot write {}: {error}", manifest.display()))?;

    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let mut command = Command::new(&cargo);
    // `--exclude-lockfile`: gói thư viện trên crates.io không dùng lock file của mình; bỏ nó thì bản
    // build thử dùng phiên bản thư viện mới nhất hợp lệ — đúng thứ người phụ thuộc vào crate nhận được.
    command
        .current_dir(&copy)
        .env("CARGO_TARGET_DIR", root.join("target/publish-check/target"))
        .args(["package", "--workspace", "--exclude-lockfile"]);
    for name in UNPUBLISHED {
        command.args(["--exclude", name]);
    }
    if !verify {
        command.arg("--no-verify");
    }
    let status =
        command.status().map_err(|error| format!("cannot run `{cargo} package`: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("`cargo package` failed: the crates are not ready for crates.io (see above)".to_owned())
    }
}

/// Manifest gốc với `publish = false` của `[workspace.package]` đổi thành `publish = true` (crate con
/// thừa hưởng nên dòng phải còn); `None` nếu không có dòng đó.
pub fn allow_publish(root_manifest: &str) -> Option<String> {
    let mut in_package = false;
    let mut lifted = false;
    let mut lines = Vec::new();
    for line in root_manifest.lines() {
        if line.starts_with('[') {
            in_package = line.trim() == "[workspace.package]";
        }
        if in_package && line.trim() == "publish = false" {
            lifted = true;
            lines.push("publish = true");
            continue;
        }
        lines.push(line);
    }
    lifted.then(|| lines.join("\n") + "\n")
}

/// Chép mọi file git theo dõi (nội dung đang có trên đĩa, kể cả chưa commit) từ `root` sang `target`
/// (xoá bản cũ trước). Symlink được giữ là symlink: `crates/kernel/CHANGELOG.md` trỏ lên gốc.
fn copy_tracked_files(root: &Path, target: &Path) -> Result<(), String> {
    let listed = Command::new("git")
        .current_dir(root)
        // Cả file mới chưa `git add` (trừ file bị .gitignore bỏ qua).
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .output()
        .map_err(|error| format!("cannot run `git ls-files`: {error}"))?;
    if !listed.status.success() {
        return Err("`git ls-files` failed: publish-check needs a git checkout".to_owned());
    }
    if target.exists() {
        std::fs::remove_dir_all(target)
            .map_err(|error| format!("cannot remove {}: {error}", target.display()))?;
    }
    for name in String::from_utf8_lossy(&listed.stdout).split('\0').filter(|name| !name.is_empty())
    {
        let (from, to) = (root.join(name), target.join(name));
        let failed = |error: std::io::Error| format!("cannot copy {name}: {error}");
        // File đã xoá ở thư mục làm việc nhưng chưa `git rm`: bỏ qua.
        let Ok(metadata) = std::fs::symlink_metadata(&from) else {
            continue;
        };
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(failed)?;
        }
        if metadata.file_type().is_symlink() {
            link(&std::fs::read_link(&from).map_err(failed)?, &to).map_err(failed)?;
        } else if metadata.is_file() {
            std::fs::copy(&from, &to).map_err(failed)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn link(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::other("publish-check needs symbolic links; run it on Linux, macOS or WSL"))
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phu_thuoc_noi_bo_phai_ghim_dung_phien_ban() {
        let manifest = "[workspace.dependencies]\n\
            autoxide-kernel = { path = \"crates/kernel\", version = \"=0.17.0\" }\n\
            autoxide-auth = { path = \"modules/auth\" }\n\
            autoxide-mail = { path = \"modules/mail\", version = \"=0.16.0\" }\n\
            autoxide-notes = { path = \"modules/notes\" }\n\
            tokio = { version = \"1\" }\n";
        let found: Vec<String> = check_internal_versions(manifest, "0.17.0")
            .into_iter()
            .map(|violation| violation.location)
            .collect();
        assert_eq!(found, ["Cargo.toml:3", "Cargo.toml:4"]);
    }

    #[test]
    fn crate_khong_dang_thi_khong_co_phien_ban() {
        let manifest = "autoxide-notes = { path = \"modules/notes\", version = \"=0.17.0\" }\n";
        assert_eq!(check_internal_versions(manifest, "0.17.0").len(), 1);
    }

    #[test]
    fn ban_sao_bo_publish_false_cua_workspace() {
        let manifest =
            "[workspace.package]\nversion = \"1.0.0\"\npublish = false\n\n[x]\npublish = false\n";
        assert_eq!(
            allow_publish(manifest).unwrap(),
            "[workspace.package]\nversion = \"1.0.0\"\npublish = true\n\n[x]\npublish = false\n"
        );
        assert_eq!(allow_publish("[workspace.package]\nversion = \"1.0.0\"\n"), None);
    }

    #[test]
    fn metadata_thieu_duoc_neu_ten() {
        let good = "description = \"x\"\nrepository.workspace = true\nhomepage.workspace = true\n\
                    readme.workspace = true\nkeywords.workspace = true\ncategories.workspace = true\n";
        assert_eq!(check_metadata("crates/kernel", good), Vec::new());
        let bad = check_metadata("crates/kernel", "repository.workspace = true\n");
        assert_eq!(bad.len(), 1);
        assert!(bad[0].message.contains("`description = \"…\"`"), "{}", bad[0].message);
        assert!(bad[0].message.contains("`readme.workspace = true`"));
        assert!(!bad[0].message.contains("`repository.workspace"));
    }
}
