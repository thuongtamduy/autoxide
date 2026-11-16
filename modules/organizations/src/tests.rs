//! Test của module `organizations` qua REST và MCP, trên database gate đang chọn (cả SQLite và
//! Postgres): dựng cây, đọc theo phạm vi, vòng đời nút (khoá nhánh), dời nút, loại nút, không rò
//! rỉ giữa hai nhánh anh em hay giữa hai khách.

use autoxide_kernel::access::Scope;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, root_of};
use autoxide_testkit::{TestApp, TestAppBuilder, TestDatabase, tool};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

const ALL: [&str; 5] = [
    "organizations.units.read",
    "organizations.settings.write",
    "organizations.units.write",
    "organizations.units.manage",
    "organizations.types.manage",
];

fn root() -> String {
    root_of(DEFAULT_TENANT).as_uuid().to_string()
}

async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/organizations{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let status = answer.status();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (status, body)
}

/// Gọi và chờ đúng mã trạng thái; trả thân.
async fn expect(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
    status: StatusCode,
) -> Value {
    let (got, body) = call(app, method, uri, who, body).await;
    assert_eq!(got, status, "{method} {uri}: {body}");
    body
}

fn id(body: &Value) -> String {
    body["data"]["id"].as_str().unwrap().to_owned()
}

fn org(id: &str) -> OrgId {
    OrgId::from_uuid(Uuid::parse_str(id).unwrap())
}

/// Cây mẫu do `admin` dựng: gốc → vùng R1, R2 → cửa hàng S1 dưới R1.
struct Tree {
    r1: String,
    r2: String,
    s1: String,
}

async fn build(
    test: &TestDatabase,
    extra: impl FnOnce(TestAppBuilder) -> TestAppBuilder,
) -> TestApp {
    let builder = TestApp::builder(test).user("admin", 1, &ALL).module(crate::module());
    extra(builder).build().await.unwrap()
}

async fn tree(app: &TestApp) -> Tree {
    for (code, name) in [("region", "Region"), ("store", "Store")] {
        expect(
            app,
            "POST",
            "/unit-types",
            "admin",
            Some(json!({"code": code, "name": name})),
            StatusCode::CREATED,
        )
        .await;
    }
    let unit = async |parent: &str, kind: &str, name: &str| {
        let body = json!({"parent_id": parent, "unit_type": kind, "name": name});
        let created = expect(app, "POST", "/units", "admin", Some(body), StatusCode::CREATED).await;
        assert_eq!(created["data"]["status"], "active", "admin có quyền quản lý: hoạt động ngay");
        id(&created)
    };
    let r1 = unit(&root(), "region", "Region North").await;
    let r2 = unit(&root(), "region", "Region South").await;
    let s1 = unit(&r1, "store", "Store Hà Nội").await;
    Tree { r1, r2, s1 }
}

#[tokio::test]
async fn dung_cay_va_doc_theo_pham_vi() {
    TestDatabase::run(async |test| {
        let app = build(test, |builder| builder).await;
        let tree = tree(&app).await;
        let units = expect(&app, "GET", "/units", "admin", None, StatusCode::OK).await;
        assert_eq!(units["data"].as_array().unwrap().len(), 4, "gốc + 3 nút: {units}");
        let root_unit =
            expect(&app, "GET", &format!("/units/{}", root()), "admin", None, StatusCode::OK).await;
        assert_eq!(root_unit["data"]["unit_type"], "tenant");
        assert_eq!(root_unit["data"]["parent_id"], Value::Null);

        let ancestors = expect(
            &app,
            "GET",
            &format!("/units/{}/ancestors", tree.s1),
            "admin",
            None,
            StatusCode::OK,
        )
        .await;
        let ids: Vec<&str> = ancestors["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|unit| unit["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, [tree.r1.as_str(), root().as_str()]);

        let found = expect(&app, "GET", "/units?q=ha%20noi", "admin", None, StatusCode::OK).await;
        assert_eq!(found["data"][0]["id"], tree.s1.as_str(), "tìm không dấu: {found}");
        let children = expect(
            &app,
            "GET",
            &format!("/units?parent_id={}", tree.r1),
            "admin",
            None,
            StatusCode::OK,
        )
        .await;
        assert_eq!(children["data"].as_array().unwrap().len(), 1);
        let stores = expect(&app, "GET", "/units?type=store", "admin", None, StatusCode::OK).await;
        assert_eq!(stores["data"].as_array().unwrap().len(), 1);
        let (status, _) = call(&app, "GET", "/units?status=open", "admin", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

/// Người chỉ có `organizations.units.read` ở độ sâu `scope` tại nút `at`.
fn reader<'a>(
    builder: TestAppBuilder<'a>,
    token: &str,
    user: u128,
    at: &str,
    scope: Scope,
) -> TestAppBuilder<'a> {
    builder.member(token, user, org(at), &[("organizations.units.read", scope)])
}

#[tokio::test]
async fn khong_ro_ri_giua_hai_nhanh_anh_em() {
    TestDatabase::run(async |test| {
        // Nút có ID cố định: membership của người dùng giả cần ID lúc dựng app.
        let app = build(test, |builder| builder).await;
        let tree = tree(&app).await;
        let (r1, r2) = (tree.r1.clone(), tree.r2.clone());
        // Dựng app thứ hai trên CÙNG database (migration đã chạy, dữ liệu còn nguyên).
        let app = build(test, move |builder| {
            let builder = reader(builder, "north", 2, &r1, Scope::Subtree);
            let builder = reader(builder, "south", 3, &r2, Scope::Subtree);
            reader(builder, "unit", 4, &r1, Scope::Unit)
        })
        .await;

        let listed = |body: &Value| -> Vec<String> {
            body["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|unit| unit["id"].as_str().unwrap().to_owned())
                .collect()
        };
        let north = expect(&app, "GET", "/units", "north", None, StatusCode::OK).await;
        let mut seen = listed(&north);
        seen.sort();
        let mut want = vec![tree.r1.clone(), tree.s1.clone()];
        want.sort();
        assert_eq!(seen, want, "subtree ở R1: R1 và S1");
        let south = expect(&app, "GET", "/units", "south", None, StatusCode::OK).await;
        assert_eq!(listed(&south), std::slice::from_ref(&tree.r2), "nhánh anh em không thấy nhau");
        let unit = expect(&app, "GET", "/units", "unit", None, StatusCode::OK).await;
        assert_eq!(listed(&unit), std::slice::from_ref(&tree.r1), "unit ở R1 không thấy S1");

        for (who, hidden) in [("south", &tree.r1), ("south", &tree.s1), ("unit", &tree.s1)] {
            let (status, body) = call(&app, "GET", &format!("/units/{hidden}"), who, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{who} {hidden}: {body}");
            assert_eq!(body["code"], "organizations.unit_not_found");
        }
        // Qua MCP cũng vậy.
        let client = app.mcp("south").await.unwrap();
        let result = client.call_tool(tool("organizations.list_units", json!({}))).await.unwrap();
        let data = result.structured_content.unwrap();
        assert_eq!(listed(&data), std::slice::from_ref(&tree.r2));
        let hidden =
            client.call_tool(tool("organizations.get_unit", json!({"id": tree.s1}))).await.unwrap();
        assert_eq!(hidden.is_error, Some(true));
        // Bài rò rỉ giữa hai nhánh của testkit: mọi GET và tool chỉ đọc, người của R2 không thấy
        // gì của nhánh R1 (tên `Region North`, ID của R1 và S1).
        let seeded = [("id", tree.r1.clone()), ("id", tree.s1.clone())];
        let leaks = autoxide_testkit::branch_leak_check(&app, "south", "Region North", &seeded)
            .await
            .unwrap();
        assert_eq!(leaks, Vec::<String>::new());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nut_moi_cho_duyet_khi_khong_co_quyen_quan_ly() {
    TestDatabase::run(async |test| {
        let app = build(test, |builder| builder).await;
        let tree = tree(&app).await;
        let r1 = tree.r1.clone();
        let app = build(test, move |builder| {
            builder.member(
                "writer",
                5,
                org(&r1),
                &[
                    ("organizations.units.read", Scope::Subtree),
                    ("organizations.units.write", Scope::Subtree),
                ],
            )
        })
        .await;
        let body = json!({"parent_id": tree.r1, "unit_type": "store", "name": "Store new"});
        let created =
            expect(&app, "POST", "/units", "writer", Some(body), StatusCode::CREATED).await;
        assert_eq!(created["data"]["status"], "pending");
        let pending = id(&created);
        // Nút chờ duyệt chưa ghi được: không thêm nút con được, kể cả admin.
        let child = json!({"parent_id": pending, "unit_type": "store", "name": "Child"});
        let (status, body) = call(&app, "POST", "/units", "admin", Some(child.clone())).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("org_frozen")),
            "{body}"
        );
        // Người viết không tạo được ở nhánh khác (403, vì họ tự nêu nút cha).
        let outside = json!({"parent_id": tree.r2, "unit_type": "store", "name": "Elsewhere"});
        let (status, body) = call(&app, "POST", "/units", "writer", Some(outside)).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("org_out_of_scope")),
            "{body}"
        );
        // Người viết không duyệt được.
        let approve = json!({"status": "active"});
        let (status, _) = call(
            &app,
            "POST",
            &format!("/units/{pending}/status"),
            "writer",
            Some(approve.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        expect(
            &app,
            "POST",
            &format!("/units/{pending}/status"),
            "admin",
            Some(approve),
            StatusCode::OK,
        )
        .await;
        expect(&app, "POST", "/units", "admin", Some(child), StatusCode::CREATED).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khoa_nut_thi_ca_nhanh_ngung_ghi() {
    TestDatabase::run(async |test| {
        let app = build(test, |builder| builder).await;
        let tree = tree(&app).await;
        let suspend = json!({"status": "suspended", "reason": "Inventory check"});
        let suspended = expect(
            &app,
            "POST",
            &format!("/units/{}/status", tree.r1),
            "admin",
            Some(suspend),
            StatusCode::OK,
        )
        .await;
        assert_eq!(suspended["data"]["status_reason"], "Inventory check");
        // S1 dưới R1: không đổi tên, không thêm nút con được.
        let (status, body) = call(
            &app,
            "PATCH",
            &format!("/units/{}", tree.s1),
            "admin",
            Some(json!({"name": "Renamed"})),
        )
        .await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("org_frozen")),
            "{body}"
        );
        let child = json!({"parent_id": tree.s1, "unit_type": "store", "name": "Kiosk"});
        let (status, _) = call(&app, "POST", "/units", "admin", Some(child.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // Nhánh khác vẫn ghi được.
        expect(
            &app,
            "PATCH",
            &format!("/units/{}", tree.r2),
            "admin",
            Some(json!({"code": "R-2"})),
            StatusCode::OK,
        )
        .await;
        // Chuyển trạng thái sai, gốc không đổi được.
        let (status, body) = call(
            &app,
            "POST",
            &format!("/units/{}/status", tree.r1),
            "admin",
            Some(json!({"status": "pending"})),
        )
        .await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("organizations.invalid_transition")),
            "{body}"
        );
        let (status, body) = call(
            &app,
            "POST",
            &format!("/units/{}/status", root()),
            "admin",
            Some(json!({"status": "archived"})),
        )
        .await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("org_root_immutable")),
            "{body}"
        );
        // Mở lại: ghi lại được.
        expect(
            &app,
            "POST",
            &format!("/units/{}/status", tree.r1),
            "admin",
            Some(json!({"status": "active"})),
            StatusCode::OK,
        )
        .await;
        expect(&app, "POST", "/units", "admin", Some(child), StatusCode::CREATED).await;
        // Lưu trữ rồi kích hoạt lại.
        for status in ["archived", "active"] {
            expect(
                &app,
                "POST",
                &format!("/units/{}/status", tree.r2),
                "admin",
                Some(json!({"status": status})),
                StatusCode::OK,
            )
            .await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doi_nut_va_chan_vong() {
    TestDatabase::run(async |test| {
        let app = build(test, |builder| builder).await;
        let tree = tree(&app).await;
        let (status, body) = call(
            &app,
            "POST",
            &format!("/units/{}/move", tree.r1),
            "admin",
            Some(json!({"parent_id": tree.s1})),
        )
        .await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("org_cycle")),
            "{body}"
        );
        let (status, body) = call(
            &app,
            "POST",
            &format!("/units/{}/move", root()),
            "admin",
            Some(json!({"parent_id": tree.r1})),
        )
        .await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("org_root_immutable")),
            "{body}"
        );
        let moved = expect(
            &app,
            "POST",
            &format!("/units/{}/move", tree.s1),
            "admin",
            Some(json!({"parent_id": tree.r2})),
            StatusCode::OK,
        )
        .await;
        assert_eq!(moved["data"]["parent_id"], tree.r2.as_str());
        let ancestors = expect(
            &app,
            "GET",
            &format!("/units/{}/ancestors", tree.s1),
            "admin",
            None,
            StatusCode::OK,
        )
        .await;
        assert_eq!(ancestors["data"][0]["id"], tree.r2.as_str());
        // Mã trùng.
        expect(
            &app,
            "PATCH",
            &format!("/units/{}", tree.r1),
            "admin",
            Some(json!({"code": "X"})),
            StatusCode::OK,
        )
        .await;
        let (status, body) = call(
            &app,
            "PATCH",
            &format!("/units/{}", tree.r2),
            "admin",
            Some(json!({"code": "X"})),
        )
        .await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("organizations.unit_code_taken")),
            "{body}"
        );
        // Bỏ mã bằng chuỗi rỗng; nhiều nút không mã cùng tồn tại.
        let cleared = expect(
            &app,
            "PATCH",
            &format!("/units/{}", tree.r1),
            "admin",
            Some(json!({"code": ""})),
            StatusCode::OK,
        )
        .await;
        assert_eq!(cleared["data"]["code"], Value::Null);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn loai_nut_tu_dat() {
    TestDatabase::run(async |test| {
        let app = build(test, |builder| builder).await;
        let tree = tree(&app).await;
        let types = expect(&app, "GET", "/unit-types", "admin", None, StatusCode::OK).await;
        let codes: Vec<&str> = types["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|kind| kind["code"].as_str().unwrap())
            .collect();
        assert_eq!(codes, ["region", "store", "tenant"]);
        let (status, body) = call(
            &app,
            "POST",
            "/unit-types",
            "admin",
            Some(json!({"code": "store", "name": "Again"})),
        )
        .await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("organizations.unit_type_taken"))
        );
        let (status, body) = call(
            &app,
            "POST",
            "/unit-types",
            "admin",
            Some(json!({"code": "Bad Code", "name": ""})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["errors"].as_array().unwrap().len(), 2, "mọi lỗi báo một lần: {body}");
        let (status, body) = call(&app, "DELETE", "/unit-types/store", "admin", None).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("organizations.unit_type_in_use"))
        );
        let (status, body) = call(&app, "DELETE", "/unit-types/tenant", "admin", None).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("organizations.unit_type_reserved"))
        );
        let renamed = expect(
            &app,
            "PUT",
            "/unit-types/store",
            "admin",
            Some(json!({"name": "Shop"})),
            StatusCode::OK,
        )
        .await;
        assert_eq!(renamed["data"]["name"], "Shop");
        expect(
            &app,
            "POST",
            "/unit-types",
            "admin",
            Some(json!({"code": "kiosk", "name": "Kiosk"})),
            StatusCode::CREATED,
        )
        .await;
        expect(&app, "DELETE", "/unit-types/kiosk", "admin", None, StatusCode::NO_CONTENT).await;
        let (status, body) = call(
            &app,
            "POST",
            "/units",
            "admin",
            Some(json!({"parent_id": tree.r1, "unit_type": "kiosk", "name": "K"})),
        )
        .await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("organizations.unit_type_not_found"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_ro_ri_giua_khach() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .tenant("a")
            .tenant_dedicated("b")
            .user_in("a", "admin-a", 1, &ALL)
            .user_in("b", "admin-b", 2, &ALL)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let marker = "marker-organizations-a";
        expect(
            &app,
            "POST",
            "/unit-types",
            "admin-a",
            Some(json!({"code": "store", "name": marker})),
            StatusCode::CREATED,
        )
        .await;
        let root_a = app.tenant("a").await.unwrap().id().as_uuid().to_string();
        let body =
            json!({"parent_id": root_a, "unit_type": "store", "name": marker, "code": marker});
        let store =
            id(&expect(&app, "POST", "/units", "admin-a", Some(body), StatusCode::CREATED).await);
        // Nút gốc của khách B là của riêng B.
        let units_b = expect(&app, "GET", "/units", "admin-b", None, StatusCode::OK).await;
        assert_eq!(units_b["data"].as_array().unwrap().len(), 1);
        let (status, _) = call(&app, "GET", &format!("/units/{store}"), "admin-b", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let seeded = [("id", store), ("id", root_a)];
        let violations =
            autoxide_testkit::leak_check(&app, "admin-b", marker, &seeded).await.unwrap();
        assert_eq!(violations, Vec::<String>::new());
    })
    .await
    .unwrap();
}

mod settings;
