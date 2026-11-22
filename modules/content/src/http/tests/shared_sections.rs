//! Khu dùng chung nhiều trang (bước 67): tạo, kiểm bố cục như trang (không lồng khu), đặt vào hai
//! trang, sửa khu là trang công khai đổi theo mà không đăng lại, `id` khối con không trùng, xoá khu
//! đang dùng → 409. Gate chạy trên cả SQLite và Postgres.

use super::*;

fn layout(sections: Value) -> Value {
    json!({"type": "layout", "schema": 1, "sections": sections})
}

async fn publish_page(app: &TestApp, title: &str, sections: Value) -> String {
    let created = send(
        app,
        "POST",
        "/entries",
        Some("admin"),
        &[],
        Some(json!({"kind": "page", "title": title})),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
    let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
    let etag = send(app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None)
        .await
        .header(header::ETAG)
        .to_owned();
    let saved = send(
        app,
        "PUT",
        &format!("/entries/{id}/layout"),
        Some("admin"),
        &[("if-match", &etag)],
        Some(json!({"layout": layout(sections)})),
    )
    .await;
    assert_eq!(saved.status(), StatusCode::OK, "{}", saved.json());
    let etag = saved.header(header::ETAG).to_owned();
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
    id
}

#[tokio::test]
async fn khu_dung_chung_doi_thi_moi_trang_doi_va_xoa_dang_dung_409() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        // Bố cục sai và khu lồng khu: mọi lỗi một lần, trường bắt đầu bằng `layout`.
        let bad = json!({"name": " ", "layout": layout(json!([
            {"id": "x", "block": "content.heading", "props": {}},
            {"id": "y", "block": "content.shared_section", "props": {"id": Uuid::now_v7()}}
        ]))});
        let refused = send(&app, "POST", "/shared-sections", Some("admin"), &[], Some(bad)).await;
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", refused.json());
        let fields: Vec<String> = refused.json()["errors"].as_array().unwrap().iter().map(|e| format!("{}:{}", e["field"].as_str().unwrap(), e["code"].as_str().unwrap())).collect();
        for wanted in ["name:length", "layout.sections[0].props.text:required", "layout:nested"] {
            assert!(fields.iter().any(|f| f == wanted), "{wanted} in {fields:?}");
        }
        assert!(send(&app, "POST", "/shared-sections", Some("reader"), &[], Some(json!({"name": "x", "layout": layout(json!([]))}))).await.status() == StatusCode::FORBIDDEN);

        let body = json!({"name": "Dải khuyến mại", "layout": layout(json!([
            {"id": "t", "block": "content.heading", "props": {"text": "Giảm giá cuối tuần"}}
        ]))});
        let created = send(&app, "POST", "/shared-sections", Some("admin"), &[], Some(body)).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let section = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let listed = send(&app, "GET", "/shared-sections", Some("reader"), &[], None).await;
        assert_eq!(listed.json()["data"][0]["id"], section.as_str());

        // Khu không có: trang bị từ chối như bản ghi nhúng không có.
        let created_page = send(&app, "POST", "/entries", Some("admin"), &[], Some(json!({"kind": "page", "title": "Sai"}))).await;
        let wrong_id = created_page.json()["data"]["id"].as_str().unwrap().to_owned();
        let etag = send(&app, "GET", &format!("/entries/{wrong_id}"), Some("admin"), &[], None).await.header(header::ETAG).to_owned();
        let missing = layout(json!([{"id": "s", "block": "content.shared_section", "props": {"id": Uuid::now_v7()}}]));
        let refused = send(&app, "PUT", &format!("/entries/{wrong_id}/layout"), Some("admin"), &[("if-match", &etag)], Some(json!({"layout": missing}))).await;
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(refused.json()["errors"][0]["code"], "shared_section_not_found", "{}", refused.json());

        // Hai trang, một trang đặt khu hai lần.
        let place = |id: &str| json!({"id": id, "block": "content.shared_section", "props": {"id": section}});
        publish_page(&app, "Trang A", json!([place("top"), place("bottom")])).await;
        publish_page(&app, "Trang B", json!([place("promo")])).await;
        let read = send(&app, "GET", "/public/resolve?locale=en&path=/trang-a", None, &[], None).await;
        assert_eq!(read.status(), StatusCode::OK, "{}", read.json());
        let sections = read.json()["data"]["entry"]["body"]["sections"].clone();
        assert_eq!(sections[0]["children"][0]["id"], "top.t", "{sections}");
        assert_eq!(sections[1]["children"][0]["id"], "bottom.t");
        assert_eq!(sections[0]["children"][0]["props"]["text"], "Giảm giá cuối tuần");

        // Sửa khu: trang đã đăng đổi theo, không đăng lại.
        let etag = send(&app, "GET", &format!("/shared-sections/{section}"), Some("admin"), &[], None).await.header(header::ETAG).to_owned();
        let changed = json!({"name": "Dải khuyến mại", "layout": layout(json!([
            {"id": "t", "block": "content.heading", "props": {"text": "Tết sale"}}
        ]))});
        let put = send(&app, "PUT", &format!("/shared-sections/{section}"), Some("admin"), &[("if-match", &etag)], Some(changed.clone())).await;
        assert_eq!(put.status(), StatusCode::OK, "{}", put.json());
        let stale = send(&app, "PUT", &format!("/shared-sections/{section}"), Some("admin"), &[("if-match", &etag)], Some(changed)).await;
        assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);
        let read = send(&app, "GET", "/public/resolve?locale=en&path=/trang-b", None, &[], None).await;
        assert_eq!(read.json()["data"]["entry"]["body"]["sections"][0]["children"][0]["props"]["text"], "Tết sale");

        // Đang dùng ở hai trang → 409 kèm số trang.
        let refused = send(&app, "DELETE", &format!("/shared-sections/{section}"), Some("admin"), &[("if-match", "*")], None).await;
        assert_eq!(refused.status(), StatusCode::CONFLICT, "{}", refused.json());
        assert_eq!(refused.json()["code"], "content.shared_section_in_use");
        assert!(refused.json()["detail"].as_str().unwrap().contains('2'), "{}", refused.json());

        // Khu không ai dùng xoá được.
        let spare = send(&app, "POST", "/shared-sections", Some("admin"), &[], Some(json!({"name": "Thừa", "layout": layout(json!([]))}))).await;
        let spare = spare.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(send(&app, "DELETE", &format!("/shared-sections/{spare}"), Some("admin"), &[("if-match", "*")], None).await.status(), StatusCode::NO_CONTENT);
        assert_eq!(send(&app, "GET", &format!("/shared-sections/{spare}"), Some("admin"), &[], None).await.status(), StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}
