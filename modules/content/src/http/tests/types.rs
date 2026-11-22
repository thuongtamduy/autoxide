//! Loại nội dung tự định nghĩa (bước 67, D-235): tạo loại và mục có trường riêng (trường tự định nghĩa
//! của kernel, đích `content.type.<key>`), đường dẫn theo `path_prefix`, trùng tiền tố 409, trường
//! `internal` không lộ công khai, danh sách công khai, loại không có trang không mở được bằng đường dẫn,
//! xoá loại còn mục 409, đổi khoá khi có mục 409, tắt loại thì mục hết công khai. Gate chạy trên cả
//! SQLite và Postgres.

use super::*;

async fn types_app(test: &TestDatabase) -> TestApp {
    let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
    let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
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

fn labels(one: &str, many: &str) -> Value {
    json!({"one": {"vi": one}, "many": {"vi": many}})
}

async fn field(app: &TestApp, target: &str, key: &str, visibility: &str) {
    let body = json!({"target": target, "key": key, "data_type": "text", "labels": {"vi": key}, "visibility": visibility});
    let made =
        app.post("/api/v1/kernel/custom-fields").token("admin").json(body).send().await.unwrap();
    assert_eq!(made.status(), StatusCode::CREATED, "{}", made.text());
}

async fn publish_entry(app: &TestApp, body: Value) -> Value {
    let created = send(app, "POST", "/entries", Some("admin"), &[], Some(body)).await;
    assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
    let data = created.json()["data"].clone();
    let id = data["id"].as_str().unwrap().to_owned();
    let etag = send(app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None)
        .await
        .header(header::ETAG)
        .to_owned();
    let published = send(
        app,
        "POST",
        &format!("/entries/{id}/publish"),
        Some("admin"),
        &[("if-match", &etag)],
        None,
    )
    .await;
    assert_eq!(published.status(), StatusCode::OK, "{}", published.json());
    data
}

#[tokio::test]
async fn loai_noi_dung_tu_dinh_nghia() {
    TestDatabase::run(async |test| {
        let app = types_app(test).await;
        let project = json!({"key": "du_an", "labels": labels("Dự án", "Dự án"), "icon": "briefcase", "routable": true, "path_prefix": "du-an"});
        let created = send(&app, "POST", "/types", Some("admin"), &[], Some(project)).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let type_id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(send(&app, "POST", "/types", Some("reader"), &[], Some(json!({"key": "x", "labels": labels("x", "x")}))).await.status(), StatusCode::FORBIDDEN);
        // Mọi lỗi một lần; tiền tố trùng 409.
        let bad = json!({"key": "Bad Key", "labels": {"one": {}}, "icon": "rocket", "routable": true, "body": "html"});
        let refused = send(&app, "POST", "/types", Some("admin"), &[], Some(bad)).await;
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(refused.json()["errors"].as_array().unwrap().len(), 5, "{}", refused.json());
        let same = json!({"key": "cong_trinh", "labels": labels("Công trình", "Công trình"), "routable": true, "path_prefix": "du-an"});
        let taken = send(&app, "POST", "/types", Some("admin"), &[], Some(same)).await;
        assert_eq!((taken.status(), taken.json()["code"].as_str()), (StatusCode::CONFLICT, Some("content.path_prefix_taken")));

        // Trường riêng của loại: một công khai, một nội bộ.
        field(&app, "content.type.du_an", "linh_vuc", "public").await;
        field(&app, "content.type.du_an", "gia_hop_dong", "internal").await;

        let missing_type = send(&app, "POST", "/entries", Some("admin"), &[], Some(json!({"kind": "custom", "title": "X"}))).await;
        assert_eq!(missing_type.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let wrong_field = json!({"kind": "custom", "type_id": type_id, "title": "Y", "custom_fields": {"khong_co": "x"}});
        assert_eq!(send(&app, "POST", "/entries", Some("admin"), &[], Some(wrong_field)).await.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let entry = publish_entry(&app, json!({"kind": "custom", "type_id": type_id, "locale": "en", "title": "Nhà phố Quận 3",
            "custom_fields": {"linh_vuc": "Nhà ở", "gia_hop_dong": "2 tỷ"}})).await;
        assert_eq!((entry["kind"].as_str(), entry["type_id"].as_str()), (Some("custom"), Some(type_id.as_str())));

        // Đường dẫn theo tiền tố; trường nội bộ không lộ.
        let read = send(&app, "GET", "/public/resolve?locale=en&path=/du-an/nha-pho-quan-3", None, &[], None).await;
        assert_eq!(read.status(), StatusCode::OK, "{}", read.json());
        let fields = read.json()["data"]["entry"]["custom_fields"].clone();
        assert_eq!(fields, json!({"linh_vuc": "Nhà ở"}));
        let listed = send(&app, "GET", "/public/types/du_an/entries?locale=en", None, &[], None).await;
        assert_eq!(listed.status(), StatusCode::OK, "{}", listed.json());
        assert_eq!(listed.json()["data"].as_array().unwrap().len(), 1);
        assert_eq!(listed.json()["data"][0]["custom_fields"], json!({"linh_vuc": "Nhà ở"}));
        // Bài viết thường không có trong danh sách của loại; mục của loại không vào danh sách bài.
        let posts = send(&app, "GET", "/public/posts?locale=en", None, &[], None).await;
        assert_eq!(posts.json()["data"], json!([]));
        let admin_list = send(&app, "GET", &format!("/entries?type_id={type_id}"), Some("reader"), &[], None).await;
        assert_eq!(admin_list.json()["data"].as_array().unwrap().len(), 1);

        // Loại không có trang: mục công khai trong danh sách, không mở được bằng đường dẫn.
        let faq = json!({"key": "hoi_dap", "labels": labels("Câu hỏi", "Câu hỏi"), "icon": "question"});
        let faq = send(&app, "POST", "/types", Some("admin"), &[], Some(faq)).await;
        let faq_id = faq.json()["data"]["id"].as_str().unwrap().to_owned();
        publish_entry(&app, json!({"kind": "custom", "type_id": faq_id, "locale": "en", "title": "Bảo hành bao lâu"})).await;
        let listed = send(&app, "GET", "/public/types/hoi_dap/entries?locale=en", None, &[], None).await;
        assert_eq!(listed.json()["data"].as_array().unwrap().len(), 1);
        let no_page = send(&app, "GET", "/public/resolve?locale=en&path=/bao-hanh-bao-lau", None, &[], None).await;
        assert_eq!(no_page.status(), StatusCode::NOT_FOUND);
        let sitemap = send(&app, "GET", "/public/sitemap", None, &[], None).await.json();
        let paths: Vec<&str> = sitemap["data"].as_array().unwrap().iter().filter_map(|row| row["path"].as_str()).collect();
        assert!(paths.iter().all(|path| path.starts_with('/')), "{paths:?}");

        // Còn mục: xoá 409, đổi khoá 409.
        let in_use = send(&app, "DELETE", "/types/du_an", Some("admin"), &[], None).await;
        assert_eq!((in_use.status(), in_use.json()["code"].as_str()), (StatusCode::CONFLICT, Some("content.type_in_use")));
        let locked = send(&app, "PATCH", "/types/du_an", Some("admin"), &[("if-match", "*")], Some(json!({"key": "project"}))).await;
        assert_eq!((locked.status(), locked.json()["code"].as_str()), (StatusCode::CONFLICT, Some("content.type_key_locked")));

        // Tắt loại: mục ngừng công khai, không mất dữ liệu.
        let off = send(&app, "PATCH", "/types/du_an", Some("admin"), &[("if-match", "*")], Some(json!({"active": false}))).await;
        assert_eq!(off.status(), StatusCode::OK, "{}", off.json());
        assert_eq!(send(&app, "GET", "/public/resolve?locale=en&path=/du-an/nha-pho-quan-3", None, &[], None).await.status(), StatusCode::NOT_FOUND);
        assert_eq!(send(&app, "GET", "/public/types/du_an/entries?locale=en", None, &[], None).await.status(), StatusCode::NOT_FOUND);
        assert_eq!(send(&app, "GET", &format!("/entries/{}", entry["id"].as_str().unwrap()), Some("reader"), &[], None).await.status(), StatusCode::OK);

        // Loại rỗng xoá được.
        let spare = send(&app, "POST", "/types", Some("admin"), &[], Some(json!({"key": "thu", "labels": labels("Thử", "Thử")}))).await;
        assert_eq!(spare.status(), StatusCode::CREATED);
        assert_eq!(send(&app, "DELETE", "/types/thu", Some("admin"), &[], None).await.status(), StatusCode::NO_CONTENT);
    })
    .await
    .unwrap();
}
