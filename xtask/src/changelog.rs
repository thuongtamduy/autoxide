//! Ghi thay đổi theo file mảnh (D-20261119-changelog-fragments): mỗi PR thêm một file nhỏ vào
//! `changelog.d/` thay vì sửa `CHANGELOG.md`, nên hai PR song song không bao giờ đụng cùng dòng.
//! Lúc phát hành, `cargo xtask changelog` gộp mọi mảnh vào mục `## [Unreleased]` rồi xoá chúng.
//!
//! - Tên file: `<tên>.<mục>.md` — `<tên>` gồm chữ thường, số, `-` (thường là tên nhánh hay việc),
//!   `<mục>` là một trong [`SECTIONS`] (`added`, `changed`…).
//! - Nội dung: một hay nhiều gạch đầu dòng Markdown tiếng Anh (`- …`, dòng tiếp thụt hai dấu cách),
//!   đúng như sẽ nằm trong `CHANGELOG.md`.
//!
//! `cargo xtask check` báo tên sai, mục lạ, file rỗng hay không bắt đầu bằng `- `.

use std::fs;
use std::path::{Path, PathBuf};

use crate::boundaries::Violation;

/// Thư mục mảnh, tính từ gốc repo.
pub const DIR: &str = "changelog.d";
/// File thay đổi, tính từ gốc repo.
pub const CHANGELOG_FILE: &str = "CHANGELOG.md";
/// File hướng dẫn nằm trong thư mục mảnh, không phải mảnh.
const README: &str = "README.md";
/// Dòng mở mục chưa phát hành.
const UNRELEASED: &str = "## [Unreleased]";

/// Mục theo Keep a Changelog: (đuôi tên file, tiêu đề trong `CHANGELOG.md`), đúng thứ tự in.
pub const SECTIONS: &[(&str, &str)] = &[
    ("added", "Added"),
    ("changed", "Changed"),
    ("deprecated", "Deprecated"),
    ("removed", "Removed"),
    ("fixed", "Fixed"),
    ("security", "Security"),
];

/// Một mảnh đã đọc.
struct Fragment {
    path: PathBuf,
    heading: &'static str,
    text: String,
}

/// Kiểm mọi mảnh trong `changelog.d/`. Không có thư mục thì không có gì để kiểm.
pub fn check(root: &Path) -> Result<Vec<Violation>, String> {
    let mut violations = Vec::new();
    for path in fragment_paths(root)? {
        let location = relative(root, &path);
        if heading_of(&path).is_none() {
            violations.push(violation(
                location,
                format!(
                    "name it `<name>.<section>.md` (name: lowercase letters, digits, `-`; section: {})",
                    section_names()
                ),
            ));
            continue;
        }
        let text = fs::read_to_string(&path).map_err(|error| format!("{location}: {error}"))?;
        if let Some(problem) = content_problem(&text) {
            violations.push(violation(location, problem));
        }
    }
    Ok(violations)
}

/// Gộp mọi mảnh vào `## [Unreleased]` của `CHANGELOG.md` (mỗi mảnh vào cuối mục của nó, mục chưa
/// có thì thêm theo thứ tự của [`SECTIONS`]), rồi xoá các mảnh. Trả về số mảnh đã gộp.
pub fn fold(root: &Path) -> Result<usize, String> {
    let violations = check(root)?;
    if let Some(first) = violations.first() {
        return Err(format!("fix the fragments first: {first}"));
    }
    let mut fragments = Vec::new();
    for path in fragment_paths(root)? {
        let heading = heading_of(&path).ok_or_else(|| format!("{}: bad name", path.display()))?;
        let text =
            fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        fragments.push(Fragment { path, heading, text: text.trim().to_owned() });
    }
    if fragments.is_empty() {
        return Ok(0);
    }
    let changelog_path = root.join(CHANGELOG_FILE);
    let changelog = fs::read_to_string(&changelog_path)
        .map_err(|error| format!("{CHANGELOG_FILE}: {error}"))?;
    let updated = merge_into(&changelog, &fragments)?;
    fs::write(&changelog_path, updated).map_err(|error| format!("{CHANGELOG_FILE}: {error}"))?;
    for fragment in &fragments {
        fs::remove_file(&fragment.path)
            .map_err(|error| format!("{}: {error}", fragment.path.display()))?;
    }
    Ok(fragments.len())
}

/// Chèn mảnh vào văn bản của `CHANGELOG.md`.
fn merge_into(changelog: &str, fragments: &[Fragment]) -> Result<String, String> {
    let start = changelog
        .find(UNRELEASED)
        .ok_or_else(|| format!("{CHANGELOG_FILE}: no `{UNRELEASED}` heading"))?;
    let body_start = start + UNRELEASED.len();
    // Mục chưa phát hành kết thúc ở tiêu đề phiên bản kế tiếp, hay ở khối liên kết cuối file.
    let rest = &changelog[body_start..];
    let body_end = body_start
        + [rest.find("\n## ["), rest.find("\n[Unreleased]: ")]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(rest.len());
    let body = &changelog[body_start..body_end];

    // Tách phần thân thành phần mở đầu và các mục `### …` (giữ nguyên thứ tự và nội dung có sẵn).
    let mut preamble = String::new();
    let mut sections: Vec<(String, String)> = Vec::new();
    for line in body.split_inclusive('\n') {
        if let Some(title) = line.strip_prefix("### ") {
            sections.push((title.trim().to_owned(), String::new()));
        } else if let Some((_, text)) = sections.last_mut() {
            text.push_str(line);
        } else {
            preamble.push_str(line);
        }
    }

    for &(_, heading) in SECTIONS {
        let additions: Vec<&str> =
            fragments.iter().filter(|f| f.heading == heading).map(|f| f.text.as_str()).collect();
        if additions.is_empty() {
            continue;
        }
        let index = match sections.iter().position(|(title, _)| title == heading) {
            Some(index) => index,
            None => {
                // Chèn trước mục đầu tiên đứng sau nó trong SECTIONS; không có thì cuối cùng.
                let order = |title: &str| SECTIONS.iter().position(|&(_, h)| h == title);
                let own = order(heading);
                let at = sections
                    .iter()
                    .position(|(title, _)| order(title).is_some_and(|o| Some(o) > own))
                    .unwrap_or(sections.len());
                sections.insert(at, (heading.to_owned(), String::new()));
                at
            }
        };
        let text = &mut sections[index].1;
        let kept = text.trim_end().to_owned();
        let mut rebuilt = if kept.is_empty() { "\n".to_owned() } else { format!("{kept}\n") };
        for addition in additions {
            rebuilt.push_str(addition);
            rebuilt.push('\n');
        }
        *text = rebuilt;
    }

    let mut out = String::with_capacity(changelog.len() + 256);
    out.push_str(&changelog[..body_start]);
    let preamble = preamble.trim_end();
    out.push_str(if preamble.is_empty() { "\n" } else { preamble });
    if !preamble.is_empty() {
        out.push('\n');
    }
    for (title, text) in &sections {
        out.push_str("\n### ");
        out.push_str(title);
        out.push('\n');
        let text = text.trim_end();
        if !text.is_empty() {
            if !text.starts_with('\n') {
                out.push('\n');
            }
            out.push_str(text);
            out.push('\n');
        }
    }
    out.push_str(&changelog[body_end..]);
    Ok(out)
}

/// Mọi file trong `changelog.d/` trừ `README.md`, theo thứ tự tên (để kết quả gộp ổn định).
fn fragment_paths(root: &Path) -> Result<Vec<PathBuf>, String> {
    let dir = root.join(DIR);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{DIR}: {error}")),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("{DIR}: {error}"))?;
        let path = entry.path();
        if path.is_file() && entry.file_name() != README {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

/// Tiêu đề mục của một mảnh theo tên file; `None` khi tên sai dạng.
fn heading_of(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_suffix(".md")?;
    let (label, section) = stem.rsplit_once('.')?;
    let label_ok = !label.is_empty()
        && label.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !label_ok {
        return None;
    }
    SECTIONS.iter().find(|&&(suffix, _)| suffix == section).map(|&(_, heading)| heading)
}

/// Lỗi nội dung của một mảnh, nếu có.
fn content_problem(text: &str) -> Option<String> {
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let Some(first) = lines.next() else {
        return Some("empty fragment: write one or more `- …` bullet points".to_owned());
    };
    if !first.starts_with("- ") {
        return Some("start the fragment with a `- ` bullet point".to_owned());
    }
    if let Some(bad) = lines.find(|line| !(line.starts_with("- ") || line.starts_with("  "))) {
        return Some(format!(
            "every line is a `- ` bullet or a continuation indented by two spaces: `{bad}`"
        ));
    }
    if text.lines().any(|line| line.starts_with('#')) {
        return Some("no headings in a fragment: the file name chooses the section".to_owned());
    }
    None
}

fn section_names() -> String {
    SECTIONS.iter().map(|&(suffix, _)| suffix).collect::<Vec<_>>().join(", ")
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).display().to_string()
}

fn violation(location: String, message: String) -> Violation {
    Violation { rule: "changelog-fragment", location, message }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fragment(heading: &'static str, text: &str) -> Fragment {
        Fragment { path: PathBuf::new(), heading, text: text.to_owned() }
    }

    const BASE: &str = "# Changelog\n\nIntro.\n\n## [Unreleased]\n\n### Added\n\n- Old added.\n\n### Fixed\n\n- Old fix.\n\n## [0.1.0] - 2026-01-01\n\n### Added\n\n- Released.\n\n[Unreleased]: x\n";

    #[test]
    fn manh_vao_cuoi_muc_co_san_va_muc_moi_dung_thu_tu() {
        let out = merge_into(
            BASE,
            &[
                fragment("Added", "- New added.\n  continued."),
                fragment("Changed", "- New change."),
                fragment("Security", "- New security."),
            ],
        )
        .unwrap();
        assert_eq!(
            out,
            "# Changelog\n\nIntro.\n\n## [Unreleased]\n\n### Added\n\n- Old added.\n- New added.\n  continued.\n\n### Changed\n\n- New change.\n\n### Fixed\n\n- Old fix.\n\n### Security\n\n- New security.\n\n## [0.1.0] - 2026-01-01\n\n### Added\n\n- Released.\n\n[Unreleased]: x\n"
        );
    }

    #[test]
    fn muc_chua_phat_hanh_rong_van_gop_duoc() {
        let base = "# C\n\n## [Unreleased]\n\n## [0.1.0] - 2026-01-01\n\n- x\n";
        let out = merge_into(base, &[fragment("Fixed", "- A fix.")]).unwrap();
        assert_eq!(
            out,
            "# C\n\n## [Unreleased]\n\n### Fixed\n\n- A fix.\n\n## [0.1.0] - 2026-01-01\n\n- x\n"
        );
    }

    #[test]
    fn ten_va_noi_dung_sai_bi_bao() {
        assert_eq!(heading_of(Path::new("x/step-45-orders.added.md")), Some("Added"));
        assert_eq!(heading_of(Path::new("x/Step45.added.md")), None);
        assert_eq!(heading_of(Path::new("x/step45.new.md")), None);
        assert_eq!(heading_of(Path::new("x/added.md")), None);
        assert!(content_problem("- ok\n  more\n- two\n").is_none());
        assert!(content_problem("\n\n").is_some());
        assert!(content_problem("text\n").is_some());
        assert!(content_problem("- ok\nloose line\n").is_some());
    }

    #[test]
    fn gop_tren_dia_roi_xoa_manh() {
        let root = std::env::temp_dir().join(format!("xtask-changelog-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(DIR)).unwrap();
        fs::write(root.join(CHANGELOG_FILE), BASE).unwrap();
        fs::write(root.join(DIR).join(README), "guide").unwrap();
        fs::write(root.join(DIR).join("b.fixed.md"), "- Second fix.\n").unwrap();
        fs::write(root.join(DIR).join("a.fixed.md"), "- First fix.\n").unwrap();
        assert!(check(&root).unwrap().is_empty());
        assert_eq!(fold(&root).unwrap(), 2);
        let out = fs::read_to_string(root.join(CHANGELOG_FILE)).unwrap();
        assert!(out.contains("- Old fix.\n- First fix.\n- Second fix.\n\n## [0.1.0]"), "{out}");
        assert!(root.join(DIR).join(README).exists());
        assert_eq!(fold(&root).unwrap(), 0);
        fs::write(root.join(DIR).join("Bad.added.md"), "- x\n").unwrap();
        assert_eq!(check(&root).unwrap().len(), 1);
        assert!(fold(&root).is_err());
        fs::remove_dir_all(&root).unwrap();
    }
}
