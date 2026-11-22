//! Hồ sơ công khai của nút (bước 34 mục 7, D-200, D-315): địa chỉ kiểm bằng module `address`, giờ
//! mở cửa sai bị từ chối, nút không công khai không lộ qua `/public/stores`, hai khách cùng database
//! không thấy cửa hàng của nhau. Cả SQLite và Postgres.

use super::*;

/// App có `organizations` (danh mục hành chính giả), hai khách.
async fn app(test: &TestDatabase) -> TestApp {
    TestApp::builder(test)
        .tenant("a")
        .tenant("b")
        .domain("a", "store-a.test")
        .user_in(
            "a",
            "admin",
            1,
            &[
                "organizations.units.read",
                "organizations.units.write",
                "organizations.units.manage",
                "organizations.types.manage",
                "organizations.profile.manage",
                "kernel.domains.manage",
            ],
        )
        .user_in("b", "other", 2, &["organizations.units.read", "organizations.profile.manage"])
        .provide::<dyn autoxide_address::api::AdministrativeUnits>(std::sync::Arc::new(
            autoxide_address::api::testing::FakeAdministrativeUnits::sample(),
        ))
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

async fn send(
    app: &TestApp,
    method: &str,
    path: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/organizations{path}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let status = answer.status();
    (status, if answer.body().is_empty() { Value::Null } else { answer.json() })
}

#[tokio::test]
async fn ho_so_cong_khai_cua_nut() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let tenant_a = app.tenant("a").await.unwrap();
        let root = root_of(tenant_a.id()).as_uuid().to_string();
        let path = format!("/units/{root}/public-profile");
        let (status, fresh) = send(&app, "GET", &path, "admin", None).await;
        assert_eq!(status, StatusCode::OK, "{fresh}");
        assert_eq!(fresh["data"]["public"], false);

        // Giờ đóng trước giờ mở, ngày đặc biệt trùng: một lỗi, mọi ô.
        let bad = json!({"opening_hours": [{"day": 1, "opens": "18:00", "closes": "08:00"}],
            "special_days": [{"date": "2026-02-17", "closed": true}, {"date": "2026-02-17", "closed": true}]});
        let (status, problem) = send(&app, "PATCH", &path, "admin", Some(bad)).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("organizations.invalid_opening_hours")), "{problem}");
        assert_eq!(problem["errors"].as_array().unwrap().len(), 2);
        // Xã không thuộc tỉnh.
        let wrong = json!({"address": {"country": "VN", "province_code": "01", "commune_code": "26734", "line": "1 A"}});
        let (status, problem) = send(&app, "PATCH", &path, "admin", Some(wrong)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["errors"][0]["field"], "address.commune_code");

        // Chưa công khai: website không thấy.
        let body = json!({"display_name": "Store One", "pickup": true,
            "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Trang Tien"},
            "opening_hours": [{"day": 1, "opens": "08:00", "closes": "21:30"}]});
        let (status, saved) = send(&app, "PATCH", &path, "admin", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved["data"]["address"]["commune_name"], "Phuong Ba Dinh");
        // Module khác (vd `shipping`: giờ sẵn sàng nhận tại cửa hàng) đọc giờ mở cửa qua `OrgDirectory`.
        let directory = app.api::<dyn crate::api::OrgDirectory>().unwrap();
        let hours = directory.opening_hours(&tenant_a, root_of(tenant_a.id())).await.unwrap().unwrap();
        assert_eq!(hours.len(), 1);
        assert_eq!((hours[0].day, hours[0].opens.to_string()), (1, "08:00:00".to_owned()));
        let stores = |tenant: &'static str| {
            let app = &app;
            async move {
                let answer = app.get("/api/v1/organizations/public/stores").header("autoxide-tenant", tenant).send().await.unwrap();
                assert_eq!(answer.status(), StatusCode::OK);
                answer.json()["data"].as_array().unwrap().len()
            }
        };
        assert_eq!(stores("a").await, 0);
        let (status, _) = send(&app, "PATCH", &path, "admin", Some(json!({"public": true}))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stores("a").await, 1);
        assert_eq!(stores("b").await, 0, "khách khác không thấy cửa hàng của A");
        // Người của khách B không sửa được hồ sơ của nút A.
        let (status, _) = send(&app, "PATCH", &path, "other", Some(json!({"public": false}))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "nút của khách khác: như không có");
    })
    .await
    .unwrap();
}

/// Website của tên miền đang gọi (`GET /public/site`): tên miền gắn với một cửa hàng → hồ sơ của cửa
/// hàng đó, không phải của trụ sở; `?site=` chọn website khi không gọi qua tên miền riêng; không nêu gì
/// → cả khách (nút gốc, tên khách).
#[tokio::test]
async fn website_cua_ten_mien_la_cua_hang_duoc_gan() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let site = |host: Option<&'static str>, query: String| {
            let app = &app;
            async move {
                let uri = format!("/api/v1/organizations/public/site{query}");
                let request = match host {
                    Some(host) => app.get(&uri).host(host),
                    None => app.get(&uri).header("autoxide-tenant", "a"),
                };
                let answer = request.send().await.unwrap();
                (answer.status(), answer.json())
            }
        };
        // Chưa có cây tổ chức: website của cả khách mang tên khách, không ghi gì.
        let (status, whole) = site(None, String::new()).await;
        assert_eq!(status, StatusCode::OK, "{whole}");
        assert_eq!(whole["data"]["unit_type"], "tenant");

        let (status, _) = send(
            &app,
            "POST",
            "/unit-types",
            "admin",
            Some(json!({"code": "store", "name": "Store"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let root = root_of(app.tenant("a").await.unwrap().id()).as_uuid();
        let unit = json!({"parent_id": root, "unit_type": "store", "name": "Branch 1"});
        let (status, store) = send(&app, "POST", "/units", "admin", Some(unit)).await;
        assert_eq!(status, StatusCode::CREATED, "{store}");
        let store_id = store["data"]["id"].as_str().unwrap().to_owned();
        let path = format!("/units/{store_id}/public-profile");
        let (status, _) =
            send(&app, "PATCH", &path, "admin", Some(json!({"display_name": "Branch One"}))).await;
        assert_eq!(status, StatusCode::OK);

        // Tên miền chưa gắn nút: website của cả khách.
        let (status, before) = site(Some("store-a.test"), String::new()).await;
        assert_eq!(status, StatusCode::OK, "{before}");
        assert_eq!(before["data"]["unit_id"], root.to_string());

        // Gắn tên miền với cửa hàng: website là cửa hàng.
        let domains = app.get("/api/v1/kernel/domains").token("admin").send().await.unwrap().json();
        let domain_id = domains["data"][0]["id"].as_str().unwrap().to_owned();
        let patched = app
            .patch(&format!("/api/v1/kernel/domains/{domain_id}"))
            .token("admin")
            .json(json!({"org_id": store_id}))
            .send()
            .await
            .unwrap();
        assert_eq!(patched.status(), StatusCode::OK, "{}", patched.text());
        let (status, bound) = site(Some("store-a.test"), String::new()).await;
        assert_eq!(status, StatusCode::OK, "{bound}");
        assert_eq!(bound["data"]["unit_id"], store_id);
        assert_eq!(bound["data"]["display_name"], "Branch One");

        // `?site=` khi không gọi qua tên miền riêng; nút lạ → 404.
        let (status, named) = site(None, format!("?site={store_id}")).await;
        assert_eq!(
            (status, named["data"]["display_name"].as_str()),
            (StatusCode::OK, Some("Branch One"))
        );
        let (status, missing) = site(None, format!("?site={}", uuid::Uuid::now_v7())).await;
        assert_eq!(
            (status, missing["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("site_not_found"))
        );
    })
    .await
    .unwrap();
}
