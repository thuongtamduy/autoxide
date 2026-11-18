//! Test của module `bookings`, trên database gate đang chọn (gate chạy cả SQLite và Postgres).
//! `TestApp` dựng module như binary dựng; cây tổ chức giả ([`Orgs`]) cho múi giờ và chính sách theo
//! nút (module khác chỉ dùng qua `api`).

mod bookings;
mod fakes;
mod guests;
mod setup;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use autoxide_kernel::access::Scope;
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, TenantContext, root_of};
use autoxide_organizations::api::{EffectiveSetting, OrgDirectory, Unit};
use autoxide_testkit::{TestApp, TestAppBuilder, TestDatabase};
use axum::http::{Method, StatusCode};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use serde_json::{Value, json};

/// Quyền của người quản trị trong test.
pub(super) const ADMIN: [&str; 3] =
    ["bookings.manage", "bookings.services.manage", "bookings.staff"];

/// Cây tổ chức giả: một giá trị cho mỗi cài đặt, như nhau ở mọi nút; chưa đặt thì `None` (mặc định).
#[derive(Default)]
pub(super) struct Orgs(Mutex<HashMap<String, Value>>);

impl Orgs {
    pub(super) fn set(&self, key: &str, value: Value) {
        self.0.lock().unwrap().insert(key.to_owned(), value);
    }
}

#[async_trait::async_trait]
impl OrgDirectory for Orgs {
    async fn unit(&self, _: &TenantContext, _: OrgId) -> Result<Option<Unit>, Error> {
        Ok(None)
    }
    async fn root(&self, _: &TenantContext) -> Result<Unit, Error> {
        Err(Error::internal("not used"))
    }
    async fn ancestors(&self, _: &TenantContext, _: OrgId) -> Result<Vec<Unit>, Error> {
        Ok(Vec::new())
    }
    async fn is_within(&self, _: &TenantContext, _: OrgId, _: OrgId) -> Result<bool, Error> {
        Ok(true)
    }
    async fn setting(
        &self,
        _: &TenantContext,
        id: OrgId,
        key: &str,
    ) -> Result<Option<EffectiveSetting>, Error> {
        let value = self.0.lock().unwrap().get(key).cloned();
        Ok(Some(EffectiveSetting::new(key, value, Some(id), false)))
    }
}

/// App đã dựng cùng cây tổ chức giả của nó.
pub(super) struct Shop {
    pub app: TestApp,
    pub orgs: Arc<Orgs>,
}

impl std::ops::Deref for Shop {
    type Target = TestApp;
    fn deref(&self) -> &TestApp {
        &self.app
    }
}

/// App có cây tổ chức giả (múi giờ, chính sách) và `bookings`; `extra` thêm module / người dùng.
pub(super) async fn app_with<'a>(
    test: &'a TestDatabase,
    extra: impl FnOnce(TestAppBuilder<'a>) -> TestAppBuilder<'a>,
) -> Shop {
    let root = root_of(DEFAULT_TENANT);
    let orgs = Arc::new(Orgs::default());
    let builder = TestApp::builder(test)
        .member("admin", 0xad, root, &ADMIN.map(|name| (name, Scope::Tenant)))
        .member("reader", 0x4ead, root, &[("bookings.staff", Scope::Tenant)])
        .provide::<dyn OrgDirectory>(orgs.clone());
    let app = extra(builder).module(crate::module()).build().await.unwrap();
    Shop { app, orgs }
}

pub(super) async fn app(test: &TestDatabase) -> Shop {
    app_with(test, |builder| builder).await
}

/// Gọi `/api/v1{uri}`; trả trạng thái và thân JSON (`null` khi rỗng).
pub(super) async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1{uri}"));
    if let Some(who) = who {
        request = request.token(who);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

/// Như [`call`] với người quản trị, và đòi đúng trạng thái `expected`.
pub(super) async fn admin(
    app: &TestApp,
    method: &str,
    uri: &str,
    body: Option<Value>,
    expected: StatusCode,
) -> Value {
    let (status, body) = call(app, method, uri, Some("admin"), body).await;
    assert_eq!(status, expected, "{method} {uri}: {body}");
    body
}

/// Đặt cài đặt `key` (mọi nút).
pub(super) fn set_setting(shop: &Shop, key: &str, value: Value) {
    shop.orgs.set(key, value);
}

/// Thứ Hai đầu tiên sau ít nhất 7 ngày nữa (giờ trống chỉ chào giờ tương lai).
pub(super) fn next_monday() -> NaiveDate {
    let mut day = (Utc::now() + Duration::days(7)).date_naive();
    while day.weekday().number_from_monday() != 1 {
        day += Duration::days(1);
    }
    day
}

pub(super) fn utc(day: NaiveDate, hour: u32, minute: u32) -> DateTime<Utc> {
    day.and_hms_opt(hour, minute, 0).unwrap().and_utc()
}

/// Một nhân viên làm thứ Hai 9:00–12:00 giờ địa phương; trả ID.
pub(super) async fn staff(app: &TestApp, name: &str) -> String {
    let created = admin(
        app,
        "POST",
        "/bookings/resources",
        Some(json!({"kind": "staff", "name": name})),
        StatusCode::CREATED,
    )
    .await;
    let id = created["data"]["id"].as_str().unwrap().to_owned();
    let ranges = json!({"ranges": [{"weekday": 1, "start_minute": 540, "end_minute": 720}]});
    admin(app, "PUT", &format!("/bookings/resources/{id}/schedule"), Some(ranges), StatusCode::OK)
        .await;
    id
}

/// Một dịch vụ `minutes` phút, đệm sau `after`, giá 200.000 VND, do `performers` làm.
pub(super) async fn service(
    app: &TestApp,
    minutes: i32,
    after: i32,
    extra: Value,
    performers: &[&str],
) -> String {
    let mut body = json!({
        "name": "Haircut",
        "names": {"vi": "Cắt tóc"},
        "duration_minutes": minutes,
        "buffer_after_minutes": after,
        "price": {"amount": "200000", "currency": "VND"},
        "performers": performers,
    });
    if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        body.extend(extra.clone());
    }
    let created = admin(app, "POST", "/bookings/services", Some(body), StatusCode::CREATED).await;
    created["data"]["id"].as_str().unwrap().to_owned()
}

/// Giờ trống công khai của ngày `day` (theo UTC ± 1 ngày) — danh sách `local_start`.
pub(super) async fn openings(app: &TestApp, service: &str, day: NaiveDate) -> (Value, Vec<String>) {
    let from = utc(day, 0, 0) - Duration::days(1);
    let to = utc(day, 0, 0) + Duration::days(2);
    let uri = format!(
        "/bookings/public/availability?service_id={service}&from={}&to={}",
        from.format("%Y-%m-%dT%H:%M:%SZ"),
        to.format("%Y-%m-%dT%H:%M:%SZ")
    );
    let (status, body) = call(app, "GET", &uri, None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let starts = body["data"]["openings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|opening| opening["local_start"].as_str().unwrap().to_owned())
        .collect();
    (body, starts)
}
