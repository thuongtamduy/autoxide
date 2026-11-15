//! `cargo xtask semver [--base <tag>] [--deny] [--markdown]`: báo thay đổi phá vỡ của hợp đồng Rust
//! (kernel, testkit) so với tag phát hành trước, bằng `cargo semver-checks` (D-233, đặc tả 31 A7).
//!
//! - Còn `0.x`: chỉ báo cáo (thoát 0 dù có phá vỡ) — mọi bản minor của `0.x` được phép phá vỡ
//!   (D-167). Từ `1.0.0`, hay với `--deny`: có phá vỡ thì thoát 1.
//! - `--markdown`: in mục `### Breaking (Rust)` để chép vào `CHANGELOG.md` lúc phát hành.
//! - Công cụ ép so như bản **patch** (`--release-type patch`): với `0.x`, bản minor được coi như
//!   major nên công cụ sẽ không báo gì; so như patch thì mọi thay đổi phá vỡ đều hiện.
//! - Cần mạng và build bản của tag cũ, lâu: chạy riêng lúc phát hành, không trong `./gate.sh`.

use std::path::Path;
use std::process::Command;

/// Crate là hợp đồng cho người viết module (D-164, D-167).
pub const CRATES: [&str; 2] = ["autoxide-kernel", "autoxide-testkit"];

/// Phiên bản công cụ đã thử (ghi ở `STACK.md`).
pub const TOOL_VERSION: &str = "0.51.0";

/// Chế độ chạy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Chỉ báo cáo (còn `0.x`).
    Report,
    /// Có phá vỡ thì thất bại (từ `1.0`, hay `--deny`).
    Deny,
}

/// `Deny` khi `version` của workspace ≥ `1.0.0` hay có `--deny`.
pub fn mode(workspace_version: &str, deny: bool) -> Mode {
    let major = workspace_version.split('.').next().and_then(|major| major.parse::<u64>().ok());
    if deny || major.is_some_and(|major| major >= 1) { Mode::Deny } else { Mode::Report }
}

/// Một thay đổi phá vỡ công cụ tìm thấy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub krate: String,
    /// Mã của luật, vd `function_missing`.
    pub lint: String,
    /// Câu ngắn của luật, vd `pub fn removed or renamed`.
    pub title: String,
    /// Từng chỗ vi phạm, vd `function autoxide_kernel::x, previously in file …`.
    pub places: Vec<String>,
}

/// Đọc kết quả của `cargo semver-checks` cho một crate: mỗi khối `--- failure <lint>: <title> ---`
/// kèm các dòng dưới `Failed in:`.
pub fn parse(krate: &str, output: &str) -> Vec<Failure> {
    let mut failures = Vec::new();
    let mut in_places = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) =
            trimmed.strip_prefix("--- failure ").and_then(|rest| rest.strip_suffix(" ---"))
        {
            let (lint, title) = rest.split_once(": ").unwrap_or((rest, ""));
            failures.push(Failure {
                krate: krate.to_owned(),
                lint: lint.to_owned(),
                title: title.to_owned(),
                places: Vec::new(),
            });
            in_places = false;
        } else if trimmed == "Failed in:" {
            in_places = true;
        } else if in_places {
            if trimmed.is_empty() {
                in_places = false;
            } else if let Some(failure) = failures.last_mut() {
                failure.places.push(relative(trimmed));
            }
        }
    }
    failures
}

/// Bỏ phần đường dẫn trên máy build (`…/target/semver-checks/git-v0_13_0/<hash>/crates/…`) trong
/// `previously in file …`: repo công khai, CHANGELOG không ghi đường dẫn riêng.
fn relative(place: &str) -> String {
    let Some((head, path)) = place.split_once("previously in file ") else {
        return place.to_owned();
    };
    let start = ["/crates/", "/modules/"].iter().filter_map(|marker| path.find(marker)).min();
    match start {
        Some(start) => format!("{head}previously in file {}", &path[start + 1..]),
        None => place.to_owned(),
    }
}

/// Mục `### Breaking (Rust)` cho `CHANGELOG.md` (tiếng Anh); rỗng khi không có gì.
pub fn markdown(failures: &[Failure]) -> String {
    if failures.is_empty() {
        return String::new();
    }
    let mut text = String::from("### Breaking (Rust)\n\n");
    for failure in failures {
        let places = if failure.places.is_empty() {
            String::new()
        } else {
            format!(
                ": {}",
                failure
                    .places
                    .iter()
                    .map(|place| format!("`{place}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        text.push_str(&format!(
            "- `{}` — {} (`{}`){places}\n",
            failure.krate, failure.title, failure.lint
        ));
    }
    text
}

/// Thông báo khi chưa cài công cụ.
pub fn missing_tool() -> String {
    format!(
        "cargo-semver-checks is not installed; install it with `cargo install cargo-semver-checks \
         --version {TOOL_VERSION} --locked` (it needs the network and takes a few minutes)"
    )
}

/// Lệnh `cargo xtask semver`. Trả `Ok(true)` khi đạt.
pub fn run(root: &Path, base: Option<&str>, deny: bool, as_markdown: bool) -> Result<bool, String> {
    let installed = Command::new("cargo")
        .args(["semver-checks", "--version"])
        .current_dir(root)
        .output()
        .is_ok_and(|output| output.status.success());
    if !installed {
        return Err(missing_tool());
    }
    let base = match base {
        Some(base) => base.to_owned(),
        None => match crate::api_diff::last_release_tag(root)? {
            Some(tag) => tag,
            None => {
                println!("semver: no release tag found; nothing to compare");
                return Ok(true);
            }
        },
    };
    let version = workspace_version(root)?;
    let mode = mode(&version, deny);
    let mut failures = Vec::new();
    for krate in CRATES {
        let output = Command::new("cargo")
            .args([
                "semver-checks",
                "check-release",
                "-p",
                krate,
                "--baseline-rev",
                &base,
                "--release-type",
                "patch",
            ])
            .current_dir(root)
            .output()
            .map_err(|error| format!("cannot run cargo semver-checks: {error}"))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let found = parse(krate, &text);
        if !output.status.success() && found.is_empty() {
            return Err(format!("cargo semver-checks failed for {krate}:\n{text}"));
        }
        failures.extend(found);
    }
    if as_markdown {
        print!("{}", markdown(&failures));
    } else {
        for failure in &failures {
            println!(
                "warning: {} {}: {} ({})",
                failure.krate,
                failure.lint,
                failure.title,
                failure.places.join("; ")
            );
        }
        println!(
            "semver: {} breaking change(s) to {} since {base}{}",
            failures.len(),
            CRATES.join(", "),
            if mode == Mode::Report { " (reported only while the version is 0.x)" } else { "" }
        );
    }
    Ok(failures.is_empty() || mode == Mode::Report)
}

/// `version` của `[workspace.package]` trong `Cargo.toml` gốc.
pub fn workspace_version(root: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(root.join("Cargo.toml"))
        .map_err(|error| format!("cannot read Cargo.toml: {error}"))?;
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[workspace.package]";
        } else if in_package
            && let Some(value) =
                line.strip_prefix("version").map(str::trim).and_then(|rest| rest.strip_prefix('='))
        {
            return Ok(value.trim().trim_matches('"').to_owned());
        }
    }
    Err("Cargo.toml has no [workspace.package] version".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "
     Parsing autoxide-kernel v0.13.0 (current)
    Checking autoxide-kernel v0.13.0 -> v0.13.0 (patch change)
     Checked [   0.1s] 150 checks: 149 pass, 1 fail, 0 warn, 0 skip

--- failure inherent_method_missing: pub method removed or renamed ---

Description:
A publicly-visible method or associated fn is no longer available under its prior name.
        ref: https://doc.rust-lang.org/cargo/reference/semver.html#item-remove

Failed in:
  IfMatch::from_value, previously in file /build/target/semver-checks/git-v0_13_0/00ac47eb/crates/kernel/src/version.rs:91

     Summary semver requires new major version: 1 major and 0 minor checks failed
";

    #[test]
    fn semver_khi_0x_chi_bao_cao() {
        assert_eq!(mode("0.13.0", false), Mode::Report);
    }

    #[test]
    fn semver_tu_1_0_thi_chan() {
        assert_eq!(mode("1.0.0", false), Mode::Deny);
        assert_eq!(mode("2.3.4", false), Mode::Deny);
    }

    #[test]
    fn semver_co_deny_thi_chan() {
        assert_eq!(mode("0.13.0", true), Mode::Deny);
    }

    #[test]
    fn semver_thieu_cong_cu_bao_lenh_cai() {
        let message = missing_tool();
        assert!(message.contains("cargo install cargo-semver-checks --version"), "{message}");
        assert!(message.contains(TOOL_VERSION));
    }

    #[test]
    fn semver_doc_ket_qua_cong_cu() {
        let failures = parse("autoxide-kernel", OUTPUT);
        assert_eq!(
            failures,
            [Failure {
                krate: "autoxide-kernel".to_owned(),
                lint: "inherent_method_missing".to_owned(),
                title: "pub method removed or renamed".to_owned(),
                // Đường dẫn trên máy build bị cắt về đường dẫn trong repo.
                places: vec![
                    "IfMatch::from_value, previously in file crates/kernel/src/version.rs:91"
                        .to_owned()
                ],
            }]
        );
        assert!(parse("autoxide-kernel", "     Summary no semver update required").is_empty());
    }

    #[test]
    fn semver_markdown_dung_dang_changelog() {
        assert_eq!(markdown(&[]), "");
        let text = markdown(&parse("autoxide-kernel", OUTPUT));
        assert_eq!(
            text,
            "### Breaking (Rust)\n\n- `autoxide-kernel` — pub method removed or renamed \
             (`inherent_method_missing`): `IfMatch::from_value, previously in file \
             crates/kernel/src/version.rs:91`\n"
        );
    }
}
