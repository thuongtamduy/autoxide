//! Chủ đề theo lịch (bước 67, D-234): tạo từ mẫu là bản nháp, nháp không hiện, bật thì hiện đúng giờ,
//! hai bản trùng giờ thì ưu tiên cao thắng, không bản nào thì theme gốc, ghi đè kiểm tương phản trên
//! token đã ghép, hiệu ứng / font lạ bị từ chối, vượt trần, xoá bản đang bật 409, chép sang năm sau
//! là nháp, xem trước đúng thời điểm. Gate chạy trên cả SQLite và Postgres.

use super::*;

fn root(app: &TestApp) -> String {
    autoxide_kernel::tenancy::root_of(app.default_tenant().id()).as_uuid().to_string()
}

async fn preview(app: &TestApp, at: &str, extra: &str) -> Value {
    let uri = format!("/site/{}/preview?at={at}{extra}", root(app));
    let answer = send(app, "GET", &uri, Some("owner"), &[], None).await;
    assert_eq!(answer.status(), StatusCode::OK, "{}", answer.json());
    answer.json()["data"].clone()
}

async fn activate(app: &TestApp, id: &str) {
    let on = send(
        app,
        "POST",
        &format!("/theme-variants/{id}/activate"),
        Some("owner"),
        &[("if-match", "*")],
        None,
    )
    .await;
    assert_eq!(on.status(), StatusCode::OK, "{}", on.json());
}

async fn themed_app(test: &TestDatabase) -> TestApp {
    let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
    let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
    TestApp::builder(test)
        .setting("AUTOXIDE_CONTENT_THEME_VARIANTS_MAX", "3")
        .user("owner", 1, &["content.themes.manage"])
        .user("reader", 2, &["content.read"])
        .provide::<dyn Media>(media)
        .provide::<dyn TextModel>(text)
        .provide::<dyn Embedder>(embedder)
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

#[tokio::test]
async fn chu_de_theo_lich_chon_dung_ban_va_xem_truoc() {
    TestDatabase::run(async |test| {
        let app = themed_app(test).await;
        let presets = send(&app, "GET", "/theme-presets?limit=5", Some("owner"), &[], None).await;
        assert_eq!(presets.status(), StatusCode::OK, "{}", presets.json());
        assert_eq!(presets.json()["data"].as_array().unwrap().len(), 5);
        let cursor = presets.json()["pagination"]["next_cursor"].as_str().unwrap().to_owned();
        let rest = send(&app, "GET", &format!("/theme-presets?limit=5&cursor={cursor}"), Some("owner"), &[], None).await;
        assert_eq!(rest.json()["data"].as_array().unwrap().len(), 4);
        assert_eq!(send(&app, "GET", "/theme-presets", Some("reader"), &[], None).await.status(), StatusCode::FORBIDDEN);

        // Tết từ mẫu, năm 2027 (múi giờ của website: UTC, không có module `organizations`).
        let tet = send(&app, "POST", "/theme-variants", Some("owner"), &[], Some(json!({"preset_key": "tet", "year": 2027}))).await;
        assert_eq!(tet.status(), StatusCode::CREATED, "{}", tet.json());
        let tet = tet.json()["data"].clone();
        assert_eq!((tet["status"].as_str(), tet["effect"].as_str()), (Some("draft"), Some("petals")));
        assert_eq!(tet["schedule"]["starts_at"], "2027-01-27T00:00:00Z");
        let tet_id = tet["id"].as_str().unwrap().to_owned();

        // Nháp không bao giờ hiện; xem trước thì có.
        let during = "2027-02-01T10:00:00Z";
        assert_eq!(preview(&app, during, "").await["variant"], Value::Null);
        let forced = preview(&app, during, &format!("&variant_id={tet_id}")).await;
        assert_eq!(forced["variant"]["id"], tet_id.as_str());
        assert_eq!(forced["tokens"]["color.primary"], "#b91c1c");

        activate(&app, &tet_id).await;
        let shown = preview(&app, during, "").await;
        assert_eq!(shown["variant"]["id"], tet_id.as_str());
        assert_eq!(shown["effect"], "petals");
        assert_eq!(shown["next_change_at"], Value::Null, "the end is more than 7 days away");
        let before = preview(&app, "2027-01-26T23:59:00Z", "").await;
        assert_eq!(before["variant"], Value::Null, "no variant: the base theme");
        assert_eq!(before["tokens"]["color.primary"], "#1d4ed8");
        assert_eq!(before["next_change_at"], "2027-01-27T00:00:00Z");

        // Giờ vàng hằng ngày, ưu tiên cao hơn Tết: thắng trong giờ của nó.
        let golden = json!({"name": "Giờ vàng", "overrides": {"color.primary": "#065f46"}, "schedule": {"kind": "daily", "start": "20:00", "end": "22:00"}, "priority": 10});
        let golden = send(&app, "POST", "/theme-variants", Some("owner"), &[], Some(golden)).await;
        assert_eq!(golden.status(), StatusCode::CREATED, "{}", golden.json());
        let golden_id = golden.json()["data"]["id"].as_str().unwrap().to_owned();
        activate(&app, &golden_id).await;
        assert_eq!(preview(&app, "2027-02-01T20:30:00Z", "").await["variant"]["id"], golden_id.as_str());
        let after = preview(&app, "2027-02-01T19:00:00Z", "").await;
        assert_eq!(after["variant"]["id"], tet_id.as_str());
        assert_eq!(after["next_change_at"], "2027-02-01T20:00:00Z");

        // Sai: mọi lỗi một lần (hiệu ứng, font, lịch); vượt trần 3 bản.
        let bad = json!({"name": "Lỗi", "overrides": {"font.body": "Comic Sans"}, "effect": "rain", "schedule": {"kind": "daily", "start": "25:00", "end": "01:00"}});
        let refused = send(&app, "POST", "/theme-variants", Some("owner"), &[], Some(bad)).await;
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let fields: Vec<String> = refused.json()["errors"].as_array().unwrap().iter().map(|e| e["field"].as_str().unwrap().to_owned()).collect();
        for wanted in ["overrides.font.body", "effect", "schedule.start"] {
            assert!(fields.iter().any(|field| field == wanted), "{wanted} in {fields:?}");
        }

        // Ghi đè làm chữ kém tương phản trên nền của theme gốc: cảnh báo trên token đã ghép.
        let grey = json!({"name": "Xám", "overrides": {"color.text": "#bbbbbb"}, "schedule": {"kind": "once", "starts_at": "2028-01-01T00:00:00Z", "ends_at": "2028-01-02T00:00:00Z"}});
        let grey = send(&app, "POST", "/theme-variants", Some("owner"), &[], Some(grey)).await;
        assert_eq!(grey.status(), StatusCode::CREATED, "{}", grey.json());
        let grey_id = grey.json()["data"]["id"].as_str().unwrap().to_owned();
        let warned = preview(&app, "2028-01-01T10:00:00Z", &format!("&variant_id={grey_id}")).await;
        assert_eq!(warned["warnings"][0]["foreground"], "color.text", "{warned}");
        let full = send(&app, "POST", "/theme-variants", Some("owner"), &[], Some(json!({"preset_key": "christmas"}))).await;
        assert_eq!((full.status(), full.json()["code"].as_str()), (StatusCode::CONFLICT, Some("content.theme_variants_limit")));

        // Xoá bản đang bật → 409; tạm dừng rồi xoá được. Chép sang năm sau là nháp.
        let refused = send(&app, "DELETE", &format!("/theme-variants/{tet_id}"), Some("owner"), &[("if-match", "*")], None).await;
        assert_eq!((refused.status(), refused.json()["code"].as_str()), (StatusCode::CONFLICT, Some("content.theme_variant_active")));
        assert_eq!(send(&app, "DELETE", &format!("/theme-variants/{grey_id}"), Some("owner"), &[("if-match", "*")], None).await.status(), StatusCode::NO_CONTENT);
        let next_year = json!({"schedule": {"kind": "once", "starts_at": "2028-01-17T00:00:00Z", "ends_at": "2028-02-04T00:00:00Z"}});
        let copied = send(&app, "POST", &format!("/theme-variants/{tet_id}/copy"), Some("owner"), &[], Some(next_year)).await;
        assert_eq!(copied.status(), StatusCode::CREATED, "{}", copied.json());
        assert_eq!(copied.json()["data"]["status"], "draft");
        assert_eq!(copied.json()["data"]["overrides"], tet["overrides"]);
        assert_eq!(copied.json()["data"]["preset_key"], "tet");
        assert_eq!(preview(&app, "2028-01-20T00:00:00Z", "").await["variant"], Value::Null, "the copy is a draft");

        let paused = send(&app, "POST", &format!("/theme-variants/{golden_id}/pause"), Some("owner"), &[("if-match", "*")], None).await;
        assert_eq!(paused.json()["data"]["status"], "paused");
        assert_eq!(preview(&app, "2027-02-01T20:30:00Z", "").await["variant"]["id"], tet_id.as_str());
        let listed = send(&app, "GET", "/theme-variants", Some("owner"), &[], None).await;
        assert_eq!(listed.json()["data"].as_array().unwrap().len(), 3);
    })
    .await
    .unwrap();
}
