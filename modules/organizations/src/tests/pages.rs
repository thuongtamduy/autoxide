//! Danh sách của `organizations` trả theo trang (bước 70 P9, D-20261125-list-pages): loại nút theo mã, tổ tiên
//! gần nhất trước, cài đặt theo khoá, cửa hàng công khai theo vị trí (cùng vị trí: theo ID nút) — theo
//! `next_cursor` tới hết không trùng không sót, hai chiều; tool `organizations.get_settings` cùng trang với REST.

use super::*;

/// Duyệt hết `uri` (đã có `?limit=…`) theo `next_cursor`; trả trường `field` của mỗi dòng.
async fn walk(app: &TestApp, uri: &str, field: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut next = uri.to_owned();
    loop {
        let page = expect(app, "GET", &next, "admin", None, StatusCode::OK).await;
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

fn reversed(items: &[String]) -> Vec<String> {
    items.iter().rev().cloned().collect()
}

#[tokio::test]
async fn loai_nut_to_tien_va_cai_dat_phan_trang() {
    TestDatabase::run(async |test| {
        let app = build(test, |builder| builder).await;
        let tree = tree(&app).await;
        for code in ["warehouse", "kiosk", "branch"] {
            let body = json!({"code": code, "name": code});
            expect(&app, "POST", "/unit-types", "admin", Some(body), StatusCode::CREATED).await;
        }
        let codes = walk(&app, "/unit-types?limit=2", "code").await;
        assert_eq!(codes, ["branch", "kiosk", "region", "store", "tenant", "warehouse"]);
        assert_eq!(walk(&app, "/unit-types?limit=2&order=desc", "code").await, reversed(&codes));

        // Chuỗi sâu 4 tầng: gốc → R1 → S1 → K1 → K2; tổ tiên của K2 gần nhất trước.
        let unit = async |parent: &str, kind: &str, name: &str| {
            let body = json!({"parent_id": parent, "unit_type": kind, "name": name});
            id(&expect(&app, "POST", "/units", "admin", Some(body), StatusCode::CREATED).await)
        };
        let k1 = unit(&tree.s1, "kiosk", "Kiosk 1").await;
        let k2 = unit(&k1, "kiosk", "Kiosk 2").await;
        let ancestors = walk(&app, &format!("/units/{k2}/ancestors?limit=1"), "id").await;
        assert_eq!(ancestors, [k1.clone(), tree.s1.clone(), tree.r1.clone(), root()]);
        let down = walk(&app, &format!("/units/{k2}/ancestors?limit=3&order=desc"), "id").await;
        assert_eq!(down, reversed(&ancestors));

        // Cài đặt: theo khoá, trang 1 dòng tới hết = cả danh sách.
        let all = walk(&app, "/settings?limit=200", "key").await;
        assert!(all.len() > 1);
        let mut sorted = all.clone();
        sorted.sort();
        assert_eq!(all, sorted);
        assert_eq!(walk(&app, "/settings?limit=1", "key").await, all);
        let at = walk(&app, &format!("/units/{}/settings?limit=1", tree.r2), "key").await;
        assert_eq!(at, all, "mỗi cài đặt khai một dòng ở mọi nút");

        let (status, wrong) = call(&app, "GET", "/unit-types?cursor=nope", "admin", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(wrong["errors"][0]["field"], "cursor");
    })
    .await
    .unwrap();
}

/// Tool `organizations.get_settings` cùng tham số với `GET /units/{id}/settings` trả cùng `data` và
/// `pagination`, rồi theo `next_cursor` của tool tới hết.
#[tokio::test]
async fn tool_cai_dat_tra_dung_trang_nhu_rest() {
    TestDatabase::run(async |test| {
        let app = build(test, |builder| builder).await;
        let tree = tree(&app).await;
        let all = walk(&app, &format!("/units/{}/settings?limit=200", tree.s1), "key").await;
        let uri = format!("/units/{}/settings?limit=1&count=true", tree.s1);
        let rest = expect(&app, "GET", &uri, "admin", None, StatusCode::OK).await;
        let client = app.mcp("admin").await.unwrap();
        let first = client
            .call_tool(tool(
                "organizations.get_settings",
                json!({"id": tree.s1, "limit": 1, "count": true}),
            ))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(first["data"], rest["data"]);
        assert_eq!(first["pagination"], rest["pagination"]);
        let mut keys = vec![first["data"][0]["key"].as_str().unwrap().to_owned()];
        let mut cursor = first["pagination"]["next_cursor"].clone();
        while let Some(next) = cursor.as_str() {
            let arguments = json!({"id": tree.s1, "limit": 1, "cursor": next});
            let page = client
                .call_tool(tool("organizations.get_settings", arguments))
                .await
                .unwrap()
                .structured_content
                .unwrap();
            keys.extend(
                page["data"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| s["key"].as_str().unwrap().to_owned()),
            );
            cursor = page["pagination"]["next_cursor"].clone();
        }
        assert_eq!(keys, all);
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
