//! Luật `i18n` và `error-code-prefix`: mọi `code` lỗi trong code của kernel và module
//! đều có bản dịch tiếng Việt trong `locales/vi.json` của crate đó; bản dịch không
//! thừa; `code` của module mang tiền tố `<module>.` (D-039, D-057).
//!
//! Luật `i18n-field` (D-262): câu của lỗi từng ô (`FieldError::new(ô, mã, "câu mẫu")`) là chuỗi
//! viết sẵn — chỗ thay đổi là `{tên}` điền bằng `.with_param` — và có bản dịch
//! `"field:<câu mẫu>"` trong `locales/vi.json` của crate; câu ghép sẵn (`format!`, biến) thì không
//! dịch được nên bị chặn.
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
            let fields = field_messages(&text).into_iter().map(|site| (FIELD_PREFIX, site));
            let texts = descriptions(&text).into_iter().map(|site| (TEXT_PREFIX, site));
            for (prefix, (line, site)) in fields.chain(texts) {
                let location = format!("{}/{file}:{line}", package.dir);
                match site {
                    FieldMessage::Template(template) => {
                        codes.entry(format!("{prefix}{template}")).or_insert(location);
                    }
                    FieldMessage::Built(expression) => violations.push(Violation {
                        rule: "i18n-field",
                        location,
                        message: format!(
                            "the message of a FieldError must be a string literal (a template \
                             with `{{name}}` filled by `.with_param`) so it can be translated, \
                             not `{expression}`"
                        ),
                    }),
                }
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
        let shared = code.strip_prefix(FIELD_PREFIX).or_else(|| code.strip_prefix(TEXT_PREFIX));
        if let Some(template) = shared {
            let translated = translations.as_ref().is_some_and(|map| map.contains_key(code));
            if !translated {
                violations.push(Violation {
                    rule: "i18n-field",
                    location: location.clone(),
                    message: format!(
                        "text `{template}` has no Vietnamese translation; add \
                         `\"{code}\": \"…\"` to {file}"
                    ),
                });
            }
            continue;
        }
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

/// Tiền tố khoá bản dịch của câu lỗi từng ô (như `autoxide_kernel::i18n::FIELD_PREFIX`).
const FIELD_PREFIX: &str = "field:";

/// Tiền tố khoá bản dịch của mô tả permission / event (như `autoxide_kernel::i18n::TEXT_PREFIX`).
const TEXT_PREFIX: &str = "text:";

/// Câu của một `FieldError::new(…)`: chuỗi mẫu (đã bỏ escape), hay biểu thức dựng lúc chạy.
#[derive(Debug, PartialEq, Eq)]
pub enum FieldMessage {
    Template(String),
    Built(String),
}

/// Câu của mọi `FieldError::new(ô, mã, câu)` trong `text` kèm số dòng, bỏ chú thích và test.
pub fn field_messages(text: &str) -> Vec<(usize, FieldMessage)> {
    call_arguments(text, "FieldError::new(", 2)
}

/// Mô tả của mọi `Permission::new(tên, mô tả)` và `EventInfo::new(tên, mô tả)` (D-264).
/// Event khai bằng macro `events! { … "tên", "mô tả" … }` của module: lấy cặp chuỗi (tên event,
/// mô tả) trong khối gọi macro; tham số `$mô_tả` của định nghĩa macro thì bỏ qua.
pub fn descriptions(text: &str) -> Vec<(usize, FieldMessage)> {
    let mut found = call_arguments(text, "Permission::new(", 1);
    found.extend(call_arguments(text, "EventInfo::new(", 1));
    // Câu kết quả kiểm SEO của `content` (câu mẫu, D-264).
    found.extend(call_arguments(text, "Finding::new(", 3));
    // Nhãn của sổ điều hướng (bước 51 P1): nhóm, mục, nhóm con trong hàng tab.
    found.extend(call_arguments(text, "NavGroup::new(", 1));
    found.extend(call_arguments(text, "NavItem::new(", 2));
    found.extend(call_arguments(text, ".section(", 0));
    // Tiêu đề việc của hộp việc (bước 51 P2).
    found.extend(call_arguments(text, "WorkItem::new(", 2));
    // Câu "hoạt động gần đây" của event (bước 51 P5).
    found.extend(call_arguments(text, ".activity(", 0));
    // Nhãn và câu định nghĩa của số liệu tóm tắt (bước 51 P4, D-207).
    found.extend(call_arguments(text, "MetricInfo::new(", 1));
    found.extend(call_arguments(text, "MetricInfo::new(", 2));
    found.retain(
        |(_, site)| !matches!(site, FieldMessage::Built(expression) if expression.starts_with('$')),
    );
    let source = without_tests_and_comments(text);
    if let Some(start) = source.find("\nevents! {") {
        let block = &source[start..];
        let mut literals = Vec::new();
        let mut rest = block;
        let mut offset = start;
        while let Some(open) = rest.find('"') {
            let after = &rest[open..];
            let mut end = 1;
            let bytes = after.as_bytes();
            while end < bytes.len() && bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            let literal = &after[..=end.min(bytes.len() - 1)];
            literals.push((
                offset + open,
                literal.to_owned(),
                after[end + 1..].trim_start().to_owned(),
            ));
            offset += open + end + 1;
            rest = &after[end + 1..];
        }
        for pair in literals.windows(2) {
            let (_, name, after) = &pair[0];
            let (position, description, _) = &pair[1];
            let is_name =
                string_literal(name).is_some_and(|name| is_code(&name) && name.contains('.'));
            if is_name
                && after.starts_with(',')
                && let Some(description) = string_literal(description)
            {
                let line = source[..*position].matches('\n').count() + 1;
                found.push((line, FieldMessage::Template(description)));
            }
        }
    }
    found
}

/// Tham số thứ `position` (từ 0) của mọi lời gọi `needle…)` trong `text`.
fn call_arguments(text: &str, needle: &str, position: usize) -> Vec<(usize, FieldMessage)> {
    let source = without_tests_and_comments(text);
    let mut found = Vec::new();
    let mut index = 0;
    while let Some(offset) = source[index..].find(needle) {
        let start = index + offset + needle.len();
        index = start;
        let arguments = split_arguments(&source[start..]);
        let Some(message) = arguments.get(position) else { continue };
        let line = source[..start].matches('\n').count() + 1;
        let message = message.trim();
        let site = match string_literal(message) {
            Some(template) => FieldMessage::Template(template),
            None => FieldMessage::Built(message.split_whitespace().collect::<Vec<_>>().join(" ")),
        };
        found.push((line, site));
    }
    found
}

/// Các tham số của một lời gọi, từ ngay sau `(` tới `)` đóng nó (theo ngoặc và chuỗi).
fn split_arguments(text: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                current.push(c);
                // Chép nguyên chuỗi, kể cả `\"` bên trong.
                while let Some(inner) = chars.next() {
                    current.push(inner);
                    if inner == '\\' {
                        if let Some(next) = chars.next() {
                            current.push(next);
                        }
                    } else if inner == '"' {
                        break;
                    }
                }
            }
            '(' | '[' | '{' => {
                depth += 1;
                current.push(c);
            }
            ')' | ']' | '}' if depth == 0 => {
                arguments.push(std::mem::take(&mut current));
                return arguments;
            }
            ')' | ']' | '}' => {
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => arguments.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    arguments
}

/// Giá trị của một chuỗi Rust thường `"…"` (bỏ escape, nối dòng bằng `\`); `None` nếu `text`
/// không chỉ là một chuỗi như vậy.
fn string_literal(text: &str) -> Option<String> {
    let inner = text.strip_prefix('"')?.strip_suffix('"')?;
    let mut value = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            return None;
        }
        if c != '\\' {
            value.push(c);
            continue;
        }
        match chars.next()? {
            'n' => value.push('\n'),
            't' => value.push('\t'),
            '\\' => value.push('\\'),
            '"' => value.push('"'),
            '\'' => value.push('\''),
            '\n' => {
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
            }
            _ => return None,
        }
    }
    Some(value)
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
            } else if is_notification_template(&source[..start]) {
                // Mẫu thông báo (D-348) cần bản dịch của tiêu đề, nội dung và bản ngắn.
                codes.push((line, format!("{literal}.title")));
                codes.push((line, format!("{literal}.body")));
                codes.push((line, format!("{literal}.short")));
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

/// Văn bản `before` kết thúc bằng `NotificationTemplate::new(`.
fn is_notification_template(before: &str) -> bool {
    let tail: String =
        before.chars().rev().take(60).collect::<Vec<_>>().into_iter().rev().collect();
    tail.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end()
        .ends_with("NotificationTemplate::new(")
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
    fn cau_loi_tung_o_phai_la_chuoi_mau() {
        let text = r#"
fn a() {
    FieldError::new("title", "length", "must be at most {max} characters long")
        .with_param("max", 200);
    FieldError::new(format!("items[{i}]"), code(), "has a \"quote\" and \
                     a continued line");
    FieldError::new("x", "invalid", format!("must be {max}"));
    FieldError::new("y", "invalid", reason);
}
#[cfg(test)]
mod tests { fn t() { FieldError::new("z", "c", format!("ignored")); } }
"#;
        let found: Vec<FieldMessage> = field_messages(text).into_iter().map(|(_, m)| m).collect();
        assert_eq!(
            found,
            [
                FieldMessage::Template("must be at most {max} characters long".to_owned()),
                FieldMessage::Template("has a \"quote\" and a continued line".to_owned()),
                FieldMessage::Built("format!(\"must be {max}\")".to_owned()),
                FieldMessage::Built("reason".to_owned()),
            ]
        );
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
const SHIPPED: NotificationTemplate = NotificationTemplate::new(
    "orders.order_shipped",
    "Shipped",
    "Your order shipped.",
    "Shipped",
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
                "notes.mail.welcome.text",
                "orders.order_shipped.title",
                "orders.order_shipped.body",
                "orders.order_shipped.short"
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
