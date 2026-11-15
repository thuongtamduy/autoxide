//! Endpoint REST của module `newsletter`, dưới `/api/v1/newsletter`. Handler chỉ lấy dữ liệu
//! vào, gọi service và trả kết quả; mọi logic (kể cả kiểm permission) nằm ở service. Mọi route
//! đăng ký qua `routes!` để có tài liệu OpenAPI (luật kiến trúc 6).

mod campaigns;
mod lists;
mod public;
mod subscribers;
mod suppressions;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{PageQuery, PageRequest};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::service::NewsletterService;

pub(crate) use campaigns::CampaignResponse;
pub(crate) use lists::ListResponse;
pub(crate) use subscribers::SubscriberResponse;

pub(crate) fn routes(service: Arc<NewsletterService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(lists::list_lists, lists::create_list))
        .routes(routes!(lists::get_list, lists::update_list, lists::delete_list))
        .routes(routes!(subscribers::list_subscribers))
        .routes(routes!(subscribers::import_subscribers))
        .routes(routes!(
            subscribers::get_subscriber,
            subscribers::update_subscriber,
            subscribers::delete_subscriber
        ))
        .routes(routes!(subscribers::get_email_data, subscribers::delete_email_data))
        .routes(routes!(campaigns::list_campaigns, campaigns::create_campaign))
        .routes(routes!(
            campaigns::get_campaign,
            campaigns::update_campaign,
            campaigns::delete_campaign
        ))
        .routes(routes!(campaigns::send_campaign))
        .routes(routes!(campaigns::schedule_campaign))
        .routes(routes!(campaigns::pause_campaign))
        .routes(routes!(campaigns::resume_campaign))
        .routes(routes!(campaigns::cancel_campaign))
        .routes(routes!(campaigns::test_campaign))
        .routes(routes!(suppressions::list_suppressions, suppressions::create_suppression))
        .routes(routes!(suppressions::delete_suppression))
        .routes(routes!(suppressions::report_bounce))
        .routes(routes!(public::public_lists))
        .routes(routes!(public::subscribe))
        .routes(routes!(public::confirm))
        .routes(routes!(public::unsubscribe))
        .routes(routes!(public::get_preferences, public::update_preferences))
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

/// Một UUID từ tham số `field`; sai thì thêm lỗi.
pub(crate) fn uuid_param(
    raw: Option<&str>,
    field: &str,
    problems: &mut Vec<FieldError>,
) -> Option<uuid::Uuid> {
    let raw = raw.map(str::trim).filter(|raw| !raw.is_empty())?;
    raw.parse().map_err(|_| problems.push(FieldError::new(field, "invalid", "must be a UUID"))).ok()
}

/// Tham số bắt buộc `field`.
pub(crate) fn required<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str, Error> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| Error::validation(vec![FieldError::new(field, "required", "is required")]))
}

// Trường JSON có ba trạng thái: không gửi (`None`), `null` (`Some(None)`), có giá trị.
pub(crate) fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
