//! Contract of the `files` module:
//!
//! - the shared trait `autoxide_kernel::storage::Storage` (where files are stored); other modules get it with
//!   `context.require::<dyn Storage>()` (registered AFTER `files`);
//! - the [`Files`] trait: reads a file a user uploaded through `/api/v1/files`, under exactly the viewing
//!   rule of `files` (the file owner or `files.manage`). Other modules use it to receive a file a user
//!   uploaded (e.g. `catalog` copies the file of a digital good into its own place) without reading the
//!   `files_file` table.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::storage::ByteStream;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

/// Information about an uploaded file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileInfo {
    pub id: Uuid,
    /// Display name (validated: no `/`, `\`, or control characters).
    pub name: String,
    pub content_type: String,
    /// Size in bytes.
    pub size: u64,
    /// SHA-256 of the content, lowercase hex.
    pub sha256: String,
    pub created_at: Timestamp,
}

impl FileInfo {
    /// Builds file information (for fakes in other modules' tests).
    pub fn new(
        id: Uuid,
        name: impl Into<String>,
        content_type: impl Into<String>,
        size: u64,
        sha256: impl Into<String>,
        created_at: Timestamp,
    ) -> Self {
        FileInfo {
            id,
            name: name.into(),
            content_type: content_type.into(),
            size,
            sha256: sha256.into(),
            created_at,
        }
    }
}

/// An opened file: information and content.
#[non_exhaustive]
pub struct OpenedFile {
    pub info: FileInfo,
    pub content: ByteStream,
}

impl OpenedFile {
    pub fn new(info: FileInfo, content: ByteStream) -> Self {
        OpenedFile { info, content }
    }
}

/// Files a user has uploaded. The `files` module publishes `dyn Files`.
#[async_trait::async_trait]
pub trait Files: Send + Sync {
    /// Information about file `id` of tenant `tenant`, if `principal` may view it (the file owner who still has
    /// `files.upload`, or someone with `files.manage`). Missing or not viewable: `files.file_not_found`
    /// (does not reveal whether another person's file exists).
    async fn find(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<FileInfo, Error>;

    /// Like [`find`](Self::find), with the content to read (copy, process).
    async fn open(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<OpenedFile, Error>;
}
