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

/// (tên cũ, đường mới thay nó). Tên được đếm theo ranh giới tên (không đòi `(` theo sau): dùng làm giá trị
/// (`.and_then(time::zone)`, `let parse = time::parse_clock;`) hay gọi đầy đủ (`Timestamp::in_zone(…)`) cũng tính.
const OLD_NAMES: &[(&str, &str)] = &[
    ("SignedDuration", "`autoxide_kernel::time::Duration`"),
    ("from_local", "`TimeZone::to_timestamp`"),
    ("parse_clock", "`TimeOfDay::parse_hm`"),
    ("duration_since", "`Timestamp::since` (or `later - earlier`)"),
    ("in_zone", "`Timestamp::to_local`"),
];

/// `zone` là tên chung: chỉ tính `time::zone`, hay `zone` trần trong file nhập nó từ `autoxide_kernel::time`.
const ZONE_INSTEAD: &str = "`TimeZone::get`";
/// `weekday` cũ (`Date::weekday`, trả `jiff::civil::Weekday`): `.weekday()` và `::weekday`. Đường mới tên khác
/// (`Date::day_of_week`, `DateTime::day_of_week`), nên mọi chỗ khớp là đường cũ hay `weekday` của `jiff`.
const WEEKDAY_INSTEAD: &str = "`Date::day_of_week` / `DateTime::day_of_week` (kernel `Weekday`)";

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

/// Mỗi lần một tên cũ xuất hiện trong dòng code (bỏ dòng chú thích `//`).
fn violations_in(path: &str, text: &str) -> Vec<Violation> {
    let zone_imported = imports_zone(text);
    let mut violations = Vec::new();
    let mut push = |line: usize, name: &str, instead: &str| {
        violations.push(Violation {
            rule: "time-legacy",
            location: format!("{path}:{line}"),
            message: format!(
                "`{name}` is an old path of `autoxide_kernel::time` that takes or returns a `jiff` type: use \
                 {instead} (D-20261128-time-duration-ops)"
            ),
        });
    };
    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        for (name, instead) in OLD_NAMES {
            for _ in 0..word_occurrences(line, name) {
                push(index + 1, name, instead);
            }
        }
        let zones = if zone_imported {
            word_occurrences(line, "zone")
        } else {
            line.match_indices("time::zone")
                .filter(|(at, _)| ends_word(line, at + "time::zone".len()))
                .count()
        };
        for _ in 0..zones {
            push(index + 1, "zone", ZONE_INSTEAD);
        }
        let weekdays = line.matches(".weekday()").count()
            + line
                .match_indices("::weekday")
                .filter(|(at, _)| ends_word(line, at + "::weekday".len()))
                .count();
        for _ in 0..weekdays {
            push(index + 1, "weekday", WEEKDAY_INSTEAD);
        }
    }
    violations
}

/// File nhập `zone` từ `autoxide_kernel::time` (`use autoxide_kernel::time::zone;` hay trong ngoặc nhọn).
fn imports_zone(text: &str) -> bool {
    let mut rest = text;
    while let Some(at) = rest.find("autoxide_kernel::time::") {
        let tail = &rest[at + "autoxide_kernel::time::".len()..];
        let item = match tail.strip_prefix('{') {
            Some(inner) => &inner[..inner.find('}').unwrap_or(inner.len())],
            None => {
                &tail[..tail
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(tail.len())]
            }
        };
        if item.split(',').any(|part| part.trim() == "zone") {
            return true;
        }
        rest = tail;
    }
    false
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn ends_word(line: &str, end: usize) -> bool {
    line[end..].chars().next().is_none_or(|after| !is_ident(after))
}

/// Số lần `name` đứng riêng thành một tên (không dính chữ hay `_` hai bên).
fn word_occurrences(line: &str, name: &str) -> usize {
    line.match_indices(name)
        .filter(|(at, _)| {
            line[..*at].chars().next_back().is_none_or(|before| !is_ident(before))
                && ends_word(line, at + name.len())
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(text: &str) -> Vec<usize> {
        violations_in("modules/x/src/lib.rs", text)
            .iter()
            .map(|v| v.location.rsplit(':').next().unwrap().parse().unwrap())
            .collect()
    }

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
        assert_eq!(lines_of(text), [1, 3, 3, 4, 5, 6, 7, 8, 9]);
        let found = violations_in("modules/x/src/lib.rs", text);
        assert!(found.iter().all(|v| v.rule == "time-legacy"));
        assert!(found[0].message.contains("time::Duration"), "{}", found[0].message);
    }

    #[test]
    fn ten_lam_gia_tri_va_goi_day_du_cung_tinh() {
        let text = "let z = name.and_then(time::zone);\n\
                    let parse = time::parse_clock;\n\
                    let at = zones.map(time::from_local);\n\
                    let l = Timestamp::in_zone(now, &z);\n\
                    let s = Timestamp::duration_since(later, earlier);\n\
                    let w = Date::weekday(day);\n";
        assert_eq!(lines_of(text), [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn zone_nhap_trong_ngoac_nhon_thi_dem_ten_tran() {
        let text = "use autoxide_kernel::time::{Timestamp, zone};\n\
                    let z = zone(\"Asia/Ho_Chi_Minh\");\n\
                    let y = names.filter_map(zone);\n\
                    let time_zone = 1;\n";
        assert_eq!(lines_of(text), [1, 2, 3]);
        // Không nhập `zone` thì `zone` trần là tên khác (trường, biến).
        assert!(lines_of("let zone = row.zone;\nlet z = zone(1);\n").is_empty());
        assert_eq!(lines_of("use autoxide_kernel::time::zone;\nlet z = zone(\"UTC\");\n"), [1, 2]);
    }

    #[test]
    fn duong_moi_va_ten_gan_giong_khong_tinh() {
        let text = "let d = Duration::from_hours(1);\n\
                    let z = TimeZone::get(\"UTC\");\n\
                    let s = later.since(earlier);\n\
                    let w = date.day_of_week();\n\
                    let v = local.day_of_week();\n\
                    struct MySignedDuration;\n\
                    let x = my_from_local(1);\n\
                    let y = timezone(1);\n\
                    let weekdays = 5;\n";
        assert!(lines_of(text).is_empty(), "{:?}", lines_of(text));
    }
}
