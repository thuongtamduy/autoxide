//! Đồ dùng chung cho các luồng e2e "dùng được mọi thứ đã dựng" (P11 của bước 70):
//! dựng app với mọi module của binary và một quản trị viên có mọi permission của module, gọi REST và
//! tool MCP rồi đòi trả lời thành công kèm thông báo rõ khi hỏng.

use autoxide_testkit::{TestApp, TestAppBuilder, TestDatabase, tool};
use axum::http::{Method, StatusCode};
use serde_json::Value;

/// Token của quản trị viên có mọi permission.
pub(crate) const ADMIN: &str = "admin";
/// "Token" của người gọi chưa đăng nhập: [`call`] không gửi `Authorization`.
pub(crate) const ANONYMOUS: &str = "";
/// ID của quản trị viên.
pub(crate) const ADMIN_ID: u128 = 0xad;

/// Permission của kernel (module nội bộ của kernel không nằm trong `modules::all()`).
const KERNEL_PERMISSIONS: &[&str] = &[
    "kernel.modules.read",
    "kernel.tenants.operate",
    "kernel.settings.manage",
    "kernel.tenants.read",
    "kernel.tenants.manage",
    "kernel.placements.read",
    "kernel.domains.manage",
    "kernel.tenants.move",
    "kernel.tenants.relocate",
    "kernel.relocations.request",
    "kernel.sandboxes.read",
    "kernel.sandboxes.manage",
    "kernel.data.export",
    "kernel.tenants.delete",
    "kernel.jobs.read",
    "kernel.views.share",
    "kernel.reasons.manage",
    "kernel.tags.manage",
    "kernel.texts.manage",
    "kernel.custom_fields.manage",
    "kernel.jobs.manage",
    "kernel.devtools.read",
    "kernel.devtools.manage",
    "kernel.ops.read",
];

/// Mọi permission: của mọi module trong binary và của kernel.
pub(crate) fn every_permission() -> Vec<String> {
    let mut names: Vec<String> = crate::modules::all()
        .iter()
        .flat_map(|module| module.permissions())
        .map(|permission| permission.name().to_owned())
        .chain(KERNEL_PERMISSIONS.iter().map(|name| (*name).to_owned()))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Builder với mọi module của binary và quản trị viên [`ADMIN`]; `more` thêm người dùng hay cấu hình.
pub(crate) async fn app_with(
    test: &TestDatabase,
    more: impl FnOnce(TestAppBuilder<'_>) -> TestAppBuilder<'_>,
) -> TestApp {
    let permissions = every_permission();
    let permissions: Vec<&str> = permissions.iter().map(String::as_str).collect();
    let dir = std::env::temp_dir().join(format!("autoxide-usable-{}", uuid::Uuid::now_v7()));
    let builder = TestApp::builder(test)
        .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        .user(ADMIN, ADMIN_ID, &permissions);
    let mut builder = more(builder);
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    builder.build().await.unwrap()
}

/// [`app_with`] không thêm gì.
pub(crate) async fn app(test: &TestDatabase) -> TestApp {
    app_with(test, |builder| builder).await
}

/// Gọi `method path` (đường dẫn đầy đủ, vd `/api/v1/tax/rates`) với token `who` ([`ANONYMOUS`]: không
/// đăng nhập); trả mã và thân JSON (`Null` khi thân rỗng hay không phải JSON).
pub(crate) async fn call(
    app: &TestApp,
    method: &str,
    path: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, path);
    if who != ANONYMOUS {
        request = request.token(who);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = serde_json::from_slice(answer.body()).unwrap_or(Value::Null);
    (answer.status(), body)
}

/// Như [`call`] nhưng đòi mã `expect`; trả thân JSON.
pub(crate) async fn expect(
    app: &TestApp,
    method: &str,
    path: &str,
    who: &str,
    body: Option<Value>,
    expect: u16,
) -> Value {
    let (status, answer) = call(app, method, path, who, body).await;
    assert_eq!(status.as_u16(), expect, "{method} {path}: {answer}");
    answer
}

/// `GET path` của quản trị viên, đòi 200; trả thân JSON.
pub(crate) async fn get(app: &TestApp, path: &str) -> Value {
    expect(app, "GET", path, ADMIN, None, 200).await
}

/// Gọi tool MCP `name` bằng token `who`, đòi kết quả không lỗi; trả `data` của `structuredContent`
/// (đối tượng hay danh sách), hay cả `structuredContent` khi không có `data`.
pub(crate) async fn call_tool(app: &TestApp, who: &str, name: &str, arguments: Value) -> Value {
    let client = app.mcp(who).await.unwrap();
    let result = client.call_tool(tool(name, arguments)).await.unwrap_or_else(|error| {
        panic!("tool {name}: {error}");
    });
    assert_ne!(result.is_error, Some(true), "tool {name}: {:?}", result.content);
    let content = result.structured_content.unwrap_or(Value::Null);
    match content.get("data") {
        Some(data) => data.clone(),
        None => content,
    }
}

/// Nạp dữ liệu tham chiếu đi kèm bản cài (đơn vị hành chính, biểu thuế, giới hạn khuyến mại theo luật)
/// như người vận hành chạy `autoxide task address.import`, `tax.import_pack`, `promotions.load_legal_data`.
pub(crate) async fn import_reference_data(app: &TestApp) {
    let tasks = app.modules().tasks().unwrap();
    for name in ["address.import", "tax.import_pack", "promotions.load_legal_data"] {
        let (_, task) = tasks.iter().find(|(full, _)| full == name).unwrap();
        task.run(&task.parse(&[]).unwrap()).await.unwrap();
    }
}

/// ID trong `{"data": {"id": …}}`.
pub(crate) fn id(body: &Value) -> String {
    body["data"]["id"].as_str().unwrap_or_else(|| panic!("no data.id in {body}")).to_owned()
}

/// Chạy event và job đang chờ tới khi hết (tối đa 20 vòng): như worker của `autoxide serve`.
pub(crate) async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..20 {
        let events = dispatcher.run_once().await.unwrap();
        if worker.run_once().await.unwrap() + events == 0 {
            break;
        }
    }
}

/// App với đăng nhập thật của `auth` (không người dùng giả): tài khoản quản trị `admin@example.com`
/// (role `admin`, mọi permission) đã đăng nhập; thư được ghi thành file ở `mail_dir`.
pub(crate) struct RealApp {
    pub(crate) app: TestApp,
    /// Access token của quản trị viên.
    pub(crate) admin: String,
    /// Thư mục thư gửi đi (`AUTOXIDE_MAIL_FILE_DIR`).
    pub(crate) mail_dir: std::path::PathBuf,
}

/// Email và mật khẩu của quản trị viên ở [`RealApp`].
pub(crate) const ADMIN_EMAIL: &str = "admin@example.com";
pub(crate) const ADMIN_PASSWORD: &str = "correct horse battery staple";

/// [`RealApp`] với cấu hình thêm `settings`.
pub(crate) async fn real_app(test: &TestDatabase, settings: &[(&str, &str)]) -> RealApp {
    real_app_with(test, settings, |builder| builder).await
}

/// [`real_app`]; `more` chỉnh builder thêm (vd `multi_tenant()`).
pub(crate) async fn real_app_with(
    test: &TestDatabase,
    settings: &[(&str, &str)],
    more: impl FnOnce(TestAppBuilder<'_>) -> TestAppBuilder<'_>,
) -> RealApp {
    use autoxide_auth::api::{Accounts, NewAccount};
    use autoxide_kernel::tenancy::TenantContext;

    let dir = std::env::temp_dir().join(format!("autoxide-usable-{}", uuid::Uuid::now_v7()));
    let mail_dir = dir.join("mail");
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
        .setting("AUTOXIDE_MAIL_FILE_DIR", &mail_dir.display().to_string());
    for (name, value) in settings {
        builder = builder.setting(name, value);
    }
    builder = more(builder);
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
    app.api::<dyn Accounts>()
        .unwrap()
        .create(&TenantContext::default_on(app.database().clone()), account)
        .await
        .unwrap();
    let admin = login(&app, ADMIN_EMAIL, ADMIN_PASSWORD).await;
    RealApp { app, admin, mail_dir }
}

/// Đăng nhập bằng mật khẩu vào khách mặc định; trả access token.
pub(crate) async fn login(app: &TestApp, email: &str, password: &str) -> String {
    let tenant = app.default_tenant();
    login_in(app, tenant.slug(), email, password).await
}

/// Đăng nhập bằng mật khẩu vào khách `slug`, nói rõ khách bằng header `Autoxide-Tenant` như client
/// thật làm ở chế độ nhiều khách; trả access token.
pub(crate) async fn login_in(app: &TestApp, slug: &str, email: &str, password: &str) -> String {
    let body = serde_json::json!({"email": email, "password": password});
    let answer = app
        .post("/api/v1/auth/login")
        .header("autoxide-tenant", slug)
        .json(body)
        .send()
        .await
        .unwrap();
    let json = answer.json();
    assert_eq!(answer.status(), StatusCode::OK, "login {email}: {json}");
    json["data"]["access_token"].as_str().unwrap().to_owned()
}

/// Chủ (role `admin`) của khách `slug` đã khai bằng `.tenant(slug)`: tạo tài khoản như lệnh
/// `autoxide user create`, đăng nhập; trả access token.
pub(crate) async fn owner_of(app: &TestApp, slug: &str) -> String {
    use autoxide_auth::api::{Accounts, NewAccount};

    let tenant = app.tenant(slug).await.unwrap();
    let email = format!("owner@{slug}.example");
    let account = NewAccount {
        email: email.clone(),
        name: "Owner".to_owned(),
        password: Some(ADMIN_PASSWORD.to_owned()),
        admin: true,
    };
    app.api::<dyn Accounts>().unwrap().create(&tenant, account).await.unwrap();
    login_in(app, slug, &email, ADMIN_PASSWORD).await
}

/// Nội dung mọi thư đã gửi tới `to` (cũ trước), đọc từ thư mục thư.
pub(crate) fn mails_to(mail_dir: &std::path::Path, to: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(mail_dir) else {
        return Vec::new();
    };
    let mut files: Vec<std::path::PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter(|text| text.lines().any(|line| line.starts_with("To:") && line.contains(to)))
        .collect()
}

/// Giá trị của tham số `name=` đầu tiên trong một thư (vd token trong link), bỏ phần mã hoá dòng.
pub(crate) fn mail_param(mail: &str, name: &str) -> String {
    let joined = mail.replace("=\r\n", "").replace("=\n", "").replace("=3D", "=");
    let start =
        joined.find(&format!("{name}=")).unwrap_or_else(|| panic!("no `{name}=` in mail:\n{mail}"))
            + name.len()
            + 1;
    joined[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'))
        .collect()
}

/// Khung thời gian của một chương trình khuyến mại đang chạy, trong giới hạn số ngày của luật:
/// từ một giờ trước tới 30 ngày sau (RFC 3339).
pub(crate) fn promotion_window() -> (String, String) {
    let now = autoxide_kernel::time::Timestamp::now();
    let hour = jiff::SignedDuration::from_hours(1);
    ((now - hour).to_rfc3339(), (now + hour * 24 * 30).to_rfc3339())
}
