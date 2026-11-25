//! Luật về ID quyết định (D-20261118-decision-ids):
//!
//! - `decisions-id` — mục cũ trong `docs/DECISIONS.md` mang số `D-` + ba chữ số; mục mới là một file
//!   `docs/decisions/D-<YYYYMMDD>-<tên>.md` chứa đúng một mục, ID trong tiêu đề trùng tên file, phần
//!   ngày trùng ngày của mục, tên viết `a-z0-9` nối bằng `-`. Không ID nào xuất hiện hai lần. Dòng
//!   `Chủ đề:` (nếu có) phải là một chủ đề của mục lục. Đoạn `Thay…` phải nêu được ít nhất một ID.
//! - `decisions-ref` — mọi `D-xxx` / `D-<ngày>-<tên>` nhắc tới trong file văn bản của repo (code,
//!   tài liệu, cấu hình) là một mục có thật: số gõ nhầm hay mục bị xoá thì gate đỏ.
//!
//! Vì sao ID theo ngày và tên, không theo số tuần tự: hai phiên làm song song trên hai nhánh cùng
//! gốc đều thấy cùng "số cuối", nên luôn chọn trùng số kế tiếp; tên file khác nhau thì git cũng không
//! xung đột khi gộp. Hai phiên chọn trùng cả ngày lẫn tên (hiếm) thì git báo xung đột "cùng thêm một
//! file" và luật không trùng ở đây bắt nốt.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use super::{DIR, Entry, INDEX_FILE, LOG_FILE, TOPICS, ids_in};
use crate::boundaries::Violation;

/// Đuôi file được quét tìm tham chiếu.
const TEXT_EXTENSIONS: [&str; 9] = ["md", "rs", "toml", "sh", "json", "yml", "yaml", "ts", "txt"];
/// File không quét: test của chính các luật này dùng ID giả.
const NOT_SCANNED: [&str; 2] = ["xtask/src/decisions.rs", "xtask/src/decisions/rules.rs"];
/// Độ dài tối đa của phần tên trong ID mới.
const MAX_SLUG: usize = 40;

/// File của mục mới trong [`DIR`], tính từ gốc repo, theo tên (tức theo ngày rồi tên).
pub(super) fn entry_files(root: &Path) -> Result<Vec<String>, String> {
    let dir = root.join(DIR);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for item in fs::read_dir(&dir).map_err(|error| format!("{DIR}: {error}"))? {
        let item = item.map_err(|error| format!("{DIR}: {error}"))?;
        let name = item.file_name().to_string_lossy().into_owned();
        let path = format!("{DIR}/{name}");
        if name.ends_with(".md") && path != INDEX_FILE && name.starts_with("D-") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// ID mục cũ: `D-` + đúng ba chữ số.
fn is_legacy_id(id: &str) -> bool {
    id.strip_prefix("D-")
        .is_some_and(|digits| digits.len() == 3 && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// ID mục mới: `D-<YYYYMMDD>-<tên>`; trả phần ngày khi đúng dạng.
fn new_id_date(id: &str) -> Option<&str> {
    let rest = id.strip_prefix("D-")?;
    let (date, slug) = rest.split_once('-')?;
    let date_ok = date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit());
    let slug_ok = (3..=MAX_SLUG).contains(&slug.len())
        && slug.split('-').all(|part| {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        });
    (date_ok && slug_ok).then_some(date)
}

pub(super) fn check(root: &Path, entries: &[Entry]) -> Result<Vec<Violation>, String> {
    let mut violations = check_ids(entries);
    let mut per_file: BTreeMap<&str, usize> = BTreeMap::new();
    for entry in entries {
        *per_file.entry(entry.file.as_str()).or_default() += 1;
    }
    for file in entry_files(root)? {
        let count = per_file.get(file.as_str()).copied().unwrap_or_default();
        if count != 1 {
            violations.push(Violation {
                rule: "decisions-id",
                location: file.clone(),
                message: format!(
                    "holds {count} decisions — a file in {DIR} holds exactly one, headed `## \
                     <ID> · <title> — <YYYY-MM-DD> — <chủ dự án quyết | agent quyết>`"
                ),
            });
        }
    }
    let known: BTreeMap<&str, ()> = entries.iter().map(|entry| (entry.id.as_str(), ())).collect();
    violations.extend(dangling_references(root, &known)?);
    Ok(violations)
}

/// Dạng ID, ID trùng, chủ đề lạ.
fn check_ids(entries: &[Entry]) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut push = |location: String, message: String| {
        violations.push(Violation { rule: "decisions-id", location, message });
    };
    let mut seen: BTreeMap<&str, &Entry> = BTreeMap::new();
    for entry in entries {
        let location = format!("{}:{}", entry.file, entry.line);
        if entry.file == LOG_FILE {
            if !is_legacy_id(&entry.id) {
                push(
                    location.clone(),
                    format!(
                        "`{}` — {LOG_FILE} only holds the old numbered decisions; write a new \
                         decision as its own file {DIR}/D-<YYYYMMDD>-<short-name>.md (skill \
                         `record-decision`)",
                        entry.id
                    ),
                );
            }
        } else {
            let expected_file = format!("{DIR}/{}.md", entry.id);
            match new_id_date(&entry.id) {
                None => push(
                    location.clone(),
                    format!(
                        "`{}` — a new decision's ID is `D-<YYYYMMDD>-<short-name>`, the name in \
                         lowercase a-z, 0-9 and `-`, at most {MAX_SLUG} characters, e.g. \
                         `D-20261118-decision-ids`",
                        entry.id
                    ),
                ),
                Some(date) if date != entry.date.replace('-', "") => push(
                    location.clone(),
                    format!(
                        "`{}` — the date in the ID must be the decision's date ({})",
                        entry.id, entry.date
                    ),
                ),
                Some(_) if entry.file != expected_file => push(
                    location.clone(),
                    format!("`{}` must be in a file named {expected_file}", entry.id),
                ),
                Some(_) => {}
            }
        }
        if let Some(first) = seen.insert(entry.id.as_str(), entry) {
            push(
                location.clone(),
                format!(
                    "`{}` is used twice (also at {}:{}) — give the newer decision another ID and \
                     update its references",
                    entry.id, first.file, first.line
                ),
            );
        }
        for clause in &entry.unresolved_replace {
            push(
                location.clone(),
                format!(
                    "the `Thay…` paragraph (`{clause}`) names no decision ID, so the index cannot \
                     mark the replaced decision — write the ID(s) the way `D-001` or \
                     `D-20261118-decision-ids` are written"
                ),
            );
        }
        if let Some(topic) = &entry.topic
            && !TOPICS.iter().any(|(name, _)| name == topic)
        {
            let names: Vec<&str> = TOPICS.iter().map(|(name, _)| *name).collect();
            push(
                location.clone(),
                format!("unknown `Chủ đề: {topic}` — use one of: {}", names.join("; ")),
            );
        }
    }
    violations
}

/// Luật `decisions-ref`: tham chiếu tới ID không có.
fn dangling_references(root: &Path, known: &BTreeMap<&str, ()>) -> Result<Vec<Violation>, String> {
    let output = Command::new("git")
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .current_dir(root)
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if !output.status.success() {
        return Err(format!("git ls-files: {}", String::from_utf8_lossy(&output.stderr)));
    }
    let mut violations = Vec::new();
    for file in String::from_utf8_lossy(&output.stdout).split('\0') {
        let scanned = Path::new(file)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| TEXT_EXTENSIONS.contains(&extension));
        if !scanned || NOT_SCANNED.contains(&file) {
            continue;
        }
        // File đã xoá khỏi đĩa nhưng chưa khỏi index, hay không phải UTF-8: bỏ qua.
        let Ok(text) = fs::read_to_string(root.join(file)) else { continue };
        for (index, line) in text.lines().enumerate() {
            for id in ids_in(line) {
                if !known.contains_key(id.as_str()) {
                    violations.push(Violation {
                        rule: "decisions-ref",
                        location: format!("{file}:{}", index + 1),
                        message: format!(
                            "`{id}` is not a decision — no heading `## {id} · …` in {LOG_FILE} or \
                             {DIR}/{id}.md; fix the reference (see {INDEX_FILE})"
                        ),
                    });
                }
            }
        }
    }
    Ok(violations)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, date: &str, file: &str) -> Entry {
        Entry {
            id: id.to_owned(),
            title: "x".to_owned(),
            date: date.to_owned(),
            who: "agent quyết".to_owned(),
            topic: None,
            replaces: Vec::new(),
            unresolved_replace: Vec::new(),
            file: file.to_owned(),
            line: 1,
        }
    }

    #[test]
    fn dang_id_cu_va_moi() {
        assert!(is_legacy_id("D-001"));
        assert!(!is_legacy_id("D-0001"));
        assert_eq!(new_id_date("D-20261118-decision-ids"), Some("20261118"));
        assert_eq!(new_id_date("D-20261118-Decision"), None, "chữ hoa");
        assert_eq!(new_id_date("D-20261118-a--b"), None, "hai gạch liền");
        assert_eq!(new_id_date("D-2026118-abc"), None, "ngày thiếu số");
    }

    #[test]
    fn bat_trung_sai_file_sai_ngay() {
        let new_file = format!("{DIR}/D-20261118-abc.md");
        let entries = vec![
            entry("D-001", "2026-11-11", LOG_FILE),
            entry("D-001", "2026-11-12", LOG_FILE),
            entry("D-20261118-abc", "2026-11-18", &new_file),
            entry("D-20261117-abc", "2026-11-18", &new_file),
            entry("D-20261118-xyz", "2026-11-18", &new_file),
            entry("D-20261118-new", "2026-11-18", LOG_FILE),
        ];
        let messages: Vec<String> =
            check_ids(&entries).into_iter().map(|violation| violation.message).collect();
        assert_eq!(messages.len(), 4, "{messages:#?}");
        assert!(messages[0].contains("used twice"));
        assert!(messages[1].contains("date in the ID"));
        assert!(messages[2].contains("D-20261118-xyz.md"));
        assert!(messages[3].contains("old numbered decisions"));
    }
}
