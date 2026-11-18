//! Test của `shipping`, trên database gate đang chọn (gate chạy cả SQLite và Postgres). Dựng app
//! như binary bằng `TestApp` của testkit; gọi qua REST và qua trait trong `api`.

mod addresses;

use std::sync::Arc;

use autoxide_address::api::{AddressInput, AdministrativeUnit, AdministrativeUnits};
use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

pub(crate) const BUYER_X: u128 = 0x101;
pub(crate) const BUYER_Y: u128 = 0x102;

/// Danh mục hành chính giả: tỉnh 01 có xã 00004 (còn hiệu lực) và 00001 (hết hiệu lực từ
/// 01/7/2025); tỉnh 79 có xã 26734.
pub(crate) struct FakeUnits;

#[async_trait::async_trait]
impl AdministrativeUnits for FakeUnits {
    async fn provinces(&self, _: &str, _: Option<&str>) -> Result<Vec<AdministrativeUnit>, Error> {
        Ok(Vec::new())
    }

    async fn communes(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> Result<Vec<AdministrativeUnit>, Error> {
        Ok(Vec::new())
    }

    async fn validate(
        &self,
        input: AddressInput,
        field: &str,
    ) -> Result<autoxide_address::api::Address, Vec<FieldError>> {
        let (parent, name) = match input.commune_code.as_str() {
            "00004" => ("01", "Phuong Ba Dinh"),
            "26734" => ("79", "Phuong Sai Gon"),
            "00001" => {
                return Err(vec![FieldError::new(
                    format!("{field}commune_code"),
                    "address.unit_expired",
                    "is no longer in force",
                )]);
            }
            _ => ("", ""),
        };
        if parent != input.province_code {
            return Err(vec![FieldError::new(
                format!("{field}commune_code"),
                "address.commune_not_in_province",
                "is not in the province",
            )]);
        }
        let province = if parent == "01" { "Thanh pho Ha Noi" } else { "Thanh pho Ho Chi Minh" };
        Ok(serde_json::from_value(json!({
            "country": "VN",
            "province_code": input.province_code,
            "province_name": province,
            "commune_code": input.commune_code,
            "commune_name": name,
            "line": input.line,
        }))
        .unwrap())
    }
}

/// App có `shipping` với danh mục hành chính giả; người mua `x`, `y` (tài khoản người mua, không
/// permission) và nhân viên chăm sóc khách `care`.
pub(crate) async fn app(test: &TestDatabase) -> TestApp {
    TestApp::builder(test)
        .principal("x", Principal::customer(Uuid::from_u128(BUYER_X)))
        .principal("y", Principal::customer(Uuid::from_u128(BUYER_Y)))
        .user("care", 3, &["shipping.addresses.read"])
        .user("nobody", 4, &[])
        .provide::<dyn AdministrativeUnits>(Arc::new(FakeUnits))
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

/// Service của module trên database của `app` (gọi thẳng, vd handler event, `PersonalData`).
pub(crate) fn service() -> Arc<crate::service::ShippingService> {
    Arc::new(crate::service::ShippingService::new(Default::default(), Some(Arc::new(FakeUnits))))
}

/// Gọi `/api/v1/shipping{uri}`; trả trạng thái và thân JSON (`null` khi rỗng).
pub(crate) async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/shipping{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let status = answer.status();
    (status, if answer.body().is_empty() { Value::Null } else { answer.json() })
}
