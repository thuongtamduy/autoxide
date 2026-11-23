//! `cargo xtask semver [--base <rev>] [--changed <rev>] [--deny] [--markdown]`: báo thay đổi phá vỡ của
//! hợp đồng Rust so với tag phát hành trước, bằng `cargo semver-checks` (D-233, đặc tả 31 A7, bước 70 P4).
//!
//! - Hợp đồng (`docs/COMPATIBILITY.md`): `autoxide-kernel`, `autoxide-testkit`, crate `autoxide` (thư viện,
//!   `autoxide::Autoxide`) và crate của mọi module hạng core / official — đọc từ bảng sinh tự động của
//!   `docs/MODULES.md`, nên module đổi hạng thì danh sách tự đổi theo. Module thử nghiệm không có cam kết.
//! - `--changed <rev>`: chỉ kiểm crate có file thuộc phần công khai đổi so với `<rev>` (job CI của pull
//!   request: kiểm hết ~30 crate mất gần một giờ, nên PR chỉ kiểm thứ nó chạm).
//!
//! - Còn `0.x`: chỉ báo cáo (thoát 0 dù có phá vỡ) — mọi bản minor của `0.x` được phép phá vỡ
//!   (D-167). Từ `1.0.0`, hay với `--deny`: có phá vỡ thì thoát 1.
//! - `--markdown`: in mục `### Breaking (Rust)` để chép vào `CHANGELOG.md` lúc phát hành.
//! - Còn `0.x`, công cụ ép so như bản **patch** (`--release-type patch`): với `0.x`, bản minor được coi
//!   như major nên công cụ sẽ không báo gì; so như patch thì mọi thay đổi phá vỡ đều hiện. Từ `1.0` so
//!   như **minor**: chỉ phá vỡ bị báo; bản tăng major (`2.0.0` so với `v1.x`) chỉ báo cáo.
//! - Cần mạng và build bản cũ, lâu: chạy ở job CI `semver` (PR chạm phần công khai, PR phát hành) và lúc
//!   phát hành, không trong `./gate.sh`.

use std::path::Path;
use std::process::Command;

/// Crate nền luôn là hợp đồng (D-164, D-167, D-20261121-external-modules), kèm đường dẫn của phần
/// công khai: sửa file ở đó thì job CI của pull request kiểm crate này.
const BASE_CRATES: [(&str, &[&str]); 3] = [
    ("autoxide-kernel", &["crates/kernel/"]),
    ("autoxide-testkit", &["crates/testkit/"]),
    // Phần công khai của crate `autoxide` chỉ có `Autoxide` ở `lib.rs`; phần còn lại là `mod` riêng.
    ("autoxide", &["crates/app/src/lib.rs"]),
];

/// Một crate thuộc hợp đồng và các tiền tố đường dẫn của phần công khai của nó.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    pub krate: String,
    pub paths: Vec<String>,
}

impl Contract {
    /// Có file nào trong `changed` thuộc phần công khai của crate này không.
    fn touched_by(&self, changed: &[&str]) -> bool {
        changed.iter().any(|file| {
            self.paths.iter().any(|path| {
                if path.ends_with('/') { file.starts_with(path.as_str()) } else { file == path }
            })
        })
    }
}

/// Tên module hạng `core` hay `official` trong bảng giữa `<!-- modules:begin -->` và
/// `<!-- modules:end -->` của `docs/MODULES.md` (bảng do `autoxide` sinh, test của app giữ nó khớp code).
pub fn stable_modules(modules_md: &str) -> Result<Vec<String>, String> {
    let begin = modules_md.find("<!-- modules:begin -->");
    let end = modules_md.find("<!-- modules:end -->");
    let (Some(begin), Some(end)) = (begin, end) else {
        return Err(
            "docs/MODULES.md has no `<!-- modules:begin -->` … `<!-- modules:end -->` table"
                .to_owned(),
        );
    };
    let mut names = Vec::new();
    for line in modules_md[begin..end].lines() {
        // `| `name` | tier | needs | what |`
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        let [_, name, tier, ..] = cells.as_slice() else { continue };
        let Some(name) = name.strip_prefix('`').and_then(|name| name.strip_suffix('`')) else {
            continue;
        };
        if matches!(*tier, "core" | "official") {
            names.push(name.to_owned());
        }
    }
    if names.is_empty() {
        return Err("docs/MODULES.md lists no core or official module".to_owned());
    }
    Ok(names)
}

/// Mọi crate thuộc hợp đồng Rust: ba crate nền và crate của module core / official.
pub fn contract(root: &Path) -> Result<Vec<Contract>, String> {
    let text = std::fs::read_to_string(root.join("docs/MODULES.md"))
        .map_err(|error| format!("cannot read docs/MODULES.md: {error}"))?;
    let mut crates: Vec<Contract> = BASE_CRATES
        .iter()
        .map(|(krate, paths)| Contract {
            krate: (*krate).to_owned(),
            paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        })
        .collect();
    for module in stable_modules(&text)? {
        let dir = format!("modules/{module}");
        if !root.join(&dir).join("Cargo.toml").is_file() {
            return Err(format!(
                "docs/MODULES.md lists module `{module}` but {dir}/Cargo.toml is missing"
            ));
        }
        crates.push(Contract {
            // Quy ước tên crate: `autoxide-<module>`, `_` thành `-` (vd `einvoice_sepay`).
            krate: format!("autoxide-{}", module.replace('_', "-")),
            // Module chỉ mở `pub mod api;` và `pub fn module(…)` ở `lib.rs` (D-031).
            paths: vec![
                format!("{dir}/src/lib.rs"),
                format!("{dir}/src/api.rs"),
                format!("{dir}/src/api/"),
            ],
        });
    }
    Ok(crates)
}

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

/// Số major của một phiên bản (`1.2.3`) hay tag (`v1.2.3`).
fn major(version: &str) -> Option<u64> {
    version.trim_start_matches('v').split('.').next().and_then(|major| major.parse::<u64>().ok())
}

/// `Deny` khi `version` của workspace ≥ `1.0.0` hay có `--deny`. Ngoại lệ: bản phát hành tăng major
/// (`base` là tag `v1.4.0`, workspace `2.0.0`) được phép phá vỡ — chỉ báo cáo, để ghi vào CHANGELOG.
pub fn mode(workspace_version: &str, base: &str, deny: bool) -> Mode {
    let current = major(workspace_version);
    let major_release =
        matches!((current, major(base)), (Some(current), Some(base)) if current > base);
    if deny || (current.is_some_and(|major| major >= 1) && !major_release) {
        Mode::Deny
    } else {
        Mode::Report
    }
}

/// Loại bản phát hành đưa cho công cụ. Còn `0.x`: `patch`, để mọi thay đổi phá vỡ đều hiện (với `0.x`
/// công cụ coi bản minor như major nên sẽ im lặng). Từ `1.0`: `minor` — thêm mới (kể cả `#[deprecated]`)
/// là hợp lệ, chỉ phá vỡ mới bị báo.
pub fn release_type(workspace_version: &str) -> &'static str {
    if major(workspace_version).is_some_and(|major| major >= 1) { "minor" } else { "patch" }
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
pub fn run(
    root: &Path,
    base: Option<&str>,
    changed_since: Option<&str>,
    deny: bool,
    as_markdown: bool,
) -> Result<bool, String> {
    let mut crates = contract(root)?;
    if let Some(since) = changed_since {
        let diff =
            crate::api_diff::git(root, &["diff", "--name-only", &format!("{since}...HEAD")])?;
        let changed: Vec<&str> = diff.lines().collect();
        crates.retain(|contract| contract.touched_by(&changed));
        if crates.is_empty() {
            eprintln!("semver: no public Rust API changed since {since}; nothing to check");
            return Ok(true);
        }
    }
    let crates: Vec<String> = crates.into_iter().map(|contract| contract.krate).collect();
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
                eprintln!("semver: no release tag found; nothing to compare");
                return Ok(true);
            }
        },
    };
    let version = workspace_version(root)?;
    let mode = mode(&version, &base, deny);
    let mut failures = Vec::new();
    for krate in &crates {
        let output = Command::new("cargo")
            .args([
                "semver-checks",
                "check-release",
                "-p",
                krate,
                "--baseline-rev",
                &base,
                "--release-type",
                release_type(&version),
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
    // `--markdown`: stdout chỉ chứa mục cho CHANGELOG (rỗng khi không có gì) — CI xét file rỗng hay không;
    // dòng thông tin đi stderr.
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
            crates.join(", "),
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

    const MODULES_MD: &str = "intro `x` | y |
<!-- modules:begin -->
| Module | Tier | Needs | What it does |
| --- | --- | --- | --- |
| `mail` | core | — | Mail. |
| `catalog` | official | — | Products. |
| `einvoice_sepay` | experimental | — | E-invoices. |
<!-- modules:end -->
| `after` | official | — | Outside the table. |
";

    #[test]
    fn semver_chi_lay_module_core_va_official() {
        assert_eq!(stable_modules(MODULES_MD).unwrap(), ["mail", "catalog"]);
        assert!(stable_modules("no table").is_err());
    }

    #[test]
    fn semver_hop_dong_tu_repo_co_ca_crate_nen_va_module_chinh_thuc() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let crates: Vec<String> =
            contract(root).unwrap().into_iter().map(|contract| contract.krate).collect();
        for krate in
            ["autoxide-kernel", "autoxide-testkit", "autoxide", "autoxide-auth", "autoxide-catalog"]
        {
            assert!(crates.iter().any(|name| name == krate), "{krate} missing from {crates:?}");
        }
        // Module thử nghiệm và module mẫu không có cam kết.
        for krate in ["autoxide-bookings", "autoxide-notes", "autoxide-sepay"] {
            assert!(!crates.iter().any(|name| name == krate), "{krate} must not be in {crates:?}");
        }
    }

    #[test]
    fn semver_chi_kiem_crate_bi_cham_phan_cong_khai() {
        let contract = Contract {
            krate: "autoxide-catalog".to_owned(),
            paths: vec![
                "modules/catalog/src/lib.rs".to_owned(),
                "modules/catalog/src/api.rs".to_owned(),
                "modules/catalog/src/api/".to_owned(),
            ],
        };
        assert!(contract.touched_by(&["modules/catalog/src/api/products.rs"]));
        assert!(contract.touched_by(&["docs/x.md", "modules/catalog/src/lib.rs"]));
        assert!(!contract.touched_by(&["modules/catalog/src/products.rs"]));
        assert!(!contract.touched_by(&["modules/catalog/src/api.rs.bak"]));
    }

    #[test]
    fn semver_khi_0x_chi_bao_cao() {
        assert_eq!(mode("0.13.0", "v0.12.0", false), Mode::Report);
        assert_eq!(release_type("0.13.0"), "patch");
    }

    #[test]
    fn semver_tu_1_0_thi_chan() {
        assert_eq!(mode("1.0.0", "v0.19.0", false), Mode::Report, "1.0 itself may break 0.x");
        assert_eq!(mode("1.1.0", "v1.0.0", false), Mode::Deny);
        assert_eq!(mode("2.3.4", "v2.3.0", false), Mode::Deny);
        // Pull request: base là SHA, không phải tag.
        assert_eq!(mode("1.1.0", "0123abcd", false), Mode::Deny);
        assert_eq!(release_type("1.1.0"), "minor");
    }

    #[test]
    fn semver_ban_tang_major_chi_bao_cao() {
        assert_eq!(mode("2.0.0", "v1.4.2", false), Mode::Report);
    }

    #[test]
    fn semver_co_deny_thi_chan() {
        assert_eq!(mode("0.13.0", "v0.12.0", true), Mode::Deny);
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
