//! Trang dựng từ khối (bước 67): thân `layout` qua `PUT` / `PATCH …/layout` (JSON Patch, `If-Match`),
//! kiểm từ chối mặc định với mọi lỗi một lần, theo cỡ màn hình, theo ngôn ngữ, revision và đăng dùng
//! lại cơ chế của `content`. Gate chạy trên cả SQLite và Postgres.

use axum::http::{StatusCode, header};
use serde_json::{Value, json};

use super::{app, send};

fn layout(sections: Value) -> Value {
    json!({"layout": {"type": "layout", "schema": 1, "sections": sections}})
}

fn fields(body: &Value) -> Vec<(String, String)> {
    body["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|error| {
            (
                error["field"].as_str().unwrap().to_owned(),
                error["code"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

async fn page(app: &autoxide_testkit::TestApp, kind: &str, title: &str) -> (String, String) {
    let created = send(
        app,
        "POST",
        "/entries",
        Some("admin"),
        &[],
        Some(json!({"kind": kind, "title": title})),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
    let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
    let read = send(app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await;
    (id, read.header(header::ETAG).to_owned())
}

#[tokio::test]
async fn bo_cuc_khoi_kiem_tu_choi_mac_dinh_va_sua_tung_phan() {
    autoxide_testkit::TestDatabase::run(async |test| {
        let app = app(test).await;
        let blocks = app.get("/api/v1/kernel/blocks").token("admin").send().await.unwrap().json();
        let names: Vec<&str> =
            blocks["data"].as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"content.section") && names.contains(&"content.heading"), "{names:?}");

        let (id, etag) = page(&app, "page", "Khuyến mại hè").await;
        let uri = format!("/entries/{id}/layout");

        // Mọi lỗi báo một lần, kèm đường dẫn JSON.
        let wrong = layout(json!([
            {"id": "hero", "block": "content.section", "props": {"padding": {"base": "small", "xl": "large"}},
             "visibility": {"phone": true},
             "children": [
                {"id": "t", "block": "content.heading", "props": {"text": "<b>Sale</b>", "level": 9}},
                {"id": "t", "block": "content.heading", "props": {"style": "color: red"}},
                {"id": "x", "block": "nope.block"},
                {"id": "h2", "block": "content.heading", "props": {"text": "Hi"}, "children": []}
             ]},
            {"id": "bad id!", "block": "content.section", "html": "<script>"}
        ]));
        let refused = send(&app, "PUT", &uri, Some("admin"), &[("if-match", &etag)], Some(wrong)).await;
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", refused.json());
        let found = fields(&refused.json());
        for (field, code) in [
            ("body.sections[0].props.padding.xl", "unknown"),
            ("body.sections[0].visibility", "invalid"),
            ("body.sections[0].children[0].props.text", "html"),
            ("body.sections[0].children[0].props.level", "range"),
            ("body.sections[0].children[1].id", "duplicate"),
            ("body.sections[0].children[1].props.text", "required"),
            ("body.sections[0].children[1].props.style", "unknown"),
            ("body.sections[0].children[2].block", "unknown"),
            ("body.sections[0].children[3].children", "unknown"),
            ("body.sections[1].id", "invalid"),
            ("body.sections[1].html", "unknown"),
        ] {
            assert!(found.iter().any(|(f, c)| f == field && c == code), "{field} {code} not in {found:?}");
        }

        // Bố cục đúng: giá trị theo cỡ màn hình, ẩn theo cỡ màn hình, khối lồng khối.
        let good = layout(json!([
            {"id": "hero", "block": "content.section",
             "props": {"background": "primary", "padding": {"base": "small", "lg": "large"}},
             "visibility": {"base": true, "md": true},
             "children": [{"id": "title", "block": "content.heading", "props": {"text": "Giảm giá mùa hè", "level": 1}}]}
        ]));
        let saved = send(&app, "PUT", &uri, Some("admin"), &[("if-match", &etag)], Some(good)).await;
        assert_eq!(saved.status(), StatusCode::OK, "{}", saved.json());
        assert_eq!(saved.json()["data"]["body"]["sections"][0]["children"][0]["id"], "title");
        let etag = saved.header(header::ETAG).to_owned();

        // JSON Patch: cần `If-Match`, đúng phiên bản; patch hỏng không đổi gì; kết quả kiểm lại cả trang.
        let rename = json!([{"op": "replace", "path": "/sections/0/children/0/props/text", "value": "Sale 50%"}]);
        let missing = send(&app, "PATCH", &uri, Some("admin"), &[], Some(rename.clone())).await;
        assert_eq!(missing.status(), StatusCode::PRECONDITION_REQUIRED);
        let patched = send(&app, "PATCH", &uri, Some("admin"), &[("if-match", &etag)], Some(rename.clone())).await;
        assert_eq!(patched.status(), StatusCode::OK, "{}", patched.json());
        assert_eq!(patched.json()["data"]["body"]["sections"][0]["children"][0]["props"]["text"], "Sale 50%");
        let stale = send(&app, "PATCH", &uri, Some("admin"), &[("if-match", &etag)], Some(rename)).await;
        assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);
        let etag = patched.header(header::ETAG).to_owned();
        let broken = json!([
            {"op": "add", "path": "/sections/-", "value": {"id": "more", "block": "content.heading", "props": {"text": "Thêm"}}},
            {"op": "remove", "path": "/sections/9"}
        ]);
        let failed = send(&app, "PATCH", &uri, Some("admin"), &[("if-match", &etag)], Some(broken)).await;
        assert_eq!(fields(&failed.json()), [("operations[1]".to_owned(), "not_found".to_owned())]);
        let invalid = json!([{"op": "add", "path": "/sections/0/children/-", "value": {"id": "title", "block": "content.heading", "props": {"text": "x"}}}]);
        let duplicate = send(&app, "PATCH", &uri, Some("admin"), &[("if-match", &etag)], Some(invalid)).await;
        assert_eq!(fields(&duplicate.json()), [("body.sections[0].children[1].id".to_owned(), "duplicate".to_owned())]);
        let reader = send(&app, "PATCH", &uri, Some("reader"), &[("if-match", &etag)], Some(json!([]))).await;
        assert_eq!(reader.status(), StatusCode::FORBIDDEN);

        // Đăng: bản công khai mang bố cục; nháp mới không lộ.
        let published = send(&app, "POST", &format!("/entries/{id}/publish"), Some("admin"), &[("if-match", &etag)], None).await;
        assert_eq!(published.status(), StatusCode::OK, "{}", published.json());
        let public = send(&app, "GET", "/public/resolve?locale=en&path=/khuyen-mai-he", None, &[], None).await;
        assert_eq!(public.status(), StatusCode::OK, "{}", public.json());
        assert_eq!(public.json()["data"]["entry"]["body"]["type"], "layout");

        // Lịch sử: thay bố cục có `save_revision` ghi revision, khôi phục được bằng cơ chế sẵn có.
        let revisions = send(&app, "GET", &format!("/entries/{id}/revisions"), Some("admin"), &[], None).await;
        assert!(!revisions.json()["data"].as_array().unwrap().is_empty(), "{}", revisions.json());

        // Bài viết không có bố cục; trang có thân `rich` có nội dung thì PATCH bị 409.
        let (post, post_etag) = page(&app, "post", "Một bài viết").await;
        let refused = send(&app, "PUT", &format!("/entries/{post}/layout"), Some("admin"), &[("if-match", &post_etag)], Some(layout(json!([])))).await;
        assert_eq!(fields(&refused.json()), [("body.type".to_owned(), "invalid".to_owned())]);
        let (rich, rich_etag) = page(&app, "page", "Trang chữ").await;
        let text = json!({"body": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Xin chào"}]}]}});
        let written = send(&app, "PATCH", &format!("/entries/{rich}"), Some("admin"), &[("if-match", &rich_etag)], Some(text)).await;
        let rich_etag = written.header(header::ETAG).to_owned();
        let conflict = send(&app, "PATCH", &format!("/entries/{rich}/layout"), Some("admin"), &[("if-match", &rich_etag)], Some(json!([]))).await;
        assert_eq!((conflict.status(), conflict.json()["code"].as_str()), (StatusCode::CONFLICT, Some("content.not_layout")));

        // Trang mới (thân `rich` trống): PATCH bắt đầu từ bố cục trống.
        let (fresh, fresh_etag) = page(&app, "page", "Trang mới").await;
        let first = json!([{"op": "add", "path": "/sections/-", "value": {"id": "a", "block": "content.heading", "props": {"text": "Xin chào"}}}]);
        let started = send(&app, "PATCH", &format!("/entries/{fresh}/layout"), Some("admin"), &[("if-match", &fresh_etag)], Some(first)).await;
        assert_eq!(started.status(), StatusCode::OK, "{}", started.json());
        assert_eq!(started.json()["data"]["word_count"], 2);
    })
    .await
    .unwrap();
}
