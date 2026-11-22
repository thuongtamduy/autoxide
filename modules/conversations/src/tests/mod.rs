//! Test của module `conversations`, trên database gate đang chọn (gate chạy cả SQLite và Postgres).
//! `TestApp` dựng module như binary: migration → start → route → lớp HTTP của kernel.

mod assist;
mod chat;
mod privacy;
mod stream;
mod voice;

use autoxide_kernel::time::Timestamp;
pub(crate) use autoxide_notifications::api::testing::FakeNotifier;
use jiff::SignedDuration;
use std::sync::{Arc, Mutex};

use autoxide_kernel::access::{Principal, Scope};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, TenantContext, root_of};
use autoxide_notifications::api::Notifier;
use autoxide_support::api::{AutoResponder, InboundQuestion, ReplyDecision};
use autoxide_testkit::{TestApp, TestAppBuilder, TestDatabase};
use axum::http::{Method, StatusCode};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::ExprTrait as _;
use serde_json::{Value, json};
use uuid::Uuid;

/// Nhân viên ở nút gốc, đủ quyền.
pub(crate) const AGENT: u128 = 0xa1;
/// Nhân viên thứ hai ở nút gốc.
pub(crate) const AGENT2: u128 = 0xa2;
/// Nhân viên của một nhánh khác (nút 0x7).
pub(crate) const BRANCH: u128 = 0xa7;
/// Người mua có tài khoản.
pub(crate) const BUYER: u128 = 0xb1;
pub(crate) const OTHER_BUYER: u128 = 0xb2;

pub(crate) const STAFF: [(&str, Scope); 6] = [
    ("conversations.read", Scope::Tenant),
    ("conversations.reply", Scope::Tenant),
    ("conversations.manage", Scope::Tenant),
    ("conversations.call", Scope::Tenant),
    ("conversations.recordings.read", Scope::Tenant),
    ("conversations.voice.manage", Scope::Tenant),
];

/// Bộ trả lời tự động giả, đứng chỗ `AutoResponder` của `support`: trả quyết định định sẵn và ghi lại
/// câu hỏi nhận được.
#[derive(Default)]
pub(crate) struct FakeResponder {
    pub decision: Mutex<Option<ReplyDecision>>,
    pub questions: Mutex<Vec<InboundQuestion>>,
}

#[async_trait::async_trait]
impl AutoResponder for FakeResponder {
    async fn decide(
        &self,
        _tenant: &TenantContext,
        question: InboundQuestion,
    ) -> Result<ReplyDecision, Error> {
        self.questions.lock().unwrap().push(question);
        Ok(self.decision.lock().unwrap().clone().unwrap_or(ReplyDecision::Handoff))
    }
}

pub(crate) struct Harness {
    pub app: TestApp,
    pub notifier: Arc<FakeNotifier>,
    pub responder: Arc<FakeResponder>,
}

/// Người dùng chung của mọi test; `extra` thêm người dùng / cài đặt / module riêng.
pub(crate) async fn harness_with(
    test: &TestDatabase,
    responder: bool,
    extra: impl FnOnce(TestAppBuilder<'_>) -> TestAppBuilder<'_>,
) -> Harness {
    let notifier = Arc::new(FakeNotifier::default());
    let fake = Arc::new(FakeResponder::default());
    let root = root_of(DEFAULT_TENANT);
    let mut builder = TestApp::builder(test)
        .provide::<dyn Notifier>(notifier.clone())
        .member("agent", AGENT, root, &STAFF)
        .member("agent2", AGENT2, root, &STAFF)
        .member(
            "branch",
            BRANCH,
            OrgId::from_uuid(Uuid::from_u128(0x7)),
            &[
                ("conversations.read", Scope::Unit),
                ("conversations.reply", Scope::Unit),
                ("conversations.call", Scope::Unit),
            ],
        )
        .principal("buyer", Principal::customer(Uuid::from_u128(BUYER)))
        .principal("other", Principal::customer(Uuid::from_u128(OTHER_BUYER)));
    if responder {
        builder = builder.provide::<dyn AutoResponder>(fake.clone());
    }
    let app = extra(builder).module(crate::module()).build().await.unwrap();
    Harness { app, notifier, responder: fake }
}

pub(crate) async fn harness(test: &TestDatabase) -> Harness {
    harness_with(test, false, |builder| builder).await
}

/// Gọi `/api/v1/conversations{uri}`; `who` rỗng: không đăng nhập. Trả trạng thái và thân JSON.
pub(crate) async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/conversations{uri}"));
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

/// Người mua `who` bắt đầu cuộc trò chuyện với cửa hàng; trả ID cuộc.
pub(crate) async fn start(app: &TestApp, who: &str, body: &str) -> String {
    let input = json!({"body": body, "client_id": format!("c-{}", Uuid::now_v7()), "locale": "vi"});
    let (status, started) = call(app, "POST", "/me/threads", who, Some(input)).await;
    assert_eq!(status, StatusCode::CREATED, "{started}");
    started["data"]["thread"]["id"].as_str().unwrap().to_owned()
}

/// Nhân viên `who` trả lời cuộc `id`.
pub(crate) async fn reply(app: &TestApp, who: &str, id: &str, body: &str) -> Value {
    let input = json!({"body": body, "client_id": format!("r-{}", Uuid::now_v7())});
    let (status, sent) =
        call(app, "POST", &format!("/threads/{id}/messages"), who, Some(input)).await;
    assert_eq!(status, StatusCode::CREATED, "{sent}");
    sent["data"].clone()
}

/// Chạy mọi job tới hạn (trả lời tự động, thông báo, dọn dẹp).
pub(crate) async fn run_jobs(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    for _ in 0..10 {
        if worker.run_once().await.unwrap() == 0 {
            break;
        }
    }
}

/// Đưa mọi job đang hẹn giờ của module về tới hạn ngay (thay cho chờ thật).
pub(crate) async fn due_now(app: &TestApp) {
    let past = Timestamp::now() - SignedDuration::from_mins(1);
    let statement = sea_orm::sea_query::Query::update()
        .table(sea_orm::sea_query::Alias::new("conversations_jobs"))
        .value(sea_orm::sea_query::Alias::new("run_at"), sea_orm::Value::from(past))
        .and_where(
            sea_orm::sea_query::Expr::col(sea_orm::sea_query::Alias::new("completed_at")).is_null(),
        )
        .to_owned();
    app.database().write().execute(&statement).await.unwrap();
}

/// Tin của cuộc `id` mà người mua thấy, cũ trước.
pub(crate) async fn messages(app: &TestApp, id: &str) -> Vec<Value> {
    let (status, page) =
        call(app, "GET", &format!("/me/threads/{id}/messages?order=asc"), "buyer", None).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    page["data"].as_array().unwrap().clone()
}

/// Service của module trên database của `app` (gọi thẳng job, handler event).
pub(crate) fn service_of() -> Arc<crate::service::ConversationsService> {
    let parts = crate::service::Parts {
        outbox: autoxide_kernel::event::Outbox::standalone("conversations"),
        jobs: autoxide_kernel::job::JobQueue::new("conversations"),
        settings: crate::settings::Settings::default(),
        verification: autoxide_kernel::access::PostingVerification::default(),
        notifier: None,
        media: None,
        intake: None,
        responder: None,
        ai: None,
        authenticator: None,
        resources: None,
        voice: Vec::new(),
        accounts: None,
        storage: None,
        secret: None,
        translations: autoxide_kernel::i18n::Translations::default(),
    };
    Arc::new(crate::service::ConversationsService::new(parts, Default::default()))
}
