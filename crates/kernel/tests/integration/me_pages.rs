//! Danh sách "của tôi" trả theo trang (bước 70 P9, D-20261125-list-pages): `/me/flags` theo khoá, `/me/pins`
//! theo thứ tự hiển thị, `/me/views` mới nhất trước, `/me/permissions` theo tên — theo `next_cursor` tới hết
//! không trùng không sót, hai chiều; lỗi `prefix` và lỗi tham số trang báo một lần. Cả SQLite và Postgres.

use autoxide_kernel::tenancy::DEFAULT_TENANT;
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Query};
use serde_json::{Value, json};
use uuid::Uuid;

/// Duyệt hết `uri` (đã có `?limit=…`) theo `next_cursor`; trả trường `field` của mỗi dòng.
async fn walk(app: &TestApp, uri: &str, who: &str, field: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut next = uri.to_owned();
    loop {
        let answer = app.get(&next).token(who).send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{next}: {}", answer.text());
        let page = answer.json();
        let limit = page["pagination"]["limit"].as_u64().unwrap();
        let rows = page["data"].as_array().unwrap();
        assert!(rows.len() as u64 <= limit);
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
async fn danh_sach_cua_toi_phan_trang() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .user("ada", 1, &["kernel.views.share", "kernel.tenants.read"])
            .user("bob", 2, &[])
            .build()
            .await
            .unwrap();

        // Cờ: theo khoá, chỉ của chính mình.
        for key in ["hint.c", "hint.a", "whats_new", "hint.b", "hint_x"] {
            let set = app.request(Method::PUT, &format!("/api/v1/me/flags/{key}")).token("ada");
            assert_eq!(
                set.json(json!({"value": "1"})).send().await.unwrap().status(),
                StatusCode::OK
            );
        }
        let other = app.request(Method::PUT, "/api/v1/me/flags/hint.bob").token("bob");
        other.json(json!({"value": "1"})).send().await.unwrap();
        let keys = walk(&app, "/api/v1/me/flags?limit=2", "ada", "key").await;
        assert_eq!(keys, ["hint.a", "hint.b", "hint.c", "hint_x", "whats_new"]);
        assert_eq!(
            walk(&app, "/api/v1/me/flags?limit=2&order=desc", "ada", "key").await,
            reversed(&keys)
        );
        assert_eq!(
            walk(&app, "/api/v1/me/flags?prefix=hint.&limit=1", "ada", "key").await,
            ["hint.a", "hint.b", "hint.c"]
        );
        let both =
            app.get("/api/v1/me/flags?prefix=Bad&limit=0").token("ada").send().await.unwrap();
        assert_eq!(both.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let mut fields: Vec<String> = both.json()["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|error| error["field"].as_str().unwrap().to_owned())
            .collect();
        fields.sort();
        assert_eq!(fields, ["limit", "prefix"], "every problem at once");

        // Ghim: theo thứ tự hiển thị (ghim sau đứng sau).
        let mut pinned = Vec::new();
        for n in 1..=5 {
            let target = format!("content.entry:0190a4c2-0000-7000-8000-00000000000{n}");
            let body = json!({"kind": "record", "target": target, "label": format!("Pin {n}")});
            let made = app.post("/api/v1/me/pins").token("ada").json(body).send().await.unwrap();
            assert_eq!(made.status(), StatusCode::CREATED, "{}", made.text());
            pinned.push(made.json()["data"]["id"].as_str().unwrap().to_owned());
        }
        assert_eq!(walk(&app, "/api/v1/me/pins?limit=2", "ada", "id").await, pinned);
        assert_eq!(
            walk(&app, "/api/v1/me/pins?limit=2&order=desc", "ada", "id").await,
            reversed(&pinned)
        );
        assert!(walk(&app, "/api/v1/me/pins?limit=2", "bob", "id").await.is_empty());

        // View đã lưu: mới nhất trước; view riêng của người khác không lộ, view chung thì thấy.
        let ada = Uuid::from_u128(1);
        let bob = Uuid::from_u128(2);
        let mut mine = Vec::new();
        for (n, (owner, scope)) in [
            (ada, "personal"),
            (bob, "personal"),
            (ada, "personal"),
            (bob, "tenant"),
            (ada, "personal"),
        ]
        .into_iter()
        .enumerate()
        {
            let id = Uuid::now_v7();
            let now = Timestamp::now();
            let insert = Query::insert()
                .into_table(Alias::new("kernel_saved_views"))
                .columns(
                    [
                        "id",
                        "tenant_id",
                        "resource",
                        "name",
                        "scope",
                        "owner_id",
                        "params",
                        "columns",
                        "created_at",
                        "updated_at",
                    ]
                    .map(Alias::new),
                )
                .values_panic([
                    id.into(),
                    DEFAULT_TENANT.as_uuid().into(),
                    "demo.list_things".into(),
                    format!("View {n}").into(),
                    scope.into(),
                    owner.into(),
                    "{}".into(),
                    "[]".into(),
                    now.into(),
                    now.into(),
                ])
                .to_owned();
            app.database().write().execute(&insert).await.unwrap();
            if owner == ada || scope == "tenant" {
                mine.push(id.to_string());
            }
        }
        let newest = reversed(&mine);
        assert_eq!(walk(&app, "/api/v1/me/views?limit=2", "ada", "id").await, newest);
        assert_eq!(walk(&app, "/api/v1/me/views?limit=3&order=asc", "ada", "id").await, mine);
        assert_eq!(
            walk(&app, "/api/v1/me/views?resource=demo.list_things&limit=1", "ada", "id").await,
            newest
        );
        assert!(
            walk(&app, "/api/v1/me/views?resource=other.list&limit=1", "ada", "id")
                .await
                .is_empty()
        );
        assert_eq!(walk(&app, "/api/v1/me/views?limit=2", "bob", "id").await.len(), 2);

        // Permission của tôi: theo tên.
        let names = walk(&app, "/api/v1/me/permissions?limit=1", "ada", "name").await;
        assert_eq!(names, ["kernel.tenants.read", "kernel.views.share"]);
        let counted =
            app.get("/api/v1/me/permissions?count=true").token("ada").send().await.unwrap();
        let counted: Value = counted.json();
        assert_eq!(counted["pagination"]["total"], 2);
    })
    .await
    .unwrap();
}
