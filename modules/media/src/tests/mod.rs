//! Test mức service của module `media` (gate chạy trên cả SQLite và Postgres); đồ dùng chung ở
//! đây, test theo tính năng:
//!
//! - `items.rs`: tải lên, thay nội dung, sửa, danh sách, thùng rác, quyền tải, dữ liệu cá nhân.
//! - `folders.rs`, `tags.rs`: thư mục lồng nhau, nhãn.
//! - `images.rs`: ảnh thu nhỏ; `alt.rs`: gợi ý alt bằng AI; `alts.rs`: alt theo ngôn ngữ.

mod alt;
mod alts;
// Bản AVIF chỉ có khi build với feature `avif`; gate chạy riêng các test này với feature đó.
#[cfg(feature = "avif")]
mod avif;
#[cfg(feature = "avif")]
mod avif_http;
mod direct;
mod folders;
mod images;
mod items;
mod tags;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::ErrorKind;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::pagination::{Order, PageQuery, PageRequest};
use autoxide_kernel::privacy::PersonalData;
use autoxide_kernel::storage::Storage;
use autoxide_testkit::TestDatabase;
use futures_util::TryStreamExt as _;
use sea_orm::{ConnectionTrait, Statement};
use serde_json::json;
use uuid::Uuid;

use crate::api::{Media, MediaVisibility};
use crate::library::folders::{FolderChanges, FolderSort};
use crate::library::sniff::samples;
use crate::library::upload::NewUpload;
use crate::service::{FolderFilter, Item, ItemChanges, ItemFilter, ItemSort, Visibility};
use crate::testing::{body, editor, limits, service, user};

fn upload(name: &str, content_type: &str) -> NewUpload {
    NewUpload {
        org_id: None,
        name: name.to_owned(),
        folder_id: None,
        content_type: Some(content_type.to_owned()),
        tags: Vec::new(),
    }
}

async fn events(database: &Database) -> Vec<String> {
    let backend = database.read().get_database_backend();
    let sql = "SELECT name FROM media_events ORDER BY id";
    let rows = database.read().query_all_raw(Statement::from_string(backend, sql)).await;
    rows.unwrap().iter().map(|row| row.try_get("", "name").unwrap()).collect()
}

async fn content(
    tenant: &autoxide_kernel::tenancy::TenantContext,
    service: &crate::service::MediaService,
    item: &Item,
) -> Vec<u8> {
    let stored = service.read(tenant, item, None).await.unwrap();
    let chunks: Vec<bytes::Bytes> = stored.content.try_collect().await.unwrap();
    chunks.concat()
}

fn page(limit: u32) -> PageRequest {
    PageRequest::first(limit, Order::Asc)
}
