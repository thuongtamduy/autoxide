//! Danh sách trả `Page<T>` (bước 70 P9, D-20261125-list-pages): lịch của một mục, menu, vị trí hiển thị, mẫu
//! trang bắt buộc và feed công khai — trang nhỏ hơn số dòng, theo `next_cursor` tới hết không trùng không sót,
//! hai chiều; tool `content.legal_templates` trả đúng trang REST trả.

use super::*;

/// Duyệt hết danh sách `first` (đã có `?…limit=`) theo `next_cursor`; trả trường `field` của từng dòng.
async fn walk(app: &TestApp, first: &str, who: Option<&str>, field: &str) -> Vec<String> {
    let mut seen = Vec::new();
    let mut uri = first.to_owned();
    loop {
        let answer = send(app, "GET", &uri, who, &[], None).await;
        assert_eq!(answer.status(), StatusCode::OK, "{uri}: {}", answer.json());
        let body = answer.json();
        seen.extend(
            body["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row[field].as_str().unwrap().to_owned()),
        );
        match body["pagination"]["next_cursor"].as_str() {
            Some(cursor) => uri = format!("{first}&cursor={cursor}"),
            None => return seen,
        }
    }
}

fn reversed(items: &[String]) -> Vec<String> {
    items.iter().rev().cloned().collect()
}

/// Menu theo khoá, vị trí hiển thị theo khoá, mẫu trang theo khoá: trang 2 dòng, hai chiều.
#[tokio::test]
async fn menu_vi_tri_va_mau_trang_phan_trang_theo_khoa() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let mut keys = Vec::new();
        for key in ["main", "footer", "aside", "social", "legal"] {
            let menu = json!({"items": [{"label": "Home", "url": "/"}]});
            let uri = format!("/menus/{key}?locale=en");
            let put =
                send(&app, "PUT", &uri, Some("admin"), &[("if-match", "*")], Some(menu)).await;
            assert_eq!(put.status(), StatusCode::OK, "{}", put.json());
            keys.push(key.to_owned());
        }
        keys.sort();
        assert_eq!(walk(&app, "/menus?limit=2", Some("reader"), "key").await, keys);
        assert_eq!(
            walk(&app, "/menus?limit=2&order=desc", Some("reader"), "key").await,
            reversed(&keys)
        );

        let presets = send(&app, "POST", "/placements/presets", Some("admin"), &[], None).await;
        assert_eq!(presets.status(), StatusCode::OK, "{}", presets.json());
        let placements = walk(&app, "/placements?limit=2", Some("admin"), "key").await;
        let mut sorted = placements.clone();
        sorted.sort();
        sorted.dedup();
        assert!(placements.len() > 2);
        assert_eq!(placements, sorted);
        let down = walk(&app, "/placements?limit=2&order=desc", Some("admin"), "key").await;
        assert_eq!(down, reversed(&placements));

        let templates = walk(&app, "/templates?limit=3", Some("reader"), "key").await;
        let mut sorted = templates.clone();
        sorted.sort();
        sorted.dedup();
        assert!(templates.len() > 3);
        assert_eq!(templates, sorted);
        let wrong = send(&app, "GET", "/templates?cursor=nope", Some("reader"), &[], None).await;
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

/// Tool `content.legal_templates` và `GET /api/v1/content/templates` cùng tham số trả cùng trang và cùng
/// `next_cursor`; theo `next_cursor` của tool được trang sau như REST.
#[tokio::test]
async fn tool_mau_trang_tra_dung_trang_nhu_rest() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let rest = send(&app, "GET", "/templates?limit=2&order=desc", Some("reader"), &[], None)
            .await
            .json();
        let client = app.mcp("reader").await.unwrap();
        let call = async |arguments: Value| {
            let result =
                client.call_tool(tool("content.legal_templates", arguments)).await.unwrap();
            result.structured_content.unwrap()
        };
        let first = call(json!({"limit": 2, "order": "desc"})).await;
        assert_eq!(first["data"], rest["data"]);
        assert_eq!(first["pagination"], rest["pagination"]);
        let cursor = first["pagination"]["next_cursor"].as_str().unwrap().to_owned();
        let second = call(json!({"limit": 2, "order": "desc", "cursor": cursor})).await;
        let uri = format!("/templates?limit=2&order=desc&cursor={cursor}");
        let rest_second = send(&app, "GET", &uri, Some("reader"), &[], None).await.json();
        assert_eq!(second["data"], rest_second["data"]);
        assert_ne!(second["data"], first["data"]);
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

/// Lịch của một mục, mới nhất trước; feed công khai, mới đăng trước: trang 2 dòng, hai chiều.
#[tokio::test]
async fn lich_cua_muc_va_feed_phan_trang() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let created = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "post", "title": "Lịch"})),
        )
        .await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let run_at =
            json!(autoxide_kernel::time::Timestamp::now() + jiff::SignedDuration::from_hours(48));
        let mut schedules = Vec::new();
        // Mỗi hành động chỉ một lịch chờ: tạo rồi huỷ để có nhiều lịch.
        for _ in 0..5 {
            let body = json!({"action": "publish", "run_at": run_at});
            let uri = format!("/entries/{id}/schedules");
            let made =
                send(&app, "POST", &uri, Some("admin"), &[("if-match", "*")], Some(body)).await;
            assert_eq!(made.status(), StatusCode::CREATED, "{}", made.json());
            let schedule = made.json()["data"]["id"].as_str().unwrap().to_owned();
            let canceled =
                send(&app, "DELETE", &format!("/schedules/{schedule}"), Some("admin"), &[], None)
                    .await;
            assert_eq!(canceled.status(), StatusCode::OK, "{}", canceled.json());
            schedules.push(schedule);
        }
        let newest_first = reversed(&schedules);
        let first = format!("/entries/{id}/schedules?limit=2");
        assert_eq!(walk(&app, &first, Some("reader"), "id").await, newest_first);
        let first = format!("/entries/{id}/schedules?limit=2&order=asc");
        assert_eq!(walk(&app, &first, Some("reader"), "id").await, schedules);

        let mut posts = Vec::new();
        for title in ["One", "Two", "Three", "Four", "Five"] {
            let made = send(
                &app,
                "POST",
                "/entries",
                Some("admin"),
                &[],
                Some(json!({"kind": "post", "title": title})),
            )
            .await;
            let post = made.json()["data"]["id"].as_str().unwrap().to_owned();
            let uri = format!("/entries/{post}/publish");
            let published =
                send(&app, "POST", &uri, Some("admin"), &[("if-match", "*")], None).await;
            assert_eq!(published.status(), StatusCode::OK, "{}", published.json());
            posts.push(post);
        }
        let feed = walk(&app, "/public/feed?locale=en&limit=2", None, "id").await;
        assert_eq!(feed, reversed(&posts));
        let feed = walk(&app, "/public/feed?locale=en&limit=2&order=asc", None, "id").await;
        assert_eq!(feed, posts);
        // Lỗi tham số trang và lỗi `locale` báo một lần (D-083).
        let both = send(&app, "GET", "/public/feed?locale=fr&limit=0", None, &[], None).await;
        assert_eq!(both.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let problem = both.json();
        let fields: Vec<&str> = problem["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["field"].as_str().unwrap())
            .collect();
        assert!(fields.contains(&"locale") && fields.contains(&"limit"), "{fields:?}");
    })
    .await
    .unwrap();
}
