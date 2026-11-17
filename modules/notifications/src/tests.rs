//! Test của module `notifications` qua REST và trait công bố (cả SQLite và Postgres): email + hộp thư,
//! chống trùng, tuỳ chọn nhận (bảo mật không tắt được), tin quảng cáo cần đồng ý và kiểm lại ngay trước
//! khi gửi, khung giờ hoãn tin, tin mời chỉ một lần, link huỷ nhận, đẩy bị mất thì xoá đăng ký, thông
//! báo của nền tảng, giới hạn mỗi giờ, permission kiểm trước khi tra. Người dùng, quản trị viên, sổ
//! đồng ý và quy tắc là bản giả (test của module không dựng module khác); đi qua module thật ở
//! `crates/app/src/e2e/notifications.rs`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use autoxide_auth::api::{TenantAdmins, User, Users};
use autoxide_compliance::api::{
    ComplianceRules, Consent, ConsentEvidence, ConsentKey, ConsentLedger, ConsentSubject, Rule,
};
use autoxide_kernel::access::Realm;
use autoxide_kernel::error::Error;
use autoxide_kernel::mail::{Mail, Mailer};
use autoxide_kernel::push::{MobilePushSender, WebPushSender};
use autoxide_kernel::sms::SmsSender;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, TenantContext, TenantId};
use autoxide_testkit::{
    FakeMobilePushSender, FakeSmsSender, FakeWebPushSender, TestApp, TestDatabase,
};
use axum::http::{Method, StatusCode};
use chrono::{DateTime, Duration as Span, Timelike, Utc};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{
    Category, Channel, Notification, NotificationKind, NotificationTemplate, Notifier, Recipient,
    SkipReason,
};

mod templates;

// Bọc `NotificationTemplate::new` để `cargo xtask check` không đòi bản dịch cho loại chỉ có trong test.
const fn template(
    key: &'static str,
    title: &'static str,
    body: &'static str,
    short: &'static str,
) -> NotificationTemplate {
    NotificationTemplate::new(key, title, body, short)
}

const ADMIN_PERMISSIONS: &[&str] = &[
    "notifications.send",
    "notifications.send_marketing",
    "notifications.messages.read",
    "notifications.templates.manage",
    "notifications.branding.manage",
];

const SHIPPED: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::Email, Channel::InApp],
    template(
        "shop.order_shipped",
        "Order {code} shipped",
        "Your order {code} is on its way.",
        "Order {code} shipped",
    ),
);

const SIGN_IN: NotificationKind = NotificationKind::new(
    Category::Security,
    &[Channel::Email, Channel::InApp],
    template("shop.new_sign_in", "New sign-in", "Someone signed in.", "New sign-in"),
);

const PROMO: NotificationKind = NotificationKind::new(
    Category::Marketing,
    &[Channel::Email, Channel::Sms],
    template("shop.promo", "Sale {percent}%", "Everything {percent}% off.", "Sale {percent}%"),
);

const INVITE: NotificationKind = NotificationKind::new(
    Category::Marketing,
    &[Channel::Sms],
    template("shop.invite", "Join", "Join our list.", "Join our list"),
)
.invitation();

const PUSHED: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::WebPush, Channel::MobilePush],
    template("shop.pushed", "Ping", "Ping body", "Ping"),
);

fn user(id: u128, email: &str) -> User {
    let at = Utc::now();
    User {
        id: Uuid::from_u128(id),
        email: email.to_owned(),
        name: email.to_owned(),
        disabled_at: None,
        email_verified_at: Some(at),
        created_at: at,
        updated_at: at,
        realm: Realm::Staff,
        locale: None,
        time_zone: None,
    }
}

/// Người dùng và quản trị viên giả.
struct People(HashMap<Uuid, User>);

#[async_trait::async_trait]
impl Users for People {
    async fn find(&self, _tenant: &TenantContext, id: Uuid) -> Result<Option<User>, Error> {
        Ok(self.0.get(&id).cloned())
    }
}

#[async_trait::async_trait]
impl TenantAdmins for People {
    async fn admins(&self, _tenant: &TenantContext) -> Result<Vec<User>, Error> {
        Ok(self.0.values().filter(|user| user.email.starts_with("admin")).cloned().collect())
    }
}

/// Hộp thư đi giả.
#[derive(Default)]
struct Outbox(Mutex<Vec<Mail>>);

impl Outbox {
    fn all(&self) -> Vec<Mail> {
        self.0.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl Mailer for Outbox {
    async fn send(&self, _tenant: &TenantContext, mail: Mail) -> Result<(), Error> {
        self.0.lock().unwrap().push(mail);
        Ok(())
    }
}

/// Sổ đồng ý giả: (đối tượng, mục đích, kênh); kênh rỗng bao mọi kênh.
#[derive(Default)]
struct Ledger(Mutex<HashSet<(ConsentSubject, String, String)>>);

impl Ledger {
    fn allow(&self, subject: ConsentSubject, channel: &str) {
        self.0.lock().unwrap().insert((subject, "marketing".to_owned(), channel.to_owned()));
    }

    fn deny(&self, subject: &ConsentSubject) {
        self.0.lock().unwrap().retain(|(who, _, _)| who != subject);
    }
}

fn consent(key: &ConsentKey, state: &str) -> Consent {
    Consent {
        id: Uuid::now_v7(),
        purpose: key.purpose.clone(),
        scope: key.scope.clone(),
        channel: key.channel.clone(),
        state: state.to_owned(),
        granted_at: None,
        withdrawn_at: None,
        subject_value: None,
    }
}

#[async_trait::async_trait]
impl ConsentLedger for Ledger {
    async fn grant(
        &self,
        _tenant: TenantId,
        subject: ConsentSubject,
        key: ConsentKey,
        _evidence: ConsentEvidence,
        _source_ref: &str,
    ) -> Result<Consent, Error> {
        self.0.lock().unwrap().insert((subject, key.purpose.clone(), key.channel.clone()));
        Ok(consent(&key, "granted"))
    }

    async fn withdraw(
        &self,
        _tenant: TenantId,
        subject: ConsentSubject,
        key: ConsentKey,
        _source: &str,
        _source_ref: &str,
    ) -> Result<Consent, Error> {
        self.0
            .lock()
            .unwrap()
            .retain(|(who, purpose, _)| !(*who == subject && *purpose == key.purpose));
        Ok(consent(&key, "withdrawn"))
    }

    async fn is_granted(
        &self,
        _tenant: TenantId,
        subject: &ConsentSubject,
        key: &ConsentKey,
    ) -> Result<bool, Error> {
        let granted = self.0.lock().unwrap();
        Ok(granted.contains(&(subject.clone(), key.purpose.clone(), key.channel.clone()))
            || granted.contains(&(subject.clone(), key.purpose.clone(), String::new())))
    }

    async fn granted_many(
        &self,
        tenant: TenantId,
        subjects: &[ConsentSubject],
        key: &ConsentKey,
    ) -> Result<Vec<bool>, Error> {
        let mut found = Vec::new();
        for subject in subjects {
            found.push(self.is_granted(tenant, subject, key).await?);
        }
        Ok(found)
    }

    async fn get(&self, _tenant: TenantId, _consent_id: Uuid) -> Result<Option<Consent>, Error> {
        Ok(None)
    }
}

/// Quy tắc giả: thân quy tắc theo khoá.
struct Rules(Mutex<HashMap<String, Value>>);

#[async_trait::async_trait]
impl ComplianceRules for Rules {
    async fn effective(
        &self,
        _tenant: TenantId,
        key: &str,
        at: DateTime<Utc>,
    ) -> Result<Option<Rule>, Error> {
        Ok(self.0.lock().unwrap().get(key).map(|body| Rule {
            jurisdiction: "vn".to_owned(),
            key: key.to_owned(),
            version: 1,
            effective_from: at - Span::days(1),
            effective_to: None,
            body: body.clone(),
            legal_basis: "Test".to_owned(),
            needs_legal_review: true,
        }))
    }
}

struct Fixture {
    app: TestApp,
    notifier: Arc<dyn Notifier>,
    mails: Arc<Outbox>,
    ledger: Arc<Ledger>,
    rules: Arc<Rules>,
    sms: Arc<FakeSmsSender>,
    web_push: Arc<FakeWebPushSender>,
    mobile_push: Arc<FakeMobilePushSender>,
}

async fn fixture_with(test: &TestDatabase, per_hour: &str) -> Fixture {
    let people = Arc::new(People(HashMap::from([
        (Uuid::from_u128(1), user(1, "admin@example.com")),
        (Uuid::from_u128(2), user(2, "alice@example.com")),
    ])));
    let mails = Arc::new(Outbox::default());
    let ledger = Arc::new(Ledger::default());
    let rules = Arc::new(Rules(Mutex::new(HashMap::from([
        (
            "notifications.marketing_labels".to_owned(),
            json!({"sms": {"prefix": "QC "}, "email": {"subject_prefix": "[QC] "}}),
        ),
        (
            "notifications.send_windows".to_owned(),
            json!({"utc_offset_minutes": 0, "marketing": {"sms": null, "email": null}}),
        ),
    ]))));
    let sms = Arc::new(FakeSmsSender::new());
    let web_push = Arc::new(FakeWebPushSender::new());
    let mobile_push = Arc::new(FakeMobilePushSender::new());
    let users: Arc<dyn Users> = people.clone();
    let admins: Arc<dyn TenantAdmins> = people;
    let mailer: Arc<dyn Mailer> = mails.clone();
    let consents: Arc<dyn ConsentLedger> = ledger.clone();
    let compliance: Arc<dyn ComplianceRules> = rules.clone();
    let sender: Arc<dyn SmsSender> = sms.clone();
    let browser: Arc<dyn WebPushSender> = web_push.clone();
    let mobile: Arc<dyn MobilePushSender> = mobile_push.clone();
    let app = TestApp::builder(test)
        .setting(
            "AUTOXIDE_NOTIFICATIONS_UNSUBSCRIBE_URL",
            "https://shop.example/unsubscribe?token={token}",
        )
        .setting("AUTOXIDE_NOTIFICATIONS_PER_RECIPIENT_PER_HOUR", per_hour)
        .user("admin", 1, ADMIN_PERMISSIONS)
        .user("alice", 2, &[])
        .user("staff", 3, &["notifications.platform_notices.send"])
        .provide::<dyn Users>(users)
        .provide::<dyn TenantAdmins>(admins)
        .provide::<dyn Mailer>(mailer)
        .provide::<dyn ConsentLedger>(consents)
        .provide::<dyn ComplianceRules>(compliance)
        .provide::<dyn SmsSender>(sender)
        .provide::<dyn WebPushSender>(browser)
        .provide::<dyn MobilePushSender>(mobile)
        .module(crate::module())
        .build()
        .await
        .unwrap();
    let notifier = app.api::<dyn Notifier>().unwrap();
    for kind in [SHIPPED, SIGN_IN, PROMO, INVITE, PUSHED] {
        notifier.declare(kind).unwrap();
    }
    Fixture { app, notifier, mails, ledger, rules, sms, web_push, mobile_push }
}

async fn fixture(test: &TestDatabase) -> Fixture {
    fixture_with(test, "20").await
}

async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/notifications{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

async fn run_jobs(app: &TestApp) {
    for _ in 0..3 {
        app.modules().worker(4).unwrap().blocked_delay(Duration::ZERO).run_once().await.unwrap();
    }
}

fn to_alice(kind: &NotificationKind, key: &str) -> Notification {
    Notification::new(
        kind.key(),
        Recipient::User(Uuid::from_u128(2)),
        json!({"code": "DH001", "percent": 30}),
        key,
    )
}

/// Trạng thái các tin (theo kênh) mang khoá chống trùng `key`.
async fn statuses(app: &TestApp, key: &str) -> HashMap<String, (String, String)> {
    let (status, page) = call(app, "GET", "/messages?limit=200", "admin", None).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    page["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["dedupe_key"] == key)
        .map(|row| {
            (
                row["channel"].as_str().unwrap().to_owned(),
                (
                    row["status"].as_str().unwrap().to_owned(),
                    row["skip_reason"].as_str().unwrap().to_owned(),
                ),
            )
        })
        .collect()
}

#[tokio::test]
async fn email_va_hop_thu_khong_gui_trung() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        let outcome =
            fx.notifier.notify(DEFAULT_TENANT, to_alice(&SHIPPED, "order-1")).await.unwrap();
        assert_eq!(outcome.message_ids.len(), 2, "{outcome:?}");
        // Gọi lại cùng khoá: cùng các tin, không thêm.
        let again =
            fx.notifier.notify(DEFAULT_TENANT, to_alice(&SHIPPED, "order-1")).await.unwrap();
        assert_eq!(again.message_ids, outcome.message_ids);
        run_jobs(&fx.app).await;
        run_jobs(&fx.app).await;

        let mails = fx.mails.all();
        assert_eq!(mails.len(), 1);
        assert_eq!(mails[0].to, "alice@example.com");
        assert_eq!(mails[0].subject, "Order DH001 shipped");
        assert!(mails[0].headers.is_empty(), "tin giao dịch không có List-Unsubscribe");

        let (status, inbox) = call(&fx.app, "GET", "/me/inbox", "alice", None).await;
        assert_eq!(status, StatusCode::OK, "{inbox}");
        assert_eq!(inbox["data"][0]["title"], "Order DH001 shipped");
        assert!(inbox["pagination"].is_object());
        let (_, count) = call(&fx.app, "GET", "/me/inbox/unread-count", "alice", None).await;
        assert_eq!(count["data"]["unread"], 1);
        let id = inbox["data"][0]["id"].as_str().unwrap().to_owned();
        // Người khác không đánh dấu được tin của alice.
        let (status, _) =
            call(&fx.app, "POST", &format!("/me/inbox/{id}/read"), "admin", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, read) =
            call(&fx.app, "POST", &format!("/me/inbox/{id}/read"), "alice", None).await;
        assert_eq!(status, StatusCode::OK, "{read}");
        assert!(read["data"]["read_at"].is_string());
        let (_, count) = call(&fx.app, "GET", "/me/inbox/unread-count", "alice", None).await;
        assert_eq!(count["data"]["unread"], 0);
        let (_, all) = call(&fx.app, "POST", "/me/inbox/read-all", "alice", None).await;
        assert_eq!(all["data"]["updated"], 0);

        let sent = statuses(&fx.app, "order-1").await;
        assert_eq!(sent["email"].0, "sent");
        assert_eq!(sent["in_app"].0, "sent");
        let (status, _) = call(&fx.app, "GET", "/messages", "alice", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Loại chưa khai.
        let error = fx
            .notifier
            .notify(
                DEFAULT_TENANT,
                Notification::new("shop.nope", Recipient::User(Uuid::from_u128(2)), json!({}), "x"),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), "notifications.kind_unknown");
        let (status, kinds) = call(&fx.app, "GET", "/kinds", "alice", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            kinds["data"]
                .as_array()
                .unwrap()
                .iter()
                .any(|kind| kind["key"] == "shop.new_sign_in" && kind["locked"] == true)
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tuy_chon_tat_bo_qua_nhung_bao_mat_khong_tat_duoc() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        let off = json!({"items": [{"target": "category:transactional", "channel": "email", "enabled": false}]});
        let (status, prefs) = call(&fx.app, "PUT", "/me/preferences", "alice", Some(off)).await;
        assert_eq!(status, StatusCode::OK, "{prefs}");
        let shipped = prefs["data"].as_array().unwrap().iter().find(|kind| kind["kind"] == "shop.order_shipped").unwrap();
        assert_eq!(shipped["channels"][0]["enabled"], false);
        let locked = json!({"items": [{"target": "kind:shop.new_sign_in", "channel": "email", "enabled": false}]});
        let (status, problem) = call(&fx.app, "PUT", "/me/preferences", "alice", Some(locked)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["code"], "notifications.preference_locked");
        let bad = json!({"items": [{"target": "kind:nope", "channel": "fax", "enabled": true}]});
        let (_, problem) = call(&fx.app, "PUT", "/me/preferences", "alice", Some(bad)).await;
        assert_eq!(problem["errors"].as_array().unwrap().len(), 2, "{problem}");

        let outcome = fx.notifier.notify(DEFAULT_TENANT, to_alice(&SHIPPED, "order-2")).await.unwrap();
        assert_eq!(outcome.skipped.len(), 1);
        assert_eq!(outcome.skipped[0].channel, Channel::Email);
        assert_eq!(outcome.skipped[0].reason, SkipReason::PreferenceOff);
        // Tin bảo mật bỏ qua tuỳ chọn: vẫn gửi email.
        fx.notifier.notify(DEFAULT_TENANT, to_alice(&SIGN_IN, "signin-1")).await.unwrap();
        run_jobs(&fx.app).await;
        let mails = fx.mails.all();
        assert_eq!(mails.len(), 1);
        assert_eq!(mails[0].subject, "New sign-in");
        let (_, mine) = call(&fx.app, "GET", "/me/preferences", "alice", None).await;
        let security = mine["data"].as_array().unwrap().iter().find(|kind| kind["kind"] == "shop.new_sign_in").unwrap();
        assert_eq!(security["channels"][0]["locked"], true);
        assert_eq!(security["channels"][0]["enabled"], true);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn quang_cao_can_dong_y_kiem_lai_truoc_khi_gui_va_link_huy_nhan() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        let bob = || Notification::new(PROMO.key(), Recipient::Email("Bob@Example.com".to_owned()), json!({"percent": 30}), "");
        let mut first = bob();
        first.dedupe_key = "promo-1".to_owned();
        let outcome = fx.notifier.notify(DEFAULT_TENANT, first).await.unwrap();
        // Không có đồng ý: email bỏ qua; SMS không có số.
        assert!(outcome.message_ids.is_empty());
        assert!(outcome.skipped.iter().any(|skip| skip.channel == Channel::Email && skip.reason == SkipReason::NoConsent));
        assert!(outcome.skipped.iter().any(|skip| skip.channel == Channel::Sms && skip.reason == SkipReason::NoAddress));

        let subject = ConsentSubject::Email("bob@example.com".to_owned());
        fx.ledger.allow(subject.clone(), "email");
        let mut second = bob();
        second.dedupe_key = "promo-2".to_owned();
        assert_eq!(fx.notifier.notify(DEFAULT_TENANT, second).await.unwrap().message_ids.len(), 1);
        // Rút đồng ý trước khi job chạy: không gửi.
        fx.ledger.deny(&subject);
        run_jobs(&fx.app).await;
        assert!(fx.mails.all().is_empty());
        assert_eq!(statuses(&fx.app, "promo-2").await["email"], ("skipped".to_owned(), "consent_withdrawn".to_owned()));

        fx.ledger.allow(subject.clone(), "");
        let mut third = bob();
        third.dedupe_key = "promo-3".to_owned();
        fx.notifier.notify(DEFAULT_TENANT, third).await.unwrap();
        run_jobs(&fx.app).await;
        let mails = fx.mails.all();
        assert_eq!(mails.len(), 1);
        assert_eq!(mails[0].subject, "[QC] Sale 30%");
        let (name, value) = &mails[0].headers[0];
        assert_eq!(name, "List-Unsubscribe");
        assert!(mails[0].text.contains("https://shop.example/unsubscribe?token="), "{}", mails[0].text);
        let token = value.trim_start_matches("<https://shop.example/unsubscribe?token=").trim_end_matches('>').to_owned();

        let (status, problem) = call(&fx.app, "POST", "/public/unsubscribe", "", Some(json!({"token": "nope"}))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(problem["code"], "notifications.link_invalid");
        let answer = fx.app.post("/api/v1/notifications/public/unsubscribe").json(json!({"token": token})).send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::NO_CONTENT);
        let ledger: &dyn ConsentLedger = fx.ledger.as_ref();
        let key = ConsentKey::new("marketing", "", "email");
        assert!(!ledger.is_granted(DEFAULT_TENANT, &subject, &key).await.unwrap());

        // Gửi tay: quảng cáo cần thêm `notifications.send_marketing`.
        let body = json!({"kind": "shop.promo", "recipient": {"email": "x@example.com"}, "params": {"percent": 5}, "dedupe_key": "m"});
        let (status, _) = call(&fx.app, "POST", "/messages", "alice", Some(body.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, sent) = call(&fx.app, "POST", "/messages", "admin", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{sent}");
        assert_eq!(sent["data"]["skipped"][0]["reason"], "no_consent");
    })
    .await
    .unwrap();
}

/// Đặt mọi job của `notifications` tới hạn và lùi giờ tạo tin `key` đi `hours` giờ.
async fn age(app: &TestApp, key: &str, hours: i64) {
    let database = app.database();
    let jobs = Query::update()
        .table(Alias::new("notifications_jobs"))
        .value(Alias::new("run_at"), Utc::now() - Span::minutes(1))
        .to_owned();
    database.write().execute(&jobs).await.unwrap();
    let messages = Query::update()
        .table(Alias::new("notifications_messages"))
        .value(Alias::new("created_at"), Utc::now() - Span::hours(hours))
        .and_where(Expr::col(Alias::new("dedupe_key")).eq(key))
        .to_owned();
    database.write().execute(&messages).await.unwrap();
}

#[tokio::test]
async fn ngoai_khung_gio_thi_hoan_qua_24_gio_thi_bo() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        // Khung SMS quảng cáo mở sau 2 giờ nữa (giờ UTC): bây giờ là ngoài khung.
        let now = Utc::now();
        let from = (now + Span::hours(2)).with_minute(0).unwrap();
        let to = from + Span::hours(1);
        let window =
            json!({"from": from.format("%H:%M").to_string(), "to": to.format("%H:%M").to_string()});
        fx.rules.0.lock().unwrap().insert(
            "notifications.send_windows".to_owned(),
            json!({"utc_offset_minutes": 0, "marketing": {"sms": window, "email": null}}),
        );
        fx.ledger.allow(ConsentSubject::Phone("+84912345678".to_owned()), "sms");
        let promo = |key: &str| {
            Notification::new(
                PROMO.key(),
                Recipient::Phone("0912 345 678".to_owned()),
                json!({"percent": 10}),
                key,
            )
        };
        fx.notifier.notify(DEFAULT_TENANT, promo("late-1")).await.unwrap();
        run_jobs(&fx.app).await;
        assert!(fx.sms.sent.lock().unwrap().is_empty());
        let (_, page) = call(&fx.app, "GET", "/messages?status=deferred", "admin", None).await;
        let row = &page["data"][0];
        assert_eq!(row["channel"], "sms", "{page}");
        let not_before: DateTime<Utc> = row["not_before"].as_str().unwrap().parse().unwrap();
        assert!(not_before > now && not_before <= now + Span::hours(3), "{not_before}");

        // Tới giờ mở khung nhưng tin đã chờ hơn 24 giờ: bỏ.
        age(&fx.app, "late-1", 30).await;
        run_jobs(&fx.app).await;
        assert_eq!(
            statuses(&fx.app, "late-1").await["sms"],
            ("skipped".to_owned(), "quiet_hours_expired".to_owned())
        );

        // Không có khung: gửi ngay, kèm nhãn QC.
        fx.rules.0.lock().unwrap().insert(
            "notifications.send_windows".to_owned(),
            json!({"utc_offset_minutes": 0, "marketing": {"sms": null}}),
        );
        fx.notifier.notify(DEFAULT_TENANT, promo("late-2")).await.unwrap();
        run_jobs(&fx.app).await;
        assert_eq!(fx.sms.last_text("+84912345678").as_deref(), Some("QC Sale 10%"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tin_moi_dang_ky_chi_gui_mot_lan() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        fx.ledger.allow(ConsentSubject::Phone("+84912000111".to_owned()), "");
        let invite = |key: &str| {
            Notification::new(
                INVITE.key(),
                Recipient::Phone("+84912000111".to_owned()),
                json!({}),
                key,
            )
        };
        let first = fx.notifier.notify(DEFAULT_TENANT, invite("invite-1")).await.unwrap();
        assert_eq!(first.message_ids.len(), 1);
        let second = fx.notifier.notify(DEFAULT_TENANT, invite("invite-2")).await.unwrap();
        assert!(second.message_ids.is_empty());
        assert_eq!(second.skipped[0].reason, SkipReason::InvitationUsed);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn day_bi_mat_thi_xoa_dang_ky() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        let p256dh = "B".repeat(87);
        let endpoint = "https://push.example/send/abc";
        let bad = json!({"endpoint": "http://insecure.example", "keys": {"p256dh": p256dh, "auth": "a".repeat(22)}});
        let (status, problem) = call(&fx.app, "POST", "/me/push-subscriptions", "alice", Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["code"], "notifications.push_subscription_invalid");
        let good = json!({"endpoint": endpoint, "keys": {"p256dh": p256dh, "auth": "a".repeat(22)}, "expirationTime": null});
        let (status, created) = call(&fx.app, "POST", "/me/push-subscriptions", "alice", Some(good.clone())).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let subscription = created["data"]["id"].as_str().unwrap().to_owned();
        // Đăng ký lại cùng trình duyệt: cùng dòng.
        let (_, again) = call(&fx.app, "POST", "/me/push-subscriptions", "alice", Some(good)).await;
        assert_eq!(again["data"]["id"], subscription.as_str());
        let device = json!({"platform": "android", "token": "device-token-1", "app_id": "com.example.shop"});
        let (status, created) = call(&fx.app, "POST", "/me/device-tokens", "alice", Some(device)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        assert!(!created.to_string().contains("device-token-1"));
        // Secret lưu đã mã hoá.
        let stored = fx
            .app
            .database()
            .read()
            .query_one(&Query::select().column(Alias::new("auth_secret")).from(Alias::new("notifications_push_subscriptions")).to_owned())
            .await
            .unwrap()
            .unwrap()
            .try_get::<String>("", "auth_secret")
            .unwrap();
        assert!(stored.starts_with("sealed:"), "{stored}");

        fx.web_push.mark_gone(endpoint);
        fx.notifier.notify(DEFAULT_TENANT, to_alice(&PUSHED, "push-1")).await.unwrap();
        run_jobs(&fx.app).await;
        assert_eq!(fx.web_push.count(), 0);
        assert_eq!(fx.mobile_push.count(), 1);
        let sent = statuses(&fx.app, "push-1").await;
        assert_eq!(sent["web_push"], ("skipped".to_owned(), "no_address".to_owned()));
        assert_eq!(sent["mobile_push"].0, "sent");
        // Dịch vụ đẩy báo mất: đăng ký đã bị xoá.
        let (status, _) = call(&fx.app, "DELETE", &format!("/me/push-subscriptions/{subscription}"), "alice", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Không còn đăng ký: lần sau web push không có địa chỉ ngay từ đầu.
        let outcome = fx.notifier.notify(DEFAULT_TENANT, to_alice(&PUSHED, "push-2")).await.unwrap();
        assert!(outcome.skipped.iter().any(|skip| skip.channel == Channel::WebPush && skip.reason == SkipReason::NoAddress));
        let (status, key) = call(&fx.app, "GET", "/public/web-push-key", "", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(key["data"]["public_key"].is_string());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn thong_bao_nen_tang_toi_quan_tri_vien() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        let notice =
            json!({"category": "transactional", "title": "Maintenance", "body": "Sunday 02:00."});
        let (status, _) =
            call(&fx.app, "POST", "/platform-notices", "alice", Some(notice.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, problem) = call(
            &fx.app,
            "POST",
            "/platform-notices",
            "staff",
            Some(json!({"category": "x", "title": "", "body": "b"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["errors"].as_array().unwrap().len(), 2, "{problem}");
        let (status, queued) =
            call(&fx.app, "POST", "/platform-notices", "staff", Some(notice)).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
        run_jobs(&fx.app).await;
        let (_, inbox) = call(&fx.app, "GET", "/me/inbox", "admin", None).await;
        assert_eq!(inbox["data"].as_array().unwrap().len(), 1, "{inbox}");
        assert_eq!(inbox["data"][0]["title"], "Maintenance");
        let (_, alice) = call(&fx.app, "GET", "/me/inbox", "alice", None).await;
        assert!(alice["data"].as_array().unwrap().is_empty());
        assert!(
            fx.mails
                .all()
                .iter()
                .any(|mail| mail.to == "admin@example.com" && mail.subject == "Maintenance")
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gioi_han_moi_gio_khong_ap_cho_tin_bao_mat() {
    TestDatabase::run(async |test| {
        let fx = fixture_with(test, "1").await;
        assert_eq!(
            fx.notifier
                .notify(DEFAULT_TENANT, to_alice(&SHIPPED, "t-1"))
                .await
                .unwrap()
                .message_ids
                .len(),
            2
        );
        let limited = fx.notifier.notify(DEFAULT_TENANT, to_alice(&SHIPPED, "t-2")).await.unwrap();
        assert!(limited.message_ids.is_empty());
        assert!(limited.skipped.iter().all(|skip| skip.reason == SkipReason::Throttled));
        let security = fx.notifier.notify(DEFAULT_TENANT, to_alice(&SIGN_IN, "t-3")).await.unwrap();
        assert_eq!(security.message_ids.len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn permission_kiem_truoc_khi_tra_va_mau_thay_the() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        let unknown = json!({"kind": "shop.nope", "recipient": {"user_id": Uuid::from_u128(2)}, "dedupe_key": "k"});
        let (status, _) = call(&fx.app, "POST", "/messages", "alice", Some(unknown.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, problem) = call(&fx.app, "POST", "/messages", "admin", Some(unknown)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["code"], "notifications.kind_unknown");
        let two = json!({"kind": "shop.order_shipped", "recipient": {"email": "a@example.com", "phone": "+84912345678"}, "dedupe_key": "k"});
        let (status, _) = call(&fx.app, "POST", "/messages", "admin", Some(two)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let missing = Uuid::now_v7();
        let (status, _) = call(&fx.app, "GET", &format!("/messages/{missing}"), "alice", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, problem) = call(&fx.app, "GET", &format!("/messages/{missing}"), "admin", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(problem["code"], "notifications.message_not_found");

        let text = json!({"title": "Đơn {code} đã gửi", "body": "Đơn {code} đang trên đường.", "short": "Đơn {code}"});
        let (status, _) = call(&fx.app, "PUT", "/templates/shop.order_shipped/email/vi", "alice", Some(text.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let bad = json!({"title": "{secret}", "body": "b", "short": "s"});
        let (status, problem) = call(&fx.app, "PUT", "/templates/shop.order_shipped/email/vi", "admin", Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["code"], "notifications.template_invalid");
        let (status, problem) = call(&fx.app, "PUT", "/templates/shop.order_shipped/sms/xx", "admin", Some(text.clone())).await;
        assert_eq!(problem["errors"].as_array().unwrap().len(), 2, "{status} {problem}");
        let (status, saved) = call(&fx.app, "PUT", "/templates/shop.order_shipped/email/vi", "admin", Some(text)).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        let (_, list) = call(&fx.app, "GET", "/templates", "admin", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1);

        fx.notifier.notify(DEFAULT_TENANT, to_alice(&SHIPPED, "custom-1").locale("vi")).await.unwrap();
        run_jobs(&fx.app).await;
        assert_eq!(fx.mails.all()[0].subject, "Đơn DH001 đã gửi");
        let (status, _) = call(&fx.app, "DELETE", "/templates/shop.order_shipped/email/vi", "admin", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, problem) = call(&fx.app, "DELETE", "/templates/shop.order_shipped/email/vi", "admin", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(problem["code"], "notifications.template_not_found");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tool_mcp_dung_chung_service() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        let client = fx.app.mcp("admin").await.unwrap();
        let sent = client
            .call_tool(autoxide_testkit::tool(
                "notifications.send_notification",
                json!({"kind": "shop.order_shipped", "user_id": Uuid::from_u128(2), "params": {"code": "A1"}, "dedupe_key": "mcp-1"}),
            ))
            .await
            .unwrap();
        assert_ne!(sent.is_error, Some(true), "{sent:?}");
        run_jobs(&fx.app).await;
        let alice = fx.app.mcp("alice").await.unwrap();
        let inbox = alice.call_tool(autoxide_testkit::tool("notifications.list_my_inbox", json!({}))).await.unwrap();
        assert!(format!("{inbox:?}").contains("Order A1 shipped"), "{inbox:?}");
        let read = alice.call_tool(autoxide_testkit::tool("notifications.mark_inbox_read", json!({}))).await.unwrap();
        assert!(format!("{read:?}").contains("updated"), "{read:?}");
        let denied = alice
            .call_tool(autoxide_testkit::tool("notifications.list_messages", json!({})))
            .await;
        assert!(denied.is_err() || denied.unwrap().is_error == Some(true));
    })
    .await
    .unwrap();
}
