//! Uploading large files straight to the store (step 40 P4, D-379), shared by every module that accepts files
//! (`files`, `media`).
//!
//! 1. The module records an **upload attempt** in the `<module>_uploads` table ([`uploads_migration`]):
//!    size, type, the SHA-256 the client declared, storage key, expiry; then asks the store for a presigned
//!    link ([`Storage::presign_upload`](crate::storage::Storage::presign_upload)); if the store is a disk
//!    there is no link, and the client uploads through the server as before.
//! 2. The client sends the file straight to the link (not through the server).
//! 3. The client calls the confirm command: the module rereads the object ([`inspect`]: size, SHA-256, the
//!    first bytes to detect the real type), OUTSIDE a transaction; if it passes then, in one transaction,
//!    it locks the upload row ([`DirectUploads::lock`]), creates its own record and marks it done
//!    ([`DirectUploads::complete`]); two concurrent confirm commands create only one record.
//! 4. An attempt that is never confirmed: the module's cleanup job deletes the object and the row
//!    ([`DirectUploads::expired`]).
//!
//! The attempt's ID is also the ID of the record to be created: the storage key is placed right at its final
//! location, with no copying.

use crate::time::Timestamp;
use std::sync::Arc;

use crate::migration::prelude::{
    DbErr, Index, MigrationName, MigrationTrait, SchemaManager, Table,
};
use crate::migration::schema;
use futures_util::StreamExt as _;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Order, Query, SelectStatement};
use sea_orm::{ConnectionTrait, QueryResult};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::error::Error;
use crate::storage::Storage;
use crate::tenancy::{TenantContext, TenantId};

/// The largest size of one `PUT` to S3 (5 GiB); larger needs a multipart upload (out of scope).
pub const MAX_SINGLE_PUT_BYTES: u64 = 5 * 1024 * 1024 * 1024;

/// State of an upload attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadStatus {
    /// A link was issued, not yet confirmed.
    Pending,
    /// Confirmed: the record with the same ID has been created.
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

/// An upload attempt that has been recorded.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct DirectUpload {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub org_id: Option<Uuid>,
    pub owner_id: Uuid,
    /// Key in the tenant's store (without the tenant prefix yet).
    pub storage_key: String,
    pub name: String,
    pub content_type: String,
    pub size: u64,
    /// Declared SHA-256, 64 lowercase hex digits.
    pub sha256: String,
    /// The module's own data (e.g. folder, label), JSON.
    pub details: serde_json::Value,
    pub status: UploadStatus,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
    pub completed_at: Option<Timestamp>,
}

/// A new upload attempt.
#[derive(Debug, Clone)]
pub struct NewDirectUpload {
    /// The attempt's ID, also the ID of the record to be created.
    pub id: Uuid,
    pub owner_id: Uuid,
    /// The organization node that owns the record to be created.
    pub org_id: Option<Uuid>,
    /// Key in the tenant's store.
    pub storage_key: String,
    pub name: String,
    pub content_type: String,
    pub size: u64,
    /// Declared SHA-256, 64 lowercase hex digits.
    pub sha256: String,
    /// The module's own data, reread at confirmation.
    pub details: serde_json::Value,
    pub expires_at: Timestamp,
}

/// The upload-attempts table of a module. Obtain it with [`DirectUploads::new`]; cheap to clone.
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
    /// The `<module>_uploads` table of module `module` (created with [`uploads_migration`]).
    pub fn new(module: &str) -> Self {
        DirectUploads { table: table_name(module).into() }
    }

    /// Records a new attempt for tenant `tenant`.
    pub async fn insert<C: crate::db::Executor>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        new: NewDirectUpload,
    ) -> Result<DirectUpload, Error> {
        let connection = &crate::db::Via::of(connection);
        let size = i64::try_from(new.size).map_err(Error::internal)?;
        let now = Timestamp::now();
        let expires_at = new.expires_at;
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
                Option::<Timestamp>::None.into(),
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

    /// Attempt `id` of tenant `tenant`, if it exists.
    pub async fn find<C: crate::db::Executor>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Option<DirectUpload>, Error> {
        let connection = &crate::db::Via::of(connection);
        let select = self.select_one(tenant, id);
        connection.query_one(&select).await?.map(|row| parse(&row)).transpose()
    }

    /// Like [`find`](Self::find), and locks the row until the end of `transaction` (`FOR UPDATE` on
    /// Postgres; SQLite has only one write transaction at a time anyway): the second confirm command waits
    /// for the first to finish and then sees the attempt as `completed`.
    pub async fn lock<C: crate::db::Executor>(
        &self,
        transaction: &C,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Option<DirectUpload>, Error> {
        let transaction = &crate::db::Via::of(transaction);
        let mut select = self.select_one(tenant, id);
        select.lock_exclusive();
        transaction.query_one(&select).await?.map(|row| parse(&row)).transpose()
    }

    /// Marks attempt `id` as confirmed (call it in the transaction that creates the record, after
    /// [`lock`](Self::lock)). `false`: the attempt is no longer in the `pending` state.
    pub async fn complete<C: crate::db::Executor>(
        &self,
        transaction: &C,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<bool, Error> {
        let transaction = &crate::db::Via::of(transaction);
        let now = Timestamp::now();
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

    /// Deletes the row of attempt `id` (tenant `tenant_id`).
    pub async fn delete<C: crate::db::Executor>(
        &self,
        connection: &C,
        tenant_id: TenantId,
        id: Uuid,
    ) -> Result<(), Error> {
        let connection = &crate::db::Via::of(connection);
        let mut delete = Query::delete();
        delete
            .from_table(Alias::new(&*self.table))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant_id.as_uuid()))
            .and_where(Expr::col(Alias::new("id")).eq(id));
        connection.execute(&delete).await?;
        Ok(())
    }

    /// Up to `limit` attempts (of every tenant on this database) still `pending` that expired before `before`,
    /// oldest first, for the cleanup job.
    pub async fn expired<C: crate::db::Executor>(
        &self,
        connection: &C,
        before: Timestamp,
        limit: u64,
    ) -> Result<Vec<DirectUpload>, Error> {
        let connection = &crate::db::Via::of(connection);
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

    /// Deletes the rows of attempts confirmed before `before` (the record exists; the row remained only so
    /// a confirm command called again returns the same result). Returns the number of rows deleted.
    pub async fn forget_completed<C: crate::db::Executor>(
        &self,
        connection: &C,
        before: Timestamp,
    ) -> Result<u64, Error> {
        let connection = &crate::db::Via::of(connection);
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
    pub expires_at: Timestamp,
}

impl UploadLink {
    /// The link of attempt `id`.
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

/// An object reread for checking.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Inspected {
    /// The real number of bytes.
    pub size: u64,
    /// SHA-256 of the content, 64 lowercase hex digits.
    pub sha256: String,
    /// Up to the first `head_bytes` bytes (to detect the real type by magic bytes).
    pub head: Vec<u8>,
}

/// Reads the whole object at `key` to learn its real size and SHA-256 (the store is not trusted: not every
/// S3-compatible service checks the signed checksum), keeping the first `head_bytes` bytes. `None`: no
/// object yet. Do not call it inside a write transaction (this is a network call).
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

/// 64 hex digits (upper or lower case) into 32 bytes; a wrong form → `None`.
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

/// Bytes to lowercase hex digits.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Name of the upload-attempts table of module `module`.
fn table_name(module: &str) -> String {
    format!("{module}_uploads")
}

/// Migration that creates the `<module>_uploads` table. A module that accepts direct uploads adds it to its
/// migration list, like [`jobs_migration`](crate::job::jobs_migration).
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
