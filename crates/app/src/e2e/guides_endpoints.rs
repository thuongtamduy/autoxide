//! Mọi lệnh `curl` trong `docs/guides/` (cả khối không chạy thật như `<!-- example -->` của `guides.rs`, vd bước cần
//! tài khoản nhà cung cấp thật) gọi đúng một endpoint có trong `docs/openapi.json`, đúng method, và thân JSON chỉ
//! dùng trường có thật (P11 của bước 70: mọi hướng dẫn làm theo được đúng như viết). `writing-a-module.md` gọi
//! module của chính người đọc nên bỏ qua.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

fn guides_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/guides")
}

/// Hướng dẫn bản tiếng Anh, cả thư mục con (`providers/`).
fn guides(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            guides(&path, found);
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.ends_with(".md") && !name.ends_with(".vi.md") && name != "writing-a-module.md" {
            found.push(path);
        }
    }
}

/// Các lệnh `curl` trong khối `bash`, mỗi lệnh một dòng (đã nối dòng kết thúc bằng `\`).
fn curl_commands(text: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut in_bash = false;
    let mut current: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_bash = !in_bash && trimmed == "```bash";
            commands.extend(current.take());
            continue;
        }
        if !in_bash {
            continue;
        }
        let (body, continues) = match trimmed.strip_suffix('\\') {
            Some(body) => (body, true),
            None => (trimmed, false),
        };
        match &mut current {
            Some(command) => {
                command.push(' ');
                command.push_str(body);
            }
            None if body.starts_with("curl ") => current = Some(body.to_owned()),
            None => {}
        }
        if !continues {
            commands.extend(current.take());
        }
    }
    commands
}

/// Đường dẫn sau `$API` (bỏ query, thay biến `$TÊN` / `${TÊN}` bằng `x`).
fn path_of(command: &str) -> Option<String> {
    let start = command.find("$API/")? + "$API".len();
    let rest = &command[start..];
    let end = rest.find(['"', '\'', ' ', '?']).unwrap_or(rest.len());
    let raw = &rest[..end];
    let segments: Vec<String> = raw
        .split('/')
        .map(|segment| if segment.contains('$') { "x".to_owned() } else { segment.to_owned() })
        .collect();
    Some(segments.join("/"))
}

/// Thân JSON trong `--json '…'` / `-d '…'` / `--data '…'`, với biến chèn kiểu `'"$TÊN"'` thay bằng `x`.
fn body_of(command: &str) -> Option<Value> {
    let flags = ["--json '", "-d '", "--data '"];
    let start = flags.iter().find_map(|flag| command.find(flag).map(|at| at + flag.len()))?;
    let mut text = String::new();
    let mut rest = &command[start..];
    loop {
        let end = rest.find('\'')?;
        text.push_str(&rest[..end]);
        rest = &rest[end + 1..];
        // `'"$TÊN"'`: đóng nháy đơn, biến trong nháy kép, mở lại nháy đơn.
        if let Some(after) = rest.strip_prefix("\"$") {
            let close = after.find("\"'")?;
            text.push('x');
            rest = &after[close + 2..];
            continue;
        }
        break;
    }
    serde_json::from_str(&text).ok()
}

fn method_of(command: &str, has_body: bool) -> String {
    let mut words = command.split_whitespace();
    while let Some(word) = words.next() {
        if word == "-X" {
            return words.next().unwrap_or("GET").to_ascii_lowercase();
        }
    }
    if has_body { "post".to_owned() } else { "get".to_owned() }
}

/// Mẫu đường dẫn của tài liệu khớp `path` (đoạn `{…}` khớp một đoạn bất kỳ; ưu tiên mẫu nhiều đoạn cố định hơn).
fn template<'a>(paths: &'a Map<String, Value>, path: &str) -> Option<&'a str> {
    let segments: Vec<&str> = path.split('/').collect();
    let mut best: Option<(usize, &str)> = None;
    for template in paths.keys() {
        let parts: Vec<&str> = template.split('/').collect();
        if parts.len() != segments.len() {
            continue;
        }
        let fits = parts.iter().zip(&segments).all(|(part, segment)| {
            (part.starts_with('{') && part.ends_with('}')) || part == segment
        });
        if fits {
            let literal = parts.iter().filter(|part| !part.starts_with('{')).count();
            if best.is_none_or(|(score, _)| literal > score) {
                best = Some((literal, template));
            }
        }
    }
    best.map(|(_, template)| template)
}

/// Tên các trường của một schema (theo `$ref`, gộp `allOf`); `None` khi schema không liệt kê trường.
fn fields(document: &Value, schema: &Value) -> Option<Vec<String>> {
    let mut schema = schema;
    while let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        schema = document.pointer(&reference.replace('#', ""))?;
    }
    let mut names: Vec<String> = Vec::new();
    for part in schema.get("allOf").and_then(Value::as_array).into_iter().flatten() {
        names.extend(fields(document, part).unwrap_or_default());
    }
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        names.extend(properties.keys().cloned());
    }
    if names.is_empty() { None } else { Some(names) }
}

#[test]
fn moi_lenh_curl_cua_huong_dan_goi_endpoint_co_that() {
    let text = std::fs::read_to_string(guides_dir().join("../openapi.json")).unwrap();
    let document: Value = serde_json::from_str(&text).unwrap();
    let paths = document["paths"].as_object().unwrap();
    let mut files = Vec::new();
    guides(&guides_dir(), &mut files);
    files.sort();
    let mut problems = Vec::new();
    let mut checked = 0;
    for file in &files {
        let name = file.strip_prefix(guides_dir()).unwrap().display().to_string();
        for command in curl_commands(&std::fs::read_to_string(file).unwrap()) {
            let Some(path) = path_of(&command) else { continue };
            let body = body_of(&command);
            let sends =
                [" -d ", " --json ", " --data", " -F "].iter().any(|flag| command.contains(flag));
            let method = method_of(&command, sends);
            checked += 1;
            let Some(found) = template(paths, &path) else {
                problems.push(format!(
                    "{name}: {} {path} is not in docs/openapi.json",
                    method.to_uppercase()
                ));
                continue;
            };
            let Some(operation) = paths[found].get(&method) else {
                problems.push(format!("{name}: {found} has no {}", method.to_uppercase()));
                continue;
            };
            let Some(Value::Object(body)) = body else { continue };
            let schema = operation.pointer("/requestBody/content/application~1json/schema");
            let Some(known) = schema.and_then(|schema| fields(&document, schema)) else { continue };
            for key in body.keys().filter(|key| !known.contains(key)) {
                // `errors.md` gửi sai tên trường có chủ đích để chỉ cách đọc lỗi.
                if name == "errors.md" {
                    continue;
                }
                problems.push(format!(
                    "{name}: {} {found} has no field `{key}`",
                    method.to_uppercase()
                ));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "guides call what the API does not have:\n{}",
        problems.join("\n")
    );
    assert!(checked >= 60, "only {checked} curl commands found");
}

#[test]
fn doc_lenh_curl_noi_dong_va_bien() {
    let text = "```bash\ncurl -sS -X PATCH \"$API/api/v1/notes/$ID?x=1\" \\\n  --json '{\"title\":\"'\"$T\"'\"}'\n```\n";
    let commands = curl_commands(text);
    assert_eq!(commands.len(), 1);
    assert_eq!(path_of(&commands[0]).as_deref(), Some("/api/v1/notes/x"));
    assert_eq!(body_of(&commands[0]), Some(serde_json::json!({"title": "x"})));
    assert_eq!(method_of(&commands[0], true), "patch");
}
