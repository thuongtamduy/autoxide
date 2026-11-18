//! Tải item lớn thẳng lên S3 (bước 40 P4, D-378): cấp link ký sẵn, xác nhận, dọn lượt bỏ dở —
//! cùng khung với `files` (`autoxide_kernel::upload`, bảng `media_uploads`). Nơi lưu là ổ đĩa thì
//! không có link: tải qua `POST /api/v1/media` như cũ.
//!
//! Xác nhận đọc lại cả item qua **cùng bộ kiểm** như tải qua máy chủ (`upload.rs`): loại thật bằng
//! magic bytes, SVG không có nội dung chạy được, kích thước ảnh, cỡ và SHA-256 đúng như khai. Không
//! đạt thì xoá nội dung (lượt còn: PUT lại đúng file rồi xác nhận lại trước hạn). Đạt thì tạo item
//! như tải qua máy chủ (cùng event, cùng việc tạo ảnh thu nhỏ) trong transaction khoá dòng lượt.

use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::storage::{PresignedUpload, Storage as _, UploadConstraints};
use autoxide_kernel::tenancy::{DatabaseRegistry, OrgId, ScopeExt, TenantContext};
use autoxide_kernel::upload::{DirectUpload, NewDirectUpload, UploadStatus, parse_sha256};
use chrono::{DateTime, Utc};
use sea_orm::EntityTrait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::normalize_type;
use crate::entity::item::{Entity, storage_key};
use crate::library::sniff;
use crate::library::upload::{NewUpload, Uploaded};
use crate::permissions::MEDIA_WRITE;
use crate::service::{MediaService, check_name, item_not_found};

/// Job dọn lượt tải lên không được xác nhận (mỗi giờ, trên mọi database).
pub(crate) const PURGE_UPLOADS: &str = "media.purge_expired_uploads";
/// Lượt quá hạn chừng này mới bị dọn: client có thể vừa PUT xong lúc hết hạn.
const PURGE_AFTER: chrono::Duration = chrono::Duration::hours(1);
/// Dòng của lượt đã xác nhận được giữ chừng này (xác nhận gọi lại trả cùng item).
const KEEP_COMPLETED: chrono::Duration = chrono::Duration::days(7);
/// Số lượt dọn mỗi lần chạy.
const PURGE_BATCH: u64 = 200;

/// Lượt tải lên xin cấp.
#[derive(Debug, Clone, Default)]
pub(crate) struct StartMediaUpload {
    pub name: String,
    pub content_type: String,
    pub size: u64,
    pub sha256: String,
    pub folder_id: Option<Uuid>,
    pub tags: Vec<String>,
    pub org_id: Option<Uuid>,
}

/// Dữ liệu riêng của lượt (cột `details`), đọc lại lúc xác nhận.
#[derive(Debug, Serialize, Deserialize)]
struct Details {
    version_id: Uuid,
    folder_id: Option<Uuid>,
    tags: Vec<String>,
}

/// Kết quả xác nhận: item (kèm item trùng nội dung) và có phải vừa tạo không.
#[derive(Debug)]
pub(crate) struct Confirmed {
    pub uploaded: Uploaded,
    pub created: bool,
}

impl MediaService {
    /// Bắt đầu một lượt tải thẳng. Cần `media.write`. Mọi lỗi của đầu vào báo một lần.
    pub(crate) async fn start_upload(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: StartMediaUpload,
    ) -> Result<(DirectUpload, PresignedUpload), Error> {
        principal.require(&MEDIA_WRITE)?;
        let mut problems = Vec::new();
        let name = check_name(&input.name, &mut problems);
        let org = match input.org_id {
            Some(org) => Some(OrgId::from_uuid(org)),
            None => principal.home_org(tenant.id(), &MEDIA_WRITE),
        };
        if org.is_none() {
            problems.push(FieldError::new(
                "org_id",
                "required",
                "you work in several organization units: choose the unit of the item",
            ));
        }
        let content_type = self.direct_type(&input.content_type, &mut problems);
        let max = self.config.direct.max_bytes;
        if input.size == 0 || input.size > max {
            problems.push(
                FieldError::new("size", "out_of_range", "must be 1 to {max} bytes")
                    .with_param("max", max),
            );
        }
        let digest = parse_sha256(&input.sha256);
        if digest.is_none() {
            problems.push(FieldError::new(
                "sha256",
                "invalid",
                "must be the SHA-256 of the content, as 64 hex digits",
            ));
        }
        if let Some(folder) = input.folder_id {
            self.check_folder_exists(tenant, folder, "folder_id", &mut problems).await?;
        }
        let tags = crate::library::tags::check_tags(&input.tags, "tags", &mut problems);
        let (Some(org), Some(content_type), Some(digest), true) =
            (org, content_type, digest, problems.is_empty())
        else {
            return Err(Error::validation(problems));
        };
        // Nút do người tải nêu: ngoài phạm vi thì báo ngay, khỏi để client gửi cả file.
        let target = OrgTarget::new(org, Some(principal.user_id()));
        principal.require_at(tenant.database().read(), &MEDIA_WRITE, target).await?;

        let (id, version_id) = (Uuid::now_v7(), Uuid::now_v7());
        let key = storage_key(id, version_id);
        let constraints = UploadConstraints::new(input.size, content_type.clone(), digest);
        let link = self
            .store(tenant)
            .presign_upload(&key, &constraints, self.config.direct.ttl)
            .await?
            .ok_or_else(direct_upload_unavailable)?;
        let details = Details { version_id, folder_id: input.folder_id, tags };
        let new = NewDirectUpload {
            id,
            owner_id: principal.user_id(),
            org_id: Some(org.as_uuid()),
            storage_key: key,
            name,
            content_type,
            size: input.size,
            sha256: autoxide_kernel::upload::hex(&digest),
            details: serde_json::to_value(&details).map_err(Error::internal)?,
            expires_at: link.expires_at,
        };
        let upload = self.uploads.insert(tenant.database().write(), tenant, new).await?;
        tracing::info!(upload_id = %id, size = input.size, "direct media upload started");
        Ok((upload, link))
    }

    /// Loại khai của lượt tải thẳng: bắt buộc cụ thể (link chỉ nhận đúng `Content-Type` này), đã
    /// chuẩn hoá về tên chính và được phép.
    fn direct_type(&self, raw: &str, problems: &mut Vec<FieldError>) -> Option<String> {
        let declared = normalize_type(raw).filter(|kind| kind != "application/octet-stream");
        let Some(declared) = declared else {
            problems.push(FieldError::new(
                "content_type",
                "invalid",
                "must be the real media type of the file, like `video/mp4`",
            ));
            return None;
        };
        let declared = sniff::canonical(&declared).to_owned();
        if !self.config.limits.allows(&declared) {
            let allowed = self.config.limits.allowed_types.join(", ");
            problems.push(
                FieldError::new(
                    "content_type",
                    "not_allowed",
                    "media of this type cannot be uploaded here; allowed types: {allowed}",
                )
                .with_param("allowed", allowed),
            );
            return None;
        }
        Some(declared)
    }

    /// Xác nhận lượt `id`: đọc lại và kiểm nội dung, rồi tạo item. Cần `media.write`; chỉ người bắt
    /// đầu lượt thấy lượt.
    pub(crate) async fn complete_upload(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Confirmed, Error> {
        principal.require(&MEDIA_WRITE)?;
        let upload = self
            .uploads
            .find(tenant.database().write(), tenant, id)
            .await?
            .filter(|upload| upload.owner_id == principal.user_id())
            .ok_or_else(|| upload_not_found(id))?;
        if upload.status == UploadStatus::Completed {
            return self.existing(tenant, id).await;
        }
        if Utc::now() > upload.expires_at {
            return Err(upload_expired(id));
        }
        let details: Details =
            serde_json::from_value(upload.details.clone()).map_err(Error::internal)?;
        let key = upload.storage_key.clone();
        let declared = Some(upload.content_type.clone());
        let received =
            match self.inspect_stored(tenant, &key, declared, self.config.direct.max_bytes).await {
                Ok(Some(received)) => received,
                Ok(None) => return Err(upload_incomplete(id)),
                // Loại thật khác, SVG có script, ảnh hỏng…: không giữ nội dung đó.
                Err(error) => {
                    self.discard(tenant, &key).await;
                    return Err(error);
                }
            };
        let mut wrong = Vec::new();
        if u64::try_from(received.size).ok() != Some(upload.size) {
            wrong.push(format!("size is {} bytes, not {}", received.size, upload.size));
        }
        if received.sha256 != upload.sha256 {
            wrong.push(format!("SHA-256 is {}, not {}", received.sha256, upload.sha256));
        }
        if !wrong.is_empty() {
            self.discard(tenant, &key).await;
            return Err(upload_mismatch(&wrong.join("; ")));
        }
        let input = NewUpload {
            name: upload.name.clone(),
            folder_id: details.folder_id,
            content_type: None,
            tags: details.tags,
            org_id: upload.org_id,
        };
        let ids = (id, details.version_id);
        let Some(item) = self.save_direct(tenant, principal, ids, input, received).await? else {
            // Một lệnh xác nhận khác vừa xong trước.
            return self.existing(tenant, id).await;
        };
        let duplicates = self.duplicates(tenant, &item).await?;
        tracing::info!(media_item_id = %id, size = item.size, "direct media upload completed");
        metrics::counter!("media_items_uploaded_total").increment(1);
        Ok(Confirmed { uploaded: Uploaded { item, duplicates }, created: true })
    }

    /// Item đã tạo bởi lần xác nhận trước.
    async fn existing(&self, tenant: &TenantContext, id: Uuid) -> Result<Confirmed, Error> {
        let item = Entity::find_by_id(id).scoped(tenant).one(tenant.database().write()).await?;
        let item = item.ok_or_else(|| item_not_found(id))?;
        let duplicates = self.duplicates(tenant, &item).await?;
        Ok(Confirmed { uploaded: Uploaded { item, duplicates }, created: false })
    }

    /// Dọn lượt quá hạn mà không được xác nhận trên `database` (mọi khách): xoá nội dung và dòng;
    /// xoá dòng của lượt đã xác nhận cũ. Chạy lại được. Trả số lượt bỏ dở đã dọn.
    pub(crate) async fn purge_uploads(
        &self,
        databases: &DatabaseRegistry,
        database: &autoxide_kernel::database::Database,
        now: DateTime<Utc>,
    ) -> Result<u64, Error> {
        let mut removed = 0;
        let expired =
            self.uploads.expired(database.write(), now - PURGE_AFTER, PURGE_BATCH).await?;
        for upload in expired {
            let Ok(tenant) = databases.tenant(upload.tenant_id).await else {
                continue;
            };
            self.store(&tenant).delete(&upload.storage_key).await?;
            self.uploads.delete(tenant.database().write(), upload.tenant_id, upload.id).await?;
            removed += 1;
        }
        self.uploads.forget_completed(database.write(), now - KEEP_COMPLETED).await?;
        if removed > 0 {
            tracing::info!(uploads = removed, "abandoned direct media uploads removed");
        }
        Ok(removed)
    }
}

pub(crate) struct PurgeUploads {
    pub service: Arc<MediaService>,
    pub databases: DatabaseRegistry,
}

#[async_trait::async_trait]
impl JobHandler for PurgeUploads {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.service.purge_uploads(&self.databases, &job.database, Utc::now()).await.map(|_| ())
    }
}

fn direct_upload_unavailable() -> Error {
    Error::conflict(
        "media.direct_upload_unavailable",
        "Files are not stored where they can be uploaded straight to. Upload through \
         `POST /api/v1/media` instead.",
    )
}

fn upload_not_found(id: Uuid) -> Error {
    Error::not_found("media.upload_not_found", "There is no upload `{id}` of yours.")
        .with_param("id", id)
}

fn upload_expired(id: Uuid) -> Error {
    Error::conflict(
        "media.upload_expired",
        "Upload `{id}` has expired. Start a new one with `POST /api/v1/media/uploads`.",
    )
    .with_param("id", id)
}

fn upload_incomplete(id: Uuid) -> Error {
    Error::conflict(
        "media.upload_incomplete",
        "The file of upload `{id}` is not in storage yet. Send it to the upload link, then call \
         this again.",
    )
    .with_param("id", id)
}

fn upload_mismatch(detail: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "media.upload_mismatch",
        "The uploaded file does not match what was declared: {detail}. It was removed; send the \
         right file to the same link and complete again.",
    )
    .with_param("detail", detail)
}
