//! Test của module `support`, trên database gate đang chọn (gate chạy cả SQLite và Postgres).
//! `TestApp` dựng module như binary: migration → start → route → lớp HTTP của kernel.

mod auto;
mod extras;
mod guests;
mod scope;
mod sla;
mod tickets;
mod tools;

pub(crate) use autoxide_notifications::api::testing::FakeNotifier;
use std::sync::{Arc, Mutex};

use autoxide_kernel::access::{Principal, Scope};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, TenantContext, root_of};
use autoxide_media::api::{Media, MediaRef, MediaVisibility};
use autoxide_notifications::api::Notifier;
use autoxide_testkit::{TestApp, TestAppBuilder, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::Value;
use uuid::Uuid;

/// Nhân viên ở nút gốc, đủ quyền.
pub(crate) const AGENT: u128 = 0xa1;
/// Người mua có tài khoản.
pub(crate) const BUYER: u128 = 0xb1;
pub(crate) const OTHER_BUYER: u128 = 0xb2;

pub(crate) const STAFF: [(&str, Scope); 5] = [
    ("support.tickets.read", Scope::Tenant),
    ("support.tickets.reply", Scope::Tenant),
    ("support.tickets.manage", Scope::Tenant),
    ("support.settings.manage", Scope::Tenant),
    ("support.auto_reply.manage", Scope::Tenant),
];

/// Mã xác thực cuối cùng đã gửi.
pub(crate) fn last_code(notifier: &FakeNotifier) -> String {
    let sent = notifier.calls();
    let code = sent
        .iter()
        .rev()
        .find(|notification| notification.kind == "support.verification_code")
        .expect("a verification code was sent");
    code.params["code"].as_str().unwrap().to_owned()
}

/// Thư viện media giả: item `id` do người `owner` tải lên.
#[derive(Default)]
pub(crate) struct FakeMedia(pub Mutex<Vec<(Uuid, Option<Uuid>)>>);

#[async_trait::async_trait]
impl Media for FakeMedia {
    async fn find_many(&self, _: &TenantContext, ids: &[Uuid]) -> Result<Vec<MediaRef>, Error> {
        let known = self.0.lock().unwrap().clone();
        Ok(ids
            .iter()
            .filter(|id| known.iter().any(|(item, _)| item == *id))
            .map(|id| MediaRef {
                id: *id,
                visibility: MediaVisibility::Private,
                content_type: "image/png".to_owned(),
                width: None,
                height: None,
                alt: String::new(),
                alt_ai_assisted: false,
                url: format!("/api/v1/media/{id}/content"),
                variants: Vec::new(),
            })
            .collect())
    }

    async fn uploaders(
        &self,
        _: &TenantContext,
        ids: &[Uuid],
    ) -> Result<Vec<(Uuid, Option<Uuid>)>, Error> {
        let known = self.0.lock().unwrap().clone();
        Ok(known.into_iter().filter(|(id, _)| ids.contains(id)).collect())
    }
}

pub(crate) struct Harness {
    pub app: TestApp,
    pub notifier: Arc<FakeNotifier>,
    pub media: Arc<FakeMedia>,
}

/// Người dùng chung của mọi test; `extra` thêm người dùng / cài đặt riêng.
pub(crate) async fn harness_with(
    test: &TestDatabase,
    extra: impl FnOnce(TestAppBuilder<'_>) -> TestAppBuilder<'_>,
) -> Harness {
    let notifier = Arc::new(FakeNotifier::default());
    let media = Arc::new(FakeMedia::default());
    let root = root_of(DEFAULT_TENANT);
    let builder = TestApp::builder(test)
        .setting("AUTOXIDE_ANTISPAM_MIN_FILL_SECONDS", "0")
        .provide::<dyn Notifier>(notifier.clone())
        .provide::<dyn Media>(media.clone())
        .member("agent", AGENT, root, &STAFF)
        .principal("buyer", Principal::customer(Uuid::from_u128(BUYER)))
        .principal("other", Principal::customer(Uuid::from_u128(OTHER_BUYER)));
    let app = extra(builder).module(crate::module()).build().await.unwrap();
    Harness { app, notifier, media }
}

pub(crate) async fn harness(test: &TestDatabase) -> Harness {
    harness_with(test, |builder| builder).await
}

/// Gọi `/api/v1/support{uri}`; `who` rỗng: không đăng nhập. Trả trạng thái và thân JSON.
pub(crate) async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/support{uri}"));
    if !who.is_empty() {
        request = request.token(who);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let status = answer.status();
    (status, if answer.body().is_empty() { Value::Null } else { answer.json() })
}

/// Token form chống spam cho form mở phiếu.
pub(crate) async fn form_token(app: &TestApp) -> String {
    let issued = app.get("/api/v1/antispam/token?scope=support:tickets").send().await.unwrap();
    assert_eq!(issued.status(), StatusCode::OK, "{}", issued.text());
    issued.json()["data"]["token"].as_str().unwrap().to_owned()
}

/// Giao mọi event đang chờ cho bên nghe (thông báo, ẩn danh hoá…).
pub(crate) async fn deliver(app: &TestApp) {
    let dispatcher = app.modules().dispatcher().unwrap();
    while dispatcher.run_once().await.unwrap() > 0 {}
}

/// Service của module trên database của `app` (gọi thẳng job, handler event; không cần `auth`).
pub(crate) fn service_of(app: &TestApp) -> Arc<crate::service::SupportService> {
    let parts = crate::service::Parts {
        outbox: autoxide_kernel::event::Outbox::standalone("support"),
        guard: app.api::<autoxide_kernel::antispam::SpamGuard>().unwrap(),
        secret: (*app.api::<autoxide_kernel::secret::SecretKey>().unwrap()).clone(),
        notifier: None,
        media: None,
        accounts: None,
        authenticator: None,
        resources: None,
        complaints: None,
        ai: None,
        facts: crate::facts::FactRegistry::default(),
        jobs: autoxide_kernel::job::JobQueue::new("support"),
    };
    Arc::new(crate::service::SupportService::new(parts, Default::default()))
}
