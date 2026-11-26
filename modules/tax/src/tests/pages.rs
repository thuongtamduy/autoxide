//! Danh sách dữ liệu luật của `tax` trả theo trang (bước 70 P9, D-20261125-list-pages): theo `next_cursor` tới
//! hết đúng bằng cả danh sách, không trùng, đúng thứ tự cũ, hai chiều; tool `tax.rates` cùng trang với REST.

use super::*;

/// Duyệt hết `uri` (đã có `?…limit=…`) theo `next_cursor`; trả các dòng theo thứ tự nhận.
async fn walk(app: &TestApp, uri: &str) -> Vec<Value> {
    let mut rows = Vec::new();
    let mut next = uri.to_owned();
    loop {
        let (status, page) = call(app, "GET", &next, "ops", None).await;
        assert_eq!(status, StatusCode::OK, "{next}: {page}");
        let limit = page["pagination"]["limit"].as_u64().unwrap();
        let data = page["data"].as_array().unwrap();
        assert!(data.len() as u64 <= limit);
        rows.extend(data.iter().cloned());
        match page["pagination"]["next_cursor"].as_str() {
            Some(cursor) => next = format!("{uri}&cursor={cursor}"),
            None => return rows,
        }
    }
}

fn ids(rows: &[Value]) -> Vec<String> {
    rows.iter().map(|row| row["id"].as_str().unwrap().to_owned()).collect()
}

#[tokio::test]
async fn du_lieu_luat_phan_trang_dung_thu_tu_cu() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        // Thuế suất: trang 3 dòng tới hết = trang 200 dòng; theo (khu vực, nhóm, tầng, ngày bắt đầu).
        let all = walk(&app, "/rates?limit=200").await;
        assert!(all.len() > 3, "the bundled pack has more than one page of rates");
        let paged = walk(&app, "/rates?limit=3").await;
        assert_eq!(ids(&paged), ids(&all));
        let mut unique = ids(&all);
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), all.len(), "no rate twice");
        let tuple = |row: &Value| {
            (
                row["jurisdiction"].as_str().unwrap().to_owned(),
                row["category"].as_str().unwrap().to_owned(),
                row["stage"].as_i64().unwrap(),
                row["effective_from"].as_str().unwrap().to_owned(),
            )
        };
        assert!(all.windows(2).all(|pair| tuple(&pair[0]) <= tuple(&pair[1])), "old order kept");
        let down = walk(&app, "/rates?limit=3&order=desc").await;
        assert_eq!(ids(&down), ids(&all).into_iter().rev().collect::<Vec<_>>());
        // Bộ lọc vẫn áp, cùng trang.
        let filtered = walk(&app, "/rates?category=vat.standard&limit=1").await;
        assert!(!filtered.is_empty());
        assert!(filtered.iter().all(|row| row["category"] == "vat.standard"));

        // Nhóm thuế, khu vực, quy tắc, khấu trừ thay: trang 2 tới hết = cả danh sách.
        for (uri, field) in [
            ("/categories", "code"),
            ("/jurisdictions", "code"),
            ("/rules", "id"),
            ("/withholding-rates", "id"),
        ] {
            let whole = walk(&app, &format!("{uri}?limit=200")).await;
            let paged = walk(&app, &format!("{uri}?limit=2")).await;
            let keys = |rows: &[Value]| -> Vec<String> {
                rows.iter().map(|row| row[field].as_str().unwrap().to_owned()).collect()
            };
            assert_eq!(keys(&paged), keys(&whole), "{uri}");
        }
        // Gói đã nhập: mới nhất trước, có `count`.
        let (status, packs) = call(&app, "GET", "/packs?count=true", "ops", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(packs["pagination"]["total"], 1);
        assert_eq!(packs["data"][0]["code"], "vn");
        let (status, wrong) = call(&app, "GET", "/rates?cursor=nope", "ops", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(wrong["errors"][0]["field"], "cursor");
    })
    .await
    .unwrap();
}

/// Tool `tax.rates` cùng tham số với `GET /rates?jurisdiction=VN&on=…` trả cùng `pagination` và cùng dòng (dạng
/// gọn của tool), rồi theo `next_cursor` của tool tới hết.
#[tokio::test]
async fn tool_thue_suat_tra_dung_trang_nhu_rest() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let client = app.mcp("ops").await.unwrap();
        let (_, rest) = call(
            &app,
            "GET",
            "/rates?jurisdiction=VN&on=2026-06-01&limit=2&count=true",
            "ops",
            None,
        )
        .await;
        let arguments =
            json!({"jurisdiction": "VN", "on": "2026-06-01", "limit": 2, "count": true});
        let first = client
            .call_tool(autoxide_testkit::tool("tax.rates", arguments))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(first["pagination"], rest["pagination"]);
        let short = |rows: &Value| -> Vec<(String, i64, String)> {
            rows.as_array()
                .unwrap()
                .iter()
                .map(|row| {
                    (
                        row["category"].as_str().unwrap().to_owned(),
                        row["stage"].as_i64().unwrap(),
                        row["effective_from"].as_str().unwrap().to_owned(),
                    )
                })
                .collect()
        };
        assert_eq!(short(&first["data"]), short(&rest["data"]));
        let all = walk(&app, "/rates?jurisdiction=VN&on=2026-06-01&limit=200").await;
        let mut seen = short(&first["data"]);
        let mut cursor = first["pagination"]["next_cursor"].clone();
        while let Some(next) = cursor.as_str() {
            let arguments =
                json!({"jurisdiction": "VN", "on": "2026-06-01", "limit": 2, "cursor": next});
            let page = client
                .call_tool(autoxide_testkit::tool("tax.rates", arguments))
                .await
                .unwrap()
                .structured_content
                .unwrap();
            seen.extend(short(&page["data"]));
            cursor = page["pagination"]["next_cursor"].clone();
        }
        assert_eq!(seen, short(&Value::Array(all)));
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
