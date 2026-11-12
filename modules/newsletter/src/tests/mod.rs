//! Test của service `newsletter` (chạy trên SQLite và Postgres qua `TestDatabase::run`).
//!
//! - `subscribe.rs`: đăng ký công khai (chống spam hai pha, cùng câu trả lời, đồng thời), xác
//!   nhận, giới hạn mail xác nhận, event không mang email.
//! - `sending.rs`: gửi chiến dịch (không gửi trùng khi chạy lại / song song, tạm dừng rồi tiếp
//!   tục, người bị chặn không nhận, giới hạn thư quảng cáo, header `List-Unsubscribe`), huỷ một
//!   chạm, trang lựa chọn, bounce, hẹn giờ.
//! - `admin.rs`: danh sách, nhập, xuất / xoá theo email, dọn dẹp, bài mới, tài khoản bị xoá.

mod admin;
mod sending;
mod subscribe;

use std::net::IpAddr;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ClientInfo;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Order, Query};
use sea_orm::{ConnectionTrait, EntityTrait, PaginatorTrait, Statement};
use serde_json::Value;
use uuid::Uuid;

use crate::campaigns::{Action, Campaign, CampaignInput};
use crate::entity::{subscriber, subscription};
use crate::lists::{List, ListInput};
use crate::public::{SCOPE, SUBSCRIBE, SubscribeJob, SubscribeRequest};
use crate::sending::{PREPARE, PrepareJob, SEND, SendJob};
use crate::service::NewsletterService;
use crate::subscribers::{ImportInput, ImportPerson};
use crate::testing::{FakeMailer, Harness, admin};
use crate::text::Localized;

pub(crate) fn visitor(ip: &str) -> ClientInfo {
    ClientInfo { ip: Some(ip.parse::<IpAddr>().unwrap()), ..ClientInfo::default() }
}

/// Tạo danh sách công khai `key`.
pub(crate) async fn list(service: &NewsletterService, key: &str, double_opt_in: bool) -> List {
    let input = ListInput {
        key: Some(key.to_owned()),
        name: Some(Localized { en: Some(format!("{key} news")), vi: Some(format!("Tin {key}")) }),
        double_opt_in: Some(double_opt_in),
        public: Some(true),
        ..ListInput::default()
    };
    service.create_list(&admin(), input).await.unwrap()
}

pub(crate) async fn form_token(service: &NewsletterService) -> String {
    service.guard.issue(SCOPE).await.unwrap().token
}

/// Đăng ký `email` vào `lists` như người lạ, với token mới.
pub(crate) async fn subscribe(
    service: &NewsletterService,
    email: &str,
    lists: &[&str],
    ip: &str,
) -> Result<(), Error> {
    let request = SubscribeRequest {
        email: Some(email.to_owned()),
        lists: lists.iter().map(|key| (*key).to_owned()).collect(),
        consent_text: Some("Send me the newsletter.".to_owned()),
        form_token: Some(form_token(service).await),
        ..SubscribeRequest::default()
    };
    service.subscribe(None, &visitor(ip), request).await?;
    store_subscriptions(service).await;
    Ok(())
}

/// Chạy mọi job đăng ký đang chờ (như worker), theo thứ tự xếp hàng. Trả số job đã chạy.
pub(crate) async fn store_subscriptions(service: &NewsletterService) -> usize {
    let mut ran = 0;
    loop {
        let select = Query::select()
            .columns([Alias::new("id"), Alias::new("payload")])
            .from(Alias::new("newsletter_jobs"))
            .and_where(Expr::col(Alias::new("kind")).eq(SUBSCRIBE))
            .and_where(Expr::col(Alias::new("completed_at")).is_null())
            .order_by(Alias::new("id"), Order::Asc)
            .limit(1)
            .to_owned();
        let Some(row) = service.database.read().query_one(&select).await.unwrap() else {
            return ran;
        };
        let id: Uuid = row.try_get("", "id").unwrap();
        let payload: String = row.try_get("", "payload").unwrap();
        let job: SubscribeJob = serde_json::from_str(&payload).unwrap();
        service.apply_subscription(&job).await.unwrap();
        let done = Query::update()
            .table(Alias::new("newsletter_jobs"))
            .value(Alias::new("completed_at"), chrono::Utc::now())
            .and_where(Expr::col(Alias::new("id")).eq(id))
            .to_owned();
        service.database.write().execute(&done).await.unwrap();
        ran += 1;
    }
}

/// Nhập `emails` vào danh sách `list_id` (đã xác nhận).
pub(crate) async fn import(service: &NewsletterService, list_id: Uuid, emails: &[String]) {
    let input = ImportInput {
        people: emails
            .iter()
            .map(|email| ImportPerson { email: email.clone(), ..ImportPerson::default() })
            .collect(),
        list_ids: vec![list_id],
        consent_source: Some("test".to_owned()),
        ..ImportInput::default()
    };
    service.import_subscribers(&admin(), input).await.unwrap();
}

pub(crate) fn emails(prefix: &str, count: usize) -> Vec<String> {
    (0..count).map(|index| format!("{prefix}{index:03}@example.com")).collect()
}

/// Chiến dịch nháp cho danh sách `list_id`.
pub(crate) async fn campaign(
    service: &NewsletterService,
    list_id: Uuid,
    advertising: bool,
) -> Campaign {
    let input = CampaignInput {
        list_id: Some(list_id),
        subject: Some("Hello".to_owned()),
        body_text: Some("News of the week.".to_owned()),
        body_html: Some(Some("<p>News of the week.</p>".to_owned())),
        advertising: Some(advertising),
        ..CampaignInput::default()
    };
    service.create_campaign(&admin(), input).await.unwrap()
}

/// Gửi chiến dịch `id` (phiên bản bất kỳ).
pub(crate) async fn send(service: &NewsletterService, id: Uuid) -> Campaign {
    service.change_state(&admin(), id, Action::Send, None).await.unwrap()
}

/// Một job chưa chạy của module (bỏ qua `run_at` nếu `any_time`).
pub(crate) async fn next_job(database: &Database, any_time: bool) -> Option<(Uuid, String, Value)> {
    let mut select = Query::select();
    select
        .columns([Alias::new("id"), Alias::new("kind"), Alias::new("payload")])
        .from(Alias::new("newsletter_jobs"))
        .and_where(Expr::col(Alias::new("completed_at")).is_null())
        .order_by(Alias::new("run_at"), Order::Asc)
        .order_by(Alias::new("id"), Order::Asc)
        .limit(1);
    if !any_time {
        select.and_where(Expr::col(Alias::new("run_at")).lte(chrono::Utc::now()));
    }
    let row = database.read().query_one(&select).await.unwrap()?;
    let payload: String = row.try_get("", "payload").unwrap();
    Some((
        row.try_get("", "id").unwrap(),
        row.try_get("", "kind").unwrap(),
        serde_json::from_str(&payload).unwrap(),
    ))
}

/// Chạy một job như worker (không có lease): theo loại, rồi đánh dấu xong.
pub(crate) async fn run_job(harness: &Harness, any_time: bool) -> Option<String> {
    let (id, kind, payload) = next_job(&harness.database, any_time).await?;
    match kind.as_str() {
        PREPARE => {
            let job: PrepareJob = serde_json::from_value(payload).unwrap();
            harness.service.prepare(&job).await.unwrap();
        }
        SEND => {
            let job: SendJob = serde_json::from_value(payload).unwrap();
            harness.service.send_batch(&job).await.unwrap();
        }
        SUBSCRIBE => {
            let job: SubscribeJob = serde_json::from_value(payload).unwrap();
            harness.service.apply_subscription(&job).await.unwrap();
        }
        _ => {}
    }
    let done = Query::update()
        .table(Alias::new("newsletter_jobs"))
        .value(Alias::new("completed_at"), chrono::Utc::now())
        .and_where(Expr::col(Alias::new("id")).eq(id))
        .to_owned();
    harness.database.write().execute(&done).await.unwrap();
    Some(kind)
}

/// Số job loại `kind` chưa chạy của module.
pub(crate) async fn next_jobs(database: &Database, kind: &str) -> usize {
    let select = Query::select()
        .expr(Expr::col(Alias::new("id")).count())
        .from(Alias::new("newsletter_jobs"))
        .and_where(Expr::col(Alias::new("kind")).eq(kind))
        .and_where(Expr::col(Alias::new("completed_at")).is_null())
        .to_owned();
    let row = database.read().query_one(&select).await.unwrap().unwrap();
    let count: i64 = row.try_get_by_index(0).unwrap();
    usize::try_from(count).unwrap()
}

/// Chạy mọi job (bỏ qua `run_at`) tới khi hết. Trả số job đã chạy.
pub(crate) async fn drain(harness: &Harness) -> usize {
    let mut ran = 0;
    while run_job(harness, true).await.is_some() {
        ran += 1;
        assert!(ran < 1000, "jobs never end");
    }
    ran
}

/// (tên, payload) của mọi event đã ghi, theo thứ tự.
pub(crate) async fn events(database: &Database) -> Vec<(String, String)> {
    let backend = database.read().get_database_backend();
    let rows = database
        .read()
        .query_all_raw(Statement::from_string(
            backend,
            "SELECT name, payload FROM newsletter_events ORDER BY id",
        ))
        .await
        .unwrap();
    rows.into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "name").unwrap(),
                row.try_get::<String>("", "payload").unwrap(),
            )
        })
        .collect()
}

pub(crate) async fn count_events(database: &Database, name: &str) -> usize {
    events(database).await.into_iter().filter(|(event, _)| event == name).count()
}

pub(crate) async fn subscribers(database: &Database) -> u64 {
    subscriber::Entity::find().count(database.read()).await.unwrap()
}

pub(crate) async fn subscriptions(database: &Database) -> Vec<subscription::Model> {
    subscription::Entity::find().all(database.read()).await.unwrap()
}

/// Token trong link của mail (`…?token=<token>`).
pub(crate) fn token_in(text: &str) -> String {
    let start = text.find("token=").unwrap() + "token=".len();
    text[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// Token huỷ đăng ký trong header `List-Unsubscribe` của `mail`.
pub(crate) fn unsubscribe_token(mail: &autoxide_kernel::mail::Mail) -> String {
    let header = mail
        .headers
        .iter()
        .find(|(name, _)| name == "List-Unsubscribe")
        .map(|(_, value)| value.clone())
        .unwrap();
    token_in(&header)
}

/// Mỗi người nhận đúng một mail có `dedupe_key` (mail chiến dịch).
pub(crate) fn assert_once_each(mailer: &FakeMailer, expected: &[String]) {
    let mut recipients: Vec<String> = mailer
        .sent()
        .into_iter()
        .filter(|mail| {
            mail.dedupe_key.as_deref().is_some_and(|key| key.starts_with("newsletter.delivery:"))
        })
        .map(|mail| mail.to)
        .collect();
    recipients.sort();
    let mut expected = expected.to_vec();
    expected.sort();
    assert_eq!(recipients, expected);
}
