//! Endpoint REST của module `forms`, dưới `/api/v1/forms`. Handler chỉ lấy dữ liệu vào, gọi
//! service và trả kết quả; mọi logic (kể cả kiểm permission) nằm ở service. Mọi route đăng ký
//! qua `routes!` để có tài liệu OpenAPI (luật kiến trúc 6).

mod admin;
mod exports;
mod inbox;
mod public;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{PageQuery, PageRequest};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::service::FormsService;

pub(crate) use admin::{FormResponse, counted as counted_forms};
pub(crate) use inbox::SubmissionResponse;

pub(crate) fn routes(service: Arc<FormsService>) -> OpenApiRouter {
    OpenApiRouter::new()
        // Khách chỉ đọc vẫn xuất được dữ liệu của mình (đối chiếu thiết kế 2, A-3.11).
        .routes(routes!(exports::create_export))
        .layer(autoxide_kernel::tenancy::allowed_while_read_only())
        .routes(routes!(admin::list_forms, admin::create_form))
        .routes(routes!(admin::get_form, admin::update_form, admin::delete_form))
        .routes(routes!(inbox::list_submissions))
        .routes(routes!(inbox::export_submissions))
        .routes(routes!(exports::get_export))
        .routes(routes!(exports::download_export))
        .routes(routes!(inbox::get_submission, inbox::update_submission, inbox::delete_submission))
        .routes(routes!(inbox::get_email_data, inbox::delete_email_data))
        .routes(routes!(public::get_public_form))
        .routes(routes!(public::submit_form))
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
            problems.push(
                FieldError::new(field, "invalid", "must be one of {allowed}")
                    .with_param("allowed", allowed),
            );
        })
        .ok()
}

// Trường JSON có ba trạng thái: không gửi (`None`), `null` (`Some(None)`), có giá trị.
pub(crate) fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
