//! Độ phủ phân quyền (D-205, đặc tả 54 P1, D-245), chạy trên chương trình có mọi module:
//!
//! - mọi operation không `x-public`: gọi không token → 401;
//! - operation có `x-permissions` khác rỗng: gọi bằng tài khoản không có permission nào → 403
//!   `permission_denied` (hay 404 khi operation khai `x-hides-existence`);
//! - operation `x-permissions: []` (chỉ cần đăng nhập): tài khoản đó gọi được, không 401 / 403;
//! - mọi permission khai trong tài liệu có module khai thật;
//! - tool MCP: mọi tool đòi permission; tool `read_only` chỉ đòi permission mức `read` hay
//!   `sensitive_data` (xem, không đổi gì).
//!
//! Request mẫu sinh từ tài liệu: tham số đường dẫn, query và header bắt buộc, thân tối thiểu theo
//! schema — để lỗi kiểm dữ liệu (422, 428) không che bước kiểm quyền.

use autoxide_auth::api::{Accounts, NewAccount};
use autoxide_kernel::access::Risk;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::Method;
use serde_json::{Map, Value, json};

const SAMPLE_ID: &str = "0193e3a4-7b1c-7f00-9a51-6f2b8c0d4e11";

fn resolve<'a>(document: &'a Value, value: &'a Value) -> &'a Value {
    match value.get("$ref").and_then(Value::as_str) {
        Some(reference) => document.pointer(reference.trim_start_matches('#')).unwrap_or(value),
        None => value,
    }
}

/// Giá trị tối thiểu hợp lệ theo schema (chỉ trường bắt buộc).
fn sample(document: &Value, schema: &Value, depth: usize) -> Value {
    let schema = resolve(document, schema);
    if depth > 8 {
        return Value::Null;
    }
    if let Some(parts) = schema.get("allOf").and_then(Value::as_array) {
        let mut merged = Map::new();
        for part in parts {
            if let Value::Object(fields) = sample(document, part, depth + 1) {
                merged.extend(fields);
            }
        }
        if let Value::Object(own) = sample_object(document, schema, depth) {
            merged.extend(own);
        }
        return Value::Object(merged);
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(parts) = schema.get(key).and_then(Value::as_array)
            && let Some(part) = parts.iter().find(|part| part.get("type") != Some(&json!("null")))
        {
            return sample(document, part, depth + 1);
        }
    }
    if let Some(value) = schema.get("const") {
        return value.clone();
    }
    if let Some(first) =
        schema.get("enum").and_then(Value::as_array).and_then(|values| values.first())
    {
        return first.clone();
    }
    let kind = match schema.get("type") {
        Some(Value::String(kind)) => kind.as_str(),
        Some(Value::Array(kinds)) => {
            kinds.iter().filter_map(Value::as_str).find(|kind| *kind != "null").unwrap_or("null")
        }
        _ if schema.get("properties").is_some() => "object",
        _ => "null",
    };
    match kind {
        "string" => {
            let format = schema.get("format").and_then(Value::as_str).unwrap_or_default();
            let min = schema.get("minLength").and_then(Value::as_u64).unwrap_or(1).max(1);
            match format {
                "uuid" => json!(SAMPLE_ID),
                "date-time" => json!("2030-01-01T00:00:00Z"),
                "date" => json!("2030-01-01"),
                "email" => json!("someone@example.com"),
                "uri" => json!("https://example.com/"),
                _ => json!("x".repeat(usize::try_from(min).unwrap_or(1))),
            }
        }
        "integer" | "number" => {
            json!(
                schema.get("minimum").and_then(Value::as_f64).map_or(1, |min| min.max(1.0) as i64)
            )
        }
        "boolean" => json!(false),
        "array" => {
            let count = schema.get("minItems").and_then(Value::as_u64).unwrap_or(0);
            let item =
                schema.get("items").map_or(Value::Null, |items| sample(document, items, depth + 1));
            Value::Array((0..count).map(|_| item.clone()).collect())
        }
        "object" => sample_object(document, schema, depth),
        _ => Value::Null,
    }
}

fn sample_object(document: &Value, schema: &Value, depth: usize) -> Value {
    let mut out = Map::new();
    let properties = schema.get("properties").and_then(Value::as_object);
    for name in schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let value = properties
            .and_then(|properties| properties.get(name))
            .map_or(json!("x"), |field| sample(document, field, depth + 1));
        out.insert(name.to_owned(), value);
    }
    Value::Object(out)
}

fn as_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Một request mẫu cho operation.
struct Probe {
    method: Method,
    uri: String,
    headers: Vec<(String, String)>,
    body: Option<(String, Vec<u8>)>,
}

fn probe(document: &Value, method: &str, path: &str, operation: &Value) -> Probe {
    let mut uri = path.to_owned();
    let mut query = Vec::new();
    let mut headers = Vec::new();
    for parameter in operation.get("parameters").and_then(Value::as_array).into_iter().flatten() {
        let parameter = resolve(document, parameter);
        let name = parameter.get("name").and_then(Value::as_str).unwrap_or_default();
        let value =
            parameter.get("schema").map_or(json!("x"), |schema| sample(document, schema, 0));
        match parameter.get("in").and_then(Value::as_str) {
            Some("path") => uri = uri.replace(&format!("{{{name}}}"), &as_text(&value)),
            Some("query") if parameter.get("required") == Some(&json!(true)) => {
                query.push(format!("{name}={}", as_text(&value)));
            }
            Some("header") if parameter.get("required") == Some(&json!(true)) => {
                let value = if name.eq_ignore_ascii_case("if-match") {
                    "*".to_owned()
                } else {
                    as_text(&value)
                };
                headers.push((name.to_owned(), value));
            }
            _ => {}
        }
    }
    if !query.is_empty() {
        uri = format!("{uri}?{}", query.join("&"));
    }
    let body =
        operation.pointer("/requestBody/content").and_then(Value::as_object).and_then(|content| {
            let (media, entry) = content.iter().next()?;
            Some(if media.ends_with("json") {
                let value =
                    entry.get("schema").map_or(json!({}), |schema| sample(document, schema, 0));
                (media.clone(), value.to_string().into_bytes())
            } else {
                (media.clone(), b"x".to_vec())
            })
        });
    Probe {
        method: Method::from_bytes(method.to_uppercase().as_bytes()).unwrap(),
        uri,
        headers,
        body,
    }
}

async fn send(app: &TestApp, probe: &Probe, token: Option<&str>) -> (u16, Value) {
    let mut request = app.request(probe.method.clone(), &probe.uri).unchecked();
    if let Some(token) = token {
        request = request.token(token);
    }
    for (name, value) in &probe.headers {
        request = request.header(name, value);
    }
    if let Some((media, bytes)) = &probe.body {
        request = request.header("content-type", media).bytes(bytes.clone());
    }
    let answer = request.send().await.unwrap();
    (answer.status().as_u16(), answer.json())
}

/// Access token của một tài khoản mới, không quản trị, không role nào.
async fn nobody(app: &TestApp) -> String {
    let password = "correct horse battery staple";
    let account = NewAccount {
        email: "nobody@example.com".to_owned(),
        name: "Nobody".to_owned(),
        password: Some(password.to_owned()),
        admin: false,
    };
    app.api::<dyn Accounts>()
        .unwrap()
        .create(&TenantContext::default_on(app.database().clone()), account)
        .await
        .unwrap();
    let login = app
        .post("/api/v1/auth/login")
        .json(json!({"email": "nobody@example.com", "password": password}))
        .send()
        .await
        .unwrap();
    login.json()["data"]["access_token"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn moi_endpoint_khong_cong_khai_tra_401_va_403() {
    TestDatabase::run(async |test| {
        let dir = std::env::temp_dir().join(format!("autoxide-access-{}", uuid::Uuid::now_v7()));
        let mut builder = TestApp::builder(test)
            .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
            .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string());
        for module in crate::modules::all() {
            builder = builder.boxed_module(module);
        }
        let app = builder.build().await.unwrap();
        let token = nobody(&app).await;
        let document = serde_json::to_value(app.openapi()).unwrap();
        let known: Vec<String> =
            app.modules().permissions().iter().map(|permission| permission.name().to_owned()).collect();

        let mut failures = Vec::new();
        let mut checked = 0;
        for (path, item) in document["paths"].as_object().unwrap() {
            for (method, operation) in item.as_object().unwrap() {
                if !matches!(method.as_str(), "get" | "post" | "put" | "patch" | "delete") {
                    continue;
                }
                let endpoint = format!("{} {path}", method.to_uppercase());
                if operation.get("x-public") == Some(&json!(true)) {
                    continue;
                }
                let Some(permissions) = operation.get("x-permissions").and_then(Value::as_array) else {
                    failures.push(format!("{endpoint}: declares neither x-public nor x-permissions"));
                    continue;
                };
                for name in permissions.iter().filter_map(Value::as_str) {
                    if !known.iter().any(|known| known == name) {
                        failures.push(format!("{endpoint}: `{name}` is not a permission any module declares"));
                    }
                }
                checked += 1;
                let probe = probe(&document, method, path, operation);
                let (status, _) = send(&app, &probe, None).await;
                if status != 401 {
                    failures.push(format!("{endpoint}: without a token answered {status}, not 401"));
                }
                let (status, body) = send(&app, &probe, Some(&token)).await;
                let hides = operation.get("x-hides-existence") == Some(&json!(true));
                if permissions.is_empty() {
                    if status == 401 || (status == 403 && body["code"] == "permission_denied") {
                        failures.push(format!("{endpoint}: a signed-in caller was refused ({status} {})", body["code"]));
                    }
                } else if !(status == 403 && body["code"] == "permission_denied" || hides && status == 404) {
                    failures.push(format!(
                        "{endpoint}: a caller without permissions answered {status} {}, not 403 permission_denied",
                        body["code"]
                    ));
                }
            }
        }

        // Tool MCP: mỗi tool đòi permission hay khai `for_any_signed_in`; tool chỉ đọc gọi được bằng
        // ít nhất một permission mức xem (`read` / `sensitive_data`). Ngoại lệ có lý do (D-245):
        // `files.list_files` — `files.upload` gồm cả xem file của chính mình, `files` không có
        // permission chỉ đọc; `kernel.list_domains` — tên miền riêng chỉ có một permission
        // `kernel.domains.manage` (đặc tả 33), xem là một phần của quản lý; `auth.list_invitations` —
        // lời mời chỉ có một permission `auth.invitations.manage` (đặc tả 34, D-312).
        const READ_ONLY_EXCEPTIONS: [&str; 3] =
            ["files.list_files", "kernel.list_domains", "auth.list_invitations"];
        for tool in app.modules().tools().unwrap() {
            let permissions: Vec<_> = tool.permission().into_iter().chain(tool.alternative()).collect();
            if permissions.is_empty() && !tool.is_for_any_signed_in() {
                failures.push(format!(
                    "tool {}: requires no permission; add `.requires(…)` or `.for_any_signed_in()`",
                    tool.name()
                ));
            }
            let viewable = permissions.iter().any(|permission| matches!(permission.risk(), Risk::Read | Risk::SensitiveData));
            if tool.is_read_only()
                && !permissions.is_empty()
                && !viewable
                && !READ_ONLY_EXCEPTIONS.contains(&tool.name())
            {
                failures.push(format!(
                    "tool {}: read-only but no read-level permission lets you call it ({})",
                    tool.name(),
                    permissions.iter().map(|permission| permission.name()).collect::<Vec<_>>().join(", ")
                ));
            }
        }
        assert!(checked > 150, "only {checked} operations checked");
        assert!(failures.is_empty(), "{} problem(s):\n{}", failures.len(), failures.join("\n"));
    })
    .await
    .unwrap();
}
