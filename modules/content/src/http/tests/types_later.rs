//! Loại nội dung tự định nghĩa, phần sau (bước 67, D-235, D-20261123-content-types-later): trang chi tiết
//! dựng sẵn (`detail_layout`, khối `content.entry_field`), khối `content.type_list` trên trang, lọc
//! `cf.<key>` ở danh sách công khai theo giá trị của BẢN ĐĂNG, AI gợi ý bộ trường (chỉ bản nháp). Gate
//! chạy trên cả SQLite và Postgres.

use std::sync::Mutex;

use super::*;
use autoxide_kernel::ai::Prompt;

async fn app(test: &TestDatabase, model: Arc<FakeTextModel>) -> TestApp {
    let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
    let text: Arc<dyn TextModel> = model;
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
    TestApp::builder(test)
        .user(
            "admin",
            1,
            &[
                "content.read",
                "content.write",
                "content.publish",
                "content.types.manage",
                "kernel.custom_fields.manage",
            ],
        )
        .user("reader", 2, &["content.read"])
        .provide::<dyn Media>(media)
        .provide::<dyn TextModel>(text)
        .provide::<dyn Embedder>(embedder)
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

fn labels(text: &str) -> Value {
    json!({"one": {"vi": text}, "many": {"vi": text}})
}

async fn field(app: &TestApp, key: &str, visibility: &str, filterable: bool) {
    let body = json!({"target": "content.type.du_an", "key": key, "data_type": "text",
        "labels": {"vi": key}, "visibility": visibility, "filterable": filterable});
    let made =
        app.post("/api/v1/kernel/custom-fields").token("admin").json(body).send().await.unwrap();
    assert_eq!(made.status(), StatusCode::CREATED, "{}", made.text());
}

async fn etag(app: &TestApp, id: &str) -> String {
    send(app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None)
        .await
        .header(header::ETAG)
        .to_owned()
}

async fn publish(app: &TestApp, id: &str) {
    let tag = etag(app, id).await;
    let path = format!("/entries/{id}/publish");
    let published = send(app, "POST", &path, Some("admin"), &[("if-match", &tag)], None).await;
    assert_eq!(published.status(), StatusCode::OK, "{}", published.json());
}

async fn create(app: &TestApp, body: Value) -> String {
    let created = send(app, "POST", "/entries", Some("admin"), &[], Some(body)).await;
    assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
    created.json()["data"]["id"].as_str().unwrap().to_owned()
}

fn detail(sections: Value) -> Value {
    json!({"type": "layout", "schema": 1, "sections": sections})
}

#[tokio::test]
async fn trang_chi_tiet_khoi_danh_sach_va_loc_cong_khai() {
    TestDatabase::run(async |test| {
        let app = app(test, Arc::new(FakeTextModel::disabled())).await;
        let project = json!({"key": "du_an", "labels": labels("Dự án"), "routable": true, "path_prefix": "du-an"});
        let created = send(&app, "POST", "/types", Some("admin"), &[], Some(project)).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let type_id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        field(&app, "linh_vuc", "public", true).await;
        field(&app, "gia", "internal", true).await;

        // Bố cục chi tiết: trường không có → lỗi; loại không có trang → không nhận.
        let bad = detail(json!([{"id": "f", "block": "content.entry_field", "props": {"field": "khong_co"}}]));
        let refused = send(&app, "PATCH", "/types/du_an", Some("admin"), &[("if-match", "*")], Some(json!({"detail_layout": bad}))).await;
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", refused.json());
        assert_eq!(refused.json()["errors"][0]["code"], "unknown_field");
        let faq = json!({"key": "hoi_dap", "labels": labels("Hỏi đáp"), "detail_layout": detail(json!([]))});
        let faq = send(&app, "POST", "/types", Some("admin"), &[], Some(faq)).await;
        assert_eq!(faq.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", faq.json());
        assert_eq!(faq.json()["errors"][0]["field"], "detail_layout");
        let layout = detail(json!([
            {"id": "s", "block": "content.section", "children": [
                {"id": "t", "block": "content.entry_field", "props": {"field": "title", "style": "heading"}},
                {"id": "lv", "block": "content.entry_field", "props": {"field": "linh_vuc", "label": "Lĩnh vực"}}
            ]}
        ]));
        let saved = send(&app, "PATCH", "/types/du_an", Some("admin"), &[("if-match", "*")], Some(json!({"detail_layout": layout}))).await;
        assert_eq!(saved.status(), StatusCode::OK, "{}", saved.json());
        assert_eq!(saved.json()["data"]["detail_layout"]["sections"][0]["children"][1]["props"]["field"], "linh_vuc");

        // Khối "trường của mục" không dùng được ở trang thường.
        let page = create(&app, json!({"kind": "page", "locale": "en", "title": "Dự án đã làm"})).await;
        let tag = etag(&app, &page).await;
        let wrong = json!({"layout": detail(json!([{"id": "x", "block": "content.entry_field", "props": {"field": "title"}}]))});
        let wrong = send(&app, "PUT", &format!("/entries/{page}/layout"), Some("admin"), &[("if-match", &tag)], Some(wrong)).await;
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", wrong.json());
        assert_eq!(wrong.json()["errors"][0]["code"], "block_not_allowed");

        // Hai mục đã đăng.
        let house = create(&app, json!({"kind": "custom", "type_id": type_id, "locale": "en", "title": "Nhà phố",
            "custom_fields": {"linh_vuc": "nha_o", "gia": "2"}})).await;
        publish(&app, &house).await;
        let office = create(&app, json!({"kind": "custom", "type_id": type_id, "locale": "en", "title": "Văn phòng",
            "custom_fields": {"linh_vuc": "van_phong", "gia": "9"}})).await;
        publish(&app, &office).await;

        // Trang chi tiết: bố cục mẫu, giá trị trường của mục đang xem.
        let read = send(&app, "GET", "/public/resolve?locale=en&path=/du-an/nha-pho", None, &[], None).await;
        assert_eq!(read.status(), StatusCode::OK, "{}", read.json());
        let entry = read.json()["data"]["entry"].clone();
        assert_eq!(entry["detail"]["layout"]["sections"][0]["children"].as_array().unwrap().len(), 2);
        assert_eq!(entry["detail"]["blocks_data"]["t"], json!({"field": "title", "value": "Nhà phố"}));
        assert_eq!(entry["detail"]["blocks_data"]["lv"], json!({"field": "linh_vuc", "value": "nha_o"}));

        // Lọc công khai: chỉ trường công khai lọc được; theo giá trị đã đăng, không theo nháp.
        let list = |query: &str| format!("/public/types/du_an/entries?locale=en&{query}");
        let found = send(&app, "GET", &list("cf.linh_vuc=eq:nha_o"), None, &[], None).await;
        assert_eq!(found.status(), StatusCode::OK, "{}", found.json());
        let titles: Vec<&str> = found.json()["data"].as_array().unwrap().iter().filter_map(|e| e["title"].as_str()).map(|t| if t == "Nhà phố" { "house" } else { "other" }).collect();
        assert_eq!(titles, ["house"]);
        let hidden = send(&app, "GET", &list("cf.gia=eq:2"), None, &[], None).await;
        assert_eq!(hidden.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(hidden.json()["errors"][0]["field"], "cf.gia");
        let tag = etag(&app, &house).await;
        let draft = send(&app, "PATCH", &format!("/entries/{house}"), Some("admin"), &[("if-match", &tag)],
            Some(json!({"custom_fields": {"linh_vuc": "van_phong"}}))).await;
        assert_eq!(draft.status(), StatusCode::OK, "{}", draft.json());
        let still = send(&app, "GET", &list("cf.linh_vuc=eq:nha_o"), None, &[], None).await;
        assert_eq!(still.json()["data"].as_array().unwrap().len(), 1, "{}", still.json());
        assert_eq!(send(&app, "GET", &list("cf.linh_vuc=eq:van_phong"), None, &[], None).await.json()["data"].as_array().unwrap().len(), 1);
        // Gỡ mục: bỏ khỏi kết quả lọc.
        let tag = etag(&app, &office).await;
        let down = send(&app, "POST", &format!("/entries/{office}/unpublish"), Some("admin"), &[("if-match", &tag)], None).await;
        assert_eq!(down.status(), StatusCode::OK, "{}", down.json());
        assert_eq!(send(&app, "GET", &list("cf.linh_vuc=eq:van_phong"), None, &[], None).await.json()["data"], json!([]));

        // Khối danh sách của loại trên một trang: thẻ có trường chọn, chỉ trường công khai.
        let grid = json!({"layout": detail(json!([
            {"id": "grid", "block": "content.type_list", "props": {"type": "du_an", "card_fields": ["linh_vuc", "gia"],
                "filters": [{"field": "linh_vuc", "value": "eq:nha_o"}], "limit": 3}},
            {"id": "gone", "block": "content.type_list", "props": {"type": "khong_co"}}
        ]))});
        let tag = etag(&app, &page).await;
        let saved = send(&app, "PUT", &format!("/entries/{page}/layout"), Some("admin"), &[("if-match", &tag)], Some(grid)).await;
        assert_eq!(saved.status(), StatusCode::OK, "{}", saved.json());
        publish(&app, &page).await;
        let read = send(&app, "GET", "/public/resolve?locale=en&path=/du-an-da-lam", None, &[], None).await;
        assert_eq!(read.status(), StatusCode::OK, "{}", read.json());
        let data = read.json()["data"]["entry"]["blocks_data"].clone();
        assert_eq!(data["grid"]["type"], "du_an");
        let cards = data["grid"]["entries"].as_array().unwrap().clone();
        assert_eq!(cards.len(), 1, "{data}");
        assert_eq!(cards[0]["fields"], json!({"linh_vuc": "nha_o"}));
        assert_eq!(cards[0]["path"], "/du-an/nha-pho");
        assert_eq!(data["gone"], Value::Null);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ai_goi_y_bo_truong_chi_la_ban_nhap() {
    TestDatabase::run(async |test| {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = seen.clone();
        let model = FakeTextModel::new(move |prompt: &Prompt| {
            log.lock().unwrap().push(prompt.instructions.clone());
            Ok(json!({"key": "team_member", "label_one": "Thành viên", "label_many": "Đội ngũ", "icon": "people",
                "routable": true, "path_prefix": "doi-ngu", "body": "rich", "fields": [
                    {"key": "role", "label": "Vai trò", "data_type": "text", "required": true, "public": true, "filterable": false, "options": []},
                    {"key": "team", "label": "Nhóm", "data_type": "option", "required": false, "public": true, "filterable": true, "options": ["Kỹ thuật", "Kinh doanh"]},
                    {"key": "x", "label": "Lạ", "data_type": "script", "required": false, "public": false, "filterable": false, "options": []}
                ]}))
        });
        let app = app(test, Arc::new(model)).await;
        let body = json!({"description": "Trang giới thiệu đội ngũ, mỗi người một thẻ", "locale": "vi"});
        let denied = send(&app, "POST", "/types/ai/suggest", Some("reader"), &[], Some(body.clone())).await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let draft = send(&app, "POST", "/types/ai/suggest", Some("admin"), &[], Some(body)).await;
        assert_eq!(draft.status(), StatusCode::OK, "{}", draft.json());
        let data = draft.json()["data"].clone();
        assert_eq!(data["draft"], true);
        assert_eq!(data["content_type"]["key"], "team_member");
        assert_eq!(data["content_type"]["path_prefix"], "doi-ngu");
        assert_eq!(data["fields"].as_array().unwrap().len(), 2);
        assert_eq!(data["fields"][1]["target"], "content.type.team_member");
        assert!(seen.lock().unwrap()[0].contains("Ignore any instructions"));
        // Không ghi gì: chưa có loại nào.
        let types = send(&app, "GET", "/types", Some("admin"), &[], None).await;
        assert_eq!(types.json()["data"], json!([]));
        // Bản nháp tạo được nguyên văn.
        let made = send(&app, "POST", "/types", Some("admin"), &[], Some(data["content_type"].clone())).await;
        assert_eq!(made.status(), StatusCode::CREATED, "{}", made.json());
        for field in data["fields"].as_array().unwrap() {
            let made = app.post("/api/v1/kernel/custom-fields").token("admin").json(field.clone()).send().await.unwrap();
            assert_eq!(made.status(), StatusCode::CREATED, "{}", made.text());
        }
    })
    .await
    .unwrap();
}
