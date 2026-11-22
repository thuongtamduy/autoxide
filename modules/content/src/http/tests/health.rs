//! Kiểm tra sức khoẻ website (bước 67): tìm thiếu mô tả SEO, tiêu đề trùng, ảnh thiếu `alt`, link nội bộ
//! gãy, bản ghi nhúng không còn công khai, đếm ngược đã qua; vào hộp việc; tối đa một lần mỗi giờ; quyền
//! theo nút. Gate chạy trên cả SQLite và Postgres.

use super::*;

async fn publish(app: &TestApp, body: Value, layout: Option<Value>) -> String {
    let created = send(app, "POST", "/entries", Some("admin"), &[], Some(body)).await;
    assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
    let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
    let mut etag = send(app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None)
        .await
        .header(header::ETAG)
        .to_owned();
    if let Some(layout) = layout {
        let saved = send(
            app,
            "PUT",
            &format!("/entries/{id}/layout"),
            Some("admin"),
            &[("if-match", &etag)],
            Some(json!({"layout": layout})),
        )
        .await;
        assert_eq!(saved.status(), StatusCode::OK, "{}", saved.json());
        etag = saved.header(header::ETAG).to_owned();
    }
    let done = send(
        app,
        "POST",
        &format!("/entries/{id}/publish"),
        Some("admin"),
        &[("if-match", &etag)],
        None,
    )
    .await;
    assert_eq!(done.status(), StatusCode::OK, "{}", done.json());
    id
}

#[tokio::test]
async fn suc_khoe_tim_link_gay_anh_thieu_alt_va_dem_nguoc_da_qua() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let root = autoxide_kernel::tenancy::root_of(app.default_tenant().id()).as_uuid();
        // Bài tốt: có mô tả, link tới trang có thật.
        publish(&app, json!({"kind": "page", "title": "Liên hệ", "meta_description": "Gọi cho chúng tôi"}), None).await;
        let rich = json!({"type": "doc", "content": [{"type": "paragraph", "content": [
            {"type": "text", "text": "xem", "marks": [{"type": "link", "attrs": {"href": "/lien-he"}}]},
            {"type": "text", "text": " cũ", "marks": [{"type": "link", "attrs": {"href": "/khong-con"}}]}
        ]}]});
        let post = publish(&app, json!({"kind": "post", "title": "Tin vui", "body": rich}), None).await;
        let page = publish(&app, json!({"kind": "page", "title": "Tin vui", "meta_description": "x"}), Some(json!({"type": "layout", "schema": 1, "sections": [
            {"id": "c", "block": "content.countdown", "props": {"ends_at": "2020-01-01T00:00:00Z"}},
            {"id": "b", "block": "content.button", "props": {"label": "Mua", "link": "/het-roi?x=1"}}
        ]}))).await;

        let never = send(&app, "GET", &format!("/site-health?org_id={root}"), Some("reader"), &[], None).await;
        assert_eq!((never.status(), never.json()["data"].clone()), (StatusCode::OK, Value::Null));
        assert_eq!(send(&app, "POST", &format!("/site-health/{root}/run"), Some("reader"), &[], None).await.status(), StatusCode::FORBIDDEN);
        let ran = send(&app, "POST", &format!("/site-health/{root}/run"), Some("admin"), &[], None).await;
        assert_eq!(ran.status(), StatusCode::OK, "{}", ran.json());
        let findings = ran.json()["data"]["findings"].as_array().unwrap().clone();
        let has = |kind: &str, entry: &str, detail: &str| {
            findings.iter().any(|item| item["kind"] == kind && item["entry_id"] == entry && item["detail"].as_str().unwrap_or_default().contains(detail))
        };
        assert!(has("seo_description_missing", &post, ""), "{findings:#?}");
        assert!(has("link_broken", &post, "/khong-con"), "{findings:#?}");
        assert!(!has("link_broken", &post, "/lien-he"), "an existing page is not broken");
        assert!(has("link_broken", &page, "/het-roi"), "{findings:#?}");
        assert!(has("countdown_over", &page, "2020"), "{findings:#?}");
        assert!(has("title_duplicate", &post, "Tin vui") && has("title_duplicate", &page, "Tin vui"));
        assert!(findings.iter().all(|item| item["fix"].as_str().unwrap().contains("/api/v1/content/entries/")));

        // Lần kiểm lưu lại, vào hộp việc; chạy lại trong giờ → 429.
        let stored = send(&app, "GET", &format!("/site-health?org_id={root}"), Some("reader"), &[], None).await;
        assert_eq!(stored.json()["data"]["findings"].as_array().unwrap().len(), findings.len());
        let again = send(&app, "POST", &format!("/site-health/{root}/run"), Some("admin"), &[], None).await;
        assert_eq!((again.status(), again.json()["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("content.site_health_too_soon")));
        let inbox = app.get("/api/v1/kernel/work").token("admin").send().await.unwrap();
        assert_eq!(inbox.status(), StatusCode::OK, "{}", inbox.text());
        assert!(inbox.text().contains("content.site_health"), "{}", inbox.text());
    })
    .await
    .unwrap();
}
