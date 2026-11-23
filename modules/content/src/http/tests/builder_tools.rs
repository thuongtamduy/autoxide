//! Tool MCP của trang dựng từ khối (bước 67): danh sách khối, đọc / sửa bố cục với `version` (cả trang
//! hay JSON Patch), loại nội dung, bản chủ đề (luôn nháp). Cùng service và quyền như REST: người thiếu
//! quyền không thấy tool. Gate chạy trên cả SQLite và Postgres.

use super::*;

async fn builder_app(test: &TestDatabase) -> TestApp {
    let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
    let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
    TestApp::builder(test)
        .user(
            "admin",
            1,
            &["content.read", "content.write", "content.themes.manage", "content.types.manage"],
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

#[tokio::test]
async fn tool_dung_trang_tu_khoi_nhu_rest() {
    TestDatabase::run(async |test| {
        let app = builder_app(test).await;
        let reader = app.mcp("reader").await.unwrap();
        let names: Vec<String> = reader.list_all_tools().await.unwrap().into_iter().map(|tool| tool.name.to_string()).collect();
        assert!(names.contains(&"content.get_page_layout".to_owned()));
        assert!(!names.contains(&"content.update_page_layout".to_owned()), "{names:?}");
        assert!(!names.contains(&"content.save_theme_variant".to_owned()));
        reader.cancel().await.unwrap();

        let admin = app.mcp("admin").await.unwrap();
        let call = async |name: &str, arguments: Value| {
            let result = admin.call_tool(tool(name, arguments)).await.unwrap();
            (result.is_error == Some(true), result.structured_content.unwrap_or(Value::Null))
        };
        let (failed, blocks) = call("content.list_blocks", json!({})).await;
        assert!(!failed, "{blocks}");
        let blocks = blocks["data"].as_array().unwrap().clone();
        assert!(blocks.iter().any(|block| block["name"] == "content.hero" && block["props_schema"].is_object()));
        assert!(blocks.iter().all(|block| !block["use_when"].as_str().unwrap_or_default().is_empty()));

        let created = send(&app, "POST", "/entries", Some("admin"), &[], Some(json!({"kind": "page", "title": "Sale"}))).await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let (_, read) = call("content.get_page_layout", json!({"id": id})).await;
        assert_eq!(read["data"]["is_layout"], false);
        let version = read["data"]["version"].as_str().unwrap().to_owned();
        let layout = json!({"type": "layout", "schema": 1, "sections": [{"id": "hero", "block": "content.hero", "props": {"heading": "Sale hè"}}]});
        let (failed, both) = call("content.update_page_layout", json!({"id": id, "version": version, "layout": layout, "operations": []})).await;
        assert!(failed, "{both}");
        let (failed, saved) = call("content.update_page_layout", json!({"id": id, "version": version, "layout": layout})).await;
        assert!(!failed, "{saved}");
        assert_eq!(saved["data"]["body"]["sections"][0]["props"]["heading"], "Sale hè");
        // Phiên bản cũ bị từ chối như `If-Match` của REST.
        let (failed, stale) = call("content.update_page_layout", json!({"id": id, "version": version, "operations": [{"op": "replace", "path": "/sections/0/props/heading", "value": "x"}]})).await;
        assert!(failed, "{stale}");
        let version = saved["data"]["version"].as_str().unwrap().to_owned();
        let (failed, patched) = call("content.update_page_layout", json!({"id": id, "version": version, "operations": [{"op": "replace", "path": "/sections/0/props/heading", "value": "Sale thu"}]})).await;
        assert!(!failed, "{patched}");
        assert_eq!(patched["data"]["body"]["sections"][0]["props"]["heading"], "Sale thu");
        // Khối lạ: bộ kiểm bố cục từ chối y như REST.
        let version = patched["data"]["version"].as_str().unwrap().to_owned();
        let (failed, refused) = call("content.update_page_layout", json!({"id": id, "version": version, "operations": [{"op": "replace", "path": "/sections/0/block", "value": "shop.nothing"}]})).await;
        assert!(failed, "{refused}");

        let labels = json!({"one": {"vi": "Dự án"}, "many": {"vi": "Dự án"}});
        let (failed, kind) = call("content.save_type", json!({"fields": {"key": "du_an", "labels": labels}})).await;
        assert!(!failed, "{kind}");
        let version = kind["data"]["version"].as_str().expect(&kind.to_string()).to_owned();
        let (failed, changed) = call("content.save_type", json!({"key": "du_an", "version": version, "fields": {"icon": "star"}})).await;
        assert!(!failed, "{changed}");
        assert_eq!(changed["data"]["icon"], "star");
        let (_, listed) = call("content.list_types", json!({})).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 1, "{listed}");

        let golden = json!({"name": "Giờ vàng", "overrides": {"color.primary": "#065f46"}, "schedule": {"kind": "daily", "start": "20:00", "end": "22:00"}});
        let (failed, variant) = call("content.save_theme_variant", json!({"fields": golden})).await;
        assert!(!failed, "{variant}");
        assert_eq!(variant["data"]["status"], "draft", "a tool never switches a variant on");
        let (_, variants) = call("content.list_theme_variants", json!({})).await;
        assert_eq!(variants["data"].as_array().unwrap().len(), 1, "{variants}");
        admin.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
