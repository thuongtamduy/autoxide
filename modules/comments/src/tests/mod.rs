//! Test của service `comments` (không qua HTTP), chạy trên SQLite và Postgres qua
//! `TestDatabase::run`.
//!
//! - `posting.rs`: pipeline kiểm duyệt, chống spam hai pha, gửi lại cùng token, chế độ duyệt
//!   (khách / người dùng, email tự khai không vượt duyệt), XSS, thời hạn sửa, lệnh cấm hết hạn,
//!   trả lời đồng thời giữ số đếm, cursor của luồng.
//! - `moderation.rs`: báo cáo đồng thời giữ lại đúng một lần, duyệt / xoá hẳn và số đếm, mail
//!   báo trả lời, xuất / xoá theo email, tài khoản bị xoá, dọn theo thời hạn, event của module
//!   sở hữu đích, event không mang email hay chữ.

mod moderation;
mod posting;

use std::net::IpAddr;

use autoxide_kernel::access::Principal;
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::pagination::{Order, PageRequest};
use sea_orm::{ConnectionTrait, EntityTrait, Statement};

use crate::config::{CommentsConfig, Mode};
use crate::entity::target;
use crate::posting::{NewComment, Posted};
use crate::service::CommentsService;

pub(super) const KIND: &str = "content.entry";
pub(super) const POST: &str = "public-1";

pub(super) fn visitor(ip: &str) -> ClientInfo {
    ClientInfo { ip: Some(ip.parse::<IpAddr>().unwrap()), ..ClientInfo::default() }
}

pub(super) async fn token(service: &CommentsService, kind: &str) -> String {
    service.guard.issue(&format!("comments:{kind}")).await.unwrap().token
}

/// Bình luận của khách `name` trên bài `POST`.
pub(super) fn guest(name: &str, body: &str) -> NewComment {
    NewComment {
        target_type: KIND.to_owned(),
        target_id: POST.to_owned(),
        body: body.to_owned(),
        author_name: Some(name.to_owned()),
        ..NewComment::default()
    }
}

/// Bình luận của người dùng đăng nhập trên bài `POST`.
pub(super) fn signed(body: &str) -> NewComment {
    NewComment {
        target_type: KIND.to_owned(),
        target_id: POST.to_owned(),
        body: body.to_owned(),
        ..NewComment::default()
    }
}

/// Gửi như khách, với token form mới.
pub(super) async fn as_guest(
    service: &CommentsService,
    client: &ClientInfo,
    mut input: NewComment,
) -> Result<Posted, Error> {
    input.form_token = Some(token(service, &input.target_type).await);
    service.post(None, client, input).await
}

/// Người dùng có `comments.post` (và các permission khác) với tên hiển thị `name`.
pub(super) async fn member(
    service: &CommentsService,
    name: &str,
    permissions: &[&str],
) -> Principal {
    let mut all = vec!["post"];
    all.extend_from_slice(permissions);
    let principal = crate::testing::user(&all);
    service.set_profile(&principal, name).await.unwrap();
    principal
}

/// Mọi bình luận hiện ngay (khách và người dùng).
pub(super) fn open_config() -> CommentsConfig {
    CommentsConfig { guest_mode: Mode::Post, user_mode: Mode::Post, ..crate::testing::config() }
}

/// (đang hiện, chờ duyệt) của đích.
pub(super) async fn counts(database: &Database, id: &str) -> (i32, i32) {
    let row = target::Entity::find_by_id((KIND.to_owned(), id.to_owned()))
        .one(database.read())
        .await
        .unwrap()
        .unwrap();
    (row.published_count, row.pending_count)
}

/// (tên, payload) của mọi event đã ghi, theo thứ tự.
pub(super) async fn events(database: &Database) -> Vec<(String, String)> {
    let backend = database.read().get_database_backend();
    let rows = database
        .read()
        .query_all_raw(Statement::from_string(
            backend,
            "SELECT name, payload FROM comments_events ORDER BY id",
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

pub(super) fn page(order: Order) -> PageRequest {
    PageRequest::first(50, order)
}
