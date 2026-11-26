//! Danh sách có phân trang (bước 70 P9 mục 3, D-20261125-list-pages): cây danh mục quản trị và công khai (keyset
//! trên độ sâu, vị trí, id — có hoà vị trí giữa các nhánh), thuộc tính theo mã, quy tắc làm tròn, số sản phẩm theo
//! khoá cố định; tool `catalog.list_categories` trả đúng trang REST trả.

use autoxide_testkit::mcp::CallToolRequestParams;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};

use super::public::public;
use super::{app, call};

fn tool(name: &str, arguments: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.to_owned())
        .with_arguments(arguments.as_object().unwrap().clone())
}

/// Theo `next_cursor` của `uri` (đã có `limit`, có `?`) tới hết; trả trường `field` của mọi dòng theo thứ tự.
async fn walk(app: &TestApp, uri: &str, field: &str) -> Vec<Value> {
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page_uri = match &cursor {
            Some(cursor) => format!("{uri}&cursor={cursor}"),
            None => uri.to_owned(),
        };
        let (status, page) = call(app, "GET", &page_uri, "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        seen.extend(page["data"].as_array().unwrap().iter().map(|row| row[field].clone()));
        match page["pagination"]["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => return seen,
        }
    }
}

/// Như [`walk`] cho trang công khai.
async fn walk_public(app: &TestApp, uri: &str) -> Vec<Value> {
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page_uri = match &cursor {
            Some(cursor) => format!("{uri}&cursor={cursor}"),
            None => uri.to_owned(),
        };
        let (status, page, _) = public(app, &page_uri, None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        seen.extend(page["data"].as_array().unwrap().iter().map(|row| row["id"].clone()));
        match page["pagination"]["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => return seen,
        }
    }
}

async fn category(app: &TestApp, name: &str, parent: Option<&str>) -> String {
    let body = json!({"parent_id": parent, "texts": {"vi": {"name": name}}});
    let (status, made) = call(app, "POST", "/categories", "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    made["data"]["id"].as_str().unwrap().to_owned()
}

fn ids(page: &Value) -> Vec<Value> {
    page["data"].as_array().unwrap().iter().map(|row| row["id"].clone()).collect()
}

#[tokio::test]
async fn cay_danh_muc_theo_trang_quan_tri_cong_khai_va_tool() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        // Hai nhánh, mỗi nhánh hai con: con của hai nhánh trùng vị trí (hoà) — phá hoà bằng id.
        let fashion = category(&app, "Thời trang", None).await;
        let home = category(&app, "Nhà cửa", None).await;
        let hidden = category(&app, "Ẩn", None).await;
        for (index, parent) in [&fashion, &home, &hidden].into_iter().enumerate() {
            category(&app, &format!("Con {index} một"), Some(parent)).await;
            category(&app, &format!("Con {index} hai"), Some(parent)).await;
        }
        let (status, all) = call(&app, "GET", "/categories?limit=200", "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{all}");
        let all_ids = ids(&all);
        assert_eq!(all_ids.len(), 9);
        let depths: Vec<i64> = all["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["depth"].as_i64().unwrap())
            .collect();
        assert!(depths.windows(2).all(|pair| pair[0] <= pair[1]), "cha trước con: {depths:?}");
        assert_eq!(walk(&app, "/categories?limit=2", "id").await, all_ids);
        let mut reversed = all_ids.clone();
        reversed.reverse();
        assert_eq!(walk(&app, "/categories?limit=4&order=desc", "id").await, reversed);

        // Công khai: ẩn một nhánh thì cả nhánh vắng; theo trang ra đúng danh sách một trang lớn.
        let (status, _) = call(
            &app,
            "PATCH",
            &format!("/categories/{hidden}"),
            "boss",
            Some(json!({"visible": false})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, shown, _) = public(&app, "/categories?locale=vi&limit=200", None).await;
        assert_eq!(status, StatusCode::OK, "{shown}");
        let shown_ids = ids(&shown);
        assert_eq!(shown_ids.len(), 6, "{shown}");
        assert!(!shown_ids.contains(&json!(hidden)));
        assert_eq!(walk_public(&app, "/categories?locale=vi&limit=2").await, shown_ids);
        let (_, other, _) = public(&app, "/categories?locale=en&limit=200", None).await;
        assert_eq!(other["data"], json!([]), "không có tên tiếng Anh");

        // Tool và REST cùng tham số: cùng trang, cùng `next_cursor`.
        let (_, rest) = call(&app, "GET", "/categories?limit=3&count=true", "boss", None).await;
        let client = app.mcp("boss").await.unwrap();
        let found = client
            .call_tool(tool("catalog.list_categories", json!({"limit": 3, "count": true})))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(found["data"], rest["data"]);
        assert_eq!(found["pagination"], rest["pagination"]);
        assert_eq!(found["pagination"]["total"], 9);
        let next = client
            .call_tool(tool(
                "catalog.list_categories",
                json!({"limit": 200, "cursor": found["pagination"]["next_cursor"]}),
            ))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(ids(&next), all_ids[3..]);
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn thuoc_tinh_lam_tron_va_so_san_pham_theo_trang() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        for code in ["material", "brand", "color"] {
            let body = json!({"code": code, "data_type": "text", "names": {"vi": code}});
            let (status, made) = call(&app, "POST", "/attributes", "boss", Some(body)).await;
            assert_eq!(status, StatusCode::CREATED, "{made}");
        }
        assert_eq!(
            walk(&app, "/attributes?limit=2", "code").await,
            vec![json!("brand"), json!("color"), json!("material")]
        );

        let mut rules = Vec::new();
        for step in [1_000, 5_000, 10_000] {
            let rule = json!({"name": format!("Bước {step}"), "step_minor": step, "mode": "up", "ending_minor": 0});
            let (status, made) = call(&app, "POST", "/rounding-rules", "boss", Some(rule)).await;
            assert_eq!(status, StatusCode::CREATED, "{made}");
            rules.push(made["data"]["id"].clone());
        }
        assert_eq!(walk(&app, "/rounding-rules?limit=2&order=asc", "id").await, rules);

        // Số theo khoá cố định: giữ thứ tự khai báo, kể cả khi theo trang.
        assert_eq!(
            walk(&app, "/products/counts?limit=3", "key").await,
            vec![json!("draft"), json!("active"), json!("inactive"), json!("archived")]
        );
        assert_eq!(
            walk(&app, "/products/counts?group_by=readiness&limit=1&order=desc", "key").await,
            vec![json!("restricted"), json!("missing_documents"), json!("missing_attributes"), json!("ready")]
        );
    })
    .await
    .unwrap();
}
