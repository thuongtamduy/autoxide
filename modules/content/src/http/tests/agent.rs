//! Lệnh rủi ro của agent, nhãn nội bộ, trường tự định nghĩa, việc/tìm không lộ sang nhánh anh em.

use super::*;

/// Lệnh cần duyệt khi agent gọi (bước 51 P12): agent xoá bài thì nhận 202 và yêu cầu duyệt, bài
/// còn nguyên; agent không tự duyệt được; gửi lại trước khi duyệt là 409; người duyệt xong thì gửi
/// lại cùng request (cùng `Idempotency-Key`) kèm `Autoxide-Approval` mới xoá thật, một lần.
#[tokio::test]
async fn agent_goi_lenh_rui_ro_nhan_202_approval_required() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(FakeMedia::new().tagging_locale());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let perms = ["content.read", "content.write", "content.delete"].map(str::to_owned);
        let robot = Principal::new(Uuid::from_u128(1), perms.clone())
            .via_api_key(Uuid::from_u128(9))
            .with_actor_kind(autoxide_kernel::access::ActorKind::Agent);
        let app = TestApp::builder(test)
            .user(
                "editor",
                2,
                &["content.read", "content.write", "content.delete", "kernel.devtools.read"],
            )
            .principal("robot", robot)
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
            Some("editor"),
            &[],
            Some(json!({"kind": "post", "title": "Sắp xoá"})),
        )
        .await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let read = send(&app, "GET", &format!("/entries/{id}"), Some("editor"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();
        let key = Uuid::now_v7().to_string();
        let delete = |approval: Option<&str>| {
            let mut headers = vec![("if-match", etag.as_str()), ("idempotency-key", key.as_str())];
            if let Some(approval) = approval {
                headers.push(("autoxide-approval", approval));
            }
            let path = format!("/entries/{id}");
            let headers: Vec<(String, String)> =
                headers.into_iter().map(|(a, b)| (a.to_owned(), b.to_owned())).collect();
            let app = &app;
            async move {
                let pairs: Vec<(&str, &str)> =
                    headers.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
                send(app, "DELETE", &path, Some("robot"), &pairs, None).await
            }
        };

        let asked = delete(None).await;
        assert_eq!(asked.status(), StatusCode::ACCEPTED, "{}", asked.json());
        let approval = asked.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(asked.json()["data"]["status"], "pending");
        assert_eq!(
            send(&app, "GET", &format!("/entries/{id}"), Some("editor"), &[], None).await.status(),
            StatusCode::OK,
            "chưa xoá"
        );
        let early = delete(Some(&approval)).await;
        assert_eq!(
            (early.status(), early.json()["code"].as_str()),
            (StatusCode::CONFLICT, Some("kernel.approval_invalid"))
        );
        let decide = |who: &str| {
            app.post(&format!("/api/v1/kernel/approvals/{approval}/approve")).token(who).send()
        };
        let own = decide("robot").await.unwrap();
        assert_eq!(
            (own.status(), own.json()["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("kernel.approval_forbidden"))
        );
        let pending = app
            .get("/api/v1/kernel/approvals?status=pending")
            .token("editor")
            .send()
            .await
            .unwrap()
            .json();
        assert_eq!(pending["data"][0]["id"], approval, "người duyệt thấy yêu cầu");
        let approved = decide("editor").await.unwrap();
        assert_eq!(approved.json()["data"]["status"], "approved", "{}", approved.json());

        let done = delete(Some(&approval)).await;
        assert_eq!(
            done.status(),
            StatusCode::NO_CONTENT,
            "{}",
            String::from_utf8_lossy(done.body())
        );
        assert_eq!(
            send(&app, "GET", &format!("/entries/{id}"), Some("editor"), &[], None).await.status(),
            StatusCode::NOT_FOUND
        );
        // Nhật ký lệnh của agent (D-202): dòng 202 và dòng chạy thật cùng giữ `approval_id`.
        let mut rows = Vec::new();
        for _ in 0..100 {
            let page = app
                .get("/api/v1/kernel/devtools/requests?actor_kind=agent&path=/api/v1/content/entries/{id}")
                .token("editor")
                .send()
                .await
                .unwrap()
                .json();
            rows = page["data"].as_array().unwrap().clone();
            if rows.iter().filter(|row| row["method"] == "DELETE").count() >= 3 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        let statuses: Vec<(u64, &str)> = rows
            .iter()
            .filter(|row| row["method"] == "DELETE")
            .map(|row| (row["status"].as_u64().unwrap(), row["approval_id"].as_str().unwrap_or("")))
            .collect();
        assert_eq!(
            statuses,
            [(204, approval.as_str()), (409, approval.as_str()), (202, approval.as_str())],
            "lenh_cho_duyet_giu_approval_id"
        );
        let used = app
            .get(&format!("/api/v1/kernel/approvals/{approval}"))
            .token("robot")
            .send()
            .await
            .unwrap()
            .json();
        assert_eq!(used["data"]["status"], "used");
    })
    .await
    .unwrap();
}

/// Nhãn nội bộ (bước 51 P20): gắn cần quyền ghi bài; tên trùng khác hoa thường / dấu là một nhãn;
/// lọc `tags_any` / `tags_all`; bài trả `tags`; gộp nhãn chuyển hết bản ghi; khách bật
/// `kernel.tags.restricted` thì người không có `kernel.tags.manage` không tạo nhãn mới.
#[tokio::test]
async fn nhan_noi_bo_tren_bai_viet() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(FakeMedia::new().tagging_locale());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let app = TestApp::builder(test)
            .user(
                "admin",
                1,
                &["content.read", "content.write", "kernel.tags.manage", "kernel.settings.manage"],
            )
            .user("writer", 2, &["content.read", "content.write"])
            .user("reader", 3, &["content.read"])
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .module(crate::module())
            .build()
            .await
            .unwrap();
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
        let tag = |who: &str, id: &str, body: Value| {
            app.put(&format!("/api/v1/kernel/records/content.entry/{id}/tags"))
                .token(who)
                .json(body)
                .send()
        };
        let first = tag("writer", &ids[0], json!({"new_tags": ["VIP", "Cần gọi"]})).await.unwrap();
        assert_eq!(first.status(), StatusCode::OK, "{}", first.json());
        assert_eq!(first.json()["data"].as_array().unwrap().len(), 2);
        let second = tag("writer", &ids[1], json!({"new_tags": ["vip"]})).await.unwrap();
        let vip = second.json()["data"][0]["id"].as_str().unwrap().to_owned();
        assert_eq!(second.json()["data"][0]["name"], "VIP", "cùng tên khác hoa thường là một nhãn");
        assert_eq!(
            tag("reader", &ids[0], json!({"tag_ids": []})).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );

        let all = app.get("/api/v1/kernel/tags").token("reader").send().await.unwrap().json();
        let call = all["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "Cần gọi")
            .cloned()
            .unwrap();
        assert_eq!(
            all["data"].as_array().unwrap().iter().find(|t| t["name"] == "VIP").unwrap()["uses"],
            2
        );
        let call_id = call["id"].as_str().unwrap().to_owned();
        let list = |query: String| {
            let app = &app;
            async move {
                send(app, "GET", &format!("/entries?{query}"), Some("reader"), &[], None).await
            }
        };
        assert_eq!(
            list(format!("tags_any={vip}")).await.json()["data"].as_array().unwrap().len(),
            2
        );
        let both = list(format!("tags_all={vip},{call_id}")).await.json();
        assert_eq!(both["data"].as_array().unwrap().len(), 1, "{both}");
        assert_eq!(both["data"][0]["tags"].as_array().unwrap().len(), 2);

        let merged = app
            .post(&format!("/api/v1/kernel/tags/{call_id}/merge"))
            .token("admin")
            .json(json!({"into": vip}))
            .send()
            .await
            .unwrap();
        assert_eq!(merged.status(), StatusCode::OK, "{}", merged.json());
        let after = list(format!("tags_any={vip}")).await.json();
        assert!(
            after["data"].as_array().unwrap().iter().all(|entry| entry["tags"]
                .as_array()
                .unwrap()
                .len()
                == 1)
        );

        let restricted = app
            .put("/api/v1/kernel/settings/kernel.tags.restricted")
            .token("admin")
            .json(json!({"value": "true"}))
            .send()
            .await
            .unwrap();
        assert_eq!(restricted.status(), StatusCode::OK, "{}", restricted.json());
        let refused =
            tag("writer", &ids[0], json!({"tag_ids": [vip], "new_tags": ["Mới"]})).await.unwrap();
        assert_eq!(
            (refused.status(), refused.json()["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("kernel.tags_restricted"))
        );
        assert_eq!(
            tag("writer", &ids[0], json!({"tag_ids": []})).await.unwrap().status(),
            StatusCode::OK,
            "bỏ nhãn vẫn được"
        );
    })
    .await
    .unwrap();
}

/// Trường tự định nghĩa (bước 51 P19): khách định nghĩa trường cho bài viết; lỗi gom một lần; giá
/// trị mặc định; lọc `cf.<khoá>` qua index ghi cùng transaction; trường không lọc được là 422; đổi
/// kiểu trường đã dùng là 409.
#[tokio::test]
async fn truong_tu_dinh_nghia_tren_bai_viet() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(FakeMedia::new().tagging_locale());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let app = TestApp::builder(test)
            .user("admin", 1, &["content.read", "content.write", "kernel.custom_fields.manage"])
            .user("reader", 2, &["content.read"])
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let define = |body: Value| app.post("/api/v1/kernel/custom-fields").token("admin").json(body).send();
        let owner = define(json!({"target": "content.entry", "key": "owner", "data_type": "text", "labels": {"en": "Owner"}, "filterable": true})).await.unwrap();
        assert_eq!(owner.status(), StatusCode::CREATED, "{}", owner.json());
        let owner_id = owner.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(define(json!({"target": "content.entry", "key": "budget", "data_type": "integer", "labels": {"en": "Budget"}, "min": "0", "filterable": true})).await.unwrap().status(), StatusCode::CREATED);
        assert_eq!(define(json!({"target": "content.entry", "key": "stage", "data_type": "option", "labels": {"en": "Stage"}, "options": ["draft", "review"], "required": true, "default": "draft"})).await.unwrap().status(), StatusCode::CREATED);
        let bad = define(json!({"target": "content.entry", "key": "Bad Key", "data_type": "colour", "labels": {}})).await.unwrap();
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let fields = app.get("/api/v1/kernel/custom-fields?target=content.entry").token("reader").send().await.unwrap().json();
        assert_eq!(fields["data"].as_array().unwrap().len(), 3, "người đọc bài đọc được định nghĩa");

        let wrong = send(&app, "POST", "/entries", Some("admin"), &[], Some(json!({"kind": "post", "title": "Sai", "custom_fields": {"budget": -5, "colour": "x"}}))).await;
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", wrong.json());
        let mut paths: Vec<String> = wrong.json()["errors"].as_array().unwrap().iter().map(|e| e["field"].as_str().unwrap().to_owned()).collect();
        paths.sort();
        assert_eq!(paths, ["custom_fields.budget", "custom_fields.colour"]);
        let first = send(&app, "POST", "/entries", Some("admin"), &[], Some(json!({"kind": "post", "title": "Một", "custom_fields": {"owner": "Ada Lovelace", "budget": 100}}))).await;
        assert_eq!(first.status(), StatusCode::CREATED, "{}", first.json());
        assert_eq!(first.json()["data"]["custom_fields"]["stage"], "draft", "giá trị mặc định");
        let id = first.json()["data"]["id"].as_str().unwrap().to_owned();
        let second = send(&app, "POST", "/entries", Some("admin"), &[], Some(json!({"kind": "post", "title": "Hai", "custom_fields": {"owner": "Bob", "budget": 50}}))).await;
        assert_eq!(second.status(), StatusCode::CREATED);

        let count = |query: &'static str| {
            let app = &app;
            async move {
                let answer = send(app, "GET", &format!("/entries?{query}"), Some("reader"), &[], None).await;
                (answer.status(), answer.json()["data"].as_array().map_or(0, Vec::len))
            }
        };
        assert_eq!(count("cf.owner=prefix:ada").await, (StatusCode::OK, 1));
        assert_eq!(count("cf.budget=gte:60").await, (StatusCode::OK, 1));
        assert_eq!(count("cf.budget=in:50,100").await, (StatusCode::OK, 2));
        assert_eq!(count("cf.stage=eq:draft").await.0, StatusCode::UNPROCESSABLE_ENTITY, "không lọc được");

        let read = send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();
        let cleared = send(&app, "PATCH", &format!("/entries/{id}"), Some("admin"), &[("if-match", &etag)], Some(json!({"custom_fields": {"budget": null}}))).await;
        assert_eq!(cleared.status(), StatusCode::OK, "{}", cleared.json());
        assert!(cleared.json()["data"]["custom_fields"].get("budget").is_none());
        assert_eq!(count("cf.budget=gte:60").await, (StatusCode::OK, 0), "index ghi lại cùng lúc");

        let locked = app.patch(&format!("/api/v1/kernel/custom-fields/{owner_id}")).token("admin").json(json!({"data_type": "integer"})).send().await.unwrap();
        assert_eq!((locked.status(), locked.json()["code"].as_str()), (StatusCode::CONFLICT, Some("kernel.custom_field_type_locked")));
    })
    .await
    .unwrap();
}

/// Hai nhánh anh em của cây tổ chức (bước 51, test bắt buộc): bài chờ duyệt ở nhánh A không hiện
/// trong hộp việc, số đếm, kết quả tìm hay mở theo mã của người chỉ làm ở nhánh B.
#[tokio::test]
async fn viec_va_tim_khong_lo_sang_nhanh_anh_em() {
    use autoxide_kernel::access::Scope;
    use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree};
    TestDatabase::run(async |test| {
        let (a, b) = (OrgId::from_uuid(Uuid::now_v7()), OrgId::from_uuid(Uuid::now_v7()));
        let grants = [
            ("content.read", Scope::Subtree),
            ("content.write", Scope::Subtree),
            ("content.publish", Scope::Subtree),
        ];
        let media: Arc<dyn Media> = Arc::new(FakeMedia::new().tagging_locale());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let app = TestApp::builder(test)
            .member("north", 1, a, &grants)
            .member("south", 2, b, &grants)
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let write = app.database().begin_write().await.unwrap();
        let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, a, root, 8).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, b, root, 8).await.unwrap();
        write.commit().await.unwrap();

        let created = send(
            &app,
            "POST",
            "/entries",
            Some("north"),
            &[],
            Some(json!({"kind": "post", "title": "Nhanh bac bi mat"})),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let read = send(&app, "GET", &format!("/entries/{id}"), Some("north"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();
        let asked = send(
            &app,
            "POST",
            &format!("/entries/{id}/request-review"),
            Some("north"),
            &[("if-match", &etag)],
            None,
        )
        .await;
        assert_eq!(asked.status(), StatusCode::OK, "{}", asked.json());

        let get = |who: &'static str, uri: String| {
            let app = &app;
            async move { app.get(&uri).token(who).send().await.unwrap().json() }
        };
        let (a_id, b_id) = (a.as_uuid(), b.as_uuid());
        let north = get("north", format!("/api/v1/kernel/work?org_id={a_id}")).await;
        assert_eq!(north["data"]["items"].as_array().unwrap().len(), 1, "{north}");
        for unit in [format!("org_id={b_id}"), String::new()] {
            let south = get("south", format!("/api/v1/kernel/work?{unit}")).await;
            assert_eq!(south["data"]["items"], json!([]), "{south}");
            let counts = get("south", format!("/api/v1/kernel/work/counts?{unit}")).await;
            assert_eq!(counts["data"]["counts"]["work"], 0, "{counts}");
            let found = get("south", format!("/api/v1/kernel/find?q=bi%20mat&{unit}")).await;
            assert_eq!(found["data"]["groups"], json!([]), "{found}");
        }
        let theirs = get("north", format!("/api/v1/kernel/find?q=bi%20mat&org_id={a_id}")).await;
        assert_eq!(theirs["data"]["groups"].as_array().unwrap().len(), 1, "{theirs}");
        let resolved = app
            .get("/api/v1/kernel/resolve?code=nhanh-bac-bi-mat")
            .token("south")
            .send()
            .await
            .unwrap();
        assert_eq!(resolved.status(), StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}
