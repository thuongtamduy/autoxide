//! Danh sách có `sort` (bước 70 P9, D-20261125-list-pages): mặc định của `order` theo cột sắp, và tool MCP
//! `media.list` trả đúng trang REST trả — mẫu cho tool danh sách của các module khác.

use super::*;

fn names(page: &Value) -> Vec<String> {
    page["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap().to_owned())
        .collect()
}

/// `sort=name` không gửi `order`: A→Z (trước đây theo mặc định của `PageQuery` thành Z→A); `order` gửi thì giữ;
/// `sort=created_at` (mặc định) vẫn mới nhất trước. Nhãn mặc định theo tên: A→Z.
#[tokio::test]
async fn mac_dinh_cua_order_theo_sort() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let auth = as_user("reader");
        let headers = [("authorization", auth.as_str())];
        let admin = as_user("admin");
        for (index, name) in ["b.png", "a.png", "c.png"].into_iter().enumerate() {
            let size = u32::try_from(index).unwrap() + 1;
            let uri = format!("?name={name}&tags=t{name}");
            let upload_headers = [("authorization", admin.as_str()), ("content-type", "image/png")];
            let answer = send(&app, "POST", &uri, &upload_headers, samples::png(size, size)).await;
            assert_eq!(answer.status(), StatusCode::CREATED, "{}", answer.json());
        }
        let list = async |uri: &str| {
            let answer = send(&app, "GET", uri, &headers, Vec::new()).await;
            assert_eq!(answer.status(), StatusCode::OK, "{}", answer.json());
            answer.json()
        };
        assert_eq!(names(&list("?sort=name").await), ["a.png", "b.png", "c.png"]);
        assert_eq!(names(&list("?sort=name&order=desc").await), ["c.png", "b.png", "a.png"]);
        assert_eq!(names(&list("").await), ["c.png", "a.png", "b.png"], "newest first");
        assert_eq!(names(&list("?sort=created_at&order=asc").await), ["b.png", "a.png", "c.png"]);
        let tags = list("/tags").await;
        let tags: Vec<&str> = tags["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tag| tag["name"].as_str().unwrap())
            .collect();
        assert_eq!(tags, ["ta.png", "tb.png", "tc.png"]);
    })
    .await
    .unwrap();
}

/// Tool `media.list` và `GET /api/v1/media` cùng tham số trả cùng trang và cùng `next_cursor`; theo `next_cursor`
/// của tool tới hết danh sách được đúng phần còn lại.
#[tokio::test]
async fn tool_danh_sach_tra_dung_trang_nhu_rest() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        for (index, name) in ["b.png", "a.png", "c.png"].into_iter().enumerate() {
            let size = u32::try_from(index).unwrap() + 1;
            upload(&app, name, "image/png", &samples::png(size, size)).await;
        }
        let auth = as_user("reader");
        let rest = send(
            &app,
            "GET",
            "?sort=name&limit=2",
            &[("authorization", auth.as_str())],
            Vec::new(),
        )
        .await
        .json();
        let client = app.http.mcp("reader").await.unwrap();
        let first = client
            .call_tool(tool("media.list", json!({"sort": "name", "limit": 2})))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(first["data"], rest["data"]);
        assert_eq!(first["pagination"], rest["pagination"]);
        let cursor = first["pagination"]["next_cursor"].clone();
        assert!(cursor.is_string());
        let second = client
            .call_tool(tool("media.list", json!({"sort": "name", "limit": 2, "cursor": cursor})))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(names(&second), ["c.png"]);
        assert!(second["pagination"]["next_cursor"].is_null());
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
