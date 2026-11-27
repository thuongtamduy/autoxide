//! Test mức service của module `content`. Gate chạy trên cả SQLite và Postgres.

use autoxide_kernel::database::Database;
use autoxide_kernel::error::ErrorKind;
use autoxide_kernel::pagination::{Order, PageRequest};
use autoxide_testkit::TestDatabase;
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entries::{EntryFields, EntryFilter, Kind, NewEntry, ParentFilter};
use crate::testing::{config, editor, service, user};

pub(crate) fn fields(title: &str) -> EntryFields {
    EntryFields { title: Some(title.to_owned()), ..Default::default() }
}

pub(crate) fn post(title: &str) -> NewEntry {
    NewEntry {
        kind: Kind::Post,
        type_id: None,
        locale: None,
        parent_id: None,
        org_id: None,
        fields: fields(title),
    }
}

pub(crate) fn page(title: &str, parent: Option<Uuid>) -> NewEntry {
    NewEntry {
        kind: Kind::Page,
        type_id: None,
        locale: None,
        parent_id: parent,
        org_id: None,
        fields: fields(title),
    }
}

pub(crate) fn paragraph(text: &str) -> Value {
    json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": text}]}]})
}

pub(crate) async fn count(database: &Database, sql: &str) -> i64 {
    let backend = database.read().get_database_backend();
    let row =
        database.read().query_one_raw(Statement::from_string(backend, sql)).await.unwrap().unwrap();
    row.try_get_by_index::<i64>(0).unwrap()
}

pub(crate) async fn events(database: &Database) -> Vec<String> {
    let backend = database.read().get_database_backend();
    let sql = "SELECT name FROM content_events ORDER BY id";
    let rows = database.read().query_all_raw(Statement::from_string(backend, sql)).await;
    rows.unwrap().iter().map(|row| row.try_get("", "name").unwrap()).collect()
}

mod agent;
mod banners;
mod bao_mat;
mod dormant;
mod entries;
mod history;
mod library;
mod privacy;
mod public;
mod publishing;
mod ra_soat;
mod search;
mod seo;
mod templates;
mod tenancy;
mod translation;
mod translation_recovery;
mod word_index;
