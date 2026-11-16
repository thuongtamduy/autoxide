//! Service của file: tải lên, xem, tải xuống, xoá. Handler REST (và tool MCP ở bước
//! sau) chỉ gọi vào đây.
//!
//! Nội dung nằm ở nơi lưu (`Storage`), thông tin nằm trong bảng `files_file`. Thứ tự
//! ghi để không bao giờ có dòng trỏ tới nội dung không tồn tại:
//!
//! - tải lên: ghi nội dung trước, rồi dòng (kèm event) trong một transaction; ghi dòng
//!   hỏng thì xoá nội dung vừa ghi;
//! - xoá: xoá dòng (kèm event) trước, rồi nội dung; xoá nội dung hỏng thì chỉ còn một
//!   file mồ côi ở nơi lưu (ghi log), người dùng không thấy gì sai.
//!
//! Theo khách (bước 33): mọi hàm nhận `&TenantContext`; dòng ở database của khách, lọc `.scoped`;
//! nội dung ở nơi lưu của khách (`TenantStorage`: khoá có tiền tố khách, D-259).

use std::sync::{Arc, Mutex};

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Outbox};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::storage::{ByteStream, Storage, TenantStorage};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, SubsecRound, Utc};
use futures_util::StreamExt as _;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::{Limits, normalize_type};
use crate::entity::{self, Column, Entity};
use crate::events;
use crate::permissions::{FILES_MANAGE, FILES_UPLOAD};

/// Độ dài tối đa của tên file, theo ký tự.
const NAME_MAX_LENGTH: usize = 255;

/// Thông tin của một file.
pub(crate) type FileRecord = entity::Model;

/// Thông tin của một file vừa ghi nội dung, chờ lưu dòng.
struct NewFile {
    id: Uuid,
    name: String,
    content_type: String,
    size: u64,
    sha256: String,
}

/// Một file đang được tải xuống.
pub(crate) struct Download {
    pub file: FileRecord,
    pub content: ByteStream,
}

// Service không giữ database: database và nơi lưu là của khách, đi theo `TenantContext`.
pub(crate) struct FileService {
    storage: Arc<dyn Storage>,
    outbox: Outbox,
    limits: Limits,
}

impl FileService {
    pub(crate) fn new(storage: Arc<dyn Storage>, outbox: Outbox, limits: Limits) -> Self {
        FileService { storage, outbox, limits }
    }

    pub(crate) fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Nơi lưu chung (mọi khách); chỉ job dọn mồ côi dùng trực tiếp.
    pub(crate) fn storage(&self) -> &Arc<dyn Storage> {
        &self.storage
    }

    /// Nơi lưu của khách `tenant`.
    pub(crate) fn store(&self, tenant: &TenantContext) -> TenantStorage {
        TenantStorage::new(self.storage.clone(), tenant)
    }

    /// Tải lên `content` với tên `name` và loại `content_type`; người tải là chủ file.
    /// Tên và loại được kiểm TRƯỚC khi đọc nội dung.
    pub(crate) async fn upload(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        name: &str,
        content_type: Option<&str>,
        content: ByteStream,
    ) -> Result<FileRecord, Error> {
        principal.require(&FILES_UPLOAD)?;
        let name = check_name(name)?;
        let content_type = self.check_type(content_type)?;

        let id = Uuid::now_v7();
        let key = entity::storage_key(id);
        let hasher = Arc::new(Mutex::new(Sha256::new()));
        let content = self.measure(content, hasher.clone());
        let store = self.store(tenant);
        let size = store.put(&key, content).await?;
        let sha256 = {
            let hasher = hasher.lock().map_err(|_| Error::internal("hasher lock"))?;
            hex(&hasher.clone().finalize())
        };

        let saved =
            self.save(tenant, principal, NewFile { id, name, content_type, size, sha256 }).await;
        if saved.is_err() {
            // Không để nội dung không ai trỏ tới nằm lại ở nơi lưu.
            if let Err(error) = store.delete(&key).await {
                tracing::warn!(file_id = %id, error = %error, "cannot remove an unsaved upload");
            }
        }
        let file = saved?;
        tracing::info!(file_id = %file.id, size = file.size, "file uploaded");
        metrics::counter!("files_uploaded_total").increment(1);
        Ok(file)
    }

    /// Thông tin file `id`, nếu người gọi được xem.
    pub(crate) async fn get(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<FileRecord, Error> {
        let file = Entity::find_by_id(id).scoped(tenant).one(tenant.database().read()).await?;
        visible(principal, file, id)
    }

    /// Một trang file của người gọi, mới tải trước (mặc định).
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<FileRecord>, Error> {
        principal.require(&FILES_UPLOAD)?;
        let select = Entity::find().filter(Column::OwnerId.eq(principal.user_id())).scoped(tenant);
        fetch_page(select, Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Thông tin và nội dung của file `id`. `unchanged(etag)` trả `true` (client đã có đúng bản
    /// này, `If-None-Match`) thì trả `None` mà KHÔNG mở nội dung — với S3, mở là một lần GET tính
    /// tiền rồi bỏ dở (D-157).
    pub(crate) async fn download(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        unchanged: impl FnOnce(&str) -> bool,
    ) -> Result<Option<Download>, Error> {
        let file = self.get(tenant, principal, id).await?;
        if unchanged(&file.etag()) {
            return Ok(None);
        }
        let stored = self.store(tenant).get(&file.storage_key()).await?.ok_or_else(|| {
            // Có dòng mà không có nội dung: nơi lưu bị đổi từ bên ngoài.
            Error::internal(format!("the content of file {id} is missing from storage"))
        })?;
        Ok(Some(Download { file, content: stored.content }))
    }

    /// Xoá file `id`.
    pub(crate) async fn delete(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let file = self.get(tenant, principal, id).await?;
        let transaction = tenant.database().begin_write().await?;
        let deleted = Entity::delete_by_id(id).scoped(tenant).exec(&transaction).await?;
        if deleted.rows_affected == 0 {
            // Một request khác vừa xoá trước.
            transaction.rollback().await?;
            return Err(file_not_found(id));
        }
        let event = events::FileDeleted { before: details(&file) };
        let actor = Actor::from(principal);
        self.outbox.record(&transaction, tenant, actor, events::file_subject(id), &event).await?;
        transaction.commit().await?;
        if let Err(error) = self.store(tenant).delete(&file.storage_key()).await {
            tracing::warn!(file_id = %id, error = %error, "file deleted but its content remains");
        }
        tracing::info!(file_id = %id, "file deleted");
        Ok(())
    }

    fn check_type(&self, content_type: Option<&str>) -> Result<String, Error> {
        let content_type = match content_type {
            None => "application/octet-stream".to_owned(),
            Some(raw) => normalize_type(raw).ok_or_else(|| {
                Error::bad_request(
                    "files.invalid_content_type",
                    "The `Content-Type` header must be a media type like `application/pdf`.",
                )
            })?,
        };
        if !self.limits.allows(&content_type) {
            let allowed = self.limits.allowed_types.clone().unwrap_or_default().join(", ");
            return Err(Error::new(
                ErrorKind::UnsupportedMediaType,
                "files.type_not_allowed",
                "Files of type `{type}` cannot be uploaded here. Allowed types: {allowed}.",
            )
            .with_param("type", &content_type)
            .with_param("allowed", allowed));
        }
        Ok(content_type)
    }

    /// Luồng `content` qua bộ đếm (vượt giới hạn thì dừng bằng lỗi 413) và bộ băm.
    fn measure(&self, content: ByteStream, hasher: Arc<Mutex<Sha256>>) -> ByteStream {
        let max = self.limits.max_bytes;
        let mut seen: u64 = 0;
        content
            .map(move |chunk| {
                let chunk = chunk?;
                seen += chunk.len() as u64;
                if seen > max {
                    return Err(too_large(max));
                }
                hasher.lock().map_err(|_| Error::internal("hasher lock"))?.update(&chunk);
                Ok(chunk)
            })
            .boxed()
    }

    async fn save(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        new: NewFile,
    ) -> Result<FileRecord, Error> {
        let NewFile { id, name, content_type, size, sha256 } = new;
        let transaction = tenant.database().begin_write().await?;
        let file = entity::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            owner_id: Set(principal.user_id()),
            name: Set(name),
            content_type: Set(content_type),
            size: Set(i64::try_from(size).map_err(Error::internal)?),
            sha256: Set(sha256),
            created_at: Set(now()),
        }
        .insert(&transaction)
        .await?;
        let event = events::FileUploaded { after: details(&file) };
        let actor = Actor::from(principal);
        self.outbox.record(&transaction, tenant, actor, events::file_subject(id), &event).await?;
        transaction.commit().await?;
        Ok(file)
    }
}

/// Người gọi xem được file nếu là chủ (và còn `files.upload`) hoặc có `files.manage`.
/// Không được xem thì báo "không có" — không để lộ file của người khác có tồn tại.
fn visible(principal: &Principal, file: Option<FileRecord>, id: Uuid) -> Result<FileRecord, Error> {
    if !principal.has(&FILES_UPLOAD) && !principal.has(&FILES_MANAGE) {
        principal.require(&FILES_UPLOAD)?;
    }
    file.filter(|file| {
        principal.has(&FILES_MANAGE)
            || (file.owner_id == principal.user_id() && principal.has(&FILES_UPLOAD))
    })
    .ok_or_else(|| file_not_found(id))
}

/// Tên file: bỏ khoảng trắng hai đầu; 1 tới 255 ký tự; không có `/`, `\`, ký tự điều khiển
/// hay ký tự định dạng vô hình (tên chỉ để hiển thị và đặt khi tải về, không bao giờ thành
/// đường dẫn).
fn check_name(name: &str) -> Result<String, Error> {
    let name = name.trim();
    let length = name.chars().count();
    let valid = (1..=NAME_MAX_LENGTH).contains(&length)
        && !name.chars().any(|c| c == '/' || c == '\\' || c.is_control() || is_invisible(c));
    if valid {
        return Ok(name.to_owned());
    }
    Err(Error::validation(vec![
        FieldError::new(
            "name",
            "invalid",
            "must be 1 to {name_max_length} characters, without `/`, `\\`, control or invisible \
             formatting characters",
        )
        .with_param("name_max_length", NAME_MAX_LENGTH),
    ]))
}

/// Ký tự định dạng vô hình đổi cách tên hiện ra: đảo chiều chữ (`invoice\u{202E}fdp.exe` hiện
/// thành `invoiceexe.pdf`), chữ rộng 0, gạch nối mềm (D-157).
fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

/// Số file xoá mỗi lượt khi dọn file của một tài khoản đã xoá.
const ERASE_BATCH: u64 = 100;

/// Thông tin mọi file của người dùng (không kèm nội dung), cho bản xuất dữ liệu cá
/// nhân. Nội dung tải từng file bằng `GET /api/v1/files/{id}/content`.
#[async_trait::async_trait]
impl PersonalData for FileService {
    async fn export(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<serde_json::Value, Error> {
        let files = Entity::find()
            .filter(Column::OwnerId.eq(user_id))
            .scoped(tenant)
            .order_by_asc(Column::Id)
            .all(tenant.database().read())
            .await?;
        let files: Vec<serde_json::Value> = files
            .iter()
            .map(|file| {
                json!({
                    "id": file.id,
                    "name": file.name,
                    "content_type": file.content_type,
                    "size": file.size,
                    "sha256": file.sha256,
                    "created_at": file.created_at,
                })
            })
            .collect();
        Ok(serde_json::Value::Array(files))
    }
}

/// Tài khoản bị xoá thì xoá mọi file của người đó, từng lượt: dòng trước, nội dung sau
/// (như [`FileService::delete`]). Nhận lại event thì làm tiếp phần còn lại.
#[async_trait::async_trait]
impl EventHandler for FileService {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(user_id) = deleted_user(event) else {
            return Ok(());
        };
        let tenant = &event.tenant;
        let store = self.store(tenant);
        let mut removed: u64 = 0;
        loop {
            let files = Entity::find()
                .filter(Column::OwnerId.eq(user_id))
                .scoped(tenant)
                .order_by_asc(Column::Id)
                .limit(ERASE_BATCH)
                .all(tenant.database().write())
                .await?;
            if files.is_empty() {
                break;
            }
            let ids: Vec<Uuid> = files.iter().map(|file| file.id).collect();
            Entity::delete_many()
                .filter(Column::Id.is_in(ids))
                .scoped(tenant)
                .exec(tenant.database().write())
                .await?;
            for file in &files {
                if let Err(error) = store.delete(&file.storage_key()).await {
                    tracing::warn!(file_id = %file.id, error = %error, "file deleted but its content remains");
                }
            }
            removed += files.len() as u64;
        }
        tracing::info!(user_id = %user_id, files = removed, "files of a deleted user removed");
        Ok(())
    }
}

fn details(file: &FileRecord) -> serde_json::Value {
    json!({
        "name": file.name,
        "content_type": file.content_type,
        "size": file.size,
        "sha256": file.sha256,
        "owner_id": file.owner_id,
    })
}

pub(crate) fn too_large(max: u64) -> Error {
    Error::new(
        ErrorKind::PayloadTooLarge,
        "files.too_large",
        "The file is larger than {max} bytes, the most this server accepts.",
    )
    .with_param("max", max)
}

fn file_not_found(id: Uuid) -> Error {
    Error::not_found("files.file_not_found", "File `{id}` does not exist.").with_param("id", id)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Bây giờ, cắt tới micro giây để hai database trả cùng giá trị.
fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::database::Database;
    use autoxide_testkit::TestDatabase;
    use bytes::Bytes;
    use futures_util::TryStreamExt as _;
    use sea_orm::{ConnectionTrait, Statement};

    use crate::testing::{no_limits, service};

    fn uploader(id: Uuid) -> Principal {
        Principal::new(id, ["files.upload".to_owned()])
    }

    fn body(text: &'static str) -> ByteStream {
        futures_util::stream::iter([Ok(Bytes::from_static(text.as_bytes()))]).boxed()
    }

    async fn content(download: Download) -> Vec<u8> {
        let chunks: Vec<Bytes> = download.content.try_collect().await.unwrap();
        chunks.concat()
    }

    async fn events(database: &Database) -> Vec<String> {
        let backend = database.read().get_database_backend();
        let sql = "SELECT name FROM files_events ORDER BY id";
        let rows = database.read().query_all_raw(Statement::from_string(backend, sql)).await;
        rows.unwrap().iter().map(|row| row.try_get("", "name").unwrap()).collect()
    }

    #[tokio::test]
    async fn tai_len_xem_tai_xuong_va_xoa() {
        TestDatabase::run(async |test| {
            let (files, database) = service(test, no_limits()).await;
            let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
            let ada = uploader(Uuid::now_v7());

            let file = files
                .upload(
                    &tenant,
                    &ada,
                    " notes.txt ",
                    Some("Text/Plain; charset=utf-8"),
                    body("hello"),
                )
                .await
                .unwrap();

            assert_eq!(file.name, "notes.txt");
            assert_eq!(file.content_type, "text/plain");
            assert_eq!(file.size, 5);
            // SHA-256 của "hello".
            assert_eq!(
                file.sha256,
                "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
            );
            assert_eq!(files.get(&tenant, &ada, file.id).await.unwrap(), file);
            let page = files
                .list(&tenant, &ada, &PageRequest::first(10, Default::default()))
                .await
                .unwrap();
            assert_eq!(page.data, vec![file.clone()]);
            let download =
                files.download(&tenant, &ada, file.id, |_| false).await.unwrap().unwrap();
            assert_eq!(content(download).await, b"hello");
            // Client đã có đúng bản này: không mở nội dung (D-157).
            let etag = file.etag();
            let known =
                files.download(&tenant, &ada, file.id, |current| current == etag).await.unwrap();
            assert!(known.is_none());

            files.delete(&tenant, &ada, file.id).await.unwrap();
            let gone = files.get(&tenant, &ada, file.id).await.unwrap_err();
            assert_eq!(gone.code(), "files.file_not_found");
            assert!(files.storage.get(&file.storage_key()).await.unwrap().is_none());
            assert_eq!(events(&database).await, ["files.file_uploaded", "files.file_deleted"]);
        })
        .await
        .unwrap();
    }

    /// Xuất dữ liệu có thông tin file của người dùng; tài khoản bị xoá thì cả dòng lẫn
    /// nội dung file của người đó bị xoá (nhận lại event cũng không sao).
    #[tokio::test]
    async fn xuat_du_lieu_va_xoa_theo_tai_khoan() {
        TestDatabase::run(async |test| {
            let (files, database) = service(test, no_limits()).await;
            let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
            let (ada, bob) = (uploader(Uuid::now_v7()), uploader(Uuid::now_v7()));
            let mine = files.upload(&tenant, &ada, "a.txt", None, body("a")).await.unwrap();
            files.upload(&tenant, &ada, "b.txt", None, body("b")).await.unwrap();
            files.upload(&tenant, &bob, "c.txt", None, body("c")).await.unwrap();

            let exported = files.export(&tenant, ada.user_id()).await.unwrap();
            assert_eq!(exported.as_array().unwrap().len(), 2);
            assert_eq!(exported[0]["name"], "a.txt");

            let deleted = Delivered::new(
                tenant.clone(),
                "auth",
                autoxide_kernel::privacy::USER_DELETED.to_owned(),
                json!({"before": {"id": ada.user_id()}}),
            );
            files.handle(&deleted).await.unwrap();
            files.handle(&deleted).await.unwrap();
            assert_eq!(files.export(&tenant, ada.user_id()).await.unwrap(), json!([]));
            assert!(files.storage.get(&mine.storage_key()).await.unwrap().is_none());
            assert_eq!(
                files.export(&tenant, bob.user_id()).await.unwrap().as_array().unwrap().len(),
                1
            );
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn chi_chu_file_va_nguoi_quan_ly_thay_file() {
        TestDatabase::run(async |test| {
            let (files, database) = service(test, no_limits()).await;
            let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
            let (ada, bob) = (uploader(Uuid::now_v7()), uploader(Uuid::now_v7()));
            let manager = Principal::new(Uuid::now_v7(), ["files.manage".to_owned()]);
            let nobody = Principal::new(ada.user_id(), []);
            let file = files.upload(&tenant, &ada, "a.txt", None, body("a")).await.unwrap();
            assert_eq!(file.content_type, "application/octet-stream");

            // Người khác: như thể file không có.
            assert_eq!(
                files.get(&tenant, &bob, file.id).await.unwrap_err().code(),
                "files.file_not_found"
            );
            assert_eq!(
                files.delete(&tenant, &bob, file.id).await.unwrap_err().code(),
                "files.file_not_found"
            );
            let empty =
                files.list(&tenant, &bob, &PageRequest::first(10, Default::default())).await;
            assert!(empty.unwrap().data.is_empty());
            // Mất permission: kể cả file của mình cũng không.
            let denied = files.get(&tenant, &nobody, file.id).await.unwrap_err();
            assert_eq!(denied.code(), "permission_denied");
            let denied =
                files.upload(&tenant, &nobody, "b.txt", None, body("b")).await.unwrap_err();
            assert_eq!(denied.code(), "permission_denied");

            assert_eq!(files.get(&tenant, &manager, file.id).await.unwrap().id, file.id);
            files.delete(&tenant, &manager, file.id).await.unwrap();
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn gioi_han_kich_thuoc_loai_va_ten() {
        TestDatabase::run(async |test| {
            let limits = Limits { max_bytes: 4, allowed_types: Some(vec!["image/*".into()]) };
            let (files, database) = service(test, limits).await;
            let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
            let ada = uploader(Uuid::now_v7());

            let large =
                files.upload(&tenant, &ada, "a.png", Some("image/png"), body("12345")).await;
            let large = large.unwrap_err();
            assert_eq!(
                (large.kind(), large.code()),
                (ErrorKind::PayloadTooLarge, "files.too_large")
            );
            let exact = files.upload(&tenant, &ada, "b.png", Some("image/png"), body("1234")).await;
            assert_eq!(exact.unwrap().size, 4);

            let pdf =
                files.upload(&tenant, &ada, "a.pdf", Some("application/pdf"), body("1")).await;
            let pdf = pdf.unwrap_err();
            assert_eq!(pdf.kind(), ErrorKind::UnsupportedMediaType);
            assert!(pdf.message().contains("Allowed types: image/*"), "{}", pdf.message());
            let broken =
                files.upload(&tenant, &ada, "a.png", Some("png"), body("1")).await.unwrap_err();
            assert_eq!(broken.code(), "files.invalid_content_type");
            for name in ["", "   ", "a/b.png", "a\\b.png", "a\nb.png", &"x".repeat(256)] {
                let invalid = files.upload(&tenant, &ada, name, Some("image/png"), body("1")).await;
                assert_eq!(invalid.unwrap_err().code(), "validation_failed", "{name:?}");
            }

            // Chỉ file hợp lệ có dòng trong database.
            let page = files.list(&tenant, &ada, &PageRequest::first(10, Default::default())).await;
            assert_eq!(page.unwrap().data.len(), 1);
            assert_eq!(events(&database).await, ["files.file_uploaded"]);
        })
        .await
        .unwrap();
    }

    /// Nhiều request cùng xoá một file: đúng một bên xoá được — trên cả hai database.
    #[tokio::test]
    async fn xoa_dong_thoi_chi_mot_ben_duoc() {
        TestDatabase::run(async |test| {
            let (files, database) = service(test, no_limits()).await;
            let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
            let ada = uploader(Uuid::now_v7());
            let file = files.upload(&tenant, &ada, "a.txt", None, body("a")).await.unwrap();

            let attempts: Vec<_> = (0..5)
                .map(|_| {
                    let (files, ada, tenant) = (files.clone(), ada.clone(), tenant.clone());
                    tokio::spawn(async move { files.delete(&tenant, &ada, file.id).await })
                })
                .collect();
            let mut deleted = 0;
            for attempt in attempts {
                match attempt.await.unwrap() {
                    Ok(()) => deleted += 1,
                    Err(error) => assert_eq!(error.code(), "files.file_not_found"),
                }
            }

            assert_eq!(deleted, 1);
            assert_eq!(events(&database).await, ["files.file_uploaded", "files.file_deleted"]);
        })
        .await
        .unwrap();
    }
}

#[cfg(test)]
mod name_tests {
    use super::check_name;

    #[test]
    fn ten_file_khong_co_ky_tu_vo_hinh() {
        assert_eq!(check_name("  bao cao.pdf ").unwrap(), "bao cao.pdf");
        for bad in ["invoice\u{202E}fdp.exe", "a\u{200B}b.txt", "a/b", "a\u{0007}b", ""] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
    }
}
