//! Nội dung của item: tải về (`GET` / `HEAD`, công khai hay cần quyền) và thay (`PUT`).
//!
//! Header của response tải về (D-086):
//!
//! - `inline` CHỈ cho ảnh raster (JPEG, PNG, GIF, WebP, AVIF); mọi loại khác — kể cả SVG,
//!   HTML, PDF, có thể chứa script — là `attachment`;
//! - luôn có `X-Content-Type-Options: nosniff` và `Content-Security-Policy: default-src
//!   'none'; sandbox`: dù trình duyệt có mở, nội dung không chạy được script;
//! - `ETag` là SHA-256 của nội dung; `If-None-Match` khớp thì 304;
//! - `Accept-Ranges: bytes`; một khoảng `Range` hợp lệ thì 206, nằm ngoài nội dung thì 416;
//! - URL kèm đúng phiên bản hiện tại (`?v=<version_id>`) thì cache một năm, `immutable`;
//!   không kèm hay phiên bản cũ thì cache ngắn và hỏi lại máy chủ.

use std::ops::Range;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::http::{Disposition, content_disposition};
use autoxide_kernel::response::Problem;
use autoxide_kernel::version::{IfMatch, Tagged, if_none_match};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use super::MediaItemResponse;
use super::items::{body_stream, content_type};
use crate::service::{Item, MediaService, Visibility};
use crate::sniff::is_raster_image;

/// Cache của URL có phiên bản: một năm, nội dung không bao giờ đổi.
const LONG_CACHE: u32 = 365 * 24 * 3600;
/// Cache của URL không phiên bản (hay phiên bản cũ) của item công khai.
const SHORT_CACHE: u32 = 60;

/// Which version of the content the address names.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct VersionQuery {
    /// The `version_id` of the item, as in its `url`. With the current version the answer
    /// may be cached for a year; without it (or with an old one) only briefly.
    v: Option<String>,
    /// Width of a smaller copy (one of the item's `variants`). Until that copy is made, or
    /// when the image is narrower, the original is sent.
    w: Option<u32>,
}

/// Download the content of a media item.
///
/// Public items (not in the trash) need no sign-in; others require `media.read`. Raster
/// images (JPEG, PNG, GIF, WebP, AVIF) open in the browser (`inline`); every other type is
/// a download (`attachment`). Supports one byte range (`Range: bytes=…`, answered with
/// `206`), `If-None-Match` (`304`) and `HEAD`.
#[utoipa::path(
    get,
    path = "/{id}/content",
    operation_id = "download_media_item",
    security((), ("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Id of the media item."),
        VersionQuery,
        ("Range" = Option<String>, Header, description = "One byte range, e.g. `bytes=0-1023`, `bytes=1024-` or `bytes=-500`. Several ranges are answered with the whole content."),
        ("If-Range" = Option<String>, Header, description = "An `ETag`: apply `Range` only if the content still has it."),
        ("If-None-Match" = Option<String>, Header, description = "The `ETag` you have: `304` if the content did not change."),
    ),
    responses(
        (status = 200, description = "The content, with its own `Content-Type`.", body = Vec<u8>, content_type = "application/octet-stream", headers(
            ("ETag" = String, description = "SHA-256 of the content."),
            ("Cache-Control" = String, description = "A year and `immutable` when `v` is the current version; short otherwise."),
            ("Content-Disposition" = String, description = "`inline` for raster images, `attachment` otherwise, with the file name."),
        )),
        (status = 206, description = "The requested byte range; `Content-Range` says which.", body = Vec<u8>, content_type = "application/octet-stream"),
        (status = 304, description = "Not modified: the `ETag` in `If-None-Match` is current."),
        (status = 401, description = "A token was sent but is not valid; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No media item has this id that you may load; code `media.item_not_found`.", body = Problem),
        (status = 416, description = "The range is outside the content; code `media.range_not_satisfiable`. `Content-Range: bytes */<size>` gives the size.", body = Problem),
    )
)]
pub(crate) async fn get_content(
    State(service): State<Arc<MediaService>>,
    principal: Option<Principal>,
    Path(id): Path<Uuid>,
    Query(query): Query<VersionQuery>,
    headers: HeaderMap,
) -> Result<Response, Error> {
    serve(&service, principal, id, &query, &headers, true).await
}

/// Check the content of a media item without downloading it.
///
/// Same rules and headers as `GET`, without the body.
#[utoipa::path(
    head,
    path = "/{id}/content",
    operation_id = "head_media_item",
    security((), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the media item."), VersionQuery),
    responses(
        (status = 200, description = "The headers of the content (`Content-Length`, `Content-Type`, `ETag`…)."),
        (status = 304, description = "Not modified: the `ETag` in `If-None-Match` is current."),
        (status = 404, description = "No media item has this id that you may load."),
    )
)]
pub(crate) async fn head_content(
    State(service): State<Arc<MediaService>>,
    principal: Option<Principal>,
    Path(id): Path<Uuid>,
    Query(query): Query<VersionQuery>,
    headers: HeaderMap,
) -> Result<Response, Error> {
    serve(&service, principal, id, &query, &headers, false).await
}

/// Replace the content of a media item, keeping its id.
///
/// Requires `media.write`. Same body and checks as an upload. Everything that points to
/// the item keeps working; its `url`, `version_id` and `ETag` change, so caches pick up
/// the new content. `If-Match` is required. Retry safely with an `Idempotency-Key` header.
#[utoipa::path(
    put,
    path = "/{id}/content",
    operation_id = "replace_media_content",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the media item."), ("If-Match" = String, Header, description = "Required: the `ETag` of the item (from `GET /api/v1/media/{id}`), or `*`. The content is replaced only if nobody changed the item since; otherwise `412`.")),
    request_body(
        content = Vec<u8>,
        content_type = "application/octet-stream",
        description = "The new content, with its `Content-Type`.",
    ),
    responses(
        (status = 200, description = "The item with its new content.", body = autoxide_kernel::response::Data<MediaItemResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 400, description = "The `Content-Type` is not a media type (code `media.invalid_content_type`), or the upload was cut off (code `media.upload_interrupted`).", body = Problem),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No media item has this id; code `media.item_not_found`.", body = Problem),
        (status = 409, description = "The item is in the trash; code `media.item_in_trash`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the item changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 413, description = "The content is too large; code `media.too_large` (or `payload_too_large`).", body = Problem),
        (status = 415, description = "The type is not accepted or the content is not of that type; codes as for an upload.", body = Problem),
        (status = 422, description = "The body is empty; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn replace_content(
    State(service): State<Arc<MediaService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    headers: HeaderMap,
    body: Body,
) -> Result<Tagged<MediaItemResponse>, Error> {
    let expected = if_match.required()?;
    let content = body_stream(body, service.config.limits.max_bytes);
    let item = service.replace(&principal, id, content_type(&headers)?, content, expected).await?;
    super::tagged(&service, item).await
}

/// Trả nội dung (hay chỉ header, khi `with_body` là `false`).
async fn serve(
    service: &MediaService,
    principal: Option<Principal>,
    id: Uuid,
    query: &VersionQuery,
    request: &HeaderMap,
    with_body: bool,
) -> Result<Response, Error> {
    let item = service.viewable(principal.as_ref(), id).await?;
    // Ảnh thu nhỏ xin bằng `w`, nếu đã làm xong; không thì bản gốc.
    let copy = match query.w {
        Some(width) => service.variant(&item, width).await?,
        None => None,
    };
    let pending = query.w.is_some_and(|width| {
        copy.is_none()
            && crate::variants::wanted_widths(&service.config.image_widths, &item).contains(&width)
    });
    let (etag, content_type, size) = match &copy {
        Some(copy) => (
            format!("\"{}-w{}\"", item.version_id, copy.width),
            copy.content_type.clone(),
            u64::try_from(copy.size).unwrap_or_default(),
        ),
        None => (
            format!("\"{}\"", item.sha256),
            item.content_type.clone(),
            u64::try_from(item.size).unwrap_or_default(),
        ),
    };
    let mut headers = HeaderMap::new();
    let mut set = |name: HeaderName, value: String| -> Result<(), Error> {
        headers.insert(name, HeaderValue::from_str(&value).map_err(Error::internal)?);
        Ok(())
    };
    set(header::ETAG, etag.clone())?;
    // Ảnh thu nhỏ chưa làm xong: bản gốc tạm thời, không được cache lâu dưới địa chỉ này.
    let version = if pending { None } else { query.v.as_deref() };
    set(header::CACHE_CONTROL, cache_control(&item, version))?;
    if if_none_match(request, &etag) {
        // Đã có đúng bản này: khỏi đọc nơi lưu.
        return Ok((StatusCode::NOT_MODIFIED, headers).into_response());
    }
    let disposition =
        if is_raster_image(&content_type) { Disposition::Inline } else { Disposition::Attachment };
    set(header::CONTENT_TYPE, content_type)?;
    set(header::CONTENT_DISPOSITION, content_disposition(disposition, &item.name))?;
    set(header::ACCEPT_RANGES, "bytes".to_owned())?;
    set(header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned())?;
    set(header::CONTENT_SECURITY_POLICY, "default-src 'none'; sandbox".to_owned())?;

    let range = match requested_range(request, &etag, size) {
        Requested::Whole => None,
        Requested::Part(range) => Some(range),
        Requested::Unsatisfiable => {
            let error = Error::new(
                ErrorKind::RangeNotSatisfiable,
                "media.range_not_satisfiable",
                "The requested range is outside the content, which is {size} bytes long.",
            )
            .with_param("size", size);
            let mut response = error.into_response();
            let value =
                HeaderValue::from_str(&format!("bytes */{size}")).map_err(Error::internal)?;
            response.headers_mut().insert(header::CONTENT_RANGE, value);
            return Ok(response);
        }
    };
    let (status, length) = match &range {
        None => (StatusCode::OK, size),
        Some(range) => {
            set(header::CONTENT_RANGE, format!("bytes {}-{}/{size}", range.start, range.end - 1))?;
            (StatusCode::PARTIAL_CONTENT, range.end - range.start)
        }
    };
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    if !with_body {
        return Ok((status, headers).into_response());
    }
    let stored = match &copy {
        Some(copy) => service.read_key(&copy.storage_key(), range).await?,
        None => service.read(&item, range).await?,
    };
    Ok((status, headers, Body::from_stream(stored.content)).into_response())
}

/// `Cache-Control` theo phiên bản trong URL và việc item có công khai không.
fn cache_control(item: &Item, version: Option<&str>) -> String {
    let current = version.is_some_and(|version| version == item.version_id.to_string());
    let public = Visibility::of(item) == Visibility::Public && item.trashed_at.is_none();
    match (current, public) {
        (true, true) => format!("public, max-age={LONG_CACHE}, immutable"),
        (true, false) => format!("private, max-age={LONG_CACHE}, immutable"),
        (false, true) => format!("public, max-age={SHORT_CACHE}, must-revalidate"),
        (false, false) => "private, no-cache".to_owned(),
    }
}

/// Khoảng byte client xin.
#[derive(Debug, PartialEq, Eq)]
enum Requested {
    /// Không xin khoảng (hay xin dạng không hỗ trợ — RFC 9110 cho phép trả cả nội dung).
    Whole,
    Part(Range<u64>),
    /// Khoảng nằm ngoài nội dung: 416.
    Unsatisfiable,
}

/// Đọc header `Range` (RFC 9110 §14), chỉ một khoảng; `If-Range` khác `etag` thì bỏ qua.
fn requested_range(headers: &HeaderMap, etag: &str, size: u64) -> Requested {
    let Some(raw) = headers.get(header::RANGE).and_then(|value| value.to_str().ok()) else {
        return Requested::Whole;
    };
    if let Some(condition) = headers.get(header::IF_RANGE).and_then(|value| value.to_str().ok())
        && condition.trim() != etag
    {
        return Requested::Whole;
    }
    let Some(spec) = raw.trim().strip_prefix("bytes=") else {
        return Requested::Whole;
    };
    if spec.contains(',') {
        return Requested::Whole;
    }
    let Some((first, last)) = spec.trim().split_once('-') else {
        return Requested::Whole;
    };
    let parse = |text: &str| text.trim().parse::<u64>().ok();
    match (first.trim().is_empty(), parse(first), parse(last)) {
        // `bytes=-N`: N byte cuối. File rỗng thì không có byte nào để trả (RFC 9110 §14.1.2);
        // khoảng rỗng còn làm nơi lưu báo lỗi (D-112).
        (true, _, Some(suffix)) if suffix > 0 && size > 0 => {
            Requested::Part(size.saturating_sub(suffix)..size)
        }
        (true, _, _) => Requested::Unsatisfiable,
        (false, Some(start), _) if start >= size => Requested::Unsatisfiable,
        (false, Some(start), None) if last.trim().is_empty() => Requested::Part(start..size),
        (false, Some(start), Some(end)) if end >= start => {
            Requested::Part(start..end.saturating_add(1).min(size))
        }
        _ => Requested::Whole,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(value: &str, size: u64) -> Requested {
        let mut headers = HeaderMap::new();
        headers.insert(header::RANGE, HeaderValue::from_str(value).unwrap());
        requested_range(&headers, "\"e\"", size)
    }

    #[test]
    fn doc_header_range() {
        assert_eq!(range("bytes=0-3", 10), Requested::Part(0..4));
        assert_eq!(range("bytes=4-", 10), Requested::Part(4..10));
        assert_eq!(range("bytes=-3", 10), Requested::Part(7..10));
        assert_eq!(range("bytes=-30", 10), Requested::Part(0..10));
        assert_eq!(range("bytes=5-100", 10), Requested::Part(5..10));
        assert_eq!(range("bytes=10-", 10), Requested::Unsatisfiable);
        assert_eq!(range("bytes=-0", 10), Requested::Unsatisfiable);
        // File rỗng: không khoảng nào thoả.
        assert_eq!(range("bytes=-5", 0), Requested::Unsatisfiable);
        assert_eq!(range("bytes=0-", 0), Requested::Unsatisfiable);
        assert_eq!(range("bytes=0-1,4-5", 10), Requested::Whole);
        assert_eq!(range("items=0-1", 10), Requested::Whole);
        assert_eq!(range("bytes=5-2", 10), Requested::Whole);
        let mut headers = HeaderMap::new();
        headers.insert(header::RANGE, HeaderValue::from_static("bytes=0-1"));
        headers.insert(header::IF_RANGE, HeaderValue::from_static("\"old\""));
        assert_eq!(requested_range(&headers, "\"e\"", 10), Requested::Whole);
    }
}
