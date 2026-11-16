//! Cài đặt kế thừa theo cây (bước 34 mục 4): nút gần nhất có đặt thắng, xoá thì kế thừa lại, giá
//! trị sai bị từ chối, người ở nhánh khác không đặt được. Chạy trên SQLite và Postgres.

use super::*;

const WRITE: &str = "organizations.settings.write";

fn settings_of<'a>(body: &'a Value, key: &str) -> &'a Value {
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|setting| setting["key"] == key)
        .unwrap_or_else(|| panic!("không có {key}: {body}"))
}

async fn put(app: &TestApp, who: &str, unit: &str, key: &str, value: Value) -> (StatusCode, Value) {
    let uri = format!("/units/{unit}/settings/{key}");
    call(app, "PUT", &uri, who, Some(json!({"value": value}))).await
}

async fn at(app: &TestApp, unit: &str) -> Value {
    expect(app, "GET", &format!("/units/{unit}/settings"), "admin", None, StatusCode::OK).await
}

#[tokio::test]
async fn cai_dat_ke_thua_theo_cay() {
    TestDatabase::run(async |test| {
        let app = build(test, |builder| builder).await;
        let tree = tree(&app).await;

        let catalog = expect(&app, "GET", "/settings", "admin", None, StatusCode::OK).await;
        let keys: Vec<&str> = catalog["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["key"].as_str().unwrap())
            .collect();
        assert_eq!(
            keys,
            ["organizations.currency", "organizations.language", "organizations.timezone"]
        );

        // Chưa ai đặt: mặc định, không có nút nguồn.
        let fresh = at(&app, &tree.s1).await;
        let zone = settings_of(&fresh, "organizations.timezone");
        assert_eq!(
            (zone["value"].as_str(), zone["inherited"].as_bool()),
            (Some("UTC"), Some(true))
        );
        assert_eq!(zone["source_unit_id"], Value::Null);
        assert_eq!(settings_of(&fresh, "organizations.currency")["value"], Value::Null);

        // Gốc dùng VND, vùng R1 dùng USD: S1 (dưới R1) theo R1, R2 theo gốc.
        let (status, body) =
            put(&app, "admin", &root(), "organizations.currency", json!("vnd")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["value"], "VND", "giá trị được chuẩn hoá");
        assert_eq!(body["data"]["inherited"], false);
        put(&app, "admin", &tree.r1, "organizations.currency", json!("USD")).await;
        let s1 = at(&app, &tree.s1).await;
        let currency = settings_of(&s1, "organizations.currency");
        assert_eq!(currency["value"], "USD");
        assert_eq!(currency["source_unit_id"], tree.r1.as_str());
        assert_eq!(currency["inherited"], true);
        let r2 = at(&app, &tree.r2).await;
        assert_eq!(settings_of(&r2, "organizations.currency")["source_unit_id"], root().as_str());

        // S1 tự đặt múi giờ: của riêng nó, không kế thừa.
        let (status, body) =
            put(&app, "admin", &tree.s1, "organizations.timezone", json!("Asia/Ho_Chi_Minh")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["source_unit_id"], tree.s1.as_str());
        assert_eq!(body["data"]["inherited"], false);

        // Xoá cài đặt của R1: S1 quay về VND của gốc. Xoá lần nữa không đổi gì.
        let uri = format!("/units/{}/settings/organizations.currency", tree.r1);
        for _ in 0..2 {
            let cleared = expect(&app, "DELETE", &uri, "admin", None, StatusCode::OK).await;
            assert_eq!(cleared["data"]["value"], "VND");
            assert_eq!(cleared["data"]["inherited"], true);
        }
        let s1 = at(&app, &tree.s1).await;
        assert_eq!(settings_of(&s1, "organizations.currency")["value"], "VND");

        // Qua MCP: cùng service.
        let client = app.mcp("admin").await.unwrap();
        let listed = client
            .call_tool(tool("organizations.get_settings", json!({"id": tree.s1})))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        let zone = listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|setting| setting["key"] == "organizations.timezone")
            .unwrap()
            .clone();
        assert_eq!(zone["value"], "Asia/Ho_Chi_Minh");
        let set = client
            .call_tool(tool(
                "organizations.set_setting",
                json!({"id": tree.s1, "key": "organizations.language", "value": "vi"}),
            ))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(set["data"]["value"], "vi");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cai_dat_sai_va_ngoai_pham_vi_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let app =
            build(test, |builder| builder.user("reader", 2, &["organizations.units.read"])).await;
        let tree = tree(&app).await;

        let (status, problem) =
            put(&app, "admin", &tree.s1, "organizations.timezone", json!("Mars/Olympus")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["code"], "organizations.setting_invalid");
        let (status, problem) =
            put(&app, "admin", &tree.s1, "organizations.currency", json!(42)).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("organizations.setting_invalid"))
        );
        let (status, problem) = put(&app, "admin", &tree.s1, "shop.colour", json!("red")).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("organizations.setting_unknown")),
            "{problem}"
        );
        let (status, problem) =
            put(&app, "reader", &tree.s1, "organizations.timezone", json!("UTC")).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("permission_denied")),
            "{problem}"
        );
        // Đọc được cài đặt thì cần quyền đọc nút.
        expect(
            &app,
            "GET",
            &format!("/units/{}/settings", tree.s1),
            "reader",
            None,
            StatusCode::OK,
        )
        .await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_dat_cai_dat_ngoai_nhanh_va_nut_khoa() {
    TestDatabase::run(async |test| {
        let app = build(test, |builder| builder).await;
        let tree = tree(&app).await;
        // Người quản lý vùng R2: đọc toàn khách, chỉ đặt cài đặt trong nhánh R2.
        let r2 = org(&tree.r2);
        let manager = autoxide_kernel::access::Principal::with_grants(
            Uuid::from_u128(7),
            [
                (
                    "organizations.units.read".to_owned(),
                    autoxide_kernel::access::Grant::new(root_of(DEFAULT_TENANT), Scope::Tenant),
                ),
                (WRITE.to_owned(), autoxide_kernel::access::Grant::new(r2, Scope::Subtree)),
            ],
        );
        // Dựng app thứ hai trên CÙNG database: membership cần ID của R2, có sau khi dựng cây.
        let app = build(test, move |builder| builder.principal("south", manager)).await;

        let (status, body) =
            put(&app, "south", &tree.r2, "organizations.timezone", json!("Asia/Bangkok")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, problem) =
            put(&app, "south", &tree.s1, "organizations.timezone", json!("Asia/Bangkok")).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("org_out_of_scope")),
            "{problem}"
        );

        // Nút tạm khoá: không đặt được, nhưng vẫn đọc được.
        let suspend = json!({"status": "suspended", "reason": "Audit"});
        let uri = format!("/units/{}/status", tree.r2);
        expect(&app, "POST", &uri, "admin", Some(suspend), StatusCode::OK).await;
        let (status, problem) =
            put(&app, "south", &tree.r2, "organizations.timezone", json!("UTC")).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("org_frozen")),
            "{problem}"
        );
        let r2 = at(&app, &tree.r2).await;
        assert_eq!(settings_of(&r2, "organizations.timezone")["value"], "Asia/Bangkok");
    })
    .await
    .unwrap();
}
