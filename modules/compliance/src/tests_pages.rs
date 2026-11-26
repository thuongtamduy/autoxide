//! Danh sách có phân trang (bước 70 P9 mục 3, D-20261125-list-pages): văn bản (keyset theo loại), số chấp nhận
//! theo phiên bản (keyset trên `GROUP BY`), đồng ý của tôi; sổ mục đích, thời hạn lưu, nhà cung cấp mã theo dõi
//! (`page_of`); tool MCP trả đúng trang REST trả.

use autoxide_kernel::tenancy::DEFAULT_TENANT;
use autoxide_testkit::mcp::CallToolRequestParams;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{AcceptanceGate, Subject};
use crate::tests::call;

const ADMIN: &[&str] = &[
    "compliance.documents.read",
    "compliance.documents.manage",
    "compliance.privacy.read",
    "compliance.tracking.manage",
    "compliance.policies.manage",
    "compliance.rules.read",
];

async fn app(test: &TestDatabase) -> TestApp {
    TestApp::builder(test)
        .user("admin", 1, ADMIN)
        .user("alice", 2, &[])
        .user("bob", 3, &[])
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

fn tool(name: &str, arguments: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.to_owned())
        .with_arguments(arguments.as_object().unwrap().clone())
}

/// Theo `next_cursor` của `uri` (đã có `limit`, có `?`) tới hết; trả trường `field` của mọi dòng theo thứ tự.
async fn walk(app: &TestApp, uri: &str, who: &str, field: &str) -> Vec<Value> {
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page_uri = match &cursor {
            Some(cursor) => format!("{uri}&cursor={cursor}"),
            None => uri.to_owned(),
        };
        let (status, page) = call(app, "GET", &page_uri, who, None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        seen.extend(page["data"].as_array().unwrap().iter().map(|row| row[field].clone()));
        match page["pagination"]["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => return seen,
        }
    }
}

#[tokio::test]
async fn van_ban_va_so_chap_nhan_theo_trang_tool_nhu_rest() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let mut kinds = Vec::new();
        for kind in ["terms_of_use", "privacy_policy", "general_conditions"] {
            let body = json!({"kind": kind, "title": kind});
            let (status, doc) = call(&app, "POST", "/documents", "admin", Some(body)).await;
            assert_eq!(status, StatusCode::CREATED, "{doc}");
            kinds.push(json!(kind));
        }
        kinds.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        assert_eq!(walk(&app, "/documents?limit=1", "admin", "kind").await, kinds);
        let mut reversed = kinds.clone();
        reversed.reverse();
        assert_eq!(walk(&app, "/documents?limit=2&order=desc", "admin", "kind").await, reversed);

        let (_, rest) = call(&app, "GET", "/documents?limit=2", "admin", None).await;
        let client = app.mcp("admin").await.unwrap();
        let first = client
            .call_tool(tool("compliance.documents", json!({"limit": 2})))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(first["data"], rest["data"]);
        assert_eq!(first["pagination"], rest["pagination"]);
        let second = client
            .call_tool(tool(
                "compliance.documents",
                json!({"limit": 2, "cursor": first["pagination"]["next_cursor"]}),
            ))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(second["data"][0]["kind"], kinds[2]);
        assert!(second["pagination"]["next_cursor"].is_null());
        client.cancel().await.unwrap();

        // Ba phiên bản, mỗi phiên bản một người chấp nhận: số theo phiên bản, keyset trên nhóm.
        let (_, docs) = call(&app, "GET", "/documents?limit=200", "admin", None).await;
        let terms = docs["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|doc| doc["kind"] == "terms_of_use")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let gate = app.api::<dyn AcceptanceGate>().unwrap();
        for (number, who) in [(1u128, 2u128), (2, 3), (3, 2)] {
            let version = json!({"locale": "vi", "title": "Điều khoản", "body": format!("Bản {number}"), "requires_reacceptance": true});
            let (status, _) =
                call(&app, "POST", &format!("/documents/{terms}/versions"), "admin", Some(version)).await;
            assert_eq!(status, StatusCode::CREATED);
            let subject = Subject::User(Uuid::from_u128(who));
            gate.accept_current(DEFAULT_TENANT, subject, "terms_of_use", "api").await.unwrap();
        }
        let stats = format!("/documents/{terms}/acceptance-stats?limit=1");
        assert_eq!(walk(&app, &stats, "admin", "version").await, vec![json!(1), json!(2), json!(3)]);
        assert_eq!(
            walk(&app, &format!("{stats}&order=desc"), "admin", "accepted").await,
            vec![json!(1), json!(1), json!(1)]
        );
        let (_, counted) = call(&app, "GET", &format!("{stats}&count=true"), "admin", None).await;
        assert_eq!(counted["pagination"]["total"], 3);
        let (status, _) = call(&app, "GET", &format!("{stats}&cursor=nope"), "admin", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dong_y_cua_toi_va_so_muc_dich_theo_trang() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        for purpose in ["service", "marketing", "analytics"] {
            let body =
                json!({"purpose": purpose, "granted": true, "text": "Đồng ý", "text_version": "1"});
            let (status, saved) = call(&app, "PUT", "/me/consents", "alice", Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{saved}");
        }
        assert_eq!(
            walk(&app, "/me/consents?limit=2", "alice", "purpose").await,
            vec![json!("analytics"), json!("marketing"), json!("service")]
        );
        let (_, others) = call(&app, "GET", "/me/consents", "bob", None).await;
        assert_eq!(others["data"], json!([]));

        // Sổ mục đích, thời hạn lưu, nhà cung cấp (`page_of`): từng trang ra đúng danh sách một trang lớn.
        for (uri, field) in
            [("/purposes", "key"), ("/retention", "name"), ("/tracking-providers", "key")]
        {
            let (status, all) = call(&app, "GET", &format!("{uri}?limit=200"), "admin", None).await;
            assert_eq!(status, StatusCode::OK, "{all}");
            let all: Vec<Value> =
                all["data"].as_array().unwrap().iter().map(|row| row[field].clone()).collect();
            assert!(all.len() > 1, "{uri}: {all:?}");
            assert_eq!(walk(&app, &format!("{uri}?limit=1"), "admin", field).await, all, "{uri}");
        }

        let (_, rest) = call(&app, "GET", "/purposes?limit=2&count=true", "admin", None).await;
        let client = app.mcp("admin").await.unwrap();
        let found = client
            .call_tool(tool("compliance.purposes", json!({"limit": 2, "count": true})))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(found["data"], rest["data"]);
        assert_eq!(found["pagination"], rest["pagination"]);
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
