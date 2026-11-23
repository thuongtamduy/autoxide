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

/// Khung thời gian của một chương trình khuyến mại đang chạy, trong giới hạn số ngày của luật:
/// từ một giờ trước tới 30 ngày sau (RFC 3339).
pub(crate) fn promotion_window() -> (String, String) {
    let now = autoxide_kernel::time::Timestamp::now();
    let hour = jiff::SignedDuration::from_hours(1);
    ((now - hour).to_rfc3339(), (now + hour * 24 * 30).to_rfc3339())
}
