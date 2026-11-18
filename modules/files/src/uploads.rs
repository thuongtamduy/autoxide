//! Tải file lớn thẳng lên S3 (bước 40 P4, D-379): cấp link ký sẵn, xác nhận, dọn lượt bỏ dở.
//! Khung chung (bảng `files_uploads`, đọc lại object) ở `autoxide_kernel::upload`; nơi lưu là ổ đĩa
//! thì không có link — client tải qua `POST /api/v1/files` như cũ.
//!
//! - Bắt đầu: kiểm tên, loại, cỡ, SHA-256 (mọi lỗi báo một lần, D-083), giữ chỗ hạn mức dung lượng
//!   của khách theo cỡ khai, ghi lượt, trả link. Khoá lưu là `files/<id>` — chỗ cuối cùng của file.
//! - Xác nhận: đọc lại object (ngoài transaction) — cỡ và SHA-256 phải đúng như khai, không thì xoá
//!   object và báo `files.upload_mismatch` (lượt vẫn còn: PUT lại rồi xác nhận lại được trước hạn);
//!   đạt thì một transaction khoá dòng lượt, tạo bản ghi `files_file` cùng ID + event
//!   `files.file_uploaded`, đánh dấu lượt xong. Gọi lại sau khi xong trả cùng bản ghi.
//! - Loại nội dung: như tải qua máy chủ, `files` lưu loại đã khai (S3 nhận đúng loại đó vì đã ký) và
//!   trả file với `nosniff` + CSP `sandbox`; nhận loại thật bằng magic bytes là việc của `media`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::storage::{PresignedUpload, Storage as _, UploadConstraints};
use autoxide_kernel::tenancy::{DatabaseRegistry, ScopeExt, TenantContext};
use autoxide_kernel::upload::{DirectUpload, NewDirectUpload, UploadStatus, inspect, parse_sha256};
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use uuid::Uuid;

use crate::config::normalize_type;
use crate::entity::{self, Entity};
use crate::events;
use crate::permissions::{FILES_MANAGE, FILES_UPLOAD};
use crate::service::{
    FileRecord, FileService, STORAGE_LIMIT, check_name, details, file_not_found, now,
    storage_reference,
};

/// Job dọn lượt tải lên không được xác nhận (mỗi giờ, trên mọi database).
pub(crate) const PURGE_UPLOADS: &str = "files.purge_expired_uploads";
/// Lượt quá hạn chừng này mới bị dọn: client có thể vừa PUT xong lúc hết hạn.
const PURGE_AFTER: chrono::Duration = chrono::Duration::hours(1);
/// Dòng của lượt đã xác nhận được giữ chừng này (xác nhận gọi lại trả cùng bản ghi).
const KEEP_COMPLETED: chrono::Duration = chrono::Duration::days(7);
/// Số lượt dọn mỗi lần chạy.
const PURGE_BATCH: u64 = 200;

/// Lượt tải lên xin cấp.
#[derive(Debug, Clone)]
pub(crate) struct StartUpload {
    pub name: String,
    pub content_type: String,
    pub size: u64,
    pub sha256: String,
}

/// Lượt đã ghi và link để PUT.
#[derive(Debug, Clone)]
pub(crate) struct Started {
    pub upload: DirectUpload,
    pub link: PresignedUpload,
}

/// Kết quả xác nhận: bản ghi vừa tạo, hay bản ghi của lần xác nhận trước.
#[derive(Debug, Clone)]
pub(crate) enum Completed {
    Created(FileRecord),
    Existing(FileRecord),
}

impl FileService {
    /// Bắt đầu một lượt tải thẳng. Cần `files.upload`.
    pub(crate) async fn start_upload(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: StartUpload,
    ) -> Result<Started, Error> {
        principal.require(&FILES_UPLOAD)?;
        let mut problems: Vec<FieldError> = Vec::new();
        let name = check_name(&input.name)
            .map_err(|error| problems.extend(error.fields().iter().cloned()))
            .ok();
        let content_type = match normalize_type(&input.content_type) {
            None => {
                problems.push(FieldError::new(
                    "content_type",
                    "invalid",
                    "must be a media type like `application/pdf`",
                ));
                None
            }
            Some(kind) if !self.limits().allows(&kind) => {
                let allowed = self.limits().allowed_types.clone().unwrap_or_default().join(", ");
                problems.push(
                    FieldError::new(
                        "content_type",
                        "not_allowed",
                        "files of this type cannot be uploaded here; allowed types: {allowed}",
                    )
                    .with_param("allowed", allowed),
                );
                None
            }
            Some(kind) => Some(kind),
        };
        let max = self.direct.max_bytes;
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
        let (Some(name), Some(content_type), Some(digest), true) =
            (name, content_type, digest, problems.is_empty())
        else {
            return Err(Error::validation(problems));
        };

        let id = Uuid::now_v7();
        let key = entity::storage_key(id);
        let constraints = UploadConstraints::new(input.size, content_type.clone(), digest);
        let store = self.store(tenant);
        let link = store
            .presign_upload(&key, &constraints, self.direct.ttl)
            .await?
            .ok_or_else(direct_upload_unavailable)?;

        // Hạn mức dung lượng (bước 35): giữ chỗ theo cỡ khai ngay khi cấp link, xác nhận khi xong,
        // trả lại khi lượt bị dọn.
        let reference = storage_reference(id);
        let amount = i64::try_from(input.size).map_err(Error::internal)?;
        self.entitlements.reserve(tenant.id(), &STORAGE_LIMIT, amount, &reference).await?;
        let new = NewDirectUpload {
            id,
            owner_id: principal.user_id(),
            org_id: None,
            storage_key: key,
            name,
            content_type,
            size: input.size,
            sha256: autoxide_kernel::upload::hex(&digest),
            details: serde_json::Value::Null,
            expires_at: link.expires_at,
        };
        let upload = match self.uploads.insert(tenant.database().write(), tenant, new).await {
            Ok(upload) => upload,
            Err(error) => {
                self.entitlements.release(tenant.id(), &STORAGE_LIMIT, &reference).await?;
                return Err(error);
            }
        };
        tracing::info!(upload_id = %id, size = input.size, "direct upload started");
        metrics::counter!("files_direct_uploads_started_total").increment(1);
        Ok(Started { upload, link })
    }

    /// Xác nhận lượt `id`: kiểm object rồi tạo bản ghi. Cần `files.upload`; chỉ người bắt đầu lượt
    /// (hay người có `files.manage`) thấy lượt.
    pub(crate) async fn complete_upload(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Completed, Error> {
        principal.require(&FILES_UPLOAD)?;
        // Kết nối ghi: lượt vừa ghi chưa tới read replica vẫn thấy.
        let upload = self
            .uploads
            .find(tenant.database().write(), tenant, id)
            .await?
            .filter(|upload| upload.owner_id == principal.user_id() || principal.has(&FILES_MANAGE))
            .ok_or_else(|| upload_not_found(id))?;
        if upload.status == UploadStatus::Completed {
            return self.existing(tenant, principal, id).await.map(Completed::Existing);
        }
        if Utc::now() > upload.expires_at {
            return Err(upload_expired(id));
        }
        let store = self.store(tenant);
        let inspected =
            inspect(&store, &upload.storage_key, 0).await?.ok_or_else(|| upload_incomplete(id))?;
        let mut wrong = Vec::new();
        if inspected.size != upload.size {
            wrong.push(format!("size is {} bytes, not {}", inspected.size, upload.size));
        }
        if inspected.sha256 != upload.sha256 {
            wrong.push(format!("SHA-256 is {}, not {}", inspected.sha256, upload.sha256));
        }
        if !wrong.is_empty() {
            // Không giữ nội dung sai: client PUT lại đúng file rồi xác nhận lại (trước hạn).
            store.delete(&upload.storage_key).await?;
            tracing::info!(upload_id = %id, "direct upload did not match; content removed");
            return Err(upload_mismatch(&wrong.join("; ")));
        }
        match self.save_upload(tenant, principal, &upload).await? {
            Some(file) => {
                let reference = storage_reference(id);
                self.entitlements.confirm(tenant.id(), &STORAGE_LIMIT, &reference).await?;
                tracing::info!(file_id = %id, size = file.size, "direct upload completed");
                metrics::counter!("files_uploaded_total").increment(1);
                Ok(Completed::Created(file))
            }
            // Một lệnh xác nhận khác vừa xong trước.
            None => self.existing(tenant, principal, id).await.map(Completed::Existing),
        }
    }

    /// Tạo bản ghi của lượt `upload` và đánh dấu lượt xong, trong một transaction. `None`: lượt đã
    /// được xác nhận bởi lệnh khác.
    async fn save_upload(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        upload: &DirectUpload,
    ) -> Result<Option<FileRecord>, Error> {
        let transaction = tenant.database().begin_write().await?;
        let locked = self.uploads.lock(&transaction, tenant, upload.id).await?;
        if !locked.is_some_and(|locked| locked.status == UploadStatus::Pending) {
            transaction.rollback().await?;
            return Ok(None);
        }
        let file = entity::ActiveModel {
            id: Set(upload.id),
            tenant_id: Set(tenant.id().as_uuid()),
            owner_id: Set(upload.owner_id),
            name: Set(upload.name.clone()),
            content_type: Set(upload.content_type.clone()),
            size: Set(i64::try_from(upload.size).map_err(Error::internal)?),
            sha256: Set(upload.sha256.clone()),
            created_at: Set(now()),
        }
        .insert(&transaction)
        .await?;
        self.uploads.complete(&transaction, tenant, upload.id).await?;
        let event = events::FileUploaded { after: details(&file) };
        let actor = Actor::from(principal);
        let subject = events::file_subject(upload.id);
        self.outbox.record(&transaction, tenant, actor, subject, &event).await?;
        transaction.commit().await?;
        Ok(Some(file))
    }

    /// Bản ghi đã tạo bởi lần xác nhận trước (file có thể đã bị xoá từ đó → 404 của file).
    async fn existing(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<FileRecord, Error> {
        let file = Entity::find_by_id(id).scoped(tenant).one(tenant.database().write()).await?;
        let file = file.ok_or_else(|| file_not_found(id))?;
        self.get(tenant, principal, file.id).await
    }

    /// Dọn lượt quá hạn mà không được xác nhận trên `database` (mọi khách): xoá object, dòng, trả
    /// hạn mức; xoá dòng của lượt đã xác nhận cũ. Chạy lại được. Trả số lượt bỏ dở đã dọn.
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
                // Khách không còn trong sổ: để nguyên, không đoán.
                continue;
            };
            self.store(&tenant).delete(&upload.storage_key).await?;
            self.uploads.delete(tenant.database().write(), upload.tenant_id, upload.id).await?;
            let reference = storage_reference(upload.id);
            self.entitlements.release(upload.tenant_id, &STORAGE_LIMIT, &reference).await?;
            removed += 1;
        }
        self.uploads.forget_completed(database.write(), now - KEEP_COMPLETED).await?;
        if removed > 0 {
            tracing::info!(uploads = removed, "abandoned direct uploads removed");
        }
        Ok(removed)
    }
}

pub(crate) struct PurgeUploads {
    pub service: Arc<FileService>,
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
        "files.direct_upload_unavailable",
        "Files are not stored where they can be uploaded straight to. Upload through \
         `POST /api/v1/files` instead.",
    )
}

pub(crate) fn upload_not_found(id: Uuid) -> Error {
    Error::not_found("files.upload_not_found", "There is no upload `{id}` of yours.")
        .with_param("id", id)
}

fn upload_expired(id: Uuid) -> Error {
    Error::conflict(
        "files.upload_expired",
        "Upload `{id}` has expired. Start a new one with `POST /api/v1/files/uploads`.",
    )
    .with_param("id", id)
}

fn upload_incomplete(id: Uuid) -> Error {
    Error::conflict(
        "files.upload_incomplete",
        "The file of upload `{id}` is not in storage yet. Send it to the upload link, then call \
         this again.",
    )
    .with_param("id", id)
}

fn upload_mismatch(detail: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "files.upload_mismatch",
        "The uploaded file does not match what was declared: {detail}. It was removed; send the \
         right file to the same link and complete again.",
    )
    .with_param("detail", detail)
}

#[cfg(test)]
mod tests;
