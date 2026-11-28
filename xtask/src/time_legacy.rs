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
    let imports = zone_imports(text);
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
        // Trong câu `use`: mỗi lần nhập `zone` tính một lần, ở dòng đầu câu. Ngoài câu `use`: `time::zone` (đường đầy
        // đủ, kể cả làm giá trị) + mỗi lần gọi tên đã nhập (`zone`, hay bí danh).
        let in_use =
            imports.statements.iter().find(|(first, last, _)| (*first..=*last).contains(&index));
        let zones = match in_use {
            Some((first, _, count)) => {
                if *first == index {
                    *count
                } else {
                    0
                }
            }
            None => {
                line.match_indices("time::zone")
                    .filter(|(at, _)| ends_word(line, at + "time::zone".len()))
                    .count()
                    + imports.names.iter().map(|name| bare_occurrences(line, name)).sum::<usize>()
            }
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

/// Tên tại chỗ của hàm `zone` mà file nhập từ `autoxide_kernel::time` bằng câu `use` / `pub use` (kể cả nhiều dòng,
/// import lồng `autoxide_kernel::{time::{self, zone}}`, `time::*`, bí danh `zone as tz`). Chỉ xét câu `use`: file gọi
/// đầy đủ `autoxide_kernel::time::zone(…)` không làm biến hay trường tên `zone` khác bị đếm.
fn zone_imports(text: &str) -> ZoneImports {
    let mut imports = ZoneImports::default();
    for (first, last, statement) in use_statements(text) {
        let mut count = 0;
        for (path, alias) in flatten_use(&statement) {
            let local = match path.as_str() {
                "autoxide_kernel::time::zone" => alias.unwrap_or_else(|| "zone".to_owned()),
                "autoxide_kernel::time::*" => "zone".to_owned(),
                _ => continue,
            };
            // `time::*` không nêu tên `zone`: chỉ tính khi file thật sự gọi nó (ở dòng gọi).
            if path.ends_with("::zone") {
                count += 1;
            }
            if !imports.names.contains(&local) {
                imports.names.push(local);
            }
        }
        imports.statements.push((first, last, count));
    }
    imports
}

/// Những gì file nhập: tên tại chỗ của `zone`, và mọi câu `use` (dòng đầu, dòng cuối, số lần nhập `zone`).
#[derive(Default)]
struct ZoneImports {
    names: Vec<String>,
    statements: Vec<(usize, usize, usize)>,
}

/// Các câu `use`: (dòng đầu, dòng cuối, phần sau chữ `use` tới trước `;`), dòng tính từ 0.
fn use_statements(text: &str) -> Vec<(usize, usize, String)> {
    let mut statements = Vec::new();
    let mut offset = 0;
    let mut rest = text;
    while let Some(at) = rest.find("use ") {
        let before = &rest[..at];
        let trimmed = before.trim_end_matches([' ', '\t']);
        // `use` đứng đầu dòng (có thể sau `pub` / `pub(crate)`), không phải chữ `use` giữa câu hay trong chú thích.
        let line_start = trimmed.rsplit('\n').next().unwrap_or(trimmed).trim_start();
        let starts = line_start.is_empty()
            || line_start == "pub"
            || line_start.starts_with("pub(") && line_start.ends_with(')');
        let tail = &rest[at + "use ".len()..];
        let absolute = offset + at;
        if starts {
            let end = tail.find(';').unwrap_or(tail.len());
            let first = text[..absolute].matches('\n').count();
            let last = first + tail[..end].matches('\n').count();
            statements.push((first, last, tail[..end].to_owned()));
            offset = absolute + "use ".len() + end;
            rest = &tail[end..];
        } else {
            offset = absolute + "use ".len();
            rest = tail;
        }
    }
    statements
}

/// Các đường của một cây `use` (`a::{b, c::{d as e}}`) thành (đường đầy đủ, bí danh).
fn flatten_use(tree: &str) -> Vec<(String, Option<String>)> {
    // Gom khoảng trắng: `a :: { b , c as d }` → `a::{b,c as d}`.
    let compact: String = tree.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut compact = compact;
    for token in ["::", "{", "}", ","] {
        compact = compact.replace(&format!(" {token}"), token).replace(&format!("{token} "), token);
    }
    let mut out = Vec::new();
    flatten_into(&compact, "", &mut out);
    out
}

fn flatten_into(tree: &str, prefix: &str, out: &mut Vec<(String, Option<String>)>) {
    let tree = tree.trim();
    if tree.is_empty() {
        return;
    }
    if let Some(inner) = tree.strip_prefix('{').and_then(|t| t.strip_suffix('}')) {
        for part in split_top(inner) {
            flatten_into(part, prefix, out);
        }
        return;
    }
    // `head::rest`: tách ở `::` đầu tiên ngoài ngoặc.
    if let Some(at) = top_level_find(tree, "::") {
        let (head, rest) = (&tree[..at], &tree[at + 2..]);
        flatten_into(rest, &join(prefix, head), out);
        return;
    }
    let (name, alias) = match tree.split_once(" as ") {
        Some((name, alias)) => (name.trim(), Some(alias.trim().to_owned())),
        None => (tree, None),
    };
    // `self` trong ngoặc là chính đường phía trước (`time::{self}` nhập `time`).
    let path = if name == "self" { prefix.to_owned() } else { join(prefix, name) };
    out.push((path, alias));
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() { name.to_owned() } else { format!("{prefix}::{name}") }
}

/// Các phần của danh sách `a,b::{c,d},e` tách ở dấu phẩy ngoài ngoặc.
fn split_top(list: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0usize, 0);
    for (index, c) in list.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&list[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&list[start..]);
    parts
}

fn top_level_find(text: &str, needle: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (index, c) in text.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 && text[index..].starts_with(needle) => return Some(index),
            _ => {}
        }
    }
    None
}

/// Số lần `name` đứng riêng thành tên mà không phải đuôi của một đường (`a::zone`), một trường (`row.zone`) hay tên
/// mới sau `as` — tức là gọi đến hàm đã nhập.
fn bare_occurrences(line: &str, name: &str) -> usize {
    line.match_indices(name)
        .filter(|(at, _)| {
            let before = &line[..*at];
            before.chars().next_back().is_none_or(|c| !is_ident(c))
                && ends_word(line, at + name.len())
                && !before.ends_with("::")
                && !before.ends_with('.')
                && !before.trim_end().ends_with(" as")
        })
        .count()
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
        // Gọi đầy đủ không phải nhập: biến `zone` khác trong file không bị đếm.
        assert_eq!(lines_of("let z = autoxide_kernel::time::zone(\"UTC\");\nlet zone = 1;\n"), [1]);
        // Import nhiều dòng, lồng, bí danh, `*`.
        let nested = "use autoxide_kernel::{\n    db::Queries,\n    time::{self, zone as tz},\n};\n\
                      let z = tz(\"UTC\");\nlet zone = 1;\n";
        assert_eq!(lines_of(nested), [1, 5]);
        assert_eq!(
            lines_of("use autoxide_kernel::time::*;\nlet z = zone(\"UTC\");\nlet r = row.zone;\n"),
            [2]
        );
        assert_eq!(lines_of("pub use autoxide_kernel::time::zone;\n"), [1]);
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
