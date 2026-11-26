//! Danh sách của `plans` trả theo trang (bước 70 P9, D-20261125-list-pages): theo `next_cursor` tới hết không
//! trùng không sót, hai chiều; tool `plans.list_usage` cùng trang với REST.

use super::*;

/// Duyệt hết một danh sách REST (`uri` đã có `?limit=…`) theo `next_cursor`; trả trường `field` của mỗi dòng.
async fn walk(app: &TestApp, uri: &str, who: &str, field: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut next = uri.to_owned();
    loop {
        let page = expect(app, "GET", &next, who, None, StatusCode::OK).await;
        let limit = page["pagination"]["limit"].as_u64().unwrap();
        let rows = page["data"].as_array().unwrap();
        assert!(rows.len() as u64 <= limit, "{uri}: {page}");
        found.extend(rows.iter().map(|row| row[field].as_str().unwrap().to_owned()));
        match page["pagination"]["next_cursor"].as_str() {
            Some(cursor) => next = format!("{uri}&cursor={cursor}"),
            None => return found,
        }
    }
}

fn sorted(mut items: Vec<String>) -> Vec<String> {
    items.sort();
    items.dedup();
    items
}

/// Danh mục công khai theo mã, sản phẩm cửa hàng theo mã sản phẩm, ghi đè mới nhất trước: trang 2 dòng tới
/// hết, không trùng không sót, `order` đảo chiều; gói nháp không vào danh mục.
#[tokio::test]
async fn danh_muc_san_pham_va_ghi_de_phan_trang() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        for code in ["delta", "alpha", "echo", "charlie", "bravo"] {
            plan(&app, code, Some(1), json!({})).await;
        }
        plan(&app, "zulu", None, json!({"status": "draft"})).await;
        let mut codes = Vec::new();
        let mut uri = "/api/v1/plans/catalog?limit=2".to_owned();
        loop {
            let page = app.get(&uri).send().await.unwrap().json();
            codes.extend(
                page["data"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| p["code"].as_str().unwrap().to_owned()),
            );
            match page["pagination"]["next_cursor"].as_str() {
                Some(cursor) => uri = format!("/api/v1/plans/catalog?limit=2&cursor={cursor}"),
                None => break,
            }
        }
        assert_eq!(codes, ["alpha", "bravo", "charlie", "delta", "echo"]);
        let down = app.get("/api/v1/plans/catalog?limit=2&order=desc").send().await.unwrap().json();
        assert_eq!(down["data"][0]["code"], "echo");
        let wrong = app.get("/api/v1/plans/catalog?limit=0").send().await.unwrap();
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY);

        // Sản phẩm của hai cửa hàng: cùng mã đứng cạnh nhau (khoá phụ là ID).
        let pro = plan(&app, "pro", Some(1), json!({})).await;
        for (store, product) in [
            ("google_play", "pro_yearly"),
            ("app_store", "pro_monthly"),
            ("google_play", "pro_monthly"),
            ("app_store", "basic"),
        ] {
            let link = json!({"plan_id": pro});
            let uri = format!("/store/products/{store}/{product}");
            expect(&app, "PUT", &uri, "ops", Some(link), StatusCode::OK).await;
        }
        let products = walk(&app, "/store/products?limit=3", "ops", "product_id").await;
        assert_eq!(products, ["basic", "pro_monthly", "pro_monthly", "pro_yearly"]);
        let back = walk(&app, "/store/products?limit=3&order=desc", "ops", "product_id").await;
        assert_eq!(back, ["pro_yearly", "pro_monthly", "pro_monthly", "basic"]);

        // Ghi đè của một khách: mới nhất trước, không lẫn khách khác.
        let acme = tenant(&app, "acme").await;
        let beta = tenant(&app, "beta").await;
        let mut created = Vec::new();
        for body in [
            json!({"kind": "module", "key": "demo", "enabled": true, "reason": "Test"}),
            json!({"kind": "feature", "key": "demo.export", "enabled": true, "reason": "Test"}),
            json!({"kind": "limit", "key": "demo.items", "max_value": 3, "reason": "Test"}),
        ] {
            let uri = format!("/tenants/{acme}/overrides");
            let saved = expect(&app, "POST", &uri, "ops", Some(body), StatusCode::CREATED).await;
            created.push(saved["data"]["id"].as_str().unwrap().to_owned());
        }
        let other =
            json!({"kind": "limit", "key": "demo.calls", "max_value": 1, "reason": "Other"});
        let uri = format!("/tenants/{beta}/overrides");
        expect(&app, "POST", &uri, "ops", Some(other), StatusCode::CREATED).await;
        let newest = walk(&app, &format!("/tenants/{acme}/overrides?limit=2"), "ops", "id").await;
        assert_eq!(newest, created.iter().rev().cloned().collect::<Vec<_>>());
        let oldest =
            walk(&app, &format!("/tenants/{acme}/overrides?limit=2&order=asc"), "ops", "id").await;
        assert_eq!(oldest, created);
    })
    .await
    .unwrap();
}

/// Lượng dùng theo khoá hạn mức: trang 1 dòng tới hết đúng bằng cả danh sách; tool `plans.list_usage` cùng
/// tham số trả cùng `data` và `pagination` như REST, rồi theo `next_cursor` của tool tới hết.
#[tokio::test]
async fn luong_dung_phan_trang_va_tool_nhu_rest() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = limited(&app, 5).await;
        let all = walk(&app, &format!("/tenants/{acme}/usage?limit=200"), "ops", "key").await;
        assert!(all.len() >= 2, "{all:?}");
        assert_eq!(all, sorted(all.clone()));
        let paged = walk(&app, &format!("/tenants/{acme}/usage?limit=1"), "ops", "key").await;
        assert_eq!(paged, all);
        let own = walk(&app, "/usage?limit=1&order=desc", "owner", "key").await;
        assert_eq!(own, all.iter().rev().cloned().collect::<Vec<_>>());

        let client = app.mcp("ops").await.unwrap();
        let rest = expect(
            &app,
            "GET",
            &format!("/tenants/{acme}/usage?limit=1&count=true"),
            "ops",
            None,
            StatusCode::OK,
        )
        .await;
        let arguments = json!({"tenant_id": acme, "limit": 1, "count": true});
        let first = client
            .call_tool(tool("plans.list_usage", arguments))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(first["data"], rest["data"]);
        assert_eq!(first["pagination"], rest["pagination"]);
        assert_eq!(first["pagination"]["total"], all.len());
        let mut keys: Vec<String> = first["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u["key"].as_str().unwrap().to_owned())
            .collect();
        let mut cursor = first["pagination"]["next_cursor"].clone();
        while let Some(next) = cursor.as_str() {
            let arguments = json!({"tenant_id": acme, "limit": 1, "cursor": next});
            let page = client
                .call_tool(tool("plans.list_usage", arguments))
                .await
                .unwrap()
                .structured_content
                .unwrap();
            keys.extend(
                page["data"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|u| u["key"].as_str().unwrap().to_owned()),
            );
            cursor = page["pagination"]["next_cursor"].clone();
        }
        assert_eq!(keys, all);
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
