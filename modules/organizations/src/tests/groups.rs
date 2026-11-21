//! Nhóm nút ngoài cây (bước 38, D-232) qua REST và MCP, trên cả SQLite và Postgres: nhóm không cấp
//! quyền, thêm lại không trùng, thêm đồng thời một dòng, nhóm của khách khác không thấy.

use autoxide_kernel::access::Scope;
use autoxide_testkit::{TestApp, TestDatabase, tool};
use axum::http::StatusCode;
use serde_json::{Value, json};

use super::{build, call, expect, id, org, tree};

const MANAGE: [&str; 2] = ["organizations.units.read", "organizations.groups.manage"];

async fn group(app: &TestApp, who: &str, name: &str) -> String {
    let body = json!({"name": name, "description": "For reports"});
    id(&expect(app, "POST", "/groups", who, Some(body), StatusCode::CREATED).await)
}

fn members(body: &Value) -> Vec<String> {
    let mut found: Vec<String> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|member| member["org_id"].as_str().unwrap().to_owned())
        .collect();
    found.sort();
    found
}

#[tokio::test]
async fn nhom_khong_cap_quyen_doc_nut() {
    TestDatabase::run(async |test| {
        let app = build(test, |builder| builder).await;
        let tree = tree(&app).await;
        let r1 = tree.r1.clone();
        let app = build(test, move |builder| {
            builder.user("boss", 9, &MANAGE).member(
                "north",
                5,
                org(&r1),
                &[("organizations.units.read", Scope::Subtree)],
            )
        })
        .await;
        let north = group(&app, "boss", "Stores in the north").await;
        let body = json!({"org_ids": [tree.r1, tree.r2, tree.s1]});
        let path = format!("/groups/{north}/members");
        let added = expect(&app, "POST", &path, "boss", Some(body), StatusCode::OK).await;
        assert_eq!(added["data"]["added"], 3);

        // Người chỉ có nhánh R1: thấy R1 và S1, R2 chỉ là một con số.
        let seen =
            expect(&app, "GET", &format!("/groups/{north}"), "north", None, StatusCode::OK).await;
        assert_eq!(seen["data"]["member_count"], 2, "{seen}");
        assert_eq!(seen["data"]["excluded_org_count"], 1, "{seen}");
        let listed = expect(&app, "GET", &path, "north", None, StatusCode::OK).await;
        let mut want = vec![tree.r1.clone(), tree.s1.clone()];
        want.sort();
        assert_eq!(members(&listed), want);
        assert!(!listed.to_string().contains(&tree.r2), "không lộ ID nút ngoài phạm vi");
        // Nhóm không mở nút R2.
        let (status, _) = call(&app, "GET", &format!("/units/{}", tree.r2), "north", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Không có `organizations.groups.manage` thì không quản lý được (kể cả có nhánh).
        let (status, body) =
            call(&app, "POST", "/groups", "north", Some(json!({"name": "x"}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        // MCP: cùng phạm vi.
        let client = app.mcp("north").await.unwrap();
        let result = client.call_tool(tool("organizations.list_groups", json!({}))).await.unwrap();
        let data = result.structured_content.unwrap();
        assert_eq!(data["data"][0]["member_count"], 2, "{data}");
        assert_eq!(data["data"][0]["excluded_org_count"], 1, "{data}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn them_thanh_vien_goi_lai_khong_trung() {
    TestDatabase::run(async |test| {
        let app = build(test, |builder| builder.user("boss", 9, &MANAGE)).await;
        let tree = tree(&app).await;
        let north = group(&app, "boss", "Miền Bắc").await;
        let path = format!("/groups/{north}/members");
        let body = json!({"org_ids": [tree.r1, tree.s1]});
        let first = expect(&app, "POST", &path, "boss", Some(body.clone()), StatusCode::OK).await;
        assert_eq!(
            (first["data"]["added"].as_u64(), first["data"]["member_count"].as_u64()),
            (Some(2), Some(2))
        );
        let again = expect(&app, "POST", &path, "boss", Some(body), StatusCode::OK).await;
        assert_eq!(
            (again["data"]["added"].as_u64(), again["data"]["member_count"].as_u64()),
            (Some(0), Some(2))
        );

        // Nút không thuộc khách.
        let stranger = uuid::Uuid::now_v7().to_string();
        let (status, body) =
            call(&app, "POST", &path, "boss", Some(json!({"org_ids": [stranger]}))).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("organizations.group_member_invalid")),
            "{body}"
        );
        // Tên trùng (không phân biệt dấu, hoa thường).
        let (status, body) =
            call(&app, "POST", "/groups", "boss", Some(json!({"name": "mien bac"}))).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("organizations.group_name_taken")),
            "{body}"
        );
        // Đổi tên kèm `If-Match` cũ → 412.
        let read = app
            .get(&format!("/api/v1/organizations/groups/{north}"))
            .token("boss")
            .send()
            .await
            .unwrap();
        let etag = read.headers().get("etag").unwrap().to_str().unwrap().to_owned();
        let renamed = app
            .request(axum::http::Method::PATCH, &format!("/api/v1/organizations/groups/{north}"))
            .token("boss")
            .header("if-match", &etag)
            .json(json!({"name": "North"}))
            .send()
            .await
            .unwrap();
        assert_eq!(renamed.status(), StatusCode::OK);
        let stale = app
            .request(axum::http::Method::PATCH, &format!("/api/v1/organizations/groups/{north}"))
            .token("boss")
            .header("if-match", &etag)
            .json(json!({"name": "North again"}))
            .send()
            .await
            .unwrap();
        assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);

        // Bỏ nút không có trong nhóm vẫn 204; bỏ nút có thì còn một.
        let gone = format!("{path}/{}", tree.r2);
        expect(&app, "DELETE", &gone, "boss", None, StatusCode::NO_CONTENT).await;
        let gone = format!("{path}/{}", tree.s1);
        expect(&app, "DELETE", &gone, "boss", None, StatusCode::NO_CONTENT).await;
        let left = expect(&app, "GET", &path, "boss", None, StatusCode::OK).await;
        assert_eq!(members(&left), vec![tree.r1.clone()]);

        // Xoá nhóm: nhóm và thành viên đi, nút ở lại.
        expect(&app, "DELETE", &format!("/groups/{north}"), "boss", None, StatusCode::NO_CONTENT)
            .await;
        let (status, body) = call(&app, "GET", &format!("/groups/{north}"), "boss", None).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("organizations.group_not_found"))
        );
        expect(&app, "GET", &format!("/units/{}", tree.r1), "admin", None, StatusCode::OK).await;

        // MCP: tạo, thêm, bỏ.
        let client = app.mcp("boss").await.unwrap();
        let created = client
            .call_tool(tool("organizations.create_group", json!({"name": "Franchise"})))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        let group_id = created["data"]["id"].as_str().unwrap().to_owned();
        let added = client
            .call_tool(tool(
                "organizations.add_group_members",
                json!({"id": group_id, "org_ids": [tree.r2]}),
            ))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(added["data"]["added"], 1, "{added}");
        let removed = client
            .call_tool(tool(
                "organizations.remove_group_member",
                json!({"id": group_id, "org_id": tree.r2}),
            ))
            .await
            .unwrap();
        assert_ne!(removed.is_error, Some(true));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn them_dong_thoi_cung_nut_mot_dong() {
    TestDatabase::run(async |test| {
        let app = build(test, |builder| builder.user("boss", 9, &MANAGE)).await;
        let tree = tree(&app).await;
        let north = group(&app, "boss", "North").await;
        let path = format!("/groups/{north}/members");
        let body = json!({"org_ids": [tree.s1]});
        let (first, second) = tokio::join!(
            call(&app, "POST", &path, "boss", Some(body.clone())),
            call(&app, "POST", &path, "boss", Some(body.clone())),
        );
        let ok =
            [first, second].into_iter().filter(|(status, _)| *status == StatusCode::OK).count();
        assert!(ok >= 1, "ít nhất một lệnh thành công");
        // Gọi lại (sau khi bên thua có thể bị lỗi tạm thời): vẫn đúng một dòng.
        expect(&app, "POST", &path, "boss", Some(body), StatusCode::OK).await;
        let listed = expect(&app, "GET", &path, "boss", None, StatusCode::OK).await;
        assert_eq!(members(&listed), vec![tree.s1.clone()]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhom_cua_khach_khac_khong_tim_thay() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .tenant("a")
            .tenant("b")
            .user_in("a", "boss-a", 1, &MANAGE)
            .user_in("b", "boss-b", 2, &MANAGE)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let mine = group(&app, "boss-a", "North").await;
        for path in [format!("/groups/{mine}"), format!("/groups/{mine}/members")] {
            let (status, body) = call(&app, "GET", &path, "boss-b", None).await;
            assert_eq!(
                (status, body["code"].as_str()),
                (StatusCode::NOT_FOUND, Some("organizations.group_not_found")),
                "{path}: {body}"
            );
        }
        let listed = expect(&app, "GET", "/groups", "boss-b", None, StatusCode::OK).await;
        assert_eq!(listed["data"], json!([]));
        // Khách B dùng lại được cùng tên.
        group(&app, "boss-b", "North").await;
    })
    .await
    .unwrap();
}

/// Tìm nút và nhóm theo tên: `%` và `_` trong `q` là chữ thường, không phải ký tự đại diện của
/// `LIKE`.
#[tokio::test]
async fn tim_theo_ten_khong_coi_phan_tram_la_dai_dien() {
    TestDatabase::run(async |test| {
        let app = build(test, |builder| builder.user("boss", 9, &MANAGE)).await;
        let tree = tree(&app).await;
        let sale = group(&app, "boss", "Sale 50% off").await;
        let under = group(&app, "boss", "North_East").await;
        group(&app, "boss", "Northern Coast").await;

        let names = async |path: &str| {
            let found = expect(&app, "GET", path, "boss", None, StatusCode::OK).await;
            let mut ids: Vec<String> = found["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_str().unwrap().to_owned())
                .collect();
            ids.sort();
            ids
        };
        assert_eq!(names("/groups?q=%25").await, vec![sale.clone()], "`%` khớp đúng chữ `%`");
        assert_eq!(names("/groups?q=50%25").await, vec![sale]);
        assert_eq!(names("/groups?q=north_").await, vec![under], "`_` không khớp mọi ký tự");
        assert!(names("/units?q=%25").await.is_empty(), "không nút nào có `%` trong tên");
        assert!(names("/units?q=_").await.is_empty(), "không nút nào có `_` trong tên");
        assert_eq!(names("/units?q=ha%20noi").await, vec![tree.s1]);
    })
    .await
    .unwrap();
}
