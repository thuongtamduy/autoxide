//! Luật `time-legacy` (D-20261128-time-duration-ops, D-20261127-transition-paths): đếm chỗ còn dùng đường cũ của
//! `autoxide_kernel::time` — những tên nhận hay trả kiểu của `jiff`. Module đã gỡ `jiff` khỏi `Cargo.toml` vẫn dùng
//! được kiểu của `jiff` qua các tên này (bí danh `SignedDuration`, `Date::weekday` trả `jiff::civil::Weekday`…), tức
//! một lối vòng qua luật `wrapped-libraries`. Chỗ cũ ghi ở [`BASELINE`], chỉ được giảm; danh sách rỗng thì gắn
//! `#[deprecated]` cho đường cũ ở bản phát hành đó, bản sau gỡ.
//!
//! Đếm trong mọi file `.rs` (kể cả test) của module, `crates/app`, `crates/host`, `crates/testkit`; kernel tự dùng
//! `jiff` bên trong nên không đếm. `.duration_since(` và `.weekday()` cũng là tên của thư viện chuẩn / `jiff` trên
//! kiểu khác (`Instant::duration_since`): chỗ như vậy đã có thì nằm trong danh sách; chỗ mới viết khác đi
//! (`now - earlier`, `Instant::elapsed`).

use std::fs;

use crate::boundaries::{Violation, rust_files};
use crate::workspace::{Role, Workspace};

/// Chỗ cũ chờ chuyển (cùng định dạng `security-baseline.txt`).
pub const BASELINE: &str = "xtask/time-legacy-baseline.txt";

/// (mẫu cần tìm, đường mới thay nó). Mẫu là chuỗi con; mẫu bắt đầu bằng chữ thì phải đứng sau một ký tự không thuộc
/// tên (để `MySignedDuration` không khớp `SignedDuration`).
const OLD_PATHS: &[(&str, &str)] = &[
    ("SignedDuration", "`autoxide_kernel::time::Duration`"),
    ("time::zone(", "`TimeZone::get`"),
    ("from_local(", "`TimeZone::to_timestamp`"),
    ("parse_clock(", "`TimeOfDay::parse_hm`"),
    (".duration_since(", "`Timestamp::since` (or `later - earlier`)"),
    (".in_zone(", "`Timestamp::to_local`"),
    (".weekday()", "`Date::day_of_week` / `DateTime::weekday` (kernel `Weekday`)"),
];

/// Vi phạm `time-legacy` của mọi crate trừ kernel, macro và công cụ.
pub fn check(workspace: &Workspace) -> Result<Vec<Violation>, String> {
    let mut violations = Vec::new();
    for package in &workspace.packages {
        if !matches!(package.role, Role::Module | Role::App | Role::Host | Role::Testkit) {
            continue;
        }
        let root = workspace.root.join(&package.dir);
        for dir in ["src", "tests", "benches"] {
            if !root.join(dir).is_dir() {
                continue;
            }
            // `rust_files` luôn ghi tiền tố `src/`: đổi thành thư mục đang quét (`tests/…`, `benches/…`).
            for file in rust_files(&root.join(dir))? {
                let file = format!("{dir}{}", file.strip_prefix("src").unwrap_or(&file));
                let text = fs::read_to_string(root.join(&file))
                    .map_err(|error| format!("{}/{file}: {error}", package.dir))?;
                violations.extend(violations_in(&format!("{}/{file}", package.dir), &text));
            }
        }
    }
    Ok(violations)
}

/// Mỗi lần một mẫu cũ xuất hiện trong dòng code (bỏ dòng chú thích `//`).
fn violations_in(path: &str, text: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        for (pattern, instead) in OLD_PATHS {
            for _ in 0..occurrences(line, pattern) {
                violations.push(Violation {
                    rule: "time-legacy",
                    location: format!("{path}:{}", index + 1),
                    message: format!(
                        "`{}` is an old path of `autoxide_kernel::time` that takes or returns a `jiff` type: use \
                         {instead} (D-20261128-time-duration-ops)",
                        pattern.trim_start_matches('.').trim_end_matches('(')
                    ),
                });
            }
        }
    }
    violations
}

fn occurrences(line: &str, pattern: &str) -> usize {
    let starts_with_ident = pattern.starts_with(|c: char| c.is_ascii_alphanumeric());
    line.match_indices(pattern)
        .filter(|(at, _)| {
            !starts_with_ident
                || line[..*at]
                    .chars()
                    .next_back()
                    .is_none_or(|before| !(before.is_ascii_alphanumeric() || before == '_'))
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dem_moi_duong_cu_bo_chu_thich() {
        let text = "use autoxide_kernel::time::SignedDuration;\n\
                    // SignedDuration trong chú thích không tính\n\
                    let d = SignedDuration::from_hours(1) + SignedDuration::from_mins(1);\n\
                    let z = time::zone(\"UTC\");\n\
                    let t = time::from_local(&z, local);\n\
                    let c = time::parse_clock(\"09:00\");\n\
                    let s = later.duration_since(earlier);\n\
                    let l = now.in_zone(&z);\n\
                    let w = date.weekday();\n";
        let found = violations_in("modules/x/src/lib.rs", text);
        let lines: Vec<&str> = found.iter().map(|v| v.location.as_str()).collect();
        assert_eq!(
            lines,
            [
                "modules/x/src/lib.rs:1",
                "modules/x/src/lib.rs:3",
                "modules/x/src/lib.rs:3",
                "modules/x/src/lib.rs:4",
                "modules/x/src/lib.rs:5",
                "modules/x/src/lib.rs:6",
                "modules/x/src/lib.rs:7",
                "modules/x/src/lib.rs:8",
                "modules/x/src/lib.rs:9",
            ]
        );
        assert!(found.iter().all(|v| v.rule == "time-legacy"));
        assert!(found[0].message.contains("time::Duration"), "{}", found[0].message);
    }

    #[test]
    fn duong_moi_va_ten_gan_giong_khong_tinh() {
        let text = "let d = Duration::from_hours(1);\n\
                    let z = TimeZone::get(\"UTC\");\n\
                    let s = later.since(earlier);\n\
                    let w = date.day_of_week();\n\
                    struct MySignedDuration;\n\
                    let x = my_from_local(1);\n";
        assert!(violations_in("modules/x/src/lib.rs", text).is_empty());
    }
}
