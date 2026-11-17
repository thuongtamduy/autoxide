//! Test qua HTTP (router thật của kernel, cùng module `antispam` thật) và MCP (client thật):
//! endpoint công khai không lộ phần riêng tư, câu trả lời giống hệt cho bài nhận / dính bẫy /
//! quá nhiều lần, gửi lại cùng token, `If-Match` bắt buộc, CSV, xuất / xoá theo email, giới hạn
//! thân request, tool MCP. Gate chạy trên cả SQLite và Postgres.

use std::sync::Arc;

use autoxide_kernel::mail::Mailer;
use autoxide_kernel::storage::Storage;
use autoxide_testkit::{MemoryStorage, TestApp, TestDatabase, TestResponse, tool};
use axum::http::{Method, StatusCode, header};
use serde_json::{Value, json};

use crate::testing::FakeMailer;

/// Token → permission: `admin` mọi quyền của module; `reader` chỉ đọc; `website` chỉ
/// `forms.submit` (máy chủ của website gửi form bằng API key); `member` không quyền nào (tài
/// khoản tự đăng ký). Bên gửi mail là bản giả, thay cho module `mail`; nơi lưu trong bộ nhớ
/// thay cho module `files`.
async fn app(test: &TestDatabase) -> TestApp {
    let mailer: Arc<dyn Mailer> = Arc::new(FakeMailer::default());
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    TestApp::builder(test)
        // Không chờ 3 giây giữa lấy token và gửi.
        .setting("AUTOXIDE_ANTISPAM_MIN_FILL_SECONDS", "0")
        .setting("AUTOXIDE_FORMS_MAX_SUBMISSION_BYTES", "2048")
        .user("admin", 1, &["forms.read", "forms.manage", "forms.review", "forms.export"])
        .user("reader", 2, &["forms.read"])
        .user("website", 3, &["forms.submit"])
        .user("member", 4, &[])
        .provide::<dyn Mailer>(mailer)
        .provide::<dyn Storage>(storage)
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

/// Gửi request tới `/api/v1{uri}`; `who`: token (`None`: không đăng nhập).
async fn send(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> TestResponse {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1{uri}"));
    if let Some(who) = who {
        request = request.token(who);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    for (name, value) in headers {
        request = request.header(name, value);
    }
    request.send().await.unwrap()
}

fn definition() -> Value {
    json!({
        "key": "contact",
        "name": "Contact us",
        "fields": [
            {"key": "name", "type": "text", "label": {"en": "Name", "vi": "Tên"}, "required": true},
            {"key": "email", "type": "email", "label": {"en": "Email"}, "required": true},
            {"key": "message", "type": "textarea", "label": {"en": "Message"}},
            {"key": "topic", "type": "select", "label": {"en": "Topic"}, "options": [{"value": "sales", "label": {"en": "Sales", "vi": "Bán hàng"}}]}
        ],
        "allowed_origins": ["https://example.com"],
        "notify_emails": ["owner@example.com"],
        "success_message": {"en": "Thanks!", "vi": "Cảm ơn!"},
        "email_limit_per_hour": 1
    })
}

async fn form_token(app: &TestApp) -> String {
    let issued = send(app, "GET", "/antispam/token?scope=forms:contact", None, &[], None).await;
    assert_eq!(issued.status(), StatusCode::OK, "{}", issued.json());
    issued.json()["data"]["token"].as_str().unwrap().to_owned()
}

/// Các khoá của `data` trong câu trả lời, để so hình dạng.
fn shape(answer: &TestResponse) -> Vec<String> {
    let mut keys: Vec<String> =
        answer.json()["data"].as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

/// Vòng đời qua HTTP: tạo form, định nghĩa công khai không lộ phần riêng tư, gửi bài, câu trả
/// lời giống hệt cho bài dính bẫy và bài vượt giới hạn, gửi lại cùng token, lỗi kiểm dữ liệu,
/// `Origin`, form đóng, `If-Match`, hộp thư, CSV, xuất / xoá theo email, thân quá lớn.
#[tokio::test]
async fn vong_doi_qua_http() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let created = send(&app, "POST", "/forms", Some("admin"), &[], Some(definition())).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(created.header(header::LOCATION), format!("/api/v1/forms/{id}"));
        assert_eq!(created.json()["data"]["token_scope"], "forms:contact");
        let taken = send(&app, "POST", "/forms", Some("admin"), &[], Some(definition())).await;
        assert_eq!((taken.status(), taken.json()["code"].as_str()), (StatusCode::CONFLICT, Some("forms.key_taken")));
        let denied = send(&app, "POST", "/forms", Some("reader"), &[], Some(definition())).await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);

        // Định nghĩa công khai: không lộ email nhận thông báo, giới hạn, origin.
        let public = send(&app, "GET", "/forms/public/contact?locale=vi", None, &[], None).await;
        assert_eq!(public.status(), StatusCode::OK, "{}", public.json());
        let text = public.text();
        for private in ["owner@example.com", "notify", "allowed_origins", "email_limit", "retention"] {
            assert!(!text.contains(private), "public form shows `{private}`: {text}");
        }
        assert_eq!(public.json()["data"]["fields"][0]["label"], "Tên");
        assert_eq!(public.json()["data"]["fields"][3]["options"][0]["label"], "Bán hàng");
        assert_eq!(public.json()["data"]["token_scope"], "forms:contact");
        let etag = public.header(header::ETAG).to_owned();
        let cached = send(&app, "GET", "/forms/public/contact?locale=vi", None, &[("if-none-match", &etag)], None).await;
        assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(send(&app, "GET", "/forms/public/nope", None, &[], None).await.status(), StatusCode::NOT_FOUND);
        let french = send(&app, "GET", "/forms/public/contact?locale=fr", None, &[], None).await;
        assert_eq!((french.status(), french.json()["errors"][0]["field"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("locale")));

        // Bài thật.
        let origin = [("origin", "https://example.com")];
        let body = |token: &str, email: &str| json!({
            "data": {"name": "Ada", "email": email, "message": "Hello", "topic": "sales"},
            "locale": "vi", "form_token": token
        });
        let token = form_token(&app).await;
        let accepted = send(&app, "POST", "/forms/public/contact/submissions", None, &origin, Some(body(&token, "ada@example.com"))).await;
        assert_eq!(accepted.status(), StatusCode::CREATED, "{}", accepted.json());
        assert_eq!(accepted.json()["data"]["message"], "Cảm ơn!");
        let submission = accepted.json()["data"]["id"].as_str().unwrap().to_owned();
        // Gửi lại cùng token: cùng câu trả lời, không thêm bài.
        let again = send(&app, "POST", "/forms/public/contact/submissions", None, &origin, Some(body(&token, "ada@example.com"))).await;
        assert_eq!((again.status(), again.json()["data"]["id"].as_str()), (StatusCode::CREATED, Some(submission.as_str())));

        // Dính bẫy: cùng mã, cùng hình dạng, cùng lời nhắn.
        let mut trapped = body(&form_token(&app).await, "bot@example.com");
        trapped["trap"] = json!("http://spam.example");
        let trapped = send(&app, "POST", "/forms/public/contact/submissions", None, &origin, Some(trapped)).await;
        assert_eq!((trapped.status(), shape(&trapped), &trapped.json()["data"]["message"]), (accepted.status(), shape(&accepted), &accepted.json()["data"]["message"]));
        assert_eq!(trapped.header(header::LOCATION).len(), accepted.header(header::LOCATION).len());
        // Quá nhiều lần với một email (giới hạn 1/giờ; lần gửi lại cùng token ở trên KHÔNG bị
        // đếm, để không ai khoá được email của người khác — D-150): câu trả lời y hệt, không lưu.
        let limited = send(&app, "POST", "/forms/public/contact/submissions", None, &origin, Some(body(&form_token(&app).await, "ada@example.com"))).await;
        assert_eq!((limited.status(), shape(&limited)), (accepted.status(), shape(&accepted)));
        assert_ne!(limited.json()["data"]["id"], accepted.json()["data"]["id"]);

        // Lỗi thật thì rõ ràng: dữ liệu sai (mọi lỗi một lần), token thiếu, `Origin` lạ.
        let wrong = json!({"data": {"email": "nope", "topic": "other", "extra": 1}, "form_token": form_token(&app).await});
        let invalid = send(&app, "POST", "/forms/public/contact/submissions", None, &origin, Some(wrong)).await;
        assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let mut fields: Vec<String> = invalid.json()["errors"].as_array().unwrap().iter().map(|e| e["field"].as_str().unwrap().to_owned()).collect();
        fields.sort();
        assert_eq!(fields, ["data.email", "data.extra", "data.name", "data.topic"]);
        let no_token = send(&app, "POST", "/forms/public/contact/submissions", None, &origin, Some(json!({"data": {"name": "A", "email": "a@example.com"}}))).await;
        assert_eq!((no_token.status(), no_token.json()["code"].as_str()), (StatusCode::BAD_REQUEST, Some("antispam.token_missing")));
        let foreign = send(&app, "POST", "/forms/public/contact/submissions", None, &[("origin", "https://evil.example")], Some(body(&form_token(&app).await, "c@example.com"))).await;
        assert_eq!((foreign.status(), foreign.json()["code"].as_str()), (StatusCode::FORBIDDEN, Some("forms.origin_not_allowed")));
        // Thân quá lớn (giới hạn 2048 byte trong test).
        let huge = json!({"data": {"name": "A", "email": "a@example.com", "message": "x".repeat(3000)}, "form_token": "t"});
        let too_large = send(&app, "POST", "/forms/public/contact/submissions", None, &origin, Some(huge)).await;
        assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
        // Website gọi bằng API key: không cần token.
        let signed = json!({"data": {"name": "Web", "email": "web@example.com"}});
        let by_website = send(&app, "POST", "/forms/public/contact/submissions", Some("website"), &[], Some(signed)).await;
        assert_eq!(by_website.status(), StatusCode::CREATED, "{}", by_website.json());
        // Đăng nhập mà không có `forms.submit`: như người lạ, thiếu token thì bị từ chối.
        let signed = json!({"data": {"name": "Member", "email": "member@example.com"}});
        let by_member = send(&app, "POST", "/forms/public/contact/submissions", Some("member"), &[], Some(signed)).await;
        assert_eq!((by_member.status(), by_member.json()["code"].as_str()), (StatusCode::BAD_REQUEST, Some("antispam.token_missing")));

        // Hộp thư: hai bài (Ada, Web); đổi trạng thái cần `If-Match`.
        let inbox = send(&app, "GET", &format!("/forms/{id}/submissions"), Some("reader"), &[], None).await;
        assert_eq!(inbox.json()["data"].as_array().unwrap().len(), 2, "{}", inbox.json());
        let one = send(&app, "GET", &format!("/forms/submissions/{submission}"), Some("reader"), &[], None).await;
        assert_eq!(one.json()["data"]["answers"][0]["label"]["vi"], "Tên");
        assert_eq!(one.json()["data"]["email"], "ada@example.com");
        let version = one.header(header::ETAG).to_owned();
        let mark = json!({"status": "read"});
        let missing = send(&app, "PATCH", &format!("/forms/submissions/{submission}"), Some("admin"), &[], Some(mark.clone())).await;
        assert_eq!(missing.status(), StatusCode::PRECONDITION_REQUIRED);
        let by_reader = send(&app, "PATCH", &format!("/forms/submissions/{submission}"), Some("reader"), &[("if-match", &version)], Some(mark.clone())).await;
        assert_eq!(by_reader.status(), StatusCode::FORBIDDEN);
        let marked = send(&app, "PATCH", &format!("/forms/submissions/{submission}"), Some("admin"), &[("if-match", &version)], Some(mark.clone())).await;
        assert_eq!((marked.status(), marked.json()["data"]["status"].as_str()), (StatusCode::OK, Some("read")));
        let stale = send(&app, "PATCH", &format!("/forms/submissions/{submission}"), Some("admin"), &[("if-match", &version)], Some(json!({"status": "archived"}))).await;
        assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);

        // CSV.
        let csv = send(&app, "GET", &format!("/forms/{id}/submissions/export?format=csv"), Some("admin"), &[], None).await;
        assert_eq!(csv.status(), StatusCode::OK);
        assert_eq!(csv.header(header::CONTENT_TYPE), "text/csv; charset=utf-8");
        assert!(csv.header(header::CONTENT_DISPOSITION).starts_with("attachment; filename=\"contact-submissions.csv\""));
        assert!(csv.text().contains("\"ada@example.com\""));
        assert_eq!(send(&app, "GET", &format!("/forms/{id}/submissions/export"), Some("reader"), &[], None).await.status(), StatusCode::FORBIDDEN);
        assert_eq!(send(&app, "GET", &format!("/forms/{id}/submissions/export?format=xlsx"), Some("admin"), &[], None).await.status(), StatusCode::UNPROCESSABLE_ENTITY);

        // Xuất / xoá theo email.
        let data = send(&app, "GET", "/forms/data?email=ADA@example.com", Some("admin"), &[], None).await;
        assert_eq!(data.status(), StatusCode::OK, "{}", data.json());
        assert_eq!(data.json()["data"]["submissions"].as_array().unwrap().len(), 1);
        assert_eq!(send(&app, "GET", "/forms/data?email=ada@example.com", Some("reader"), &[], None).await.status(), StatusCode::FORBIDDEN);
        assert_eq!(send(&app, "GET", "/forms/data", Some("admin"), &[], None).await.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let erased = send(&app, "DELETE", "/forms/data?email=ada@example.com", Some("admin"), &[], None).await;
        assert_eq!(erased.json()["data"]["deleted"], 1);
        assert_eq!(send(&app, "GET", &format!("/forms/submissions/{submission}"), Some("admin"), &[], None).await.status(), StatusCode::NOT_FOUND);

        // Sửa form: `If-Match` bắt buộc; đóng form thì gửi bị từ chối rõ ràng.
        let form = send(&app, "GET", &format!("/forms/{id}"), Some("admin"), &[], None).await;
        let form_etag = form.header(header::ETAG).to_owned();
        let close = json!({"status": "closed"});
        assert_eq!(send(&app, "PATCH", &format!("/forms/{id}"), Some("admin"), &[], Some(close.clone())).await.status(), StatusCode::PRECONDITION_REQUIRED);
        let closed = send(&app, "PATCH", &format!("/forms/{id}"), Some("admin"), &[("if-match", &form_etag)], Some(close)).await;
        assert_eq!(closed.status(), StatusCode::OK, "{}", closed.json());
        let refused = send(&app, "POST", "/forms/public/contact/submissions", None, &origin, Some(body(&form_token(&app).await, "z@example.com"))).await;
        assert_eq!((refused.status(), refused.json()["code"].as_str()), (StatusCode::CONFLICT, Some("forms.closed")));
        assert_eq!(send(&app, "DELETE", &format!("/forms/{id}"), Some("admin"), &[("if-match", &form_etag)], None).await.status(), StatusCode::PRECONDITION_FAILED);
        let new_etag = closed.header(header::ETAG).to_owned();
        assert_eq!(send(&app, "DELETE", &format!("/forms/{id}"), Some("admin"), &[("if-match", &new_etag)], None).await.status(), StatusCode::NO_CONTENT);
    })
    .await
    .unwrap();
}

/// Tool MCP qua client thật: người chỉ đọc chỉ thấy tool đọc; không có tool gửi form; đổi
/// trạng thái cần `version` đúng.
#[tokio::test]
async fn tool_mcp() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let created = send(&app, "POST", "/forms", Some("admin"), &[], Some(definition())).await;
        let form_id = created.json()["data"]["id"].clone();
        let token = form_token(&app).await;
        let submit =
            json!({"data": {"name": "Ada", "email": "ada@example.com"}, "form_token": token});
        let sent =
            send(&app, "POST", "/forms/public/contact/submissions", None, &[], Some(submit))
                .await;
        let submission = sent.json()["data"]["id"].clone();

        let client = async |who: &str| app.mcp(who).await.unwrap();
        let reader = client("reader").await;
        let mut names: Vec<String> = reader
            .list_all_tools()
            .await
            .unwrap()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["forms.get", "forms.get_submission", "forms.list", "forms.list_submissions", "kernel.current_tenant", "kernel.navigation"]
        );
        reader.cancel().await.unwrap();

        let admin = client("admin").await;
        let tools = admin.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), 9, "with `kernel.current_tenant` and `kernel.navigation`");
        for tool in &tools {
            assert!(
                !serde_json::to_string(&tool.input_schema).unwrap().contains("$ref"),
                "{} has a $ref",
                tool.name
            );
        }
        let call = async |name: &str, arguments: Value| {
            let result = admin.call_tool(tool(name, arguments)).await.unwrap();
            (result.is_error == Some(true), result.structured_content.unwrap_or(Value::Null))
        };
        let (failed, listed) = call("forms.list", json!({"status": "open"})).await;
        assert!(!failed, "{listed}");
        assert_eq!(listed["data"][0]["key"], "contact");
        let (_, form) = call("forms.get", json!({"id": form_id})).await;
        assert_eq!(form["data"]["fields"].as_array().unwrap().len(), 4);
        let (_, inbox) =
            call("forms.list_submissions", json!({"form_id": form_id, "q": "ada"})).await;
        assert_eq!(inbox["data"][0]["id"], submission);
        let (_, one) = call("forms.get_submission", json!({"id": submission})).await;
        let version = one["data"]["version"].as_str().unwrap().to_owned();
        assert_eq!(one["data"]["data"]["name"], "Ada");

        let (missing, _) =
            call("forms.set_submission_status", json!({"id": submission, "status": "archived"}))
                .await;
        assert!(missing, "`version` is required");
        let (bad, _) = call(
            "forms.set_submission_status",
            json!({"id": submission, "status": "gone", "version": version}),
        )
        .await;
        assert!(bad, "unknown status");
        let (failed, archived) = call(
            "forms.set_submission_status",
            json!({"id": submission, "status": "archived", "version": version}),
        )
        .await;
        assert!(!failed, "{archived}");
        assert_eq!(archived["data"]["status"], "archived");
        let (stale, _) = call(
            "forms.set_submission_status",
            json!({"id": submission, "status": "read", "version": version}),
        )
        .await;
        assert!(stale, "an old version is refused");

        // Tạo form qua MCP: hình dạng trường sai báo theo đường dẫn, cùng lỗi trạng thái.
        let (failed, refused) = call(
            "forms.create",
            json!({"key": "survey", "name": "Survey", "status": "draft",
                   "fields": [{"key": "q", "type": "text", "label": {"en": "Q"}}, {"key": "x", "type": "slider"}]}),
        )
        .await;
        assert!(failed, "{refused}");
        let (failed, created) = call(
            "forms.create",
            json!({"key": "survey", "name": "Survey",
                   "fields": [{"key": "q", "type": "text", "label": {"en": "Question", "vi": "Câu hỏi"}, "required": true}],
                   "success_message": {"en": "Thanks"}, "retention_days": 30}),
        )
        .await;
        assert!(!failed, "{created}");
        let survey = created["data"]["id"].as_str().unwrap().to_owned();
        let etag = created["data"]["etag"].as_str().unwrap().to_owned();
        assert_eq!(created["data"]["version"], 1);
        assert_eq!(created["data"]["status"], "open");
        assert_eq!(created["data"]["retention_days"], 30);
        let (failed, taken) = call(
            "forms.create",
            json!({"key": "survey", "name": "Again", "fields": [{"key": "q", "type": "text", "label": {"en": "Q"}}]}),
        )
        .await;
        assert!(failed, "{taken}");

        // Sửa: cần `etag`; đóng form, xoá thời hạn giữ bằng `null`; bản cũ bị từ chối.
        let (missing, _) = call("forms.update", json!({"id": survey, "status": "closed"})).await;
        assert!(missing, "`etag` is required");
        let (failed, closed) = call(
            "forms.update",
            json!({"id": survey, "etag": etag, "status": "closed", "retention_days": null,
                   "fields": [{"key": "q", "type": "textarea", "label": {"en": "Question"}}]}),
        )
        .await;
        assert!(!failed, "{closed}");
        assert_eq!(closed["data"]["status"], "closed");
        assert_eq!(closed["data"]["retention_days"], Value::Null);
        assert_eq!(closed["data"]["fields"][0]["type"], "textarea");
        assert_eq!(closed["data"]["version"], 2, "the fields changed");
        assert_ne!(closed["data"]["etag"], etag.as_str());
        let (stale, _) =
            call("forms.update", json!({"id": survey, "etag": etag, "name": "Late"})).await;
        assert!(stale, "an old version is refused");
        let (_, read) = call("forms.get", json!({"id": survey})).await;
        assert_eq!(read["data"]["etag"], closed["data"]["etag"]);
        admin.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

/// Xuất CSV lớn qua HTTP: 201 + `Location`, thân bỏ trống được, `status` sai → 422, trùng → 409,
/// job ghi file, link tải không cần đăng nhập ra đúng file (header như bản đồng bộ), link sửa →
/// 403, không có quyền → 403.
#[tokio::test]
async fn xuat_csv_lon_qua_http() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let created = send(&app, "POST", "/forms", Some("admin"), &[], Some(definition())).await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let token = form_token(&app).await;
        let sent = json!({"data": {"name": "Ada", "email": "ada@example.com", "topic": "sales"}, "form_token": token});
        let accepted = send(&app, "POST", "/forms/public/contact/submissions", None, &[("origin", "https://example.com")], Some(sent)).await;
        assert_eq!(accepted.status(), StatusCode::CREATED, "{}", accepted.json());

        let path = format!("/api/v1/forms/{id}/submissions/exports");
        let invalid = app.post(&path).token("admin").json(json!({"status": "lost"})).send().await.unwrap();
        assert_eq!((invalid.status(), invalid.json()["errors"][0]["field"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("status")));
        let denied = app.post(&path).token("reader").send().await.unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        // Thân bỏ trống: mọi trạng thái trừ `spam`.
        let asked = app.post(&path).token("admin").send().await.unwrap();
        assert_eq!(asked.status(), StatusCode::CREATED, "{}", asked.json());
        let export = asked.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(asked.header(header::LOCATION), format!("/api/v1/forms/exports/{export}"));
        assert_eq!(asked.json()["data"]["state"], "pending");
        let busy = app.post(&path).token("admin").json(json!({})).send().await.unwrap();
        assert_eq!((busy.status(), busy.json()["code"].as_str()), (StatusCode::CONFLICT, Some("forms.export_in_progress")));

        let worker = app.modules().worker(4).unwrap();
        while worker.run_once().await.unwrap() > 0 {}
        let ready = send(&app, "GET", &format!("/forms/exports/{export}"), Some("admin"), &[], None).await;
        assert_eq!(ready.status(), StatusCode::OK, "{}", ready.json());
        assert_eq!((ready.json()["data"]["state"].as_str(), ready.json()["data"]["row_count"].as_i64()), (Some("ready"), Some(1)));
        let url = ready.json()["data"]["download_url"].as_str().unwrap().to_owned();

        // Link tải: không cần đăng nhập, cùng nội dung và header như bản đồng bộ.
        let file = app.get(&url).send().await.unwrap();
        assert_eq!(file.status(), StatusCode::OK);
        let direct = send(&app, "GET", &format!("/forms/{id}/submissions/export"), Some("admin"), &[], None).await;
        assert_eq!(file.body(), direct.body());
        for name in [header::CONTENT_TYPE, header::CACHE_CONTROL, header::CONTENT_DISPOSITION] {
            assert_eq!(file.header(name.clone()), direct.header(name));
        }
        let tampered = url.replace("signature=", "signature=00");
        let refused = app.get(&tampered).send().await.unwrap();
        assert_eq!((refused.status(), refused.json()["code"].as_str()), (StatusCode::FORBIDDEN, Some("forms.export_link_invalid")));
        let unsigned = app.get(&format!("/api/v1/forms/exports/{export}/file")).send().await.unwrap();
        assert_eq!(unsigned.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let missing = send(&app, "GET", &format!("/forms/exports/{id}"), Some("admin"), &[], None).await;
        assert_eq!((missing.status(), missing.json()["code"].as_str()), (StatusCode::NOT_FOUND, Some("forms.export_not_found")));
    })
    .await
    .unwrap();
}

/// Đề bài giao diện B6: số bài gửi theo trạng thái trong danh sách form, nhãn lựa chọn theo đúng
/// phiên bản form lúc gửi (kể cả khi form đổi sau đó).
#[tokio::test]
async fn so_bai_va_nhan_lua_chon() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let created = send(&app, "POST", "/forms", Some("admin"), &[], Some(definition())).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        assert!(created.json()["data"].get("submissions").is_none());

        let listed = send(&app, "GET", "/forms", Some("reader"), &[], None).await;
        assert_eq!(
            listed.json()["data"][0]["submissions"],
            json!({"new": 0, "read": 0, "archived": 0, "spam": 0})
        );

        let signed = json!({"data": {"name": "Web", "email": "web@example.com", "topic": "sales"}});
        let sent = send(
            &app,
            "POST",
            "/forms/public/contact/submissions",
            Some("website"),
            &[],
            Some(signed),
        )
        .await;
        assert_eq!(sent.status(), StatusCode::CREATED, "{}", sent.json());
        let submission = sent.json()["data"]["id"].as_str().unwrap().to_owned();

        // Đổi nhãn lựa chọn: bài cũ vẫn mang nhãn của phiên bản lúc gửi.
        let read = send(&app, "GET", &format!("/forms/{id}"), Some("admin"), &[], None).await;
        assert_eq!(read.json()["data"]["submissions"]["new"], 1);
        let etag = read.header(header::ETAG).to_owned();
        let mut fields = definition()["fields"].clone();
        fields[3]["options"] = json!([{"value": "support", "label": {"en": "Support"}}]);
        let changed = send(
            &app,
            "PATCH",
            &format!("/forms/{id}"),
            Some("admin"),
            &[("if-match", &etag)],
            Some(json!({"fields": fields})),
        )
        .await;
        assert_eq!(changed.status(), StatusCode::OK, "{}", changed.json());

        let one = send(
            &app,
            "GET",
            &format!("/forms/submissions/{submission}"),
            Some("reader"),
            &[],
            None,
        )
        .await;
        assert_eq!(one.status(), StatusCode::OK, "{}", one.json());
        let answers = one.json()["data"]["answers"].clone();
        let topic = answers
            .as_array()
            .unwrap()
            .iter()
            .find(|answer| answer["key"] == "topic")
            .unwrap()
            .clone();
        assert_eq!(
            topic["choices"],
            json!([{"value": "sales", "label": {"en": "Sales", "vi": "Bán hàng"}}])
        );
        let name = answers
            .as_array()
            .unwrap()
            .iter()
            .find(|answer| answer["key"] == "name")
            .unwrap()
            .clone();
        assert!(name.get("choices").is_none(), "{name}");
    })
    .await
    .unwrap();
}

/// Hai khách (một dùng chung database, một database riêng) cùng `key` form: endpoint công khai
/// trả form của đúng khách theo header `autoxide-tenant`; người của khách B không đọc được form
/// hay bài gửi của khách A qua bất kỳ `GET` hay tool MCP chỉ đọc nào.
#[tokio::test]
async fn hai_khach_cung_key_form_khong_ro_ri() {
    TestDatabase::run(async |test| {
        let mailer: Arc<dyn Mailer> = Arc::new(FakeMailer::default());
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let all = ["forms.read", "forms.manage", "forms.review", "forms.export"];
        let app = TestApp::builder(test)
            .tenant("a")
            .tenant_dedicated("b")
            .user_in("a", "admin-a", 1, &all)
            .user_in("a", "site-a", 2, &["forms.submit"])
            .user_in("b", "admin-b", 3, &all)
            .provide::<dyn Mailer>(mailer)
            .provide::<dyn Storage>(storage)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let marker = "marker-form-a";
        let mut forms = Vec::new();
        for (who, name) in [("admin-a", marker), ("admin-b", "Form B")] {
            let body = json!({"key": "contact", "name": name, "fields": [
                {"key": "message", "type": "textarea", "label": {"en": "Message"}, "required": true}
            ]});
            let answer = app.post("/api/v1/forms").token(who).json(body).send().await.unwrap();
            assert_eq!(answer.status(), StatusCode::CREATED, "{}", answer.json());
            forms.push(answer.json()["data"]["id"].as_str().unwrap().to_owned());
        }
        for (slug, name) in [("a", marker), ("b", "Form B")] {
            let answer = app
                .get("/api/v1/forms/public/contact")
                .header("autoxide-tenant", slug)
                .send()
                .await
                .unwrap();
            assert_eq!(answer.status(), StatusCode::OK, "{}", answer.json());
            assert_eq!(answer.json()["data"]["name"], name);
        }
        let body = json!({"data": {"message": marker}});
        let answer = app
            .post("/api/v1/forms/public/contact/submissions")
            .token("site-a")
            .json(body)
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::CREATED, "{}", answer.json());
        let submission = answer.json()["data"]["id"].as_str().unwrap().to_owned();
        let page = app
            .get(&format!("/api/v1/forms/{}/submissions", forms[0]))
            .token("admin-a")
            .send()
            .await
            .unwrap();
        assert_eq!(page.json()["data"][0]["id"], submission.as_str());

        let seeded = [
            ("id", forms[0].clone()),
            ("form_id", forms[0].clone()),
            ("id", submission.clone()),
            ("submission_id", submission),
        ];
        let violations =
            autoxide_testkit::leak_check(&app, "admin-b", marker, &seeded).await.unwrap();
        assert_eq!(violations, Vec::<String>::new());
    })
    .await
    .unwrap();
}

/// Cấu hình theo khách (đối chiếu thiết kế 2, A-b7): `forms.retention_days` của khách thay
/// `AUTOXIDE_FORMS_RETENTION_DAYS` (mặc định 0: giữ mãi) cho riêng khách đó; `retention_days` của
/// form vẫn thắng; giá trị sai bị từ chối lúc đặt.
#[tokio::test]
async fn cau_hinh_module_theo_khach_ghi_de_bien_moi_truong() {
    TestDatabase::run(async |test| {
        let mailer: Arc<dyn Mailer> = Arc::new(FakeMailer::default());
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let all = ["forms.read", "forms.manage", "forms.review", "forms.export"];
        let mut admin = all.to_vec();
        admin.push("kernel.settings.manage");
        let app = TestApp::builder(test)
            .tenant("a")
            .tenant("b")
            .user_in("a", "admin-a", 1, &admin)
            .user_in("a", "site-a", 2, &["forms.submit"])
            .user_in("b", "admin-b", 3, &admin)
            .user_in("b", "site-b", 4, &["forms.submit"])
            .provide::<dyn Mailer>(mailer)
            .provide::<dyn Storage>(storage)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        // Khách B giữ bài 1 ngày; khách A không đặt (biến môi trường: giữ mãi).
        let path = "/api/v1/kernel/settings/forms.retention_days";
        let refused =
            app.put(path).token("admin-b").json(json!({"value": "forever"})).send().await.unwrap();
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", refused.json());
        assert_eq!(refused.json()["errors"][0]["field"], "value");
        let set = app.put(path).token("admin-b").json(json!({"value": "1"})).send().await.unwrap();
        assert_eq!(set.status(), StatusCode::OK, "{}", set.json());

        // Mỗi khách: form `contact` (theo khách) và form `kept` tự đặt giữ mãi.
        let mut submissions = Vec::new();
        for (admin, site) in [("admin-a", "site-a"), ("admin-b", "site-b")] {
            for (key, retention) in [("contact", Value::Null), ("kept", json!(0))] {
                let body = json!({"key": key, "name": key, "retention_days": retention, "fields": [
                    {"key": "message", "type": "textarea", "label": {"en": "Message"}, "required": true}
                ]});
                let created = app.post("/api/v1/forms").token(admin).json(body).send().await;
                let created = created.unwrap();
                assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
                let sent = app
                    .post(&format!("/api/v1/forms/public/{key}/submissions"))
                    .token(site)
                    .json(json!({"data": {"message": "hello"}}))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(sent.status(), StatusCode::CREATED, "{}", sent.json());
                submissions.push((admin, sent.json()["data"]["id"].as_str().unwrap().to_owned()));
            }
        }

        let settings = app.api::<dyn autoxide_kernel::tenancy::TenantSettings>().unwrap();
        let antispam = autoxide_kernel::antispam::AntispamConfig::default();
        let config = crate::config::FormsConfig::default();
        assert_eq!(config.retention_days, 0);
        let (service, _, _, _) =
            crate::testing::service_with_settings(test, config, antispam, settings).await;
        let later = chrono::Utc::now() + chrono::Duration::days(2);
        let cleaned = service.cleanup(app.registry(), app.database(), later).await.unwrap();
        assert_eq!(cleaned.expired, 1);
        // Chỉ bài của form `contact` ở khách B hết hạn.
        for (index, (admin, id)) in submissions.iter().enumerate() {
            let found = app
                .get(&format!("/api/v1/forms/submissions/{id}"))
                .token(admin)
                .send()
                .await
                .unwrap()
                .status();
            let expected = if index == 2 { StatusCode::NOT_FOUND } else { StatusCode::OK };
            assert_eq!(found, expected, "submission {index}");
        }
    })
    .await
    .unwrap();
}

/// Đề bài giao diện W-059: xuất CSV đúng các bài đã chọn (`ids`), nhân bản form (đóng, không chép
/// bài, key tự đặt không trùng).
#[tokio::test]
async fn xuat_bai_da_chon_va_nhan_ban_form() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let created = send(&app, "POST", "/forms", Some("admin"), &[], Some(definition())).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let origin = [("origin", "https://example.com")];
        let mut submissions = Vec::new();
        for email in ["a@example.com", "b@example.com"] {
            let token = form_token(&app).await;
            let body = json!({"data": {"name": "Ada", "email": email}, "locale": "en", "form_token": token});
            let sent = send(&app, "POST", "/forms/public/contact/submissions", None, &origin, Some(body)).await;
            assert_eq!(sent.status(), StatusCode::CREATED, "{}", sent.json());
            submissions.push(sent.json()["data"]["id"].as_str().unwrap().to_owned());
        }

        let picked = format!("/forms/{id}/submissions/export?ids={}", submissions[1]);
        let csv = send(&app, "GET", &picked, Some("admin"), &[], None).await;
        assert_eq!(csv.status(), StatusCode::OK);
        let text = csv.text();
        assert!(text.contains("b@example.com") && !text.contains("a@example.com"), "{text}");
        let all = send(&app, "GET", &format!("/forms/{id}/submissions/export"), Some("admin"), &[], None).await;
        assert!(all.text().contains("a@example.com") && all.text().contains("b@example.com"));
        let bad = send(&app, "GET", &format!("/forms/{id}/submissions/export?ids=nope"), Some("admin"), &[], None).await;
        assert_eq!((bad.status(), bad.json()["errors"][0]["field"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("ids")));

        let copy = send(&app, "POST", &format!("/forms/{id}/duplicate"), Some("admin"), &[], Some(json!({}))).await;
        assert_eq!(copy.status(), StatusCode::CREATED, "{}", copy.json());
        let data = copy.json()["data"].clone();
        assert_eq!((data["key"].as_str(), data["status"].as_str()), (Some("contact-copy"), Some("closed")));
        assert_eq!(data["name"], "Contact us (copy)");
        assert_eq!(data["fields"].as_array().unwrap().len(), 4);
        let copy_id = data["id"].as_str().unwrap().to_owned();
        let inbox = send(&app, "GET", &format!("/forms/{copy_id}/submissions"), Some("admin"), &[], None).await;
        assert_eq!(inbox.json()["data"], json!([]), "không chép bài gửi");
        let again = send(&app, "POST", &format!("/forms/{id}/duplicate"), Some("admin"), &[], Some(json!({}))).await;
        assert_eq!(again.json()["data"]["key"], "contact-copy-2");
        let named = json!({"key": "contact-copy", "name": "Khác"});
        let taken = send(&app, "POST", &format!("/forms/{id}/duplicate"), Some("admin"), &[], Some(named)).await;
        assert_eq!((taken.status(), taken.json()["code"].as_str()), (StatusCode::CONFLICT, Some("forms.key_taken")));
        let denied = send(&app, "POST", &format!("/forms/{id}/duplicate"), Some("reader"), &[], Some(json!({}))).await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}
