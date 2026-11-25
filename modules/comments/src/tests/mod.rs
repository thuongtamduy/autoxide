//! Test của service `comments` (không qua HTTP), chạy trên SQLite và Postgres qua
//! `TestDatabase::run`.
//!
//! - `posting.rs`: pipeline kiểm duyệt, chống spam hai pha, gửi lại cùng token, chế độ duyệt
//!   (khách / người dùng, email tự khai không vượt duyệt), XSS, thời hạn sửa, lệnh cấm hết hạn,
//!   trả lời đồng thời giữ số đếm, cursor của luồng.
//! - `moderation.rs`: báo cáo đồng thời giữ lại đúng một lần, duyệt / xoá hẳn và số đếm, mail
//!   báo trả lời, xuất / xoá theo email, tài khoản bị xoá, dọn theo thời hạn, event của module
//!   sở hữu đích, event không mang email hay chữ.
//! - `questions.rs`: hỏi đáp (D-202) — nhãn tác giả, tài khoản đã xác thực, tìm đúng dấu xếp
//!   trước, số câu hỏi, câu hỏi chưa trả lời, event vòng đời của module sở hữu.
//! - `tenancy.rs`: hai khách cùng database (bước 33) — cùng đích, cùng tên không đụng nhau, không
//!   thấy bình luận của nhau, token trong link mail mang khách.

mod dormant;
mod moderation;
mod posting;
mod questions;
mod tenancy;

use std::net::IpAddr;

use autoxide_kernel::access::Principal;
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::pagination::{Order, PageRequest};
use autoxide_kernel::resource::{ResourceChange, ResourceEvent};
use autoxide_kernel::tenancy::DEFAULT_TENANT;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, Statement};

use crate::config::{CommentsConfig, Mode};
use crate::entity::{comment, target};
use crate::posting::{NewComment, Posted};
use crate::service::CommentsService;
use crate::service::Status;

pub(super) const KIND: &str = "content.entry";
pub(super) const POST: &str = "public-1";

pub(super) fn visitor(ip: &str) -> ClientInfo {
    ClientInfo::from_ip(ip.parse::<IpAddr>().unwrap())
}

pub(super) async fn token(service: &CommentsService, kind: &str) -> String {
    service
        .guard
        .issue(&service.databases.default_tenant(), &format!("comments:{kind}"))
        .await
        .unwrap()
        .token
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
    service.post(&service.databases.default_tenant(), None, client, input).await
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
    service.set_profile(&service.databases.default_tenant(), &principal, name).await.unwrap();
    principal
}

/// Mọi bình luận hiện ngay (khách và người dùng).
pub(super) fn open_config() -> CommentsConfig {
    CommentsConfig { guest_mode: Mode::Post, user_mode: Mode::Post, ..crate::testing::config() }
}

/// (đang hiện, chờ duyệt) của đích. Kiểm luôn số câu hỏi (bình luận gốc) đã lưu khớp số đếm
/// lại từ bảng — mọi đường đổi trạng thái trong các test đều đi qua đây (D-202).
pub(super) async fn counts(database: &Database, id: &str) -> (i32, i32) {
    let row =
        target::Entity::find_by_id((DEFAULT_TENANT.as_uuid(), KIND.to_owned(), id.to_owned()))
            .one(database.read())
            .await
            .unwrap()
            .unwrap();
    let roots = |status: Status| {
        comment::Entity::find()
            .filter(comment::Column::TargetType.eq(KIND))
            .filter(comment::Column::TargetId.eq(id))
            .filter(comment::Column::Depth.eq(0))
            .filter(comment::Column::Status.eq(status.as_str()))
            .count(database.read())
    };
    let stored = (row.published_thread_count, row.pending_thread_count);
    let counted = (roots(Status::Published).await.unwrap(), roots(Status::Pending).await.unwrap());
    assert_eq!(
        (i64::from(stored.0), i64::from(stored.1)),
        (counted.0 as i64, counted.1 as i64),
        "stored question counts drift from the table"
    );
    (row.published_count, row.pending_count)
}

/// Event vòng đời mà `content` khai (`Module::resource_events`), như kernel đưa cho `comments`.
pub(super) fn content_events() -> Vec<ResourceEvent> {
    vec![
        ResourceEvent::new("content.entry_archived", KIND, ResourceChange::Closed),
        ResourceEvent::new("content.entry_deleted", KIND, ResourceChange::Removed),
    ]
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
