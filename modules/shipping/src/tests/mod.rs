//! Test của `shipping`, trên database gate đang chọn (gate chạy cả SQLite và Postgres). Dựng app
//! như binary bằng `TestApp` của testkit; gọi qua REST và qua trait trong `api`.

mod addresses;
mod cod;
mod dates;
mod facts;
pub(crate) mod rates;
mod shipments;

pub(crate) use autoxide_address::api::testing::FakeAdministrativeUnits;
use std::sync::Arc;

use autoxide_address::api::AdministrativeUnits;
use autoxide_kernel::access::Principal;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

pub(crate) const BUYER_X: u128 = 0x101;
pub(crate) const BUYER_Y: u128 = 0x102;

/// App có `shipping` với danh mục hành chính giả; người mua `x`, `y` (tài khoản người mua, không
/// permission) và nhân viên chăm sóc khách `care`.
pub(crate) async fn app(test: &TestDatabase) -> TestApp {
    TestApp::builder(test)
        .principal("x", Principal::customer(Uuid::from_u128(BUYER_X)))
        .principal("y", Principal::customer(Uuid::from_u128(BUYER_Y)))
        .user("care", 3, &["shipping.addresses.read"])
        .user("nobody", 4, &[])
        .provide::<dyn AdministrativeUnits>(Arc::new(FakeAdministrativeUnits::sample()))
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

/// Service của module trên database của `app` (gọi thẳng, vd handler event, `PersonalData`), dùng
/// chung sổ hãng của app.
pub(crate) fn service_of(app: &TestApp) -> Arc<crate::service::ShippingService> {
    let registry = app.api::<dyn crate::api::CarrierRegistry>().unwrap();
    use crate::api::CarrierRegistry as _;
    let carriers = Arc::new(crate::registry::Registry::default());
    for code in registry.codes() {
        carriers.register(registry.get(code).unwrap()).unwrap();
    }
    let parts = crate::service::Parts {
        outbox: autoxide_kernel::event::Outbox::standalone("shipping"),
        jobs: autoxide_kernel::job::JobQueue::new("shipping"),
        entitlements: Default::default(),
        databases: app.registry().clone(),
        secret: (*app.api::<autoxide_kernel::secret::SecretKey>().unwrap()).clone(),
        providers: (*app.api::<autoxide_kernel::provider::ProviderAccounts>().unwrap()).clone(),
        carriers,
        ledger: None,
        reasons: Default::default(),
        catalog: None,
        orgs: None,
        units: Some(Arc::new(FakeAdministrativeUnits::sample())),
    };
    Arc::new(crate::service::ShippingService::new(parts, Default::default()))
}

/// Chạy hết event và job đang chờ.
pub(crate) async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..10 {
        let events = dispatcher.run_once().await.unwrap();
        if worker.run_once().await.unwrap() + events == 0 {
            break;
        }
    }
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
