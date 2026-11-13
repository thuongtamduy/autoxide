//! Endpoint REST của module `comments`, dưới `/api/v1/comments`. Handler chỉ lấy dữ liệu vào, gọi
//! service và trả kết quả; mọi logic (kể cả kiểm permission) nằm ở service. Mọi route đăng ký qua
//! `routes!` để có tài liệu OpenAPI (luật kiến trúc 6).
//!
//! - `public.rs`: người lạ và người dùng đăng nhập (`/public/…`, `/profile`).
//! - `admin.rs`: kiểm duyệt, cấm, đích, xuất / xoá theo email.
//! - `views.rs`: hình dạng trả về, dùng chung với tool MCP.

mod admin;
mod public;
#[cfg(test)]
mod tests;
mod views;

use std::sync::Arc;

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{PageQuery, PageRequest};
use serde::Deserialize;
use serde_json::Value;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::service::CommentsService;

#[cfg(test)]
pub(crate) use views::PublicComment;
pub(crate) use views::{CommentDetail, TargetResponse};

pub(crate) fn routes(service: Arc<CommentsService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(public::get_settings))
        .routes(routes!(public::list_thread))
        .routes(routes!(public::create_comment))
        .routes(routes!(public::get_public_comment, public::edit_comment))
        .routes(routes!(public::list_replies))
        .routes(routes!(public::delete_own_comment))
        .routes(routes!(public::report_comment))
        .routes(routes!(public::count_comments))
        .routes(routes!(public::get_public_target))
        .routes(routes!(public::verify_email))
        .routes(routes!(public::unsubscribe))
        .routes(routes!(public::get_profile, public::set_profile))
        .routes(routes!(admin::list_comments))
        .routes(routes!(admin::get_comment, admin::update_comment, admin::delete_comment))
        .routes(routes!(admin::list_reports))
        .routes(routes!(admin::list_bans, admin::create_ban))
        .routes(routes!(admin::delete_ban))
        .routes(routes!(admin::get_target, admin::set_target))
        .routes(routes!(admin::get_email_data, admin::delete_email_data))
        .with_state(service)
}

/// Yêu cầu trang, gộp lỗi của nó với lỗi của bộ lọc để báo một lần.
pub(crate) fn page_request(
    query: PageQuery,
    mut problems: Vec<FieldError>,
) -> Result<PageRequest, Error> {
    let page = PageRequest::try_from(query);
    if let Err(error) = &page {
        problems.extend(error.fields().iter().cloned());
    }
    match page {
        Ok(page) if problems.is_empty() => Ok(page),
        _ => Err(Error::validation(problems)),
    }
}

/// Một giá trị liệt kê (`snake_case`) từ tham số `field`; sai thì thêm lỗi.
pub(crate) fn choice<T: for<'de> Deserialize<'de>>(
    raw: Option<&str>,
    field: &str,
    allowed: &str,
    problems: &mut Vec<FieldError>,
) -> Option<T> {
    let raw = raw.map(str::trim).filter(|raw| !raw.is_empty())?;
    serde_json::from_value(Value::String(raw.to_owned()))
        .map_err(|_| {
            problems.push(FieldError::new(field, "invalid", format!("must be one of {allowed}")));
        })
        .ok()
}
