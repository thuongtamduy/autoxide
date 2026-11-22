//! Sổ địa chỉ người mua (bước 44 mục 1): xã phải thuộc tỉnh, xã hết hiệu lực bị từ chối, địa chỉ
//! nước ngoài dạng chung, đúng một địa chỉ mặc định kể cả khi đặt đồng thời, xuất và xoá dữ liệu cá
//! nhân, người mua không đọc được địa chỉ của nhau. Cả SQLite và Postgres.

use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::privacy::PersonalData;

use super::*;
use crate::api::AddressBook;

fn hanoi(line: &str) -> Value {
    json!({
        "recipient_name": "Nguyen Van An",
        "phone": "0912345678",
        "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": line},
    })
}

async fn create(app: &TestApp, who: &str, body: Value) -> Value {
    let (status, created) = call(app, "POST", "/me/addresses", who, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["data"].clone()
}

#[tokio::test]
async fn xa_phai_thuoc_tinh() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let saved = create(&app, "x", hanoi("12 Trang Tien")).await;
        assert_eq!(saved["address"]["commune_name"], "Phuong Ba Dinh");
        assert_eq!(saved["address"]["province_name"], "Thanh pho Ha Noi");
        assert_eq!(saved["is_default"], true, "the first address is the default");

        let mut wrong = hanoi("1 A");
        wrong["address"]["province_code"] = json!("79");
        let (status, problem) = call(&app, "POST", "/me/addresses", "x", Some(wrong)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["code"], "shipping.invalid_address");
        assert_eq!(problem["errors"][0]["field"], "address.commune_code");

        // Thiếu xã, sai số điện thoại: báo một lần cả hai.
        let body = json!({
            "recipient_name": "An",
            "phone": "abc",
            "address": {"country": "VN", "province_code": "01", "line": "1 A"},
        });
        let (status, problem) = call(&app, "POST", "/me/addresses", "x", Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let fields: Vec<&str> = problem["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|error| error["field"].as_str().unwrap())
            .collect();
        assert!(fields.contains(&"phone") && fields.contains(&"address.commune_code"), "{problem}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ma_xa_het_hieu_luc_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let mut body = hanoi("1 A");
        body["address"]["commune_code"] = json!("00001");
        let (status, problem) = call(&app, "POST", "/me/addresses", "x", Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["errors"][0]["code"], "address.unit_expired");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dia_chi_nuoc_ngoai() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let body = json!({
            "label": "Office",
            "recipient_name": "Jane Doe",
            "phone": "+1 415 555 0100",
            "address": {
                "country": "us", "province_code": "CA", "city": "San Francisco",
                "line": "1 Market St", "postal_code": "94105"
            },
        });
        let saved = create(&app, "x", body).await;
        assert_eq!(saved["address"]["country"], "US");
        assert_eq!(saved["address"]["commune_name"], "San Francisco");
        assert_eq!(saved["address"]["commune_code"], "");
        assert_eq!(saved["postal_code"], "94105");

        // Thiếu thành phố.
        let body = json!({
            "recipient_name": "Jane", "phone": "+14155550100",
            "address": {"country": "US", "province_code": "CA", "line": "1 Market St"},
        });
        let (status, problem) = call(&app, "POST", "/me/addresses", "x", Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["errors"][0]["field"], "address.city", "{problem}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mot_dia_chi_mac_dinh() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let first = create(&app, "x", hanoi("1 A")).await;
        let second = create(&app, "x", hanoi("2 B")).await;
        let third = create(&app, "x", hanoi("3 C")).await;
        assert_eq!(second["is_default"], false);
        let (a, b) = (second["id"].as_str().unwrap(), third["id"].as_str().unwrap());

        // Hai lệnh đặt mặc định cùng lúc: còn đúng một địa chỉ mặc định.
        let path_a = format!("/me/addresses/{a}/default");
        let path_b = format!("/me/addresses/{b}/default");
        let (left, right) = tokio::join!(
            call(&app, "POST", &path_a, "x", None),
            call(&app, "POST", &path_b, "x", None),
        );
        assert_eq!(left.0, StatusCode::OK, "{}", left.1);
        assert_eq!(right.0, StatusCode::OK, "{}", right.1);
        let (_, list) = call(&app, "GET", "/me/addresses", "x", None).await;
        let defaults: Vec<&Value> = list["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["is_default"] == true)
            .collect();
        assert_eq!(defaults.len(), 1, "{list}");

        // Xoá địa chỉ mặc định: địa chỉ khác thành mặc định.
        let current = defaults[0]["id"].as_str().unwrap().to_owned();
        let (status, _) =
            call(&app, "DELETE", &format!("/me/addresses/{current}"), "x", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let book = service_of(&app);
        let default = book
            .default_for(&app.default_tenant(), Uuid::from_u128(BUYER_X))
            .await
            .unwrap()
            .unwrap();
        assert_ne!(default.id.to_string(), current);
        assert!(first["id"].is_string());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gioi_han_so_dia_chi_va_sua() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .setting("AUTOXIDE_SHIPPING_MAX_ADDRESSES_PER_BUYER", "2")
            .principal("x", Principal::customer(Uuid::from_u128(BUYER_X)))
            .provide::<dyn AdministrativeUnits>(Arc::new(FakeAdministrativeUnits::sample()))
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let saved = create(&app, "x", hanoi("1 A")).await;
        create(&app, "x", hanoi("2 B")).await;
        let (status, problem) = call(&app, "POST", "/me/addresses", "x", Some(hanoi("3"))).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(problem["code"], "shipping.address_limit");

        let id = saved["id"].as_str().unwrap();
        let change = json!({"label": "Nha", "address": {"country": "VN", "province_code": "79", "commune_code": "26734", "line": "5 Le Loi"}});
        let (status, changed) =
            call(&app, "PATCH", &format!("/me/addresses/{id}"), "x", Some(change)).await;
        assert_eq!(status, StatusCode::OK, "{changed}");
        assert_eq!(changed["data"]["label"], "Nha");
        assert_eq!(changed["data"]["address"]["commune_name"], "Phuong Sai Gon");
        assert_eq!(changed["data"]["recipient_name"], "Nguyen Van An", "unchanged fields stay");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nguoi_mua_khong_doc_dia_chi_cua_nhau() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let saved = create(&app, "x", hanoi("1 A")).await;
        let id = saved["id"].as_str().unwrap();
        for (method, path) in [
            ("GET", format!("/me/addresses/{id}")),
            ("PATCH", format!("/me/addresses/{id}")),
            ("DELETE", format!("/me/addresses/{id}")),
            ("POST", format!("/me/addresses/{id}/default")),
        ] {
            let body = (method == "PATCH").then(|| json!({"label": "mine"}));
            let (status, problem) = call(&app, method, &path, "y", body).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}: {problem}");
            assert_eq!(problem["code"], "shipping.address_not_found");
        }
        let (_, list) = call(&app, "GET", "/me/addresses", "y", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 0);

        // Nhân viên chăm sóc khách đọc được khi có permission.
        let path = format!("/buyers/{}/addresses", Uuid::from_u128(BUYER_X));
        let (status, _) = call(&app, "GET", &path, "nobody", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, list) = call(&app, "GET", &path, "care", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list["data"][0]["id"], saved["id"]);

        // Qua `dyn AddressBook`: địa chỉ của người khác → 404.
        let book = app.api::<dyn AddressBook>().unwrap();
        let tenant = app.default_tenant();
        let id = Uuid::parse_str(id).unwrap();
        assert!(book.get(&tenant, Uuid::from_u128(BUYER_X), id).await.is_ok());
        let error = book.get(&tenant, Uuid::from_u128(BUYER_Y), id).await.unwrap_err();
        assert_eq!(error.code(), "shipping.address_not_found");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xuat_va_xoa_du_lieu_ca_nhan() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        create(&app, "x", hanoi("1 A")).await;
        create(&app, "y", hanoi("9 Z")).await;
        let tenant = app.default_tenant();
        let service = service_of(&app);
        let exported = service.export(&tenant, Uuid::from_u128(BUYER_X)).await.unwrap();
        assert_eq!(exported["addresses"].as_array().unwrap().len(), 1);
        assert_eq!(exported["addresses"][0]["address"]["line"], "1 A");

        let event = Delivered::new(
            tenant.clone(),
            "auth",
            autoxide_kernel::privacy::USER_DELETED,
            json!({"before": {"id": Uuid::from_u128(BUYER_X)}}),
        );
        let handler = crate::privacy::Erasure(service.clone());
        handler.handle(&event).await.unwrap();
        handler.handle(&event).await.unwrap();
        let (_, list) = call(&app, "GET", "/me/addresses", "x", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 0);
        let (_, list) = call(&app, "GET", "/me/addresses", "y", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1, "other buyers keep theirs");
    })
    .await
    .unwrap();
}
