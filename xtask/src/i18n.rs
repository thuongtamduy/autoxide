//! Luật `i18n` và `error-code-prefix`: mọi `code` lỗi trong code của kernel và module
//! đều có bản dịch tiếng Việt trong `locales/vi.json` của crate đó; bản dịch không
//! thừa; `code` của module mang tiền tố `<module>.` (D-039, D-057).
//!
//! Tìm `code` bằng cách đọc mã nguồn: chuỗi có dạng `code` đứng ngay sau một chỗ tạo
//! lỗi — `Error::not_found(` (và các hàm cùng họ), `ErrorKind::X,` (trong
//! `Error::new(ErrorKind::X, "code"` hay bộ `(ErrorKind::X, "code")`), `NNN =>` (mã của
//! trạng thái HTTP), `…_CODE: &str =`. Bỏ qua chú thích và phần `#[cfg(test)]`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use crate::boundaries::Violation;
use crate::workspace::{Package, Role, Workspace};

/// Các hàm tạo lỗi nhận `code` làm tham số đầu.
const CONSTRUCTORS: [&str; 7] = [
    "bad_request",
    "unauthenticated",
    "forbidden",
    "not_found",
    "conflict",
    "too_many_requests",
    "unavailable",
];

pub fn check_translations(workspace: &Workspace) -> Result<Vec<Violation>, String> {
    let mut violations = Vec::new();
    for package in &workspace.packages {
        if !matches!(package.role, Role::Kernel | Role::Module) {
            continue;
        }
        let crate_dir = workspace.root.join(&package.dir);
        let mut codes: BTreeMap<String, String> = BTreeMap::new();
        for file in crate::boundaries::rust_files(&crate_dir.join("src"))? {
            let path = crate_dir.join(&file);
            let text = fs::read_to_string(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            for (line, code) in error_codes(&text) {
                codes.entry(code).or_insert_with(|| format!("{}/{file}:{line}", package.dir));
            }
        }
        violations.extend(check_package(package, &codes, &read_translations(workspace, package)?));
    }
    Ok(violations)
}

/// Bản dịch tiếng Việt của crate; `None` nếu chưa có file.
fn read_translations(
    workspace: &Workspace,
    package: &Package,
) -> Result<Option<BTreeMap<String, String>>, String> {
    let path = workspace.root.join(&package.dir).join("locales/vi.json");
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| format!("{} is not a JSON object of strings: {error}", path.display()))
}

/// Luật cho một crate: `codes` tìm thấy (kèm chỗ), `translations` đọc từ file.
pub fn check_package(
    package: &Package,
    codes: &BTreeMap<String, String>,
    translations: &Option<BTreeMap<String, String>>,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    let file = format!("{}/locales/vi.json", package.dir);
    let module = package.dir.strip_prefix("modules/");
    for (code, location) in codes {
        if let Some(module) = module
            && !code.starts_with(&format!("{module}."))
        {
            violations.push(Violation {
                rule: "error-code-prefix",
                location: location.clone(),
                message: format!(
                    "error code `{code}` of module `{module}` must start with `{module}.`"
                ),
            });
        }
        let translated = translations.as_ref().is_some_and(|map| map.contains_key(code));
        if !translated {
            violations.push(Violation {
                rule: "i18n",
                location: location.clone(),
                message: format!(
                    "error code `{code}` has no Vietnamese translation; add it to {file}"
                ),
            });
        }
    }
    let used: BTreeSet<&String> = codes.keys().collect();
    for code in translations.iter().flat_map(BTreeMap::keys) {
        if !used.contains(code) {
            violations.push(Violation {
                rule: "i18n",
                location: file.clone(),
                message: format!("translation of `{code}` is not used by any error; remove it"),
            });
        }
    }
    violations
}

/// Các `code` lỗi trong `text` kèm số dòng, bỏ chú thích và phần chỉ dùng cho test.
pub fn error_codes(text: &str) -> Vec<(usize, String)> {
    let source = without_tests_and_comments(text);
    let mut codes = Vec::new();
    let bytes = source.as_bytes();
    let mut index = 0;
    while let Some(offset) = source[index..].find('"') {
        let start = index + offset;
        let Some(length) = source[start + 1..].find('"') else {
            break;
        };
        let end = start + 1 + length;
        let literal = &source[start + 1..end];
        if is_code(literal) {
            let line = bytes[..start].iter().filter(|byte| **byte == b'\n').count() + 1;
            if is_error_site(&source[..start]) {
                codes.push((line, literal.to_owned()));
            } else if is_mail_template(&source[..start]) {
                // Mẫu mail cần bản dịch của tiêu đề và nội dung.
                codes.push((line, format!("{literal}.subject")));
                codes.push((line, format!("{literal}.text")));
            }
        }
        index = end + 1;
    }
    codes
}

/// `snake_case`, có thể có tiền tố module: `auth.email_taken`, `validation_failed`.
fn is_code(text: &str) -> bool {
    !text.is_empty()
        && text.split('.').all(|part| {
            part.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

/// Văn bản `before` kết thúc bằng `MailTemplate::new(`.
fn is_mail_template(before: &str) -> bool {
    let tail: String =
        before.chars().rev().take(60).collect::<Vec<_>>().into_iter().rev().collect();
    tail.split_whitespace().collect::<Vec<_>>().join(" ").trim_end().ends_with("MailTemplate::new(")
}

/// Văn bản `before` (ngay trước một chuỗi) kết thúc bằng một chỗ tạo lỗi.
fn is_error_site(before: &str) -> bool {
    let tail: String =
        before.chars().rev().take(120).collect::<Vec<_>>().into_iter().rev().collect();
    let compact: String = tail.split_whitespace().collect::<Vec<_>>().join(" ");
    let compact = compact.trim_end();
    if CONSTRUCTORS.iter().any(|name| compact.ends_with(&format!("Error::{name}("))) {
        return true;
    }
    if compact.ends_with("_CODE: &str =") {
        return true;
    }
    // `ErrorKind::Xxx,` ngay trước chuỗi.
    if let Some(rest) = compact.strip_suffix(',')
        && let Some(position) = rest.rfind("ErrorKind::")
        && rest[position + "ErrorKind::".len()..].chars().all(|c| c.is_ascii_alphanumeric())
    {
        return true;
    }
    // `404 =>` trong bảng mã của trạng thái HTTP.
    if let Some(rest) = compact.strip_suffix("=>") {
        let status = rest.trim_end();
        let digits: String = status.chars().rev().take_while(char::is_ascii_digit).collect();
        return digits.len() == 3;
    }
    false
}

/// Bỏ dòng chú thích (`//`, `///`, `//!`) và mục ngay sau `#[cfg(test)]` (một khối
/// `{ … }` hoặc một câu kết thúc bằng `;`). Thay bằng dòng trống để giữ số dòng.
fn without_tests_and_comments(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut output = Vec::with_capacity(lines.len());
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim_start();
        if trimmed.starts_with("#[cfg(test)]") {
            output.push(String::new());
            index += 1;
            // Bỏ mục kế tiếp: đếm ngoặc tới khi đóng hết, hoặc gặp `;` khi chưa mở.
            let mut depth: i64 = 0;
            let mut opened = false;
            while index < lines.len() {
                let line = lines[index];
                output.push(String::new());
                index += 1;
                for c in line.chars() {
                    match c {
                        '{' => {
                            depth += 1;
                            opened = true;
                        }
                        '}' => depth -= 1,
                        _ => {}
                    }
                }
                if (opened && depth <= 0) || (!opened && line.trim_end().ends_with(';')) {
                    break;
                }
            }
            continue;
        }
        if trimmed.starts_with("//") {
            output.push(String::new());
        } else {
            output.push(lines[index].to_owned());
        }
        index += 1;
    }
    output.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::DepKind;

    fn codes(text: &str) -> Vec<String> {
        error_codes(text).into_iter().map(|(_, code)| code).collect()
    }

    #[test]
    fn tim_code_o_moi_cho_tao_loi() {
        let text = r#"
fn a() -> Error {
    Error::not_found("notes.note_not_found", "Note {id} does not exist.")
}
fn b() -> Error {
    Error::conflict(
        "notes.title_taken",
        "Taken.",
    )
}
fn c() -> Error {
    Error::new(ErrorKind::Validation, "validation_failed", "Bad.")
}
fn d(rejection: R) -> (ErrorKind, &'static str) {
    (ErrorKind::BadRequest, "invalid_json")
}
fn e(status: u16) -> &'static str {
    match status {
        404 => "not_found",
        _ => "http_error",
    }
}
const INTERNAL_CODE: &str = "internal_error";
fn f() { let message = "not a code site"; tracing::info!("signed_in"); }
const WELCOME: MailTemplate = MailTemplate::new(
    "notes.mail.welcome",
    "Welcome",
    "Hello.",
);
"#;
        assert_eq!(
            codes(text),
            [
                "notes.note_not_found",
                "notes.title_taken",
                "validation_failed",
                "invalid_json",
                "not_found",
                "internal_error",
                "notes.mail.welcome.subject",
                "notes.mail.welcome.text"
            ]
        );
    }

    #[test]
    fn bo_qua_chu_thich_va_test() {
        let text = r#"
/// Error::not_found("doc.example", "x")
fn real() -> Error { Error::forbidden("notes.denied", "No.") }

#[cfg(test)]
mod tests {
    fn t() { Error::conflict("notes.only_in_test", "x"); }
}

#[cfg(test)]
pub(crate) use something::Helper;
fn after() -> Error { Error::not_found("notes.after", "x") }
"#;
        assert_eq!(codes(text), ["notes.denied", "notes.after"]);
    }

    #[test]
    fn bao_thieu_ban_dich_thua_ban_dich_va_sai_tien_to() {
        let package = Package {
            name: "autoxide-notes".to_owned(),
            license: None,
            dir: "modules/notes".to_owned(),
            role: Role::Module,
            deps: vec![crate::workspace::Dep { name: "x".to_owned(), kind: DepKind::Normal }],
        };
        let codes = BTreeMap::from([
            ("notes.a".to_owned(), "src/a.rs:1".to_owned()),
            ("not_prefixed".to_owned(), "src/a.rs:2".to_owned()),
        ]);
        let translations = Some(BTreeMap::from([
            ("not_prefixed".to_owned(), "x".to_owned()),
            ("notes.unused".to_owned(), "x".to_owned()),
        ]));

        let violations = check_package(&package, &codes, &translations);

        let messages: Vec<String> = violations.iter().map(ToString::to_string).collect();
        assert_eq!(violations.len(), 3, "{messages:#?}");
        let has = |text: &str| messages.iter().any(|message| message.contains(text));
        assert!(has("`notes.a` has no Vietnamese translation"), "{messages:#?}");
        assert!(has("[error-code-prefix]"), "{messages:#?}");
        assert!(has("`notes.unused` is not used"), "{messages:#?}");
    }
}
