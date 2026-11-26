//! Danh sách của `auth` đổi sang `Page<T>` (bước 70 P9 mục 3, D-20261125-list-pages): tool MCP nhận cùng tham số
//! trang với REST và trả đúng trang, đúng con trỏ như REST. Chạy trên SQLite và Postgres.

use autoxide_kernel::event::Actor;
use autoxide_testkit::{TestDatabase, tool};
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::http::test_support::{PASSWORD, app, bearer, call};
use crate::role::RoleInput;
use crate::user::NewUser;

fn names(page: &Value) -> Vec<String> {
    page["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn tool_danh_sach_tra_dung_trang_nhu_rest() {
    TestDatabase::run(async |test| {
        let (app, services) = app(test).await;
        let tenant = &services.tenant;
        let boss = NewUser {
            email: "boss@example.com".to_owned(),
            name: "Boss".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        services.users.create_with_roles(tenant, Actor::system(), boss, true).await.unwrap();
        let ada =
            services.users.find_by_email(tenant, "ada@example.com").await.unwrap().unwrap().id;
        let mut held = Vec::new();
        for name in ["delta", "alpha", "charlie"] {
            let input = RoleInput {
                denies: Vec::new(),
                requires_mfa: false,
                name: name.to_owned(),
                description: String::new(),
                permissions: vec!["notes.write".to_owned()],
                grants: Vec::new(),
            };
            held.push(services.roles.create(tenant, Actor::system(), input).await.unwrap().id);
        }
        services.roles.set_user_roles(tenant, Actor::system(), ada, held).await.unwrap();
        let session = bearer(&app, "boss@example.com", PASSWORD).await;
        let body = json!({"name": "agent", "current_password": PASSWORD,
                          "permissions": ["auth.users.read", "auth.roles.read"]});
        let (status, _, created) =
            call(&app, "POST", "/api-keys", Some(&session), Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let key = created["data"]["key"].as_str().unwrap().to_owned();
        let with_key = format!("Bearer {key}");
        let client = app.mcp(&key).await.unwrap();
        let tool_page = async |name: &str, input: Value| -> Value {
            client.call_tool(tool(name, input)).await.unwrap().structured_content.unwrap()
        };

        // Role của người dùng, theo tên.
        let path = format!("/users/{ada}/roles?limit=2&count=true");
        let (status, _, rest) = call(&app, "GET", &path, Some(&with_key), None).await;
        assert_eq!(status, StatusCode::OK, "{rest}");
        let first =
            tool_page("auth.get_user_roles", json!({"id": ada, "limit": 2, "count": true})).await;
        assert_eq!(first["data"], rest["data"]);
        assert_eq!(first["pagination"], rest["pagination"]);
        assert_eq!(first["pagination"]["total"], 3);
        let cursor = first["pagination"]["next_cursor"].clone();
        assert!(cursor.is_string());
        let second =
            tool_page("auth.get_user_roles", json!({"id": ada, "limit": 2, "cursor": cursor}))
                .await;
        assert_eq!([names(&first), names(&second)].concat(), ["alpha", "charlie", "delta"]);
        assert!(second["pagination"]["next_cursor"].is_null());

        // Danh mục permission, theo tên; `order=desc` đảo.
        let (_, _, rest) =
            call(&app, "GET", "/permissions?limit=3&order=desc", Some(&with_key), None).await;
        let listed = tool_page("auth.list_permissions", json!({"limit": 3, "order": "desc"})).await;
        assert_eq!(listed["data"], rest["data"]);
        assert_eq!(listed["pagination"], rest["pagination"]);
        let mut sorted = names(&listed);
        sorted.sort();
        sorted.reverse();
        assert_eq!(names(&listed), sorted);

        // Số người theo role, cùng thứ tự với danh sách role.
        let (_, _, rest) = call(&app, "GET", "/roles/counts?limit=2", Some(&with_key), None).await;
        let counts = tool_page("auth.count_role_members", json!({"limit": 2})).await;
        assert_eq!(counts["data"], rest["data"]);
        assert_eq!(counts["pagination"], rest["pagination"]);
        let (_, _, roles) = call(&app, "GET", "/roles?limit=2", Some(&with_key), None).await;
        let ids = |page: &Value, field: &str| -> Vec<String> {
            page["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item[field].as_str().unwrap().to_owned())
                .collect()
        };
        assert_eq!(ids(&counts, "role_id"), ids(&roles, "id"));

        // Tham số trang sai: cùng một lỗi ở REST và tool.
        let (status, _, wrong) =
            call(&app, "GET", "/permissions?limit=0", Some(&with_key), None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let refused = tool_page("auth.list_permissions", json!({"limit": 0})).await;
        assert_eq!(refused["errors"], wrong["errors"]);
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
