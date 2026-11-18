//! Tải file lớn thẳng lên nơi lưu (bước 40 P4, D-378), dùng chung cho mọi module nhận file
//! (`files`, `media`).
//!
//! 1. Module ghi một **lượt tải lên** vào bảng `<module>_uploads` ([`uploads_migration`]) — cỡ, loại,
//!    SHA-256 client khai, khoá lưu, hạn — rồi xin nơi lưu một link ký sẵn
//!    ([`Storage::presign_upload`](crate::storage::Storage::presign_upload)); nơi lưu là ổ đĩa thì
//!    không có link, client tải qua máy chủ như cũ.
//! 2. Client gửi file thẳng tới link (không qua máy chủ).
//! 3. Client gọi lệnh xác nhận: module đọc lại object ([`inspect`]: cỡ, SHA-256, phần đầu để nhận
//!    loại thật), NGOÀI transaction; đạt thì trong một transaction khoá dòng lượt tải
//!    ([`DirectUploads::lock`]), tạo bản ghi của mình và đánh dấu xong ([`DirectUploads::complete`]) —
//!    hai lệnh xác nhận đồng thời chỉ tạo một bản ghi.
//! 4. Lượt không bao giờ được xác nhận: job dọn của module xoá object và dòng
//!    ([`DirectUploads::expired`]).
//!
//! ID của lượt cũng là ID của bản ghi sẽ tạo: khoá lưu đặt đúng chỗ cuối cùng, không phải chép.

use std::sync::Arc;

use chrono::{DateTime, SubsecRound, Utc};
use futures_util::StreamExt as _;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Order, Query, SelectStatement};
use sea_orm::{ConnectionTrait, QueryResult};
use sea_orm_migration::prelude::{
    DbErr, Index, MigrationName, MigrationTrait, SchemaManager, Table,
};
use sea_orm_migration::schema;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::error::Error;
use crate::storage::Storage;
use crate::tenancy::{TenantContext, TenantId};

/// Cỡ lớn nhất của một lần `PUT` lên S3 (5 GiB); lớn hơn cần upload nhiều phần (ngoài phạm vi).
pub const MAX_SINGLE_PUT_BYTES: u64 = 5 * 1024 * 1024 * 1024;

/// Trạng thái của một lượt tải lên.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadStatus {
    /// Đã cấp link, chưa xác nhận.
    Pending,
    /// Đã xác nhận: bản ghi mang cùng ID đã được tạo.
    Completed,
}

impl UploadStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            UploadStatus::Pending => "pending",
            UploadStatus::Completed => "completed",
        }
    }
}

/// Một lượt tải lên đã ghi.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct DirectUpload {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub org_id: Option<Uuid>,
    pub owner_id: Uuid,
    /// Khoá ở nơi lưu của khách (chưa có tiền tố khách).
    pub storage_key: String,
    pub name: String,
    pub content_type: String,
    pub size: u64,
    /// SHA-256 khai, 64 chữ số hex thường.
    pub sha256: String,
    /// Dữ liệu riêng của module (vd thư mục, nhãn), JSON.
    pub details: serde_json::Value,
    pub status: UploadStatus,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Lượt tải lên mới.
#[derive(Debug, Clone)]
pub struct NewDirectUpload {
    /// ID của lượt, cũng là ID của bản ghi sẽ tạo.
    pub id: Uuid,
    pub owner_id: Uuid,
    /// Nút tổ chức sở hữu bản ghi sẽ tạo.
    pub org_id: Option<Uuid>,
    /// Khoá ở nơi lưu của khách.
    pub storage_key: String,
    pub name: String,
    pub content_type: String,
    pub size: u64,
    /// SHA-256 khai, 64 chữ số hex thường.
    pub sha256: String,
    /// Dữ liệu riêng của module, đọc lại lúc xác nhận.
    pub details: serde_json::Value,
    pub expires_at: DateTime<Utc>,
}

/// Bảng lượt tải lên của một module. Lấy bằng [`DirectUploads::new`]; clone rẻ.
#[derive(Debug, Clone)]
pub struct DirectUploads {
    table: Arc<str>,
}

const COLUMNS: [&str; 14] = [
    "id",
    "tenant_id",
    "org_id",
    "owner_id",
    "storage_key",
    "name",
    "content_type",
    "size",
    "sha256",
    "details",
    "status",
    "created_at",
    "expires_at",
    "completed_at",
];

impl DirectUploads {
    /// Bảng `<module>_uploads` của module `module` (tạo bằng [`uploads_migration`]).
    pub fn new(module: &str) -> Self {
        DirectUploads { table: table_name(module).into() }
    }

    /// Ghi lượt mới cho khách `tenant`.
    pub async fn insert<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        new: NewDirectUpload,
    ) -> Result<DirectUpload, Error> {
        let size = i64::try_from(new.size).map_err(Error::internal)?;
        let now = Utc::now().trunc_subsecs(6);
        let expires_at = new.expires_at.trunc_subsecs(6);
        let details = serde_json::to_string(&new.details).map_err(Error::internal)?;
        let mut insert = Query::insert();
        insert
            .into_table(Alias::new(&*self.table))
            .columns(COLUMNS.map(Alias::new))
            .values([
                new.id.into(),
                tenant.id().as_uuid().into(),
                new.org_id.into(),
                new.owner_id.into(),
                new.storage_key.clone().into(),
                new.name.clone().into(),
                new.content_type.clone().into(),
                size.into(),
                new.sha256.clone().into(),
                details.into(),
                UploadStatus::Pending.as_str().into(),
                now.into(),
                expires_at.into(),
                Option::<DateTime<Utc>>::None.into(),
            ])
            .map_err(|error| Error::internal(format!("upload insert: {error}")))?;
        connection.execute(&insert).await?;
        Ok(DirectUpload {
            id: new.id,
            tenant_id: tenant.id(),
            org_id: new.org_id,
            owner_id: new.owner_id,
            storage_key: new.storage_key,
            name: new.name,
            content_type: new.content_type,
            size: new.size,
            sha256: new.sha256,
            details: new.details,
            status: UploadStatus::Pending,
            created_at: now,
            expires_at,
            completed_at: None,
        })
    }

    /// Lượt `id` của khách `tenant`, nếu có.
    pub async fn find<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Option<DirectUpload>, Error> {
        let select = self.select_one(tenant, id);
        connection.query_one(&select).await?.map(|row| parse(&row)).transpose()
    }

    /// Như [`find`](Self::find), và khoá dòng tới hết `transaction` (`FOR UPDATE` trên Postgres;
    /// SQLite vốn chỉ một transaction ghi một lúc): lệnh xác nhận thứ hai chờ lệnh đầu xong rồi thấy
    /// lượt đã `completed`.
    pub async fn lock<C: ConnectionTrait>(
        &self,
        transaction: &C,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Option<DirectUpload>, Error> {
        let mut select = self.select_one(tenant, id);
        select.lock_exclusive();
        transaction.query_one(&select).await?.map(|row| parse(&row)).transpose()
    }

    /// Đánh dấu lượt `id` đã xác nhận (gọi trong transaction tạo bản ghi, sau [`lock`](Self::lock)).
    /// `false`: lượt không còn ở trạng thái `pending`.
    pub async fn complete<C: ConnectionTrait>(
        &self,
        transaction: &C,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<bool, Error> {
        let now = Utc::now().trunc_subsecs(6);
        let mut update = Query::update();
        update
            .table(Alias::new(&*self.table))
            .value(Alias::new("status"), UploadStatus::Completed.as_str())
            .value(Alias::new("completed_at"), now)
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("id")).eq(id))
            .and_where(Expr::col(Alias::new("status")).eq(UploadStatus::Pending.as_str()));
        Ok(transaction.execute(&update).await?.rows_affected() == 1)
    }

    /// Xoá dòng của lượt `id` (khách `tenant_id`).
    pub async fn delete<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant_id: TenantId,
        id: Uuid,
    ) -> Result<(), Error> {
        let mut delete = Query::delete();
        delete
            .from_table(Alias::new(&*self.table))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant_id.as_uuid()))
            .and_where(Expr::col(Alias::new("id")).eq(id));
        connection.execute(&delete).await?;
        Ok(())
    }

    /// Tối đa `limit` lượt (mọi khách trên database này) còn `pending` mà hết hạn trước `before`,
    /// cũ trước — cho job dọn.
    pub async fn expired<C: ConnectionTrait>(
        &self,
        connection: &C,
        before: DateTime<Utc>,
        limit: u64,
    ) -> Result<Vec<DirectUpload>, Error> {
        let mut select = Query::select();
        select
            .columns(COLUMNS.map(Alias::new))
            .from(Alias::new(&*self.table))
            .and_where(Expr::col(Alias::new("status")).eq(UploadStatus::Pending.as_str()))
            .and_where(Expr::col(Alias::new("expires_at")).lt(before))
            .order_by(Alias::new("expires_at"), Order::Asc)
            .limit(limit);
        let rows = connection.query_all(&select).await?;
        rows.iter().map(parse).collect()
    }

    /// Xoá dòng của các lượt đã xác nhận trước `before` (bản ghi đã có, dòng chỉ còn để lệnh xác
    /// nhận gọi lại trả cùng kết quả). Trả số dòng đã xoá.
    pub async fn forget_completed<C: ConnectionTrait>(
        &self,
        connection: &C,
        before: DateTime<Utc>,
    ) -> Result<u64, Error> {
        let mut delete = Query::delete();
        delete
            .from_table(Alias::new(&*self.table))
            .and_where(Expr::col(Alias::new("status")).eq(UploadStatus::Completed.as_str()))
            // Theo `expires_at` (có index): lượt xác nhận trước hạn của nó.
            .and_where(Expr::col(Alias::new("expires_at")).lt(before));
        Ok(connection.execute(&delete).await?.rows_affected())
    }

    fn select_one(&self, tenant: &TenantContext, id: Uuid) -> SelectStatement {
        let mut select = Query::select();
        select
            .columns(COLUMNS.map(Alias::new))
            .from(Alias::new(&*self.table))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("id")).eq(id));
        select
    }
}

fn parse(row: &QueryResult) -> Result<DirectUpload, Error> {
    let size: i64 = row.try_get("", "size")?;
    let details: String = row.try_get("", "details")?;
    let status: String = row.try_get("", "status")?;
    let tenant: Uuid = row.try_get("", "tenant_id")?;
    Ok(DirectUpload {
        id: row.try_get("", "id")?,
        tenant_id: TenantId::from_uuid(tenant),
        org_id: row.try_get("", "org_id")?,
        owner_id: row.try_get("", "owner_id")?,
        storage_key: row.try_get("", "storage_key")?,
        name: row.try_get("", "name")?,
        content_type: row.try_get("", "content_type")?,
        size: u64::try_from(size).map_err(Error::internal)?,
        sha256: row.try_get("", "sha256")?,
        details: serde_json::from_str(&details).unwrap_or(serde_json::Value::Null),
        status: if status == UploadStatus::Completed.as_str() {
            UploadStatus::Completed
        } else {
            UploadStatus::Pending
        },
        created_at: row.try_get("", "created_at")?,
        expires_at: row.try_get("", "expires_at")?,
        completed_at: row.try_get("", "completed_at")?,
    })
}

/// Where and how to send a file straight to storage: send it with exactly this method, URL and
/// headers before `expires_at`, then complete the upload with its `id`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct UploadLink {
    /// Id of the upload; also the id of the record it becomes.
    pub id: Uuid,
    /// HTTP method to send the file with.
    #[schema(example = "PUT")]
    pub method: String,
    /// Signed URL of the storage to send the file to (not this server).
    pub url: String,
    /// Headers to send with the file, exactly as given (the signature covers them).
    #[schema(example = json!({"content-type": "video/mp4", "content-length": "734003200"}))]
    pub headers: std::collections::BTreeMap<String, String>,
    /// The URL stops working at this time.
    pub expires_at: DateTime<Utc>,
}

impl UploadLink {
    /// Link của lượt `id`.
    pub fn new(id: Uuid, link: crate::storage::PresignedUpload) -> Self {
        UploadLink {
            id,
            method: link.method,
            url: link.url,
            headers: link.headers.into_iter().collect(),
            expires_at: link.expires_at,
        }
    }
}

/// Object đã đọc lại để kiểm.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Inspected {
    /// Số byte thật.
    pub size: u64,
    /// SHA-256 của nội dung, 64 chữ số hex thường.
    pub sha256: String,
    /// Tối đa `head_bytes` byte đầu (nhận loại thật bằng magic bytes).
    pub head: Vec<u8>,
}

/// Đọc hết object ở `key` để biết cỡ và SHA-256 thật (không tin nơi lưu: dịch vụ tương thích S3
/// không phải cái nào cũng kiểm checksum đã ký), giữ `head_bytes` byte đầu. `None`: chưa có object.
/// Không gọi trong transaction ghi (đây là gọi mạng).
pub async fn inspect(
    storage: &dyn Storage,
    key: &str,
    head_bytes: usize,
) -> Result<Option<Inspected>, Error> {
    let Some(file) = storage.get(key).await? else {
        return Ok(None);
    };
    let mut content = file.content;
    let mut hasher = Sha256::new();
    let mut size: u64 = 0;
    let mut head = Vec::new();
    while let Some(chunk) = content.next().await {
        let chunk = chunk?;
        size += chunk.len() as u64;
        hasher.update(&chunk);
        let room = head_bytes.saturating_sub(head.len());
        head.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }
    Ok(Some(Inspected { size, sha256: hex(&hasher.finalize()), head }))
}

/// 64 chữ số hex (hoa hay thường) thành 32 byte; sai dạng → `None`.
pub fn parse_sha256(text: &str) -> Option<[u8; 32]> {
    let text = text.trim();
    if text.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let pair = text.get(index * 2..index * 2 + 2)?;
        *byte = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(bytes)
}

/// Byte thành chữ số hex thường.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Tên bảng lượt tải lên của module `module`.
fn table_name(module: &str) -> String {
    format!("{module}_uploads")
}

/// Migration tạo bảng `<module>_uploads`. Module nhận tải thẳng thêm nó vào danh sách migration của
/// mình, như [`jobs_migration`](crate::job::jobs_migration).
pub fn uploads_migration(name: &'static str, module: &'static str) -> Box<dyn MigrationTrait> {
    Box::new(UploadsMigration { name, module })
}

struct UploadsMigration {
    name: &'static str,
    module: &'static str,
}

impl MigrationName for UploadsMigration {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for UploadsMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = table_name(self.module);
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(&table))
                    // `tenant_id` đứng đầu khoá chính: mọi truy vấn của một khách đi theo index (bước 33).
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("id"))
                    .col(schema::uuid_null("org_id"))
                    .col(schema::uuid("owner_id"))
                    .col(schema::string_len("storage_key", 512))
                    .col(schema::string_len("name", 255))
                    .col(schema::string_len("content_type", 255))
                    .col(schema::big_integer("size"))
                    .col(schema::string_len("sha256", 64))
                    // JSON dạng chữ: chạy giống nhau trên SQLite và Postgres.
                    .col(schema::text("details"))
                    .col(schema::string_len("status", 16))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::timestamp_with_time_zone_null("completed_at"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await?;
        // Job dọn quét mọi khách: lượt `pending` hết hạn, lượt `completed` cũ.
        manager
            .create_index(
                Index::create()
                    .name(format!("{table}_sweep_idx"))
                    .table(Alias::new(&table))
                    .col("status")
                    .col("expires_at")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = table_name(self.module);
        manager.drop_table(Table::drop().table(Alias::new(&table)).to_owned()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_sha256_hex() {
        let text = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let bytes = parse_sha256(text).unwrap();
        assert_eq!(hex(&bytes), text);
        assert_eq!(parse_sha256(&text.to_uppercase()), Some(bytes));
        assert!(parse_sha256("abc").is_none());
        assert!(parse_sha256(&"zz".repeat(32)).is_none());
    }
}
