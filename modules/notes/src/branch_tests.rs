//! Ghi chú theo nút của cây tổ chức (bước 34 mục 2) — ví dụ mẫu của "dữ liệu theo nút": độ sâu
//! `own` / `unit` / `subtree` / `tenant` quyết ai thấy gì, nhánh anh em không thấy nhau, nút đang
//! khoá không ghi được, người làm ở nhiều nút phải chọn nút. Chạy trên SQLite và Postgres.

use autoxide_kernel::access::Scope;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, root_of};
use autoxide_testkit::http::{Method, StatusCode};
use autoxide_testkit::{TestApp, TestDatabase, branch_leak_check};
use serde_json::{Value, json};
use uuid::Uuid;

const MARKER: &str = "marker-branch-a";

/// Cây gốc → A, B; A → A1. ID biết trước để khai người dùng giả, dựng cây sau khi có app.
struct Tree {
    a: OrgId,
    b: OrgId,
    a1: OrgId,
}

const READ: &str = "notes.read";
const WRITE: &str = "notes.write";

async fn app(test: &TestDatabase) -> (TestApp, Tree) {
    let org = || OrgId::from_uuid(Uuid::now_v7());
    let tree = Tree { a: org(), b: org(), a1: org() };
    let app = TestApp::builder(test)
        .member("north", 1, tree.a, &[(READ, Scope::Subtree), (WRITE, Scope::Subtree)])
        .member("south", 2, tree.b, &[(READ, Scope::Subtree), (WRITE, Scope::Subtree)])
        .member("unit", 3, tree.a, &[(READ, Scope::Unit)])
        .member("own", 4, tree.a, &[(READ, Scope::Own), (WRITE, Scope::Own)])
        .member(
            "boss",
            5,
            root_of(DEFAULT_TENANT),
            &[(READ, Scope::Tenant), (WRITE, Scope::Tenant)],
        )
        .module(crate::module())
        .build()
        .await
        .unwrap();
    let write = app.database().begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.a, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.b, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.a1, tree.a, 8).await.unwrap();
    write.commit().await.unwrap();
    (app, tree)
}

async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_name(method).unwrap();
    let mut request = app.request(method, &format!("/api/v1/notes{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let status = answer.status_code();
    (status, if answer.body().is_empty() { Value::Null } else { answer.json() })
}

async fn create(app: &TestApp, who: &str, title: &str, org: Option<OrgId>) -> String {
    let mut body = json!({"title": title, "body": MARKER});
    if let Some(org) = org {
        body["org_id"] = json!(org.as_uuid());
    }
    let (status, created) = call(app, "POST", "", who, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["data"]["id"].as_str().unwrap().to_owned()
}

async fn titles(app: &TestApp, who: &str) -> Vec<String> {
    let (status, listed) = call(app, "GET", "?order=asc", who, None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|note| note["title"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn do_sau_quyet_dinh_ai_thay_ghi_chu() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let in_a = create(&app, "north", "in A", Some(tree.a)).await;
        let in_a1 = create(&app, "north", "in A1", Some(tree.a1)).await;
        create(&app, "own", "own in A", None).await;

        assert_eq!(titles(&app, "north").await, ["in A", "in A1", "own in A"], "subtree ở A");
        assert_eq!(titles(&app, "unit").await, ["in A", "own in A"], "unit ở A: không thấy A1");
        assert_eq!(titles(&app, "own").await, ["own in A"], "own: chỉ của mình");
        assert_eq!(titles(&app, "boss").await.len(), 3, "toàn khách");
        assert!(titles(&app, "south").await.is_empty(), "nhánh anh em");

        // Một ghi chú ngoài phạm vi: 404 khi đọc, sửa, xoá — không lộ là có.
        for (method, body) in
            [("GET", None), ("PATCH", Some(json!({"body": "x"}))), ("DELETE", None)]
        {
            let (status, problem) =
                call(&app, method, &format!("/{in_a1}"), "south", body.clone()).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method}: {problem}");
            assert_eq!(problem["code"], "notes.note_not_found");
        }
        // Đọc được nhưng không ghi được: 403.
        let (status, problem) =
            call(&app, "PATCH", &format!("/{in_a}"), "unit", Some(json!({"body": "x"}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
        // Tạo ở nút ngoài phạm vi: 403 `org_out_of_scope` (người gọi tự nêu nút).
        let body = json!({"title": "elsewhere", "org_id": tree.a.as_uuid()});
        let (status, problem) = call(&app, "POST", "", "south", Some(body)).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("org_out_of_scope")),
            "{problem}"
        );

        // Bài rò rỉ giữa hai nhánh: người của B không thấy gì của A, qua mọi GET và tool chỉ đọc.
        let seeded = [("id", in_a), ("id", in_a1)];
        let leaks = branch_leak_check(&app, "south", MARKER, &seeded).await.unwrap();
        assert_eq!(leaks, Vec::<String>::new());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nut_khoa_khong_ghi_duoc_va_nut_mac_dinh() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let note = create(&app, "north", "in A1", Some(tree.a1)).await;
        // Mặc định khi không nêu nút: nút duy nhất của người tạo (A cho `north`), gốc cho `boss`.
        let (_, made) = call(&app, "POST", "", "north", Some(json!({"title": "default"}))).await;
        assert_eq!(made["data"]["org_id"], tree.a.as_uuid().to_string());
        let (_, made) = call(&app, "POST", "", "boss", Some(json!({"title": "boss"}))).await;
        assert_eq!(made["data"]["org_id"], DEFAULT_TENANT.as_uuid().to_string());

        let write = app.database().begin_write().await.unwrap();
        OrgTree::set_writable(&write, DEFAULT_TENANT, tree.a, false).await.unwrap();
        write.commit().await.unwrap();
        let (status, problem) =
            call(&app, "PATCH", &format!("/{note}"), "north", Some(json!({"body": "x"}))).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("org_frozen")),
            "{problem}"
        );
        let (status, _) = call(&app, "GET", &format!("/{note}"), "north", None).await;
        assert_eq!(status, StatusCode::OK, "đọc vẫn được");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lam_o_nhieu_nut_thi_phai_chon_nut() {
    TestDatabase::run(async |test| {
        let org = || OrgId::from_uuid(Uuid::now_v7());
        let (a, b) = (org(), org());
        let app = TestApp::builder(test)
            .principal(
                "both",
                autoxide_kernel::access::Principal::with_grants(
                    Uuid::from_u128(9),
                    [a, b].map(|unit| {
                        (WRITE.to_owned(), autoxide_kernel::access::Grant::new(unit, Scope::Unit))
                    }),
                ),
            )
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let (status, problem) =
            call(&app, "POST", "", "both", Some(json!({"title": "where?"}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["code"], "notes.org_required");
        assert_eq!(problem["errors"][0]["field"], "org_id");
        let body = json!({"title": "here", "org_id": b.as_uuid()});
        let (status, created) = call(&app, "POST", "", "both", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
    })
    .await
    .unwrap();
}
