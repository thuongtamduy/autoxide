//! Test của service `forms` (chạy trên SQLite và Postgres qua `TestDatabase::run`); đồ dùng chung
//! ở đây, test theo tính năng:
//!
//! - `definitions.rs`: vòng đời form và phiên bản, kiểm câu trả lời theo kiểu trường, nơi nhận
//!   mail báo.
//! - `submissions.rs`: chống spam hai pha (bẫy / quá nhanh / quá nhiều cùng một câu trả lời, gửi
//!   lại cùng token tuần tự và đồng thời), form đóng và `Origin`, event không mang câu trả lời,
//!   mail báo.
//! - `data.rs`: CSV, xuất / xoá theo email, dọn theo thời hạn, tài khoản bị xoá.

mod data;
mod definitions;
mod submissions;

use std::net::IpAddr;
use std::time::Duration;

use autoxide_kernel::access::Principal;
use autoxide_kernel::antispam::AntispamConfig;
use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::pagination::{Order, PageRequest};
use autoxide_kernel::privacy::{PersonalData, USER_DELETED};
use autoxide_testkit::TestDatabase;
use sea_orm::{ConnectionTrait, EntityTrait, PaginatorTrait, Statement};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::config::FormsConfig;
use crate::entity::submission;
use crate::forms::fields::{Field, Localized};
use crate::forms::{Form, FormInput, FormStatus, scope};
use crate::service::FormsService;
use crate::submissions::inbox::{InboxFilter, SubmissionStatus};
use crate::submissions::public::{PublicSubmission, Receipt};
use crate::testing::{admin, service, service_with, user};

fn fields(value: Value) -> Vec<Field> {
    serde_json::from_value(value).unwrap()
}

fn contact_fields() -> Vec<Field> {
    fields(json!([
        {"key": "name", "type": "text", "label": {"en": "Name", "vi": "Tên"}, "required": true},
        {"key": "email", "type": "email", "label": {"en": "Email"}, "required": true},
        {"key": "message", "type": "textarea", "label": {"en": "Message", "vi": "Lời nhắn"}},
        {"key": "agree", "type": "consent", "label": {"en": "You may contact me.", "vi": "Bạn được liên hệ với tôi."}}
    ]))
}

fn contact(key: &str) -> FormInput {
    FormInput {
        key: Some(key.to_owned()),
        name: Some("Contact us".to_owned()),
        fields: Some(contact_fields()),
        notify_emails: Some(vec!["Owner@Example.com".to_owned()]),
        success_message: Some(Localized {
            en: Some("Thanks!".to_owned()),
            vi: Some("Cảm ơn!".to_owned()),
        }),
        ..FormInput::default()
    }
}

fn visitor(ip: &str) -> ClientInfo {
    ClientInfo::from_ip(ip.parse::<IpAddr>().unwrap())
}

async fn token(service: &FormsService, key: &str) -> String {
    service.guard.issue(&scope(key)).await.unwrap().token
}

fn answers(name: &str, email: &str) -> serde_json::Map<String, Value> {
    json!({"name": name, "email": email, "message": "Hello there", "agree": true})
        .as_object()
        .unwrap()
        .clone()
}

/// Gửi một bài như người lạ, với token mới.
async fn send(
    service: &FormsService,
    key: &str,
    client: &ClientInfo,
    data: serde_json::Map<String, Value>,
) -> Result<Receipt, Error> {
    let input = PublicSubmission {
        data,
        locale: Some("vi".to_owned()),
        page_url: Some("https://example.com/contact".to_owned()),
        form_token: Some(token(service, key).await),
        ..PublicSubmission::default()
    };
    service.submit(key, None, client, input).await
}

async fn count(database: &autoxide_kernel::database::Database) -> u64 {
    submission::Entity::find().count(database.read()).await.unwrap()
}

/// (tên, payload) của mọi event đã ghi, theo thứ tự.
async fn events(database: &autoxide_kernel::database::Database) -> Vec<(String, String)> {
    let backend = database.read().get_database_backend();
    let rows = database
        .read()
        .query_all_raw(Statement::from_string(
            backend,
            "SELECT name, payload FROM forms_events ORDER BY id",
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

fn page() -> PageRequest {
    PageRequest::first(50, Order::Desc)
}
