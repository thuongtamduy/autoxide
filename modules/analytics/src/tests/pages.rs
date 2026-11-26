//! Bảng xếp hạng và sổ khai báo của `analytics` trả theo trang (bước 70 P9, D-20261125-list-pages): nhiều nhất
//! trước, cùng số theo giá trị; theo `next_cursor` tới hết không trùng không sót, hai chiều; tool
//! `analytics.top_pages` cùng trang với REST.

use autoxide_testkit::{TestDatabase, tool};
use axum::http::StatusCode;
use serde_json::{Value, json};

use super::{CHROME, at, browser, harness, page};

const RANGE: &str = "from=2026-11-01&to=2026-11-30";

#[tokio::test]
async fn xep_hang_phan_trang_nhieu_nhat_truoc_va_tool_nhu_rest() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        // /a: 3 lượt, /c và /d: 2 (hoà), /b và /e: 1 (hoà).
        let events: Vec<Value> =
            ["/a", "/a", "/a", "/c", "/c", "/d", "/d", "/b", "/e"].into_iter().map(page).collect();
        h.send(None, &browser("203.0.113.1", CHROME), None, events, at(10, 9)).await;
        h.service.rollup_tenant(&h.tenant, at(10, 12)).await.unwrap();
        let app = &h.app;
        let walk_by = async |limit: u32, query: &str| {
            let mut values = Vec::new();
            let mut uri = format!("/api/v1/analytics/pages?{RANGE}&limit={limit}{query}");
            loop {
                let answer = app.get(&uri).token("boss").send().await.unwrap();
                assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
                let body = answer.json();
                let rows = body["data"].as_array().unwrap();
                assert!(rows.len() <= limit as usize);
                values.extend(rows.iter().map(|row| row["value"].as_str().unwrap().to_owned()));
                match body["pagination"]["next_cursor"].as_str() {
                    Some(cursor) => {
                        uri = format!(
                            "/api/v1/analytics/pages?{RANGE}&limit={limit}{query}&cursor={cursor}"
                        );
                    }
                    None => return values,
                }
            }
        };
        let up = walk_by(2, "").await;
        assert_eq!(up, ["/a", "/c", "/d", "/b", "/e"]);
        let down = walk_by(2, "&order=desc").await;
        assert_eq!(down, up.iter().rev().cloned().collect::<Vec<_>>());
        // Keyset trong SQL (`HAVING` trên tổng): mọi cỡ trang, kể cả trang cắt giữa hai giá trị hoà, ghép lại
        // đúng bằng một trang lớn.
        for limit in [1, 3, 4] {
            assert_eq!(walk_by(limit, "").await, up, "limit {limit}");
            assert_eq!(walk_by(limit, "&order=desc").await, down, "limit {limit} desc");
        }
        assert_eq!(walk_by(200, "").await, up);
        let forged = autoxide_kernel::pagination::text_key_cursor("rank", "not-a-number");
        let tampered = app
            .get(&format!("/api/v1/analytics/pages?{RANGE}&cursor={forged}"))
            .token("boss")
            .send();
        assert_eq!(tampered.await.unwrap().status(), StatusCode::UNPROCESSABLE_ENTITY);
        let wrong =
            app.get(&format!("/api/v1/analytics/pages?{RANGE}&limit=0")).token("boss").send();
        assert_eq!(wrong.await.unwrap().status(), StatusCode::UNPROCESSABLE_ENTITY);
        // Không có quyền: 403 trước lỗi tham số trang.
        let denied =
            app.get(&format!("/api/v1/analytics/pages?{RANGE}&limit=0")).token("buyer").send();
        assert_eq!(denied.await.unwrap().status(), StatusCode::FORBIDDEN);

        // Tool cùng tham số: cùng `data` và `pagination`, rồi theo `next_cursor` của tool tới hết.
        let rest =
            app.get(&format!("/api/v1/analytics/pages?{RANGE}&limit=2&count=true")).token("boss");
        let rest = rest.send().await.unwrap().json();
        let client = app.mcp("boss").await.unwrap();
        let arguments =
            json!({"from": "2026-11-01", "to": "2026-11-30", "limit": 2, "count": true});
        let first = client
            .call_tool(tool("analytics.top_pages", arguments))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(first["data"], rest["data"]);
        assert_eq!(first["pagination"], rest["pagination"]);
        assert_eq!(first["pagination"]["total"], 5);
        let mut seen: Vec<String> = first["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["value"].as_str().unwrap().to_owned())
            .collect();
        let mut cursor = first["pagination"]["next_cursor"].clone();
        while let Some(next) = cursor.as_str() {
            let arguments =
                json!({"from": "2026-11-01", "to": "2026-11-30", "limit": 2, "cursor": next});
            let page = client
                .call_tool(tool("analytics.top_pages", arguments))
                .await
                .unwrap()
                .structured_content
                .unwrap();
            seen.extend(
                page["data"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| r["value"].as_str().unwrap().to_owned()),
            );
            cursor = page["pagination"]["next_cursor"].clone();
        }
        assert_eq!(seen, up);
        client.cancel().await.unwrap();

        // Sổ loại event (công khai) và phễu: theo khoá, trang 1 tới hết.
        let mut keys = Vec::new();
        let mut uri = "/api/v1/analytics/event-types?limit=1".to_owned();
        loop {
            let body = app.get(&uri).send().await.unwrap().json();
            keys.extend(
                body["data"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| e["key"].as_str().unwrap().to_owned()),
            );
            match body["pagination"]["next_cursor"].as_str() {
                Some(cursor) => {
                    uri = format!("/api/v1/analytics/event-types?limit=1&cursor={cursor}")
                }
                None => break,
            }
        }
        let mut sorted = keys.clone();
        sorted.sort();
        sorted.dedup();
        assert!(keys.len() > 1);
        assert_eq!(keys, sorted);
        let funnels =
            app.get("/api/v1/analytics/funnels?count=true").token("boss").send().await.unwrap();
        assert_eq!(funnels.json()["pagination"]["total"], 1);
    })
    .await
    .unwrap();
}
