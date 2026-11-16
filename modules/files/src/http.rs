//! Endpoint REST của module `files`, dưới `/api/v1/files`. Handler chỉ lấy dữ liệu vào,
//! gọi service và trả kết quả; mọi logic nằm ở service.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::storage::ByteStream;
use autoxide_kernel::tenancy::TenantContext;
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
    extensions(("x-permissions" = json!(["files.upload"]))),
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
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<PageQuery>,
) -> Result<Page<FileResponse>, Error> {
    let page = service.list(&tenant, &principal, &PageRequest::try_from(query)?).await?;
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
    extensions(("x-permissions" = json!(["files.upload"]))),
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
    tenant: TenantContext,
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
    let file = service.upload(&tenant, &principal, &query.name, content_type, content).await?;
    Ok(Created::new(format!("/files/{}", file.id), file.into()))
}

/// Get the details of a file.
///
/// Requires `files.upload` for your own files, `files.manage` for anyone's.
#[utoipa::path(
    get,
    path = "/{id}",
    extensions(("x-permissions" = json!(["files.upload", "files.manage"]))),
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
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<FileResponse>, Error> {
    Ok(Data { data: service.get(&tenant, &principal, id).await?.into() })
}

/// Download the content of a file.
///
/// Requires `files.upload` for your own files, `files.manage` for anyone's. The
/// response carries the file's `Content-Type`, its size as `Content-Length`, and its
/// name in `Content-Disposition` (always `attachment`).
#[utoipa::path(
    get,
    path = "/{id}/content",
    extensions(("x-permissions" = json!(["files.upload", "files.manage"]))),
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
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    request_headers: HeaderMap,
) -> Result<Response, Error> {
    let unchanged = |etag: &str| autoxide_kernel::version::if_none_match(&request_headers, etag);
    let Some(download) = service.download(&tenant, &principal, id, unchanged).await? else {
        // Client đã có đúng bản này: lớp `not_modified` của kernel cũng trả 304 như vầy.
        return Ok(StatusCode::NOT_MODIFIED.into_response());
    };
    let file = download.file;
    let header_value = |text: &str| HeaderValue::from_str(text).map_err(Error::internal);
    let headers = [
        (header::CONTENT_TYPE, header_value(&file.content_type)?),
        (header::CONTENT_LENGTH, HeaderValue::from(file.size)),
        (header::CONTENT_DISPOSITION, header_value(&content_disposition(&file.name))?),
        // Trình duyệt không được đoán loại khác với loại đã khai (chặn file HTML giả
        // dạng ảnh chạy script).
        (header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
        (header::ETAG, header_value(&file.etag())?),
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
    extensions(("x-permissions" = json!(["files.upload", "files.manage"]))),
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
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `attachment` kèm tên file (RFC 6266): không bao giờ mở ngay trong trình duyệt.
fn content_disposition(name: &str) -> String {
    autoxide_kernel::http::content_disposition(autoxide_kernel::http::Disposition::Attachment, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::storage::Storage as _;
    use autoxide_testkit::{TestApp, TestDatabase, TestRequest};
    use serde_json::Value;

    /// `ada`: người dùng có `files.upload`; `nobody`: không có permission nào. File lưu trên
    /// đĩa, trong một thư mục tạm riêng; tối đa 64 byte, chỉ `text/*`.
    async fn app(test: &TestDatabase) -> TestApp {
        let dir = std::env::temp_dir().join(format!("autoxide-files-{}", Uuid::now_v7()));
        TestApp::builder(test)
            .setting("AUTOXIDE_FILES_DIR", dir.to_str().unwrap())
            .setting("AUTOXIDE_FILES_MAX_BYTES", "64")
            .setting("AUTOXIDE_FILES_ALLOWED_TYPES", "text/*")
            .user("ada", 0xada, &["files.upload"])
            .user("nobody", 0xb0b, &[])
            .module(crate::module())
            .build()
            .await
            .unwrap()
    }

    async fn send(request: TestRequest<'_>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let answer = request.send().await.unwrap();
        (answer.status(), answer.headers().clone(), answer.body().to_vec())
    }

    fn upload<'a>(app: &'a TestApp, name: &str, content_type: &str, body: Body) -> TestRequest<'a> {
        app.post(&format!("/api/v1/files?name={name}"))
            .token("ada")
            .header(header::CONTENT_TYPE.as_str(), content_type)
            .body(body)
    }

    fn json(bytes: &[u8]) -> Value {
        serde_json::from_slice(bytes).unwrap()
    }

    #[tokio::test]
    async fn tai_len_roi_tai_xuong_qua_api() {
        TestDatabase::run(async |test| {
            let app = app(test).await;

            let (status, _, body) =
                send(upload(&app, "b%C3%A1o.txt", "text/plain", Body::from("hello"))).await;
            assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
            let file = json(&body)["data"].clone();
            assert_eq!((file["name"].as_str(), file["size"].as_u64()), (Some("báo.txt"), Some(5)));

            let id = file["id"].as_str().unwrap();
            let request = app.get(&format!("/api/v1/files/{id}/content")).token("ada");
            let (status, headers, content) = send(request).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(content, b"hello");
            assert_eq!(headers[header::CONTENT_TYPE], "text/plain");
            assert_eq!(headers[header::CONTENT_LENGTH], "5");
            assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
            let disposition = headers[header::CONTENT_DISPOSITION].to_str().unwrap();
            assert!(disposition.ends_with("filename*=UTF-8''b%C3%A1o.txt"), "{disposition}");

            // Người không có permission.
            let request = app.get(&format!("/api/v1/files/{id}")).token("nobody");
            assert_eq!(send(request).await.0, StatusCode::FORBIDDEN);
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
            let request = upload(&app, "a.txt", "text/plain", Body::from(large.clone()))
                .header(header::CONTENT_LENGTH.as_str(), "65");
            let (status, _, body) = send(request).await;
            assert_eq!(
                (status, json(&body)["code"].clone()),
                (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large".into())
            );
            // Gửi theo từng đoạn: dừng khi vượt.
            let stream = futures_util::stream::iter([Ok::<_, std::io::Error>(large)]);
            let (status, _, body) =
                send(upload(&app, "a.txt", "text/plain", Body::from_stream(stream))).await;
            assert_eq!(
                (status, json(&body)["code"].clone()),
                (StatusCode::PAYLOAD_TOO_LARGE, "files.too_large".into())
            );

            let (status, _, body) =
                send(upload(&app, "a.pdf", "application/pdf", Body::from("x"))).await;
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

    /// Bước 33: file của khách nằm dưới tiền tố của khách ở nơi lưu (`tenants/<id>/files/…`);
    /// người của khách khác không thấy; job dọn mồ côi (chạy trên mọi database) không xoá nhầm nội
    /// dung còn dòng ở database của đúng khách.
    #[tokio::test]
    async fn khoa_file_co_tien_to_khach_va_khong_ro_ri() {
        TestDatabase::run(async |test| {
            let dir = std::env::temp_dir().join(format!("autoxide-files-{}", Uuid::now_v7()));
            let app = TestApp::builder(test)
                .setting("AUTOXIDE_FILES_DIR", dir.to_str().unwrap())
                .tenant("a")
                .tenant_dedicated("b")
                .user_in("a", "ada", 0xada, &["files.upload", "files.manage"])
                .user_in("b", "bob", 0xb0b, &["files.upload", "files.manage"])
                .module(crate::module())
                .build()
                .await
                .unwrap();
            let marker = "MARKER-files-a";
            let request = app
                .post(&format!("/api/v1/files?name={marker}.txt"))
                .token("ada")
                .header(header::CONTENT_TYPE.as_str(), "text/plain")
                .body(Body::from("secret of a"));
            let (status, _, body) = send(request).await;
            assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
            let id = json(&body)["data"]["id"].as_str().unwrap().to_owned();

            let a = app.tenant("a").await.unwrap();
            let shared = app.api::<dyn autoxide_kernel::storage::Storage>().unwrap();
            let key = format!("tenants/{}/files/{id}", a.id().as_uuid());
            assert!(shared.get(&key).await.unwrap().is_some(), "content under the tenant's prefix");
            assert!(shared.get(&format!("files/{id}")).await.unwrap().is_none());

            let seeded = [("id", id.clone())];
            let violations =
                autoxide_testkit::leak_check(&app, "bob", marker, &seeded).await.unwrap();
            assert_eq!(violations, Vec::<String>::new());
            let (status, _, _) =
                send(app.get(&format!("/api/v1/files/{id}/content")).token("bob")).await;
            assert_eq!(status, StatusCode::NOT_FOUND);

            // Một ngày sau: job dọn (ở mọi database) giữ nội dung còn dòng, xoá nội dung mồ côi.
            let orphan = format!("tenants/{}/files/{}", a.id().as_uuid(), Uuid::now_v7());
            let stream = futures_util::stream::iter([Ok(bytes::Bytes::from_static(b"x"))]).boxed();
            shared.put(&orphan, stream).await.unwrap();
            let service = FileService::new(
                shared.clone(),
                autoxide_kernel::event::Outbox::standalone("files"),
                crate::config::Limits { max_bytes: 64, allowed_types: None },
            );
            let later = chrono::Utc::now() + chrono::Duration::hours(25);
            let removed = service.clean_orphans(app.registry(), later).await.unwrap();
            assert_eq!(removed, 1);
            assert!(shared.get(&key).await.unwrap().is_some());
            assert!(shared.get(&orphan).await.unwrap().is_none());
        })
        .await
        .unwrap();
    }

    /// Bước 33: placement có nơi lưu riêng (`autoxide placement storage`): file của khách trên đó
    /// nằm ở nơi lưu riêng, không ở nơi lưu chung; đọc lại được. Job dọn mồ côi quét cả nơi lưu riêng
    /// và ở mỗi nơi chỉ xét khách đang lưu file ở đó (bản sao còn sót ở nơi lưu chung để nguyên).
    #[tokio::test]
    async fn placement_co_noi_luu_rieng() {
        TestDatabase::run(async |test| {
            let root = std::env::temp_dir().join(format!("autoxide-files-{}", Uuid::now_v7()));
            let (dir, own_dir) = (root.join("shared"), root.join("own"));
            let app = TestApp::builder(test)
                .setting("AUTOXIDE_FILES_DIR", dir.to_str().unwrap())
                .tenant("a")
                .tenant_dedicated("b")
                .user_in("a", "ada", 0xada, &["files.upload", "files.manage"])
                .user_in("b", "bob", 0xb0b, &["files.upload", "files.manage"])
                .module(crate::module())
                .build()
                .await
                .unwrap();
            let placements = app.modules().placements(app.registry()).unwrap();
            let location = autoxide_kernel::storage::StorageLocation::Local {
                dir: own_dir.to_str().unwrap().to_owned(),
            };
            // `b` đã nằm trên placement `b` (chưa có file nào): đổi nơi lưu cần `force`.
            let error = placements.set_storage("b", Some(location.clone()), false).await;
            assert_eq!(error.unwrap_err().code(), "tenant_placement_storage_in_use");
            placements.set_storage("b", Some(location), true).await.unwrap();

            let upload = |token: &'static str, text: &'static str| {
                app.post("/api/v1/files?name=x.txt")
                    .token(token)
                    .header(header::CONTENT_TYPE.as_str(), "text/plain")
                    .body(Body::from(text))
            };
            let (status, _, body) = send(upload("bob", "of b")).await;
            assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
            let id = json(&body)["data"]["id"].as_str().unwrap().to_owned();
            let (status, _, body) = send(upload("ada", "of a")).await;
            assert_eq!(status, StatusCode::CREATED);
            let a_id = json(&body)["data"]["id"].as_str().unwrap().to_owned();

            let (a, b) = (app.tenant("a").await.unwrap(), app.tenant("b").await.unwrap());
            let shared = app.api::<dyn autoxide_kernel::storage::Storage>().unwrap();
            let own = crate::store::ObjectStorage::open(&crate::config::Backend::Local {
                dir: own_dir.clone(),
            })
            .unwrap();
            let key = format!("tenants/{}/files/{id}", b.id());
            assert!(own.get(&key).await.unwrap().is_some(), "in the placement's own storage");
            assert!(shared.get(&key).await.unwrap().is_none());
            assert!(
                shared.get(&format!("tenants/{}/files/{a_id}", a.id())).await.unwrap().is_some()
            );
            let (status, _, body) =
                send(app.get(&format!("/api/v1/files/{id}/content")).token("bob")).await;
            assert_eq!((status, body.as_slice()), (StatusCode::OK, b"of b".as_slice()));

            // Mồ côi ở nơi lưu riêng (của `b`) và ở nơi lưu chung (của `a`): bị xoá. Khoá của `b` ở
            // nơi lưu chung (`b` không lưu ở đó): để nguyên.
            let put = async |storage: &dyn autoxide_kernel::storage::Storage, key: String| {
                let stream =
                    futures_util::stream::iter([Ok(bytes::Bytes::from_static(b"x"))]).boxed();
                storage.put(&key, stream).await.unwrap()
            };
            let orphan_own = format!("tenants/{}/files/{}", b.id(), Uuid::now_v7());
            let orphan_shared = format!("tenants/{}/files/{}", a.id(), Uuid::now_v7());
            let stray = format!("tenants/{}/files/{}", b.id(), Uuid::now_v7());
            put(&own, orphan_own.clone()).await;
            put(&*shared, orphan_shared.clone()).await;
            put(&*shared, stray.clone()).await;
            let service = FileService::new(
                shared.clone(),
                autoxide_kernel::event::Outbox::standalone("files"),
                crate::config::Limits { max_bytes: 64, allowed_types: None },
            );
            let later = chrono::Utc::now() + chrono::Duration::hours(25);
            assert_eq!(service.clean_orphans(app.registry(), later).await.unwrap(), 2);
            assert!(own.get(&orphan_own).await.unwrap().is_none());
            assert!(own.get(&key).await.unwrap().is_some());
            assert!(shared.get(&orphan_shared).await.unwrap().is_none());
            assert!(shared.get(&stray).await.unwrap().is_some());
        })
        .await
        .unwrap();
    }
}
