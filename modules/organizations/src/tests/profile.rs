//! Hồ sơ công khai của nút (bước 34 mục 7, D-200, D-315): địa chỉ kiểm bằng module `address`, giờ
//! mở cửa sai bị từ chối, nút không công khai không lộ qua `/public/stores`, hai khách cùng database
//! không thấy cửa hàng của nhau. Cả SQLite và Postgres.

use super::*;

/// Danh mục giả (module `address` không được gọi thẳng từ đây): tỉnh 01 có xã 00004, tỉnh 79 có
/// xã 26734.
struct FakeUnits;

#[async_trait::async_trait]
impl autoxide_address::api::AdministrativeUnits for FakeUnits {
    async fn provinces(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Vec<autoxide_address::api::AdministrativeUnit>, autoxide_kernel::error::Error> {
        Ok(Vec::new())
    }

    async fn communes(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> Result<Vec<autoxide_address::api::AdministrativeUnit>, autoxide_kernel::error::Error> {
        Ok(Vec::new())
    }

    async fn validate(
        &self,
        input: autoxide_address::api::AddressInput,
        field: &str,
    ) -> Result<autoxide_address::api::Address, Vec<autoxide_kernel::error::FieldError>> {
        let parent = match input.commune_code.as_str() {
            "00004" => "01",
            "26734" => "79",
            _ => "",
        };
        if parent != input.province_code {
            return Err(vec![autoxide_kernel::error::FieldError::new(
                format!("{field}commune_code"),
                "address.commune_not_in_province",
                "commune `{commune}` is not in province `{province}`",
            )]);
        }
        Ok(serde_json::from_value(json!({
            "country": input.country,
            "province_code": input.province_code,
            "province_name": "Thanh pho Ha Noi",
            "commune_code": input.commune_code,
            "commune_name": "Phuong Ba Dinh",
            "line": input.line,
        }))
        .unwrap())
    }
}

/// App có `organizations` (danh mục hành chính giả), hai khách.
async fn app(test: &TestDatabase) -> TestApp {
    TestApp::builder(test)
        .tenant("a")
        .tenant("b")
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
            ],
        )
        .user_in("b", "other", 2, &["organizations.units.read", "organizations.profile.manage"])
        .provide::<dyn autoxide_address::api::AdministrativeUnits>(std::sync::Arc::new(FakeUnits))
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
