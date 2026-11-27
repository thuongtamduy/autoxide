//! `cargo xtask migration-releases record <X.Y.Z> [--from <file>]`: ghi vào `crates/app/migration-releases.json` các
//! migration lần đầu có ở bản X.Y.Z (D-20261126-upgrade-rollback). `autoxide rollback --to X.Y.Z` dựa vào bảng này
//! để biết bản X.Y.Z gồm những migration nào. Chạy lúc phát hành, trước khi tag (`docs/RELEASING.md`).
//!
//! Danh sách migration lấy từ `cargo run -p autoxide -- migrate manifest` (binary đang build: mọi module, kể cả
//! module đang tắt), hay từ `--from <file>` (mỗi dòng `<module>/<tên>`; `scripts/migration-releases-backfill.sh` sinh
//! file này từ image Docker của một bản đã phát hành). Migration đã có trong bảng giữ bản đầu tiên của nó; ghi lại
//! cùng một bản là không đổi gì.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

pub const FILE: &str = "crates/app/migration-releases.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct Table {
    releases: Vec<String>,
    introduced: BTreeMap<String, String>,
}

pub fn record(root: &Path, version: &str, from: Option<&str>) -> Result<usize, String> {
    if !is_version(version) {
        return Err(format!("`{version}` is not a version `X.Y.Z`"));
    }
    let names = match from {
        Some(file) => {
            fs::read_to_string(file).map_err(|error| format!("cannot read {file}: {error}"))?
        }
        None => manifest(root)?,
    };
    let path = root.join(FILE);
    let mut table: Table = match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|error| format!("{FILE}: {error}"))?,
        Err(_) => Table::default(),
    };
    let added = add(&mut table, version, names.lines());
    let mut text = serde_json::to_string_pretty(&table).map_err(|error| error.to_string())?;
    text.push('\n');
    fs::write(&path, text).map_err(|error| format!("cannot write {FILE}: {error}"))?;
    Ok(added)
}

/// Phần không đọc đĩa của [`record`]: thêm bản và các migration mới; trả số migration mới.
fn add<'a>(table: &mut Table, version: &str, names: impl Iterator<Item = &'a str>) -> usize {
    if !table.releases.iter().any(|release| release == version) {
        table.releases.push(version.to_owned());
        table.releases.sort_by(|a, b| compare(a, b));
    }
    let mut added = 0;
    for name in names.map(str::trim).filter(|name| name.contains('/')) {
        let first = table.introduced.entry(name.to_owned()).or_insert_with(|| {
            added += 1;
            version.to_owned()
        });
        // Bản cũ hơn được ghi sau (điền ngược): giữ bản sớm nhất.
        if compare(version, first) == Ordering::Less {
            *first = version.to_owned();
        }
    }
    added
}

fn manifest(root: &Path) -> Result<String, String> {
    let output = Command::new("cargo")
        .args(["run", "-q", "-p", "autoxide", "--", "migrate", "manifest"])
        .current_dir(root)
        .output()
        .map_err(|error| format!("cannot run `autoxide migrate manifest`: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "`autoxide migrate manifest` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

fn is_version(version: &str) -> bool {
    let core = version.split_once('-').map_or(version, |(core, _)| core);
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// So hai phiên bản theo SemVer (như `crates/app/src/releases.rs`).
fn compare(left: &str, right: &str) -> Ordering {
    let parse = |version: &str| {
        let (core, pre) = match version.split_once('-') {
            Some((core, pre)) => (core, Some(pre.to_owned())),
            None => (version, None),
        };
        let numbers: Vec<u64> = core.split('.').map(|part| part.parse().unwrap_or(0)).collect();
        (numbers, pre)
    };
    let (left_numbers, left_pre) = parse(left);
    let (right_numbers, right_pre) = parse(right);
    left_numbers.cmp(&right_numbers).then_with(|| match (left_pre, right_pre) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => left.cmp(&right),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn giu_ban_som_nhat_va_sap_ban_theo_semver() {
        let mut table = Table::default();
        assert_eq!(add(&mut table, "0.10.0", ["a/m1", "a/m2"].into_iter()), 2);
        assert_eq!(add(&mut table, "0.9.0", ["a/m1"].into_iter()), 0);
        assert_eq!(add(&mut table, "0.10.0", ["a/m1", "a/m2"].into_iter()), 0);
        assert_eq!(table.releases, ["0.9.0", "0.10.0"]);
        assert_eq!(table.introduced["a/m1"], "0.9.0");
        assert_eq!(table.introduced["a/m2"], "0.10.0");
        assert!(is_version("1.0.0-rc.1") && !is_version("1.0") && !is_version("v1.0.0"));
    }
}
