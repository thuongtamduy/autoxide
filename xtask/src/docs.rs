//! Luật cho tài liệu của dự án mã nguồn mở:
//!
//! - `docs-required` — các file người lạ cần (README, license, `SECURITY`, `CONTRIBUTING`,
//!   mẫu issue và PR…) không được mất.
//! - `docs-translation` — mỗi tài liệu cho người ngoài có hai bản, `X.md` (tiếng Anh) và
//!   `X.vi.md` (tiếng Việt), cùng nội dung (`CLAUDE.md`, "Luật ngôn ngữ của tài liệu"). Không
//!   so được câu chữ giữa hai ngôn ngữ, nên so bộ khung: hai bản phải có cùng dãy cấp tiêu đề
//!   (`#`, `##`, `###`…, ngoài khối code). Thêm hay bỏ một mục ở một bên mà quên bên kia thì
//!   dãy lệch, gate đỏ — cách làm giống kiểm bản dịch i18n (`i18n.rs`).

use std::fs;
use std::path::Path;

use crate::boundaries::Violation;

/// File phải có, tính từ gốc repo. Cặp `X.md` / `X.vi.md` trong danh sách còn được luật
/// `docs-translation` so khung.
const REQUIRED: [&str; 22] = [
    "README.md",
    "README.vi.md",
    "SECURITY.md",
    "SECURITY.vi.md",
    "CONTRIBUTING.md",
    "CONTRIBUTING.vi.md",
    "CODE_OF_CONDUCT.md",
    "CODE_OF_CONDUCT.vi.md",
    "SUPPORT.md",
    "SUPPORT.vi.md",
    "GOVERNANCE.md",
    "GOVERNANCE.vi.md",
    "DCO",
    ".github/CODEOWNERS",
    "CHANGELOG.md",
    "LICENSE-MIT",
    "LICENSE-APACHE",
    "docs/DEPLOY.md",
    "docs/DEPLOY.vi.md",
    ".github/pull_request_template.md",
    ".github/ISSUE_TEMPLATE/bug_report.yml",
    ".github/ISSUE_TEMPLATE/feature_request.yml",
];

/// Thư mục không quét tìm bản dịch: kết quả build và thư mục ẩn (`.git`…).
const SKIPPED_DIRS: [&str; 2] = ["target", "node_modules"];

pub fn check_docs(root: &Path) -> Result<Vec<Violation>, String> {
    let mut violations = Vec::new();
    for file in REQUIRED {
        if !root.join(file).is_file() {
            violations.push(Violation {
                rule: "docs-required",
                location: file.to_owned(),
                message: "missing: this file is part of the open-source project (README, \
                          license, security policy, contributing guide, code of conduct, issue \
                          and pull request templates); restore it"
                    .to_owned(),
            });
        }
    }

    let mut translations = Vec::new();
    collect_translations(root, "", &mut translations)?;
    translations.sort();
    for vietnamese in translations {
        let english = format!("{}.md", vietnamese.trim_end_matches(".vi.md"));
        let english_path = root.join(&english);
        if !english_path.is_file() {
            violations.push(Violation {
                rule: "docs-translation",
                location: vietnamese.clone(),
                message: format!(
                    "has no English version `{english}`: write it, with the same sections"
                ),
            });
            continue;
        }
        let english_text = read(&english_path)?;
        let vietnamese_text = read(&root.join(&vietnamese))?;
        if let Some(violation) = compare(&english, &english_text, &vietnamese, &vietnamese_text) {
            violations.push(violation);
        }
    }
    Ok(violations)
}

/// Mọi file `*.vi.md` dưới `dir`, đường dẫn tương đối dùng `/`.
fn collect_translations(root: &Path, prefix: &str, files: &mut Vec<String>) -> Result<(), String> {
    let dir = root.join(prefix);
    let entries =
        fs::read_dir(&dir).map_err(|error| format!("cannot list {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot list {}: {error}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        let path = entry.path();
        if path.is_dir() {
            if !name.starts_with('.') && !SKIPPED_DIRS.contains(&name.as_str()) {
                collect_translations(root, &relative, files)?;
            }
        } else if name.ends_with(".vi.md") {
            files.push(relative);
        }
    }
    Ok(())
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

/// Một tiêu đề Markdown: cấp (số dấu `#`), chữ, số dòng.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: usize,
    pub text: String,
    pub line: usize,
}

/// Các tiêu đề kiểu `# …` của `text`, bỏ qua dòng trong khối code (```` ``` ```` hay `~~~`).
pub fn headings(text: &str) -> Vec<Heading> {
    let mut found = Vec::new();
    // Ký tự mở khối code đang mở (`` ` `` hay `~`); khối chỉ đóng bằng cùng ký tự.
    let mut fence: Option<char> = None;
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start_matches(' ');
        let indent = line.len() - trimmed.len();
        if indent <= 3
            && let Some(marker) = ['`', '~']
                .into_iter()
                .find(|marker| trimmed.chars().take_while(|c| c == marker).count() >= 3)
        {
            match fence {
                None => fence = Some(marker),
                Some(open) if open == marker => fence = None,
                Some(_) => {}
            }
            continue;
        }
        if fence.is_some() || indent > 3 {
            continue;
        }
        let level = trimmed.chars().take_while(|c| *c == '#').count();
        let rest = &trimmed[level..];
        if (1..=6).contains(&level) && (rest.is_empty() || rest.starts_with([' ', '\t'])) {
            found.push(Heading { level, text: rest.trim().to_owned(), line: index + 1 });
        }
    }
    found
}

/// So khung hai bản; `None` khi khớp, không thì báo chỗ lệch đầu tiên.
pub fn compare(
    english: &str,
    english_text: &str,
    vietnamese: &str,
    vietnamese_text: &str,
) -> Option<Violation> {
    let en = headings(english_text);
    let vi = headings(vietnamese_text);
    let describe = |heading: Option<&Heading>, file: &str| match heading {
        Some(heading) => {
            format!("{file}:{} `{} {}`", heading.line, "#".repeat(heading.level), heading.text)
        }
        None => format!("{file}: nothing (it ends earlier)"),
    };
    let position = (0..en.len().max(vi.len()))
        .find(|&i| en.get(i).map(|h| h.level) != vi.get(i).map(|h| h.level))?;
    Some(Violation {
        rule: "docs-translation",
        location: english.to_owned(),
        message: format!(
            "the English and Vietnamese versions have different sections ({} and {} headings); \
             heading {} is {} but {}. Change both versions in the same commit",
            en.len(),
            vi.len(),
            position + 1,
            describe(en.get(position), english),
            describe(vi.get(position), vietnamese),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tim_tieu_de_bo_qua_khoi_code() {
        let text = "# Title\n\nText\n\n```bash\n# a shell comment\n```\n\n## Part\n\
                    ~~~\n## not a heading\n~~~\n#not-a-heading\n    # indented code\n### Sub ###\n";
        let found: Vec<(usize, String, usize)> =
            headings(text).into_iter().map(|h| (h.level, h.text, h.line)).collect();
        assert_eq!(
            found,
            vec![
                (1, "Title".to_owned(), 1),
                (2, "Part".to_owned(), 9),
                (3, "Sub ###".to_owned(), 15),
            ]
        );
    }

    #[test]
    fn khoi_code_chi_dong_bang_cung_ky_tu() {
        let text = "```\n~~~\n# inside\n```\n# outside\n";
        assert_eq!(headings(text).len(), 1);
    }

    #[test]
    fn hai_ban_cung_khung_thi_khop() {
        let en = "# Guide\n## Install\n## Use\n### Tips\n";
        let vi = "# Hướng dẫn\n## Cài đặt\n## Dùng\n### Mẹo\n";
        assert_eq!(compare("G.md", en, "G.vi.md", vi), None);
    }

    #[test]
    fn bao_cho_lech_dau_tien() {
        let en = "# Guide\n## Install\n## Use\n";
        let vi = "# Hướng dẫn\n## Cài đặt\n### Mẹo\n## Dùng\n";
        let violation = compare("G.md", en, "G.vi.md", vi).unwrap();
        assert_eq!(violation.rule, "docs-translation");
        assert_eq!(violation.location, "G.md");
        assert!(violation.message.contains("(3 and 4 headings)"), "{}", violation.message);
        assert!(
            violation.message.contains("heading 3 is G.md:3 `## Use`"),
            "{}",
            violation.message
        );
        assert!(violation.message.contains("G.vi.md:3 `### Mẹo`"), "{}", violation.message);
    }

    #[test]
    fn mot_ban_ngan_hon_cung_bi_bao() {
        let violation = compare("G.md", "# A\n## B\n", "G.vi.md", "# A\n").unwrap();
        assert!(violation.message.contains("G.vi.md: nothing"), "{}", violation.message);
    }

    #[test]
    fn repo_hien_tai_khong_vi_pham() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let violations = check_docs(&root).unwrap();
        assert!(violations.is_empty(), "{violations:#?}");
    }
}
