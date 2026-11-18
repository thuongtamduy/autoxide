//! Tool MCP của module `files`: cùng service và permission với `/api/v1/files`. Nội dung
//! file đi qua MCP dạng base64, nên tool chỉ dành cho file nhỏ; file lớn dùng REST.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use futures_util::StreamExt as _;
use futures_util::TryStreamExt as _;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::http::{FileResponse, StartUploadRequest};
use crate::permissions::{FILES_MANAGE, FILES_UPLOAD};
use crate::service::FileService;

/// Lớn nhất mà `read_file` trả (base64 to hơn khoảng một phần ba).
const MAX_TOOL_BYTES: u64 = 1024 * 1024;

/// A file to act on.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileId {
    /// Id of the file.
    id: Uuid,
}

/// A small file to upload.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UploadInput {
    /// File name, 1 to 255 characters.
    name: String,
    /// Media type, e.g. `text/plain`. Omit for `application/octet-stream`.
    content_type: Option<String>,
    /// The content, base64-encoded (standard alphabet, with padding).
    content_base64: String,
}

/// A file with its content.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct FileContent {
    #[serde(flatten)]
    file: FileResponse,
    /// The content, base64-encoded.
    content_base64: String,
}

/// An upload straight to storage to complete.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UploadId {
    /// Id of the upload, from `files.create_upload`.
    id: Uuid,
}

pub(crate) fn all(service: &Arc<FileService>) -> Vec<Tool> {
    let (list, get, read, upload, delete) =
        (service.clone(), service.clone(), service.clone(), service.clone(), service.clone());
    let (start, complete) = (service.clone(), service.clone());
    vec![
        Tool::new(
            "create_upload",
            "Start uploading a large file straight to storage (S3): give its name, type, exact \
             size and SHA-256 (hex); you get a signed link. Send the file there yourself with the \
             given method and headers, then call `files.confirm_upload`. On disk storage this \
             answers `files.direct_upload_unavailable`: use `files.upload_file` instead.",
            move |tenant: TenantContext, principal: Principal, input: StartUploadRequest| {
                let service = start.clone();
                async move {
                    let started = service.start_upload(&tenant, &principal, input.into()).await?;
                    let id = started.upload.id;
                    Ok::<_, Error>(autoxide_kernel::upload::UploadLink::new(id, started.link))
                }
            },
        )
        .requires(FILES_UPLOAD),
        Tool::new(
            "confirm_upload",
            "Complete an upload started with `files.create_upload` after sending the file: the \
             server checks its size and SHA-256 and creates the file (a mismatching file is \
             removed). Calling it again returns the same file.",
            move |tenant: TenantContext, principal: Principal, input: UploadId| {
                let service = complete.clone();
                async move {
                    let file = match service.complete_upload(&tenant, &principal, input.id).await? {
                        crate::uploads::Completed::Created(file)
                        | crate::uploads::Completed::Existing(file) => file,
                    };
                    Ok::<_, Error>(FileResponse::from(file))
                }
            },
        )
        .requires(FILES_UPLOAD)
        .or_requires(FILES_MANAGE),
        Tool::new(
            "list_files",
            "List the files you uploaded, newest first; follow `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: PageInput| {
                let service = list.clone();
                async move {
                    let page = service.list(&tenant, &principal, &input.page()?).await?;
                    Ok::<Page<FileResponse>, Error>(page.map(FileResponse::from))
                }
            },
        )
        .requires(FILES_UPLOAD)
        .read_only(),
        Tool::new(
            "get_file",
            "Get the details of a file (name, type, size, SHA-256). Your own files need \
             `files.upload`; anyone's need `files.manage`.",
            move |tenant: TenantContext, principal: Principal, input: FileId| {
                let service = get.clone();
                async move {
                    Ok::<_, Error>(FileResponse::from(
                        service.get(&tenant, &principal, input.id).await?,
                    ))
                }
            },
        )
        .requires(FILES_UPLOAD)
        .or_requires(FILES_MANAGE)
        .read_only(),
        Tool::new(
            "read_file",
            "Get a file with its content as base64, for files up to 1 MiB; download larger ones \
             with `GET /api/v1/files/{id}/content`. Same access as `get_file`.",
            move |tenant: TenantContext, principal: Principal, input: FileId| {
                let service = read.clone();
                async move {
                    let file = service.get(&tenant, &principal, input.id).await?;
                    if u64::try_from(file.size).unwrap_or(u64::MAX) > MAX_TOOL_BYTES {
                        return Err(too_large_for_tool(input.id));
                    }
                    let download = service
                        .download(&tenant, &principal, input.id, |_| false)
                        .await?
                        .ok_or_else(|| Error::internal("a download without a known version"))?;
                    let chunks: Vec<Bytes> = download.content.try_collect().await?;
                    let content_base64 = STANDARD.encode(chunks.concat());
                    Ok(FileContent { file: download.file.into(), content_base64 })
                }
            },
        )
        .requires(FILES_UPLOAD)
        .or_requires(FILES_MANAGE)
        .read_only(),
        Tool::new(
            "upload_file",
            "Upload a small file, its content base64-encoded (the size and type limits of the \
             server apply). Upload large files with `POST /api/v1/files`.",
            move |tenant: TenantContext, principal: Principal, input: UploadInput| {
                let service = upload.clone();
                async move {
                    let content = STANDARD.decode(input.content_base64.trim()).map_err(|_| {
                        Error::bad_request(
                            "files.invalid_base64",
                            "`content_base64` is not valid base64.",
                        )
                    })?;
                    let stream = futures_util::stream::iter([Ok(Bytes::from(content))]).boxed();
                    let file = service
                        .upload(
                            &tenant,
                            &principal,
                            &input.name,
                            input.content_type.as_deref(),
                            stream,
                        )
                        .await?;
                    Ok::<_, Error>(FileResponse::from(file))
                }
            },
        )
        .requires(FILES_UPLOAD),
        Tool::new(
            "delete_file",
            "Delete a file and its content; it cannot be restored. Same access as `get_file`.",
            move |tenant: TenantContext, principal: Principal, input: FileId| {
                let service = delete.clone();
                async move {
                    service.delete(&tenant, &principal, input.id).await?;
                    Ok::<_, Error>(serde_json::json!({ "deleted": input.id }))
                }
            },
        )
        .requires(FILES_UPLOAD)
        .or_requires(FILES_MANAGE)
        .destructive(),
    ]
}

fn too_large_for_tool(id: Uuid) -> Error {
    Error::new(
        ErrorKind::PayloadTooLarge,
        "files.too_large_for_tool",
        "This file is larger than {max} bytes, the most a tool returns. Download it with \
         `GET /api/v1/files/{id}/content`.",
    )
    .with_param("max", MAX_TOOL_BYTES)
    .with_param("id", id)
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::access::Principal;
    use autoxide_testkit::TestDatabase;
    use serde_json::json;

    use super::*;
    use crate::testing::{no_limits, service};

    #[tokio::test]
    async fn tai_len_doc_va_xoa_file_qua_tool() {
        TestDatabase::run(async |test| {
            let (files, database) = service(test, no_limits()).await;
            let tools = all(&files);
            let tool = |name: &str| tools.iter().find(|tool| tool.name() == name).unwrap();
            let tenant = TenantContext::default_on(database);
            let ada = Principal::new(Uuid::now_v7(), ["files.upload".to_owned()]);
            let bob = Principal::new(Uuid::now_v7(), ["files.upload".to_owned()]);

            let input = json!({"name": "a.txt", "content_type": "text/plain", "content_base64": "aGVsbG8="});
            let file = tool("upload_file").call(tenant.clone(), ada.clone(), input).await.unwrap();
            assert_eq!((file["name"].as_str(), file["size"].as_u64()), (Some("a.txt"), Some(5)));
            let id = file["id"].clone();

            let read = tool("read_file").call(tenant.clone(), ada.clone(), json!({"id": id})).await.unwrap();
            assert_eq!(read["content_base64"], "aGVsbG8=");
            assert_eq!(read["sha256"], file["sha256"]);
            let page = tool("list_files").call(tenant.clone(), ada.clone(), json!({})).await.unwrap();
            assert_eq!(page["data"][0]["id"], id);
            // File của người khác: như không có, giống REST.
            let theirs = tool("get_file").call(tenant.clone(), bob, json!({"id": id})).await.unwrap_err();
            assert_eq!(theirs.code(), "files.file_not_found");
            let bad = json!({"name": "b.txt", "content_base64": "%%%"});
            let bad = tool("upload_file").call(tenant.clone(), ada.clone(), bad).await.unwrap_err();
            assert_eq!(bad.code(), "files.invalid_base64");

            tool("delete_file").call(tenant.clone(), ada.clone(), json!({"id": id})).await.unwrap();
            let gone = tool("read_file").call(tenant.clone(), ada, json!({"id": id})).await.unwrap_err();
            assert_eq!(gone.code(), "files.file_not_found");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn file_lon_thi_tool_bao_dung_rest() {
        TestDatabase::run(async |test| {
            let limits =
                crate::config::Limits { max_bytes: 4 * MAX_TOOL_BYTES, allowed_types: None };
            let (files, database) = service(test, limits).await;
            let tenant = TenantContext::default_on(database);
            let ada = Principal::new(Uuid::now_v7(), ["files.upload".to_owned()]);
            let big = Bytes::from(vec![b'x'; usize::try_from(MAX_TOOL_BYTES).unwrap() + 1]);
            let stream = futures_util::stream::iter([Ok(big)]).boxed();
            let file = files.upload(&tenant, &ada, "big.bin", None, stream).await.unwrap();
            let tools = all(&files);
            let read = tools.iter().find(|tool| tool.name() == "read_file").unwrap();
            let error = read.call(tenant, ada, json!({"id": file.id})).await.unwrap_err();
            assert_eq!(error.code(), "files.too_large_for_tool");
            assert!(error.to_string().contains(&format!("/api/v1/files/{}/content", file.id)));
        })
        .await
        .unwrap();
    }

    /// Tool tải thẳng: cùng service với REST (link giả của nơi lưu trong bộ nhớ).
    #[tokio::test]
    async fn tai_thang_qua_tool() {
        TestDatabase::run(async |test| {
            let storage = std::sync::Arc::new(autoxide_testkit::MemoryStorage::new());
            storage.presign(true);
            let (files, database) =
                crate::testing::service_on(test, no_limits(), storage.clone()).await;
            let tools = all(&files);
            let tool = |name: &str| tools.iter().find(|tool| tool.name() == name).unwrap();
            let tenant = TenantContext::default_on(database);
            let ada = Principal::new(Uuid::now_v7(), ["files.upload".to_owned()]);
            let sha = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
            let input =
                json!({"name": "a.txt", "content_type": "text/plain", "size": 5, "sha256": sha});
            let link =
                tool("create_upload").call(tenant.clone(), ada.clone(), input).await.unwrap();
            assert_eq!(link["method"], "PUT");
            storage.put_presigned(link["url"].as_str().unwrap(), &b"hello"[..]);
            let confirm = json!({"id": link["id"]});
            let file = tool("confirm_upload")
                .call(tenant.clone(), ada.clone(), confirm.clone())
                .await
                .unwrap();
            assert_eq!((file["id"].clone(), file["size"].as_u64()), (link["id"].clone(), Some(5)));
            let again = tool("confirm_upload").call(tenant, ada, confirm).await.unwrap();
            assert_eq!(again, file);
        })
        .await
        .unwrap();
    }
}
