//! Endpoint REST của module `files`, dưới `/api/v1/files`. Handler chỉ lấy dữ liệu vào,
//! gọi service và trả kết quả; mọi logic nằm ở service.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::storage::ByteStream;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use futures_util::{StreamExt as _, TryStreamExt as _};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::service::{FileRecord, FileService, too_large};

pub(crate) fn routes(service: Arc<FileService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_files, upload_file))
        .routes(routes!(get_file, delete_file))
        .routes(routes!(download_file))
        .with_state(service)
}

/// An uploaded file. The content is at `GET /api/v1/files/{id}/content`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct FileResponse {
    id: Uuid,
    /// File name given at upload.
    #[schema(example = "report.pdf")]
    name: String,
    /// Media type given at upload.
    #[schema(example = "application/pdf")]
    content_type: String,
    /// Size in bytes.
    #[schema(example = 48213)]
    size: u64,
    /// SHA-256 of the content, as 64 hex digits: check a download against it.
    sha256: String,
    /// The user who uploaded the file.
    owner_id: Uuid,
    created_at: DateTime<Utc>,
}

impl From<FileRecord> for FileResponse {
    fn from(file: FileRecord) -> Self {
        FileResponse {
            id: file.id,
            name: file.name,
            content_type: file.content_type,
            size: u64::try_from(file.size).unwrap_or_default(),
            sha256: file.sha256,
            owner_id: file.owner_id,
            created_at: file.created_at,
        }
    }
}

/// Details of an upload.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct UploadQuery {
    /// File name, 1 to 255 characters, without `/` or `\`. Given back on download.
    #[param(example = "report.pdf")]
    name: String,
}

/// List your files.
///
/// Requires `files.upload`. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/",
    operation_id = "list_files",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of your files.", body = Page<FileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `files.upload`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_files(
    State(service): State<Arc<FileService>>,
    principal: Principal,
    Query(query): Query<PageQuery>,
) -> Result<Page<FileResponse>, Error> {
    let page = service.list(&principal, &PageRequest::try_from(query)?).await?;
    Ok(page.map(FileResponse::from))
}

/// Upload a file.
///
/// Requires `files.upload`. Send the raw content as the body (not a multipart form),
/// its media type as `Content-Type` (default `application/octet-stream`) and its name
/// in `name`. The server may limit the size (413) and the media types (415) it
/// accepts. Retry safely with an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/",
    operation_id = "upload_file",
    security(("bearer" = [])),
    params(UploadQuery),
    request_body(
        content = Vec<u8>,
        content_type = "application/octet-stream",
        description = "The content of the file, with any `Content-Type`.",
    ),
    responses(
        (status = 201, description = "Uploaded.", body = Data<FileResponse>),
        (status = 400, description = "The `Content-Type` is not a media type (code `files.invalid_content_type`), or the upload was cut off (code `files.upload_interrupted`).", body = Problem),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `files.upload`; code `permission_denied`.", body = Problem),
        (status = 413, description = "The file is too large; code `files.too_large` (or `payload_too_large` when `Content-Length` already says so).", body = Problem),
        (status = 415, description = "This media type is not accepted; code `files.type_not_allowed`. The message lists the accepted types.", body = Problem),
        (status = 422, description = "The name is missing or invalid; see `errors`.", body = Problem),
    )
)]
async fn upload_file(
    State(service): State<Arc<FileService>>,
    principal: Principal,
    Query(query): Query<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<Created<FileResponse>, Error> {
    let content_type = headers.get(header::CONTENT_TYPE).map(HeaderValue::to_str).transpose();
    let content_type = content_type.map_err(|_| {
        Error::bad_request(
            "files.invalid_content_type",
            "The `Content-Type` header must be a media type like `application/pdf`.",
        )
    })?;
    let max = service.limits().max_bytes;
    let content: ByteStream = body
        .into_data_stream()
        .map_err(move |error| {
            if autoxide_kernel::http::is_too_large(&error) {
                too_large(max)
            } else {
                Error::bad_request(
                    "files.upload_interrupted",
                    "The upload was cut off before the whole file arrived. Send it again.",
                )
            }
        })
        .boxed();
    let file = service.upload(&principal, &query.name, content_type, content).await?;
    Ok(Created::new(format!("/files/{}", file.id), file.into()))
}

/// Get the details of a file.
///
/// Requires `files.upload` for your own files, `files.manage` for anyone's.
#[utoipa::path(
    get,
    path = "/{id}",
    operation_id = "get_file",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the file.")),
    responses(
        (status = 200, description = "The file.", body = Data<FileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `files.upload`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No file with this id that you may see; code `files.file_not_found`.", body = Problem),
    )
)]
async fn get_file(
    State(service): State<Arc<FileService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<FileResponse>, Error> {
    Ok(Data { data: service.get(&principal, id).await?.into() })
}

/// Download the content of a file.
///
/// Requires `files.upload` for your own files, `files.manage` for anyone's. The
/// response carries the file's `Content-Type`, its size as `Content-Length`, and its
/// name in `Content-Disposition` (always `attachment`).
#[utoipa::path(
    get,
    path = "/{id}/content",
    operation_id = "download_file",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the file.")),
    responses(
        (status = 200, description = "The content, with the file's own `Content-Type`.", body = Vec<u8>, content_type = "application/octet-stream"),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `files.upload`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No file with this id that you may see; code `files.file_not_found`.", body = Problem),
    )
)]
async fn download_file(
    State(service): State<Arc<FileService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Response, Error> {
    let download = service.download(&principal, id).await?;
    let file = download.file;
    let header_value = |text: &str| HeaderValue::from_str(text).map_err(Error::internal);
    let headers = [
        (header::CONTENT_TYPE, header_value(&file.content_type)?),
        (header::CONTENT_LENGTH, HeaderValue::from(file.size)),
        (header::CONTENT_DISPOSITION, header_value(&content_disposition(&file.name))?),
        // Trình duyệt không được đoán loại khác với loại đã khai (chặn file HTML giả
        // dạng ảnh chạy script).
        (header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
        (header::ETAG, header_value(&format!("\"{}\"", file.sha256))?),
        (header::CACHE_CONTROL, HeaderValue::from_static("private, no-cache")),
    ];
    Ok((headers, Body::from_stream(download.content)).into_response())
}

/// Delete a file.
///
/// Requires `files.upload` for your own files, `files.manage` for anyone's.
#[utoipa::path(
    delete,
    path = "/{id}",
    operation_id = "delete_file",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the file.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `files.upload`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No file with this id that you may see (or it was already deleted); code `files.file_not_found`.", body = Problem),
    )
)]
async fn delete_file(
    State(service): State<Arc<FileService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete(&principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `attachment` kèm tên file (RFC 6266): không bao giờ mở ngay trong trình duyệt.
fn content_disposition(name: &str) -> String {
    autoxide_kernel::http::content_disposition(autoxide_kernel::http::Disposition::Attachment, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::access::Authenticator;
    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::http::BodyLimit;
    use autoxide_testkit::TestDatabase;
    use axum::Router;
    use axum::http::{Method, Request};
    use serde_json::Value;
    use tower::ServiceExt as _;

    use crate::config::Limits;

    /// `Bearer ada`: người dùng có `files.upload`; còn lại: không có permission nào.
    struct FakeAuthenticator(Uuid);

    #[async_trait::async_trait]
    impl Authenticator for FakeAuthenticator {
        async fn authenticate(&self, authorization: Option<&str>) -> Result<Principal, Error> {
            Ok(match authorization {
                Some("Bearer ada") => Principal::new(self.0, ["files.upload".to_owned()]),
                _ => Principal::new(Uuid::now_v7(), []),
            })
        }
    }

    async fn app(test: &TestDatabase) -> Router {
        let limits = Limits { max_bytes: 64, allowed_types: Some(vec!["text/*".into()]) };
        let (service, _) = crate::testing::service(test, limits).await;
        let source = ConfigSource::default();
        let mut reader = ConfigReader::new(&source);
        let config = Config::read(&mut reader);
        let (config, _) = reader.finish(config).unwrap();
        let authenticator: Arc<dyn Authenticator> = Arc::new(FakeAuthenticator(Uuid::now_v7()));
        autoxide_kernel::http::build(
            &config.http,
            OpenApiRouter::new(),
            vec![("files", routes(service))],
            autoxide_kernel::http::Extras {
                authenticator: Some(authenticator),
                body_limits: vec![("files", BodyLimit::new(Method::POST, "/", 64))],
                ..Default::default()
            },
        )
        .router
    }

    async fn send(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let response = app.clone().oneshot(request).await.unwrap();
        let (parts, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        (parts.status, parts.headers, bytes.to_vec())
    }

    fn upload(name: &str, content_type: &str, body: Body) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(format!("/api/v1/files?name={name}"))
            .header(header::AUTHORIZATION, "Bearer ada")
            .header(header::CONTENT_TYPE, content_type)
            .body(body)
            .unwrap()
    }

    fn json(bytes: &[u8]) -> Value {
        serde_json::from_slice(bytes).unwrap()
    }

    #[tokio::test]
    async fn tai_len_roi_tai_xuong_qua_api() {
        TestDatabase::run(async |test| {
            let app = app(test).await;

            let (status, _, body) =
                send(&app, upload("b%C3%A1o.txt", "text/plain", Body::from("hello"))).await;
            assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
            let file = json(&body)["data"].clone();
            assert_eq!((file["name"].as_str(), file["size"].as_u64()), (Some("báo.txt"), Some(5)));

            let id = file["id"].as_str().unwrap();
            let request = Request::builder()
                .uri(format!("/api/v1/files/{id}/content"))
                .header(header::AUTHORIZATION, "Bearer ada")
                .body(Body::empty())
                .unwrap();
            let (status, headers, content) = send(&app, request).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(content, b"hello");
            assert_eq!(headers[header::CONTENT_TYPE], "text/plain");
            assert_eq!(headers[header::CONTENT_LENGTH], "5");
            assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
            let disposition = headers[header::CONTENT_DISPOSITION].to_str().unwrap();
            assert!(disposition.ends_with("filename*=UTF-8''b%C3%A1o.txt"), "{disposition}");

            // Người không có permission.
            let request =
                Request::builder().uri(format!("/api/v1/files/{id}")).body(Body::empty()).unwrap();
            assert_eq!(send(&app, request).await.0, StatusCode::FORBIDDEN);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn file_qua_lon_hay_sai_loai_bi_tu_choi() {
        TestDatabase::run(async |test| {
            let app = app(test).await;
            let large = "x".repeat(65);

            // Có `Content-Length`: kernel chặn trước khi đọc.
            let mut request = upload("a.txt", "text/plain", Body::from(large.clone()));
            request.headers_mut().insert(header::CONTENT_LENGTH, HeaderValue::from(65));
            let (status, _, body) = send(&app, request).await;
            assert_eq!(
                (status, json(&body)["code"].clone()),
                (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large".into())
            );
            // Gửi theo từng đoạn: dừng khi vượt.
            let stream = futures_util::stream::iter([Ok::<_, std::io::Error>(large)]);
            let (status, _, body) =
                send(&app, upload("a.txt", "text/plain", Body::from_stream(stream))).await;
            assert_eq!(
                (status, json(&body)["code"].clone()),
                (StatusCode::PAYLOAD_TOO_LARGE, "files.too_large".into())
            );

            let (status, _, body) =
                send(&app, upload("a.pdf", "application/pdf", Body::from("x"))).await;
            assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
            assert_eq!(json(&body)["code"], "files.type_not_allowed");
        })
        .await
        .unwrap();
    }

    #[test]
    fn ten_file_trong_content_disposition() {
        assert_eq!(
            content_disposition("báo cáo \"Q1\".pdf"),
            "attachment; filename=\"b_o c_o _Q1_.pdf\"; \
             filename*=UTF-8''b%C3%A1o%20c%C3%A1o%20%22Q1%22.pdf"
        );
    }
}
