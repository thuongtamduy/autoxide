//! Restore, dry-run, gán nhãn hàng loạt, trường công khai, tra "đang dùng ở đâu".

use super::*;

/// Hoàn tác an toàn (bước 52): bài đã lưu trữ về lại nháp nguyên nội dung; bài chưa lưu trữ thì
/// 409; chạy thử thao tác hàng loạt không ghi gì, không phát event.
#[tokio::test]
async fn restore_tra_lai_trang_thai_truoc() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let created = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "post", "title": "Lưu rồi lấy lại", "excerpt": "Tóm tắt"})),
        )
        .await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let etag_of = async |id: &str| {
            send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None)
                .await
                .header(header::ETAG)
                .to_owned()
        };
        let etag = etag_of(&id).await;
        let early = send(
            &app,
            "POST",
            &format!("/entries/{id}/unarchive"),
            Some("admin"),
            &[("if-match", etag.as_str())],
            None,
        )
        .await;
        assert_eq!(
            (early.status(), early.json()["code"].as_str()),
            (StatusCode::CONFLICT, Some("content.not_archived"))
        );
        let archived = send(
            &app,
            "POST",
            &format!("/entries/{id}/archive"),
            Some("admin"),
            &[("if-match", etag.as_str())],
            None,
        )
        .await;
        assert_eq!(archived.json()["data"]["status"], "archived", "{}", archived.json());
        let etag = etag_of(&id).await;
        let denied = send(
            &app,
            "POST",
            &format!("/entries/{id}/unarchive"),
            Some("reader"),
            &[("if-match", etag.as_str())],
            None,
        )
        .await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let restored = send(
            &app,
            "POST",
            &format!("/entries/{id}/unarchive"),
            Some("admin"),
            &[("if-match", etag.as_str())],
            None,
        )
        .await;
        assert_eq!(restored.status(), StatusCode::OK, "{}", restored.json());
        let data = restored.json()["data"].clone();
        assert_eq!(
            (data["status"].as_str(), data["title"].as_str(), data["excerpt"].as_str()),
            (Some("draft"), Some("Lưu rồi lấy lại"), Some("Tóm tắt"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dry_run_khong_ghi_khong_phat_event() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        for title in ["Một", "Hai"] {
            send(
                &app,
                "POST",
                "/entries",
                Some("admin"),
                &[],
                Some(json!({"kind": "post", "title": title})),
            )
            .await;
        }
        let before = outbox_count(&app).await;
        let preview = app
            .request(Method::POST, "/api/v1/kernel/bulk/content.list_content_entries/archive")
            .token("admin")
            .json(json!({"filter": {"kind": "post"}, "dry_run": true}))
            .send()
            .await
            .unwrap();
        assert_eq!(preview.status(), StatusCode::OK, "{}", preview.json());
        assert_eq!(preview.json()["data"]["count"], 2);
        assert_eq!(outbox_count(&app).await, before, "không phát event");
        let list = send(&app, "GET", "/entries?status=archived", Some("admin"), &[], None).await;
        assert_eq!(list.json()["data"], json!([]), "không ghi gì");
    })
    .await
    .unwrap();
}

/// Gắn nhãn hàng loạt (bước 51 P20): một nhãn lên nhiều bài một lần, bài đã có hay không có thì
/// bỏ qua; gỡ hàng loạt; cần quyền ghi bài.
#[tokio::test]
async fn gan_nhan_hang_loat() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let mut ids = Vec::new();
        for title in ["Một", "Hai"] {
            let created = send(
                &app,
                "POST",
                "/entries",
                Some("admin"),
                &[],
                Some(json!({"kind": "post", "title": title})),
            )
            .await;
            ids.push(created.json()["data"]["id"].as_str().unwrap().to_owned());
        }
        // Nhãn mới tạo bằng cách gắn lên một bài thứ ba (người viết tạo được nhãn khi khách không hạn chế).
        let third = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "post", "title": "Ba"})),
        )
        .await;
        let third = third.json()["data"]["id"].as_str().unwrap().to_owned();
        let tagged = app
            .put(&format!("/api/v1/kernel/records/content.entry/{third}/tags"))
            .token("admin")
            .json(json!({"new_tags": ["Mùa hè"]}))
            .send()
            .await
            .unwrap();
        assert_eq!(tagged.status(), StatusCode::OK, "{}", tagged.json());
        let tag = tagged.json()["data"][0]["id"].as_str().unwrap().to_owned();
        let apply = |who: &'static str, body: Value| {
            let app = &app;
            let path = format!("/api/v1/kernel/tags/{tag}/apply");
            async move { app.post(&path).token(who).json(body).send().await.unwrap() }
        };
        let missing = Uuid::now_v7().to_string();
        let added =
            apply("admin", json!({"kind": "content.entry", "add": [ids[0], ids[1], missing]}))
                .await;
        assert_eq!(added.status(), StatusCode::OK, "{}", added.json());
        assert_eq!(added.json()["data"]["added"], json!([ids[0], ids[1]]));
        assert_eq!(added.json()["data"]["skipped"], json!([missing]));
        let again =
            apply("admin", json!({"kind": "content.entry", "add": [ids[0]], "remove": [ids[1]]}))
                .await;
        assert_eq!(again.json()["data"]["skipped"], json!([ids[0]]), "đã có thì bỏ qua");
        assert_eq!(again.json()["data"]["removed"], json!([ids[1]]));
        let tagged =
            send(&app, "GET", &format!("/entries?tags_any={tag}"), Some("admin"), &[], None).await;
        let tagged = tagged.json();
        let found: Vec<&str> =
            tagged["data"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap()).collect();
        assert_eq!(found.len(), 2);
        assert!(found.contains(&ids[0].as_str()) && !found.contains(&ids[1].as_str()));
        assert_eq!(
            apply("reader", json!({"kind": "content.entry", "add": [ids[1]]})).await.status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            apply("admin", json!({"kind": "content.entry"})).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    })
    .await
    .unwrap();
}

/// Trường tự định nghĩa (bước 51 P19, phần còn lại): trường mức `public` vào bản công khai lúc đăng,
/// trường nội bộ không lộ; file xuất có cột `cf.<khoá>`; cột `cf.` lạ là 422.
#[tokio::test]
async fn truong_cong_khai_va_cot_xuat() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let storage: Arc<dyn autoxide_kernel::storage::Storage> =
            Arc::new(autoxide_testkit::MemoryStorage::new());
        let app = TestApp::builder(test)
            .user(
                "admin",
                1,
                &["content.read", "content.write", "content.publish", "kernel.custom_fields.manage"],
            )
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .provide::<dyn autoxide_kernel::storage::Storage>(storage)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let define = |body: Value| app.post("/api/v1/kernel/custom-fields").token("admin").json(body).send();
        let teaser = define(json!({"target": "content.entry", "key": "teaser", "data_type": "text", "labels": {"en": "Teaser"}, "visibility": "public"})).await.unwrap();
        assert_eq!(teaser.status(), StatusCode::CREATED, "{}", teaser.json());
        let owner = define(json!({"target": "content.entry", "key": "owner", "data_type": "text", "labels": {"en": "Owner"}})).await.unwrap();
        assert_eq!(owner.status(), StatusCode::CREATED);
        let created = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "post", "title": "Truong cong khai", "custom_fields": {"teaser": "Đọc ngay", "owner": "Ann"}})),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let etag = send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await.header(header::ETAG).to_owned();
        let published = send(&app, "POST", &format!("/entries/{id}/publish"), Some("admin"), &[("if-match", &etag)], None).await;
        assert_eq!(published.status(), StatusCode::OK, "{}", published.json());
        let found = send(&app, "GET", "/public/resolve?locale=en&path=/blog/truong-cong-khai", None, &[], None).await;
        assert_eq!(found.status(), StatusCode::OK, "{}", found.json());
        assert_eq!(found.json()["data"]["entry"]["custom_fields"], json!({"teaser": "Đọc ngay"}), "trường nội bộ không lộ");

        let export = |body: Value| app.request(Method::POST, "/api/v1/kernel/exports").token("admin").json(body).send();
        let resource = "content.list_content_entries";
        let bad = export(json!({"resource": resource, "columns": ["title", "cf.nothing"]})).await.unwrap();
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let queued = export(json!({"resource": resource, "columns": ["title", "cf.owner", "cf.teaser"]})).await.unwrap();
        assert_eq!(queued.status(), StatusCode::CREATED, "{}", queued.json());
        let job_id = queued.json()["data"]["id"].as_str().unwrap().to_owned();
        let worker = app.modules().worker(2).unwrap();
        while worker.run_once().await.unwrap() > 0 {}
        let job = app.get(&format!("/api/v1/kernel/jobs/{job_id}")).token("admin").send().await.unwrap().json();
        assert_eq!(job["data"]["status"], "succeeded", "{job}");
        let url = job["data"]["file"]["url"].as_str().unwrap().to_owned();
        let csv = app.get(&url).token("admin").send().await.unwrap().text();
        assert_eq!(
            csv,
            "\u{feff}\"title\",\"cf.owner\",\"cf.teaser\"\r\n\"Truong cong khai\",\"Ann\",\"Đọc ngay\"\r\n"
        );
    })
    .await
    .unwrap();
}

/// "Đang dùng ở đâu" (bước 51, D-267): bài nhúng bài khác thì bài kia biết mình được dùng ở đâu;
/// người không đọc được bản ghi được hỏi thì 404.
#[tokio::test]
async fn dang_dung_o_dau() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let target = send(&app, "POST", "/entries", Some("admin"), &[], Some(json!({"kind": "post", "title": "Được nhúng"}))).await;
        let target = target.json()["data"]["id"].as_str().unwrap().to_owned();
        let user = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "post", "title": "Bài nhúng", "body": embed_body(&[("content.entry", target.as_str())])})),
        )
        .await;
        assert_eq!(user.status(), StatusCode::CREATED, "{}", user.json());
        let user = user.json()["data"]["id"].as_str().unwrap().to_owned();
        let usages = app
            .get(&format!("/api/v1/kernel/records/content.entry/{target}/usages"))
            .token("admin")
            .send()
            .await
            .unwrap();
        assert_eq!(usages.status(), StatusCode::OK, "{}", usages.json());
        assert_eq!(
            usages.json()["data"],
            json!([{"kind": "content.entry", "id": user, "title": "Bài nhúng", "field": "body"}])
        );
        let none = app
            .get(&format!("/api/v1/kernel/records/content.entry/{user}/usages"))
            .token("admin")
            .send()
            .await
            .unwrap();
        assert_eq!(none.json()["data"], json!([]), "không ai dùng bài nhúng");
        let missing = app
            .get(&format!("/api/v1/kernel/records/content.entry/{}/usages", Uuid::now_v7()))
            .token("admin")
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}
