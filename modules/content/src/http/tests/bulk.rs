//! View đã lưu, xuất danh sách, lưu trữ hàng loạt, hoàn giao việc, ghi chú trên bản ghi.

use super::*;

/// Schema bộ lọc và view đã lưu (bước 51 P7): danh sách bài có `x-autoxide-filters` sinh từ tham
/// số của nó; view lưu `params` sai schema thì 422 nêu từng tham số; view chung cần quyền.
#[tokio::test]
async fn view_da_luu_kiem_theo_schema_bo_loc() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let document = serde_json::to_value(app.openapi()).unwrap();
        let list = &document["paths"]["/api/v1/content/entries"]["get"];
        assert_eq!(list["x-autoxide-list"], "content.list_content_entries");
        let filters = list["x-autoxide-filters"].as_array().unwrap();
        let named = |name: &str| filters.iter().find(|filter| filter["name"] == name).cloned();
        assert_eq!(named("status").unwrap()["values"], json!(["draft", "published", "archived"]));
        assert_eq!(named("scheduled").unwrap()["kind"], "boolean");
        assert_eq!(named("q").unwrap()["operator"], "contains");
        assert!(named("cursor").is_none() && named("sort").is_none(), "không phải bộ lọc");

        let views = |method: Method, uri: &str, who: &str, body: Option<Value>| {
            let mut request = app.request(method, &format!("/api/v1/me/views{uri}")).token(who);
            if let Some(body) = body {
                request = request.json(body);
            }
            request.send()
        };
        let wrong = views(
            Method::POST,
            "",
            "reader",
            Some(json!({"resource": "content.list_content_entries", "name": "Sai",
                "params": {"status": "gone", "scheduled": "maybe", "colour": "red"}})),
        )
        .await
        .unwrap();
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", wrong.json());
        let mut fields: Vec<String> = wrong.json()["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|error| error["field"].as_str().unwrap().to_owned())
            .collect();
        fields.sort();
        assert_eq!(fields, ["params.colour", "params.scheduled", "params.status"]);

        let saved = views(
            Method::POST,
            "",
            "reader",
            Some(json!({"resource": "content.list_content_entries", "name": "Chờ duyệt",
                "params": {"review_requested": true, "sort": "updated_at"},
                "columns": ["title", "status"], "density": "compact"})),
        )
        .await
        .unwrap();
        assert_eq!(saved.status(), StatusCode::CREATED, "{}", saved.json());
        let id = saved.json()["data"]["id"].as_str().unwrap().to_owned();
        let shared = views(
            Method::POST,
            "",
            "reader",
            Some(json!({"resource": "content.list_content_entries", "name": "Cả nhà", "scope": "tenant"})),
        )
        .await
        .unwrap();
        assert_eq!(shared.status(), StatusCode::FORBIDDEN, "cần kernel.views.share");

        let mine = views(Method::GET, "?resource=content.list_content_entries", "reader", None)
            .await
            .unwrap()
            .json();
        assert_eq!(mine["data"][0]["params"]["review_requested"], true);
        let renamed =
            views(Method::PATCH, &format!("/{id}"), "reader", Some(json!({"name": "Duyệt"})))
                .await
                .unwrap();
        assert_eq!(renamed.json()["data"]["name"], "Duyệt");
        let deleted = views(Method::DELETE, &format!("/{id}"), "reader", None).await.unwrap();
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    })
    .await
    .unwrap();
}

/// Xuất danh sách theo bộ lọc (bước 51 P16): job của kernel đọc bài bằng chính bộ lọc của danh
/// sách, ghi CSV chống CSV injection, người yêu cầu tải về; cột lạ là 422, thiếu quyền là 403.
#[tokio::test]
async fn xuat_danh_sach_bai_theo_bo_loc() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let storage: Arc<dyn autoxide_kernel::storage::Storage> =
            Arc::new(autoxide_testkit::MemoryStorage::new());
        let app = TestApp::builder(test)
            .user("admin", 1, &["content.read", "content.write"])
            .user("nobody", 2, &[])
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .provide::<dyn autoxide_kernel::storage::Storage>(storage)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        for (kind, title) in [("post", "=cmd|'/c calc'!A1"), ("post", "Bài hai"), ("page", "Giới thiệu")] {
            let created = send(&app, "POST", "/entries", Some("admin"), &[], Some(json!({"kind": kind, "title": title}))).await;
            assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        }
        let export = |who: &str, body: Value| {
            app.request(Method::POST, "/api/v1/kernel/exports").token(who).json(body).send()
        };
        let resource = "content.list_content_entries";
        let bad = export("admin", json!({"resource": resource, "columns": ["title", "secret"]}))
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", bad.json());
        assert_eq!(bad.json()["errors"][0]["field"], "columns");
        let denied = export("nobody", json!({"resource": resource})).await.unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);

        let queued = export(
            "admin",
            json!({"resource": resource, "params": {"kind": "post", "order": "asc"}, "columns": ["title", "kind"]}),
        )
        .await
        .unwrap();
        assert_eq!(queued.status(), StatusCode::CREATED, "{}", queued.json());
        let id = queued.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(queued.json()["data"]["status"], "queued");
        let worker = app.modules().worker(2).unwrap();
        while worker.run_once().await.unwrap() > 0 {}

        let job = app.get(&format!("/api/v1/kernel/jobs/{id}")).token("admin").send().await.unwrap().json();
        assert_eq!(job["data"]["status"], "succeeded", "{job}");
        assert_eq!(job["data"]["result"]["rows"], 2);
        let url = job["data"]["file"]["url"].as_str().unwrap().to_owned();
        let file = app.get(&url).token("admin").send().await.unwrap();
        assert_eq!(file.status(), StatusCode::OK);
        let csv = file.text();
        assert_eq!(
            csv,
            "\u{feff}\"title\",\"kind\"\r\n\"'=cmd|'/c calc'!A1\",\"post\"\r\n\"Bài hai\",\"post\"\r\n"
        );
        let other = app.get(&url).token("nobody").send().await.unwrap();
        assert_eq!(other.status(), StatusCode::NOT_FOUND, "job của người khác");
        // Job xong thì kernel phát `kernel.job_finished` cho người yêu cầu (module thông báo gửi tin).
        use sea_orm::ConnectionTrait;
        let database = app.database().read();
        let sql = "SELECT payload FROM kernel_events WHERE name = 'kernel.job_finished'";
        let rows = database
            .query_all_raw(sea_orm::Statement::from_string(database.get_database_backend(), sql))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let payload: String = rows[0].try_get("", "payload").unwrap();
        let payload: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!((payload["status"].as_str(), payload["job_id"].as_str()), (Some("succeeded"), Some(id.as_str())));
    })
    .await
    .unwrap();
}

/// Thao tác hàng loạt (bước 51 P8): lưu trữ theo `ids` có kết quả từng bài (bài đã lưu trữ bị bỏ
/// qua, id lạ thất bại kèm `code`); chọn theo bộ lọc mà số khớp khác `expected_count` thì 409;
/// thiếu `Idempotency-Key` là 422.
#[tokio::test]
async fn luu_tru_hang_loat_co_ket_qua_tung_bai() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let mut ids = Vec::new();
        for title in ["Một", "Hai", "Ba"] {
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
        let bulk = |who: &str, key: Option<&str>, body: Value| {
            let mut request = app
                .request(Method::POST, "/api/v1/kernel/bulk/content.list_content_entries/archive")
                .token(who)
                .json(body);
            if let Some(key) = key {
                request = request.header("idempotency-key", key);
            }
            request.send()
        };
        let no_key = bulk("admin", None, json!({"ids": [ids[0]]})).await.unwrap();
        assert_eq!(no_key.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", no_key.json());
        let denied = bulk("reader", Some(&Uuid::now_v7().to_string()), json!({"ids": [ids[0]]}))
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let changed = bulk(
            "admin",
            Some(&Uuid::now_v7().to_string()),
            json!({"filter": {"kind": "post"}, "expected_count": 2}),
        )
        .await
        .unwrap();
        assert_eq!(
            (changed.status(), changed.json()["code"].as_str()),
            (StatusCode::CONFLICT, Some("kernel.bulk_count_changed"))
        );

        let preview = bulk("admin", None, json!({"filter": {"kind": "post"}, "dry_run": true}))
            .await
            .unwrap();
        assert_eq!(preview.status(), StatusCode::OK, "{}", preview.json());
        assert_eq!(preview.json()["data"]["count"], 3);
        let first = bulk("admin", Some(&Uuid::now_v7().to_string()), json!({"ids": [ids[0]]}))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::CREATED, "{}", first.json());
        let worker = app.modules().worker(2).unwrap();
        while worker.run_once().await.unwrap() > 0 {}
        let unknown = Uuid::now_v7().to_string();
        let queued = bulk(
            "admin",
            Some(&Uuid::now_v7().to_string()),
            json!({"ids": [ids[0], ids[1], unknown]}),
        )
        .await
        .unwrap();
        assert_eq!(queued.status(), StatusCode::CREATED, "{}", queued.json());
        let job_id = queued.json()["data"]["id"].as_str().unwrap().to_owned();
        while worker.run_once().await.unwrap() > 0 {}
        let job = app
            .get(&format!("/api/v1/kernel/jobs/{job_id}"))
            .token("admin")
            .send()
            .await
            .unwrap()
            .json();
        let result = &job["data"]["result"];
        assert_eq!(
            (result["ok"].as_u64(), result["skipped"].as_u64(), result["failed"].as_u64()),
            (Some(1), Some(1), Some(1)),
            "{job}"
        );
        assert_eq!(result["items"][0]["code"], "already_archived");
        assert_eq!(result["items"][2]["code"], "content.not_found");
        let third =
            send(&app, "GET", &format!("/entries/{}", ids[2]), Some("admin"), &[], None).await;
        assert_eq!(third.json()["data"]["status"], "draft", "bài không được chọn giữ nguyên");
    })
    .await
    .unwrap();
}

/// Hộp việc theo người và việc thiết lập (bước 51 P10): hoãn ẩn việc khỏi hộp của người hoãn (bỏ
/// hoãn thì hiện lại); giao cho người không có quyền của nguồn là 422; `assigned=me` lọc; bước
/// thiết lập "trang đầu tiên" xong khi có trang đã đăng.
#[tokio::test]
async fn hoan_giao_viec_va_buoc_thiet_lap() {
    TestDatabase::run(async |test| {
        // Mỗi người một ID (app chung cho `admin` và `reader` cùng ID 1): giao việc tra quyền theo ID.
        let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let app = TestApp::builder(test)
            .user("admin", 1, &["content.read", "content.write", "content.publish"])
            .user("editor", 3, &["content.read", "content.publish"])
            .user("writer", 2, &["content.read", "content.write"])
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let created = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "page", "title": "Giới thiệu"})),
        )
        .await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let read = send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();
        let asked = send(
            &app,
            "POST",
            &format!("/entries/{id}/request-review"),
            Some("admin"),
            &[("if-match", &etag)],
            None,
        )
        .await;
        assert_eq!(asked.status(), StatusCode::OK, "{}", asked.json());
        let work =
            |query: &str| app.get(&format!("/api/v1/kernel/work{query}")).token("admin").send();
        let items = work("").await.unwrap().json();
        let key = items["data"]["items"][0]["key"].as_str().unwrap().to_owned();
        assert!(key.ends_with(&id), "{items}");
        let me = |method: Method, path: &str, body: Option<Value>| {
            let mut request =
                app.request(method, &format!("/api/v1/me/work/{key}/{path}")).token("admin");
            if let Some(body) = body {
                request = request.json(body);
            }
            request.send()
        };

        let past = me(Method::POST, "snooze", Some(json!({"until": "2000-01-01T00:00:00Z"})))
            .await
            .unwrap();
        assert_eq!(past.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let later = (chrono::Utc::now() + chrono::Duration::days(30))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let snoozed = me(Method::POST, "snooze", Some(json!({"until": later}))).await.unwrap();
        assert_eq!(snoozed.status(), StatusCode::OK, "{}", snoozed.json());
        assert_eq!(work("").await.unwrap().json()["data"]["items"], json!([]), "đã hoãn");
        let with = work("?include_snoozed=true").await.unwrap().json();
        assert!(with["data"]["items"][0]["snoozed_until"].is_string(), "{with}");
        assert_eq!(
            me(Method::DELETE, "snooze", None).await.unwrap().status(),
            StatusCode::NO_CONTENT
        );

        let refused =
            me(Method::POST, "assign", Some(json!({"user_id": Uuid::from_u128(2)}))).await.unwrap();
        assert_eq!(
            refused.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "writer không có content.publish"
        );
        let assigned =
            me(Method::POST, "assign", Some(json!({"user_id": Uuid::from_u128(1)}))).await.unwrap();
        assert_eq!(assigned.json()["data"]["assignee_id"], Uuid::from_u128(1).to_string());
        assert_eq!(work("?assigned=me").await.unwrap().json()["data"]["items"][0]["key"], key);
        assert_eq!(work("?assigned=nobody").await.unwrap().json()["data"]["items"], json!([]));
        let unknown = app
            .request(Method::POST, "/api/v1/me/work/nope:1/snooze")
            .token("admin")
            .json(json!({"until": later}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            (unknown.status(), unknown.json()["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("kernel.work_not_found"))
        );

        let setup = || app.get("/api/v1/kernel/setup").token("admin").send();
        let before = setup().await.unwrap().json();
        let step = before["data"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|step| step["id"] == "content.first_page")
            .cloned()
            .unwrap();
        assert_eq!(
            (step["done"].as_bool(), step["screen"].as_str()),
            (Some(false), Some("content.entries"))
        );
        let read = send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();
        let published = send(
            &app,
            "POST",
            &format!("/entries/{id}/publish"),
            Some("admin"),
            &[("if-match", &etag)],
            None,
        )
        .await;
        assert_eq!(published.status(), StatusCode::OK, "{}", published.json());
        let after = setup().await.unwrap().json();
        let step = after["data"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|step| step["id"] == "content.first_page")
            .cloned()
            .unwrap();
        assert_eq!(step["done"], true, "{after}");
    })
    .await
    .unwrap();
}

/// Ghi chú và dòng hoạt động trên bản ghi (bước 51 P13): người đọc được bài viết ghi chú, nhắc
/// người đọc được; nhắc người không đọc được là 422; người không có `content.read` thấy 404; chỉ
/// người viết xoá được ghi chú.
#[tokio::test]
async fn ghi_chu_tren_ban_ghi() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let app = TestApp::builder(test)
            .user("admin", 1, &["content.read", "content.write"])
            .user("editor", 3, &["content.read"])
            .user("stranger", 4, &[])
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let created = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "post", "title": "Ghi chú"})),
        )
        .await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let notes = format!("/api/v1/kernel/records/content.entry/{id}/notes");
        let post = |who: &str, body: Value| app.post(&notes).token(who).json(body).send();

        let wrong = post("admin", json!({"body": "Xem giúp", "mentions": [Uuid::from_u128(4)]}))
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", wrong.json());
        assert_eq!(wrong.json()["errors"][0]["field"], "mentions[0]");
        let written = post("admin", json!({"body": "Xem giúp", "mentions": [Uuid::from_u128(3)]}))
            .await
            .unwrap();
        assert_eq!(written.status(), StatusCode::CREATED, "{}", written.json());
        let note_id = written.json()["data"]["id"].as_str().unwrap().to_owned();
        let hidden = post("stranger", json!({"body": "x"})).await.unwrap();
        assert_eq!(
            (hidden.status(), hidden.json()["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("kernel.record_not_found"))
        );
        let missing = app
            .get(&format!("/api/v1/kernel/records/content.entry/{}/notes", Uuid::now_v7()))
            .token("admin")
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);

        let activity = app
            .get(&format!("/api/v1/kernel/records/content.entry/{id}/activity"))
            .token("editor")
            .send()
            .await
            .unwrap();
        assert_eq!(activity.status(), StatusCode::OK, "{}", activity.json());
        let first = &activity.json()["data"][0];
        assert_eq!(
            (first["kind"].as_str(), first["text"].as_str()),
            (Some("note"), Some("Xem giúp"))
        );
        let theirs =
            app.delete(&format!("{notes}/{note_id}")).token("editor").send().await.unwrap();
        assert_eq!(theirs.status(), StatusCode::NOT_FOUND, "chỉ người viết xoá được");
        let mine = app.delete(&format!("{notes}/{note_id}")).token("admin").send().await.unwrap();
        assert_eq!(mine.status(), StatusCode::NO_CONTENT);
    })
    .await
    .unwrap();
}
