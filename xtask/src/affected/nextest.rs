//! Chạy test của crate bị ảnh hưởng bằng `cargo nextest` (gate nhanh trên SQLite): khi nào dùng, bản tối thiểu,
//! lệnh chạy. Tách khỏi `affected.rs` cho file đỡ dài.

use std::num::NonZero;
use std::path::Path;
use std::process::{Command, Stdio};

use super::{Affected, filterset, without_cargo_run_env};

/// Bản nextest thấp nhất hiểu `--filterset` và `--no-tests` (bản ghim ở `scripts/cloud-setup.sh` cao hơn nhiều).
const NEXTEST_MIN: (u64, u64, u64) = (0, 9, 85);

/// `Ok(true)`: có nextest đủ mới; `Ok(false)`: không cài (dùng bộ chạy cũ); `Err`: cài nhưng quá cũ — đỏ,
/// không lặng lẽ rơi về bộ cũ để gate trên máy này khác gate ở chỗ khác.
pub(super) fn available() -> Result<bool, String> {
    let Ok(output) = Command::new("cargo-nextest").arg("--version").stdin(Stdio::null()).output()
    else {
        return Ok(false);
    };
    if !output.status.success() {
        return Ok(false);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    match parse_version(&text) {
        Some(version) if version >= NEXTEST_MIN => Ok(true),
        _ => Err(format!(
            "cargo-nextest is too old or its version is unreadable (`{}`); `test-affected` needs \
             {}.{}.{} or newer — upgrade it (`scripts/cloud-setup.sh` installs the pinned one, or \
             `cargo install cargo-nextest --locked`)",
            text.trim(),
            NEXTEST_MIN.0,
            NEXTEST_MIN.1,
            NEXTEST_MIN.2
        )),
    }
}

/// `cargo-nextest 0.9.152 (…)` → `(0, 9, 152)`.
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let word =
        text.split_whitespace().find(|word| word.starts_with(|c: char| c.is_ascii_digit()))?;
    let mut parts = word.split(|c: char| !c.is_ascii_digit()).map(|part| part.parse::<u64>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// Build và chạy test của các crate đã chọn bằng nextest, chỉ SQLite (bỏ biến Postgres và S3 của người chạy).
pub(super) fn run(root: &Path, selected: &Affected, only: Option<&[String]>) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let mut command = Command::new(&cargo);
    without_cargo_run_env(&mut command).args(["nextest", "run", "--locked"]);
    match only {
        Some(names) => {
            command.args(names.iter().flat_map(|name| ["-p", name.as_str()]));
        }
        None => {
            command.arg("--workspace");
        }
    }
    // Mỗi nhân hai test: nhiều test chờ I/O (cùng cách `./gate.sh` chạy nextest).
    let threads = std::thread::available_parallelism().map_or(2, NonZero::get) * 2;
    command
        .args(["--filterset", &filterset(selected)])
        .args(["--no-fail-fast", "--no-tests", "pass", "--test-threads", &threads.to_string()])
        .env_remove("TEST_DATABASE_URL")
        .env_remove("TEST_S3_ENDPOINT")
        .current_dir(root)
        .stdin(Stdio::null());
    let status =
        command.status().map_err(|error| format!("cannot run `{cargo} nextest`: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("nextest reported failing tests (see above)".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_phien_ban_nextest() {
        assert_eq!(parse_version("cargo-nextest 0.9.152"), Some((0, 9, 152)));
        assert_eq!(parse_version("cargo-nextest 0.9.152 (abc 2026-01-01)"), Some((0, 9, 152)));
        assert!(parse_version("cargo-nextest 0.9.64").is_some_and(|v| v < NEXTEST_MIN));
        assert_eq!(parse_version("không phải số"), None);
    }
}
