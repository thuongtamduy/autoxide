//! Độ dài file `.rs` (D-20261118-file-length): file ngắn thì agent đọc trọn một lần, sửa không
//! giẫm nhau.
//!
//! - Quá [`WARN_LINES`] dòng: chỉ **in cảnh báo** (không làm gate đỏ) — tách theo trách nhiệm khi
//!   có dịp sửa file đó.
//! - Quá [`MAX_LINES`] dòng: luật `file-length` **chặn**, trừ file có tên trong [`BASELINE_FILE`]
//!   (file đã dài từ trước khi có luật). File trong danh sách đó không được dài hơn số dòng ghi kèm;
//!   tách xong mà còn dưới [`MAX_LINES`] thì xoá dòng của nó khỏi danh sách (gate nhắc). Không thêm
//!   file mới vào danh sách — tách file.
//!
//! Đếm mọi file `.rs` git theo dõi (cả test), trừ thư mục `target/`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::boundaries::Violation;

/// Quá số dòng này: cảnh báo.
pub const WARN_LINES: usize = 800;
/// Quá số dòng này: chặn, trừ file có trong danh sách miễn.
pub const MAX_LINES: usize = 1200;
/// Danh sách file được miễn và số dòng tối đa của từng file, tính từ gốc repo.
pub const BASELINE_FILE: &str = "xtask/file-length-baseline.txt";
/// Số file dài nhất nêu tên trong cảnh báo.
const WARN_SHOWN: usize = 10;

/// Kết quả: vi phạm (chặn) và dòng cảnh báo (chỉ in).
pub struct Lengths {
    pub violations: Vec<Violation>,
    pub warning: Option<String>,
}

pub fn check(root: &Path) -> Result<Lengths, String> {
    let baseline = read_baseline(root)?;
    let files = rust_files(root)?;
    let mut lengths = BTreeMap::new();
    for file in &files {
        let Ok(text) = fs::read_to_string(root.join(file)) else {
            continue;
        };
        lengths.insert(file.clone(), text.lines().count());
    }
    Ok(evaluate(&lengths, &baseline))
}

/// Phần không đọc đĩa của [`check`], để test được.
fn evaluate(lengths: &BTreeMap<String, usize>, baseline: &BTreeMap<String, usize>) -> Lengths {
    let mut violations = Vec::new();
    let mut push = |location: &str, message: String| {
        violations.push(Violation { rule: "file-length", location: location.to_owned(), message });
    };
    for (file, &lines) in lengths {
        match baseline.get(file) {
            None if lines > MAX_LINES => push(
                file,
                format!(
                    "{lines} lines, over the limit of {MAX_LINES} — split it into submodules by \
                     responsibility (move code, do not change it); see {BASELINE_FILE}"
                ),
            ),
            Some(&allowed) if lines > allowed => push(
                file,
                format!(
                    "{lines} lines, more than the {allowed} it had when the length rule came in — \
                     it must not grow: split it into submodules, or move what you added to a new \
                     file"
                ),
            ),
            Some(_) if lines <= MAX_LINES => push(
                BASELINE_FILE,
                format!(
                    "`{file}` is now {lines} lines, within the limit of {MAX_LINES}: remove its line \
                     from the list"
                ),
            ),
            _ => {}
        }
    }
    for file in baseline.keys().filter(|file| !lengths.contains_key(*file)) {
        push(BASELINE_FILE, format!("`{file}` no longer exists: remove its line from the list"));
    }

    let mut long: Vec<(&String, usize)> =
        lengths.iter().filter(|(_, lines)| **lines > WARN_LINES).map(|(f, l)| (f, *l)).collect();
    long.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let warning = (!long.is_empty()).then(|| {
        let shown: Vec<String> = long
            .iter()
            .take(WARN_SHOWN)
            .map(|(file, lines)| format!("{file} ({lines})"))
            .collect();
        format!(
            "warning: {} .rs files are over {WARN_LINES} lines; split one by responsibility when you \
             work in it. Longest: {}",
            long.len(),
            shown.join(", ")
        )
    });
    Lengths { violations, warning }
}

/// `<số dòng> <đường dẫn>` mỗi dòng; dòng trống và dòng bắt đầu bằng `#` bỏ qua.
fn read_baseline(root: &Path) -> Result<BTreeMap<String, usize>, String> {
    let Ok(text) = fs::read_to_string(root.join(BASELINE_FILE)) else {
        return Ok(BTreeMap::new());
    };
    parse_baseline(&text)
}

fn parse_baseline(text: &str) -> Result<BTreeMap<String, usize>, String> {
    let mut baseline = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parsed = line
            .split_once(' ')
            .and_then(|(lines, file)| Some((lines.parse::<usize>().ok()?, file.trim().to_owned())));
        let Some((lines, file)) = parsed else {
            return Err(format!(
                "{BASELINE_FILE}:{}: expected `<lines> <path>`, got `{line}`",
                index + 1
            ));
        };
        baseline.insert(file, lines);
    }
    Ok(baseline)
}

/// File `.rs` git theo dõi hay mới thêm (chưa bị `.gitignore` loại).
fn rust_files(root: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", "*.rs"])
        .current_dir(root)
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if !output.status.success() {
        return Err(format!("git ls-files: {}", String::from_utf8_lossy(&output.stderr)));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|file| !file.is_empty() && !file.starts_with("target/"))
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(items: &[(&str, usize)]) -> BTreeMap<String, usize> {
        items.iter().map(|(file, lines)| ((*file).to_owned(), *lines)).collect()
    }

    #[test]
    fn chan_file_moi_qua_dai_va_file_mien_dai_them() {
        let lengths =
            map(&[("a.rs", 1300), ("b.rs", 1500), ("c.rs", 900), ("d.rs", 100), ("e.rs", 1250)]);
        let baseline = map(&[("b.rs", 1400), ("d.rs", 1300), ("e.rs", 1250), ("gone.rs", 1500)]);
        let result = evaluate(&lengths, &baseline);
        let found: Vec<(&str, bool)> = result
            .violations
            .iter()
            .map(|violation| (violation.location.as_str(), violation.message.contains("remove")))
            .collect();
        // a: mới, quá giới hạn; b: dài thêm; d: đã ngắn, xoá khỏi danh sách; gone: không còn.
        assert_eq!(
            found,
            [("a.rs", false), ("b.rs", false), (BASELINE_FILE, true), (BASELINE_FILE, true)]
        );
        let warning = result.warning.unwrap();
        assert!(warning.starts_with("warning: 4 .rs files"), "{warning}");
        assert!(warning.contains("b.rs (1500), a.rs (1300), e.rs (1250), c.rs (900)"), "{warning}");
    }

    #[test]
    fn doc_danh_sach_mien() {
        let baseline = parse_baseline("# chú thích\n\n1988 modules/a.rs\n").unwrap();
        assert_eq!(baseline, map(&[("modules/a.rs", 1988)]));
        assert!(parse_baseline("modules/a.rs\n").is_err());
    }
}
