//! Ví dụ trong `docs/guides/` chạy thật (đặc tả 31 A5, D-244): tài liệu không lệch code.
//!
//! - Mỗi khối ```bash có dòng `<!-- example: <tên> -->` ngay trước là một bước. Khối là **một** lệnh
//!   `curl` dạng hẹp: `curl -sS [-X METHOD] "$API/<đường dẫn>" [-H 'Tên: giá trị']…
//!   [--json '<JSON>' | --data-binary @<tệp mẫu>]`, kèm dòng chú thích:
//!   - `# save: TÊN=.data.access_token` (giá trị trong thân JSON) hay `# save: TÊN=header:etag`;
//!   - `# expect: 201` (mặc định: mã 2xx);
//!   - `# expect-header: idempotent-replayed: true`.
//! - Biến (`$TÊN`) theo luật shell: thay trong chuỗi `"…"` và chữ trần, không thay trong `'…'`. `$API`
//!   là gốc của máy chủ — ở đây là chuỗi rỗng.
//! - Mỗi hướng dẫn chạy trên một app mới (mọi module của binary, đăng nhập thật của `auth`) với tài
//!   khoản quản trị `admin@example.com` / `correct horse battery staple`.
//! - Khối code của bản tiếng Việt phải giống hệt bản tiếng Anh.
//! - Bảng tool của `mcp.md` (giữa `<!-- tools:start -->` và `<!-- tools:end -->`) phải khớp
//!   `Modules::tools()`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use autoxide_auth::api::{Accounts, NewAccount};
use autoxide_kernel::mcp::Tool;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::Method;
use serde_json::Value;

const ADMIN_EMAIL: &str = "admin@example.com";
const ADMIN_PASSWORD: &str = "correct horse battery staple";

fn guides_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/guides")
}

/// Hướng dẫn bản tiếng Anh, theo tên.
fn guides() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(guides_dir())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            name.ends_with(".md") && !name.ends_with(".vi.md") && name != "README.md"
        })
        .collect();
    found.sort();
    found
}

/// Một bước: tên và nội dung khối `bash`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Step {
    name: String,
    script: String,
}

fn steps(text: &str) -> Vec<Step> {
    let mut found = Vec::new();
    let mut pending: Option<String> = None;
    let mut current: Option<(String, String)> = None;
    for line in text.lines() {
        if let Some((_, script)) = &mut current {
            if line.trim_start().starts_with("```") {
                let (name, script) = current.take().unwrap();
                found.push(Step { name, script });
            } else {
                script.push_str(line);
                script.push('\n');
            }
            continue;
        }
        let trimmed = line.trim();
        if let Some(name) =
            trimmed.strip_prefix("<!-- example:").and_then(|rest| rest.strip_suffix("-->"))
        {
            pending = Some(name.trim().to_owned());
        } else if trimmed == "```bash" {
            if let Some(name) = pending.take() {
                current = Some((name, String::new()));
            }
        } else if !trimmed.is_empty() {
            pending = None;
        }
    }
    found
}

/// Mọi khối code (cả ngôn ngữ), theo thứ tự.
fn code_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            match current.take() {
                Some(block) => blocks.push(block),
                None => current = Some(format!("{}\n", line.trim())),
            }
        } else if let Some(block) = &mut current {
            block.push_str(line);
            block.push('\n');
        }
    }
    blocks
}

/// Tách một dòng lệnh thành các từ theo luật shell (nháy đơn, nháy kép, thay `$TÊN`).
fn words(command: &str, vars: &HashMap<String, String>) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars().peekable();
    let expand = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>, word: &mut String| {
        let braced = chars.peek() == Some(&'{');
        if braced {
            chars.next();
        }
        let mut name = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                name.push(c);
                chars.next();
            } else {
                break;
            }
        }
        if braced {
            chars.next();
        }
        match vars.get(&name) {
            Some(value) => {
                word.push_str(value);
                Ok(())
            }
            None => Err(format!("variable `${name}` is used before it is saved")),
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    word.push(c);
                }
            }
            '"' => {
                in_word = true;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => {
                            if let Some(next) = chars.next() {
                                word.push(next);
                            }
                        }
                        '$' => expand(&mut chars, &mut word)?,
                        _ => word.push(c),
                    }
                }
            }
            '$' => {
                in_word = true;
                expand(&mut chars, &mut word)?;
            }
            c if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            _ => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        out.push(word);
    }
    Ok(out)
}

/// Lệnh `curl` đã đọc.
#[derive(Debug, Default)]
struct Curl {
    method: Option<String>,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    json: bool,
}

fn curl(script: &str, vars: &HashMap<String, String>) -> Result<Curl, String> {
    let command: String = script
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .map(|line| line.strip_suffix('\\').unwrap_or(line))
        .collect::<Vec<_>>()
        .join(" ");
    let words = words(&command, vars)?;
    let mut words = words.iter();
    if words.next().map(String::as_str) != Some("curl") {
        return Err("the block must hold one `curl` command".to_owned());
    }
    let mut parsed = Curl::default();
    while let Some(word) = words.next() {
        match word.as_str() {
            "-sS" | "-s" | "-S" | "-i" | "--fail-with-body" => {}
            "-X" => parsed.method = words.next().cloned(),
            "-H" => {
                let header = words.next().ok_or("-H needs a value")?;
                let (name, value) = header.split_once(':').ok_or("a header is `Name: value`")?;
                parsed.headers.push((name.trim().to_owned(), value.trim().to_owned()));
            }
            "--json" => {
                parsed.body =
                    Some(words.next().ok_or("--json needs a value")?.clone().into_bytes());
                parsed.json = true;
            }
            "--data-binary" => {
                let file = words.next().ok_or("--data-binary needs a value")?;
                let file = file.strip_prefix('@').ok_or("--data-binary takes @<file>")?;
                let path = guides_dir().join("files").join(file);
                parsed.body =
                    Some(std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?);
            }
            url if parsed.url.is_empty() && url.starts_with('/') => parsed.url = url.to_owned(),
            other => return Err(format!("`{other}` is not part of the narrow curl form")),
        }
    }
    if parsed.url.is_empty() {
        return Err("no \"$API/…\" address".to_owned());
    }
    Ok(parsed)
}

/// Dòng `# key: value` của khối.
fn directives<'a>(script: &'a str, key: &str) -> Vec<&'a str> {
    script
        .lines()
        .filter_map(|line| line.trim().strip_prefix(&format!("# {key}:")).map(str::trim))
        .collect()
}

/// Giá trị theo đường `.data.items.0.id`.
fn pick<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.trim_start_matches('.').split('.').filter(|part| !part.is_empty()).try_fold(
        value,
        |current, part| match part.parse::<usize>() {
            Ok(index) => current.get(index),
            Err(_) => current.get(part),
        },
    )
}

/// Ứng dụng của một hướng dẫn: mọi module, tài khoản quản trị.
async fn app(test: &TestDatabase) -> TestApp {
    let dir = std::env::temp_dir().join(format!("autoxide-guides-{}", uuid::Uuid::now_v7()));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string());
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    let app = builder.build().await.unwrap();
    let account = NewAccount {
        email: ADMIN_EMAIL.to_owned(),
        name: "Admin".to_owned(),
        password: Some(ADMIN_PASSWORD.to_owned()),
        admin: true,
    };
    app.api::<dyn Accounts>().unwrap().create(account).await.unwrap();
    app
}

async fn run_guide(test: &TestDatabase, path: &Path) -> Result<usize, String> {
    let text = std::fs::read_to_string(path).unwrap();
    let guide = path.file_name().unwrap().to_string_lossy().into_owned();
    let app = app(test).await;
    let mut vars = HashMap::from([("API".to_owned(), String::new())]);
    let steps = steps(&text);
    for step in &steps {
        let here = format!("{guide}, example `{}`", step.name);
        let parsed = curl(&step.script, &vars).map_err(|error| format!("{here}: {error}"))?;
        let method = parsed.method.clone().unwrap_or_else(|| {
            if parsed.body.is_some() { "POST".to_owned() } else { "GET".to_owned() }
        });
        let expected: Option<u16> =
            directives(&step.script, "expect").first().map(|code| code.parse().unwrap());
        let mut request = app.request(Method::from_bytes(method.as_bytes()).unwrap(), &parsed.url);
        if parsed.json {
            request = request.header("content-type", "application/json");
        }
        for (name, value) in &parsed.headers {
            request = request.header(name, value);
        }
        if let Some(body) = parsed.body.clone() {
            request = request.bytes(body);
        }
        // 304 không khai trong tài liệu của từng endpoint (lớp HTTP của kernel trả cho mọi `GET`
        // có `ETag`): không kiểm theo hợp đồng.
        if expected == Some(304) {
            request = request.unchecked();
        }
        let answer = request.send().await.map_err(|error| format!("{here}: {error}"))?;
        let status = answer.status().as_u16();
        let ok = match expected {
            Some(code) => status == code,
            None => (200..300).contains(&status),
        };
        if !ok {
            return Err(format!(
                "{here}: answered {status}, expected {}: {}",
                expected.map_or_else(|| "2xx".to_owned(), |code| code.to_string()),
                answer.text()
            ));
        }
        for header in directives(&step.script, "expect-header") {
            let (name, value) = header.split_once(':').unwrap();
            let got = answer.header(name.trim());
            if got != value.trim() {
                return Err(format!(
                    "{here}: header `{}` is `{got}`, not `{}`",
                    name.trim(),
                    value.trim()
                ));
            }
        }
        let json = answer.json();
        for save in directives(&step.script, "save") {
            let (name, source) = save.split_once('=').unwrap();
            let value = match source.strip_prefix("header:") {
                Some(header) => answer.header(header).to_owned(),
                None => match pick(&json, source) {
                    Some(Value::String(text)) => text.clone(),
                    Some(other) => other.to_string(),
                    None => return Err(format!("{here}: the answer has no `{source}`: {json}")),
                },
            };
            vars.insert(name.trim().to_owned(), value);
        }
    }
    Ok(steps.len())
}

/// Mọi ví dụ `curl` của `docs/guides/*.md` chạy được, theo thứ tự, trên database gate đang chọn —
/// mỗi hướng dẫn một database trống.
#[tokio::test]
async fn moi_vi_du_curl_cua_huong_dan_chay_duoc() {
    let mut total = 0;
    for path in guides() {
        let ran = TestDatabase::run(async |test| run_guide(test, &path).await).await.unwrap();
        total += ran.unwrap();
    }
    assert!(total >= 20, "only {total} examples ran");
}

/// Bản tiếng Việt giữ đúng các khối code của bản tiếng Anh (chú thích trong code viết tiếng Anh).
#[test]
fn khoi_code_ban_tieng_viet_giong_ban_tieng_anh() {
    for path in guides() {
        let vi = path.with_extension("vi.md");
        let english = code_blocks(&std::fs::read_to_string(&path).unwrap());
        let vietnamese = code_blocks(&std::fs::read_to_string(&vi).unwrap_or_default());
        assert_eq!(
            english,
            vietnamese,
            "{} must hold exactly the code blocks of {}",
            vi.display(),
            path.display()
        );
    }
}

/// Dòng bảng tool cho `docs/guides/mcp.md`.
fn tool_row(tool: &Tool) -> String {
    let permissions: Vec<String> = tool
        .permission()
        .into_iter()
        .chain(tool.alternative())
        .map(|permission| format!("`{}`", permission.name()))
        .collect();
    let kind = if tool.is_read_only() {
        "read-only"
    } else if tool.is_destructive() {
        "destructive"
    } else {
        "writes"
    };
    let permissions =
        if permissions.is_empty() { "—".to_owned() } else { permissions.join(" or ") };
    format!("| `{}` | {permissions} | {kind} |", tool.name())
}

/// Bảng tool trong `mcp.md` (cả bản tiếng Việt) khớp tool thật của mọi module.
#[tokio::test]
async fn bang_tool_mcp_khop_voi_code() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let mut tools = app.modules().tools().unwrap();
        tools.sort_by(|a, b| a.name().cmp(b.name()));
        let expected: Vec<String> = tools.iter().map(tool_row).collect();
        for file in ["mcp.md", "mcp.vi.md"] {
            let text = std::fs::read_to_string(guides_dir().join(file)).unwrap();
            let start = text.find("<!-- tools:start -->").expect("tools:start marker");
            let end = text.find("<!-- tools:end -->").expect("tools:end marker");
            let rows: Vec<String> = text[start..end]
                .lines()
                .filter(|line| line.starts_with("| `"))
                .map(str::to_owned)
                .collect();
            assert_eq!(
                rows,
                expected,
                "the tool table of docs/guides/{file} is out of date; the rows must be:\n{}",
                expected.join("\n")
            );
        }
    })
    .await
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tach_tu_theo_luat_shell() {
        let vars =
            HashMap::from([("API".to_owned(), String::new()), ("T".to_owned(), "abc".to_owned())]);
        let line = r#"curl -sS "$API/x" -H "Authorization: Bearer $T" --json '{"t":"'"$T"'"}'"#;
        assert_eq!(
            words(line, &vars).unwrap(),
            ["curl", "-sS", "/x", "-H", "Authorization: Bearer abc", "--json", r#"{"t":"abc"}"#]
        );
        assert!(words("curl $NOPE", &vars).is_err());
        let parsed =
            curl("curl -sS -X DELETE \"$API/a/$T\" \\\n  -H 'If-Match: *'\n# expect: 204\n", &vars)
                .unwrap();
        assert_eq!((parsed.method.as_deref(), parsed.url.as_str()), (Some("DELETE"), "/a/abc"));
        assert_eq!(parsed.headers, [("If-Match".to_owned(), "*".to_owned())]);
    }

    #[test]
    fn doc_buoc_va_lay_gia_tri() {
        let text = "<!-- example: login -->\n```bash\ncurl -sS \"$API/x\"\n# save: T=.data.items.0.id\n```\n```bash\ncurl ignored\n```\n";
        let found = steps(text);
        assert_eq!(found.len(), 1);
        assert_eq!(directives(&found[0].script, "save"), ["T=.data.items.0.id"]);
        let value = serde_json::json!({"data": {"items": [{"id": "x"}]}});
        assert_eq!(pick(&value, ".data.items.0.id"), Some(&Value::String("x".to_owned())));
    }
}
