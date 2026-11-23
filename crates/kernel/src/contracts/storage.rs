//! File storage: a shared trait so any module can store and read files without depending on
//! where they are kept (local disk, S3 or an S3-compatible service).
//!
//! One module (`files`) publishes `dyn Storage`; a module that needs to store files obtains it with
//! `context.require::<dyn Storage>()` (registered AFTER the `files` module). Content travels as a
//! byte stream ([`ByteStream`]): a large file is never held in memory in full.

use crate::time::Timestamp;
use std::ops::Range;
use std::pin::Pin;

use bytes::Bytes;
use futures_util::{Stream, StreamExt as _};
use serde::{Deserialize, Serialize};

use crate::error::Error;

/// Byte stream of a file, read incrementally in chunks.
//
// `Pin<Box<dyn Stream…>>`: kiểu cụ thể của luồng khác nhau theo nơi lưu; đóng hộp để
// mọi cài đặt trả cùng một kiểu. `Pin`: luồng async có thể tự trỏ vào mình, nên không
// được di chuyển sau khi bắt đầu đọc.
pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, Error>> + Send>>;

/// A file that was read: its size and content.
#[non_exhaustive]
pub struct StoredFile {
    /// Number of bytes.
    pub size: u64,
    pub content: ByteStream,
}

impl StoredFile {
    /// A file of `size` bytes, read from `content`.
    pub fn new(size: u64, content: ByteStream) -> Self {
        StoredFile { size, content }
    }
}

/// Constraints of one direct upload to the storage (step 40 P4): the storage signs them into the link, so the
/// client cannot send a file of a different size, a different declared type or (with S3) a different hash.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct UploadConstraints {
    /// Exact number of bytes.
    pub size: u64,
    /// Content type (`Content-Type`) the client must send.
    pub content_type: String,
    /// SHA-256 of the content.
    pub sha256: [u8; 32],
}

impl UploadConstraints {
    pub fn new(size: u64, content_type: impl Into<String>, sha256: [u8; 32]) -> Self {
        UploadConstraints { size, content_type: content_type.into(), sha256 }
    }
}

/// Direct upload link to the storage: the client sends `method` to `url` with EXACTLY the (signed) `headers`,
/// the body being the file content, before `expires_at`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PresignedUpload {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub expires_at: Timestamp,
}

impl PresignedUpload {
    pub fn new(
        method: impl Into<String>,
        url: impl Into<String>,
        headers: Vec<(String, String)>,
        expires_at: Timestamp,
    ) -> Self {
        PresignedUpload { method: method.into(), url: url.into(), headers, expires_at }
    }
}

/// Information about a stored file, without reading its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ObjectInfo {
    /// Number of bytes.
    pub size: u64,
}

impl ObjectInfo {
    pub const fn new(size: u64) -> Self {
        ObjectInfo { size }
    }
}

/// The program's file storage. The key (`key`) is a relative path made of letters, digits,
/// `-`, `_`, `.` and `/` (e.g. `notes/0193…`); a module should put its own module name at the start.
#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    /// Writes `content` to `key` (overwriting if present) and returns the number of bytes written. If the
    /// stream fails midway, nothing is written and that error is returned unchanged.
    async fn put(&self, key: &str, content: ByteStream) -> Result<u64, Error>;

    /// The file at `key`, or `None` if there is none.
    async fn get(&self, key: &str) -> Result<Option<StoredFile>, Error>;

    /// Deletes the file at `key`. If there is no file, it counts as done.
    async fn delete(&self, key: &str) -> Result<(), Error>;

    /// Keys of all stored files, in name order (used for backup).
    async fn keys(&self) -> Result<Vec<String>, Error>;

    /// The byte range `range` (`start..end`, `end` excluded) of the file at `key`, or `None` if
    /// there is no file. The `size` of the result is the number of bytes of the range. The caller guarantees
    /// `start < end <= file size` (e.g. from a validated `Range` header).
    ///
    /// The default implementation reads from the start of the file and discards what is outside the range —
    /// correct but costly; storages that can read ranges (disk, S3) should override it.
    async fn get_range(&self, key: &str, range: Range<u64>) -> Result<Option<StoredFile>, Error> {
        let Some(file) = self.get(key).await? else {
            return Ok(None);
        };
        let size = range.end.min(file.size).saturating_sub(range.start);
        Ok(Some(StoredFile { size, content: slice(file.content, range) }))
    }

    /// Keys starting with `prefix` (e.g. `media/`), in name order. Modules use it to clean up their own files
    /// without having to read other modules' keys.
    ///
    /// The default implementation filters [`keys`](Self::keys); storages that can list by prefix should
    /// override it.
    async fn keys_with_prefix(&self, prefix: &str) -> Result<Vec<String>, Error> {
        let mut keys = self.keys().await?;
        keys.retain(|key| key.starts_with(prefix));
        Ok(keys)
    }

    /// Link for the client to upload a file directly to `key` (step 40 P4), expiring after `ttl`, signed with
    /// `constraints`. `None`: the storage cannot issue a link (disk) — the client uploads through the server
    /// as before.
    ///
    /// The default implementation returns `None`; storages such as S3 override it.
    async fn presign_upload(
        &self,
        key: &str,
        constraints: &UploadConstraints,
        ttl: std::time::Duration,
    ) -> Result<Option<PresignedUpload>, Error> {
        let _ = (key, constraints, ttl);
        Ok(None)
    }

    /// Information about the file at `key` (without reading content), or `None` if there is none.
    ///
    /// The default implementation opens the file through [`get`](Self::get) and drops the stream; storages
    /// that can answer for information alone (S3 `HEAD`) should override it.
    async fn head(&self, key: &str) -> Result<Option<ObjectInfo>, Error> {
        Ok(self.get(key).await?.map(|file| ObjectInfo::new(file.size)))
    }
}

/// Keeps only the bytes in `range` of the stream `content` (read from the start of the file).
pub fn slice(content: ByteStream, range: Range<u64>) -> ByteStream {
    let Range { start, end } = range;
    content
        // `scan` mang theo vị trí đã đọc; trả `None` thì luồng dừng (đã qua `end`, khỏi
        // đọc tiếp phần còn lại của file).
        .scan(0u64, move |position, chunk| {
            let item = match chunk {
                Err(error) => Some(Err(error)),
                Ok(bytes) => {
                    let chunk_start = *position;
                    let length = bytes.len() as u64;
                    *position += length;
                    if chunk_start >= end {
                        None
                    } else {
                        let from = start.saturating_sub(chunk_start).min(length);
                        let to = (end - chunk_start).min(length);
                        // `from`, `to` <= `length` (một `usize`), đổi kiểu không mất gì.
                        Some(Ok(bytes.slice(from as usize..to as usize)))
                    }
                }
            };
            std::future::ready(item)
        })
        .filter(|chunk| std::future::ready(!matches!(chunk, Ok(bytes) if bytes.is_empty())))
        .boxed()
}

/// File storage of **one tenant** (step 33, D-259): wraps the shared [`Storage`], adding the prefix
/// [`TenantContext::storage_prefix`](crate::tenancy::TenantContext::storage_prefix) to every key (the default
/// tenant has no prefix — old files need not move; other tenants use `tenants/<id>/`) and rejects keys
/// containing `..`, starting with `/` or having an empty segment: one tenant cannot reach another tenant's
/// files. Listing returns only the tenant's files, with the prefix stripped from keys.
///
/// A tenant on a placement with its own storage ([`StorageLocation`], `autoxide placement storage`) uses
/// that storage instead of the shared one (the database registry attaches it to the tenant context,
/// [`TenantContext::own_storage`](crate::tenancy::TenantContext::own_storage)) — modules still only call
/// `TenantStorage::new(shared_storage, tenant)`.
pub struct TenantStorage {
    inner: std::sync::Arc<dyn Storage>,
    prefix: String,
}

impl TenantStorage {
    /// Storage of the tenant `tenant`: the placement's own storage if it has one, otherwise `inner`
    /// (the shared storage).
    pub fn new(inner: std::sync::Arc<dyn Storage>, tenant: &crate::tenancy::TenantContext) -> Self {
        let inner = tenant.own_storage().cloned().unwrap_or(inner);
        TenantStorage { inner, prefix: tenant.storage_prefix() }
    }

    /// The real key of `key` (validated).
    fn key(&self, key: &str) -> Result<String, Error> {
        let bad = key.is_empty()
            || key.starts_with('/')
            || key.contains('\\')
            || key.split('/').any(|part| part.is_empty() || part == "." || part == "..");
        if bad {
            return Err(Error::internal(format!(
                "storage key `{key}` must be a relative path without `..`, `.` or empty parts"
            )));
        }
        Ok(format!("{}{key}", self.prefix))
    }

    fn strip(&self, keys: Vec<String>) -> Vec<String> {
        keys.into_iter()
            .filter_map(|key| key.strip_prefix(&self.prefix).map(str::to_owned))
            // Khách mặc định (không tiền tố): bỏ file của các khách khác.
            .filter(|key| !self.prefix.is_empty() || !key.starts_with("tenants/"))
            .collect()
    }
}

#[async_trait::async_trait]
impl Storage for TenantStorage {
    async fn put(&self, key: &str, content: ByteStream) -> Result<u64, Error> {
        self.inner.put(&self.key(key)?, content).await
    }

    async fn get(&self, key: &str) -> Result<Option<StoredFile>, Error> {
        self.inner.get(&self.key(key)?).await
    }

    async fn delete(&self, key: &str) -> Result<(), Error> {
        self.inner.delete(&self.key(key)?).await
    }

    async fn keys(&self) -> Result<Vec<String>, Error> {
        let keys = if self.prefix.is_empty() {
            self.inner.keys().await?
        } else {
            self.inner.keys_with_prefix(&self.prefix).await?
        };
        Ok(self.strip(keys))
    }

    async fn get_range(&self, key: &str, range: Range<u64>) -> Result<Option<StoredFile>, Error> {
        self.inner.get_range(&self.key(key)?, range).await
    }

    async fn keys_with_prefix(&self, prefix: &str) -> Result<Vec<String>, Error> {
        let keys = self.inner.keys_with_prefix(&format!("{}{prefix}", self.prefix)).await?;
        Ok(self.strip(keys))
    }

    async fn presign_upload(
        &self,
        key: &str,
        constraints: &UploadConstraints,
        ttl: std::time::Duration,
    ) -> Result<Option<PresignedUpload>, Error> {
        self.inner.presign_upload(&self.key(key)?, constraints, ttl).await
    }

    async fn head(&self, key: &str) -> Result<Option<ObjectInfo>, Error> {
        self.inner.head(&self.key(key)?).await
    }
}

/// The own file storage of one placement (step 33): a directory on the server or an S3 bucket (or an
/// S3-compatible service). Stored encrypted in `kernel_placements.storage_sealed` (purpose
/// [`PLACEMENT_STORAGE`](crate::tenancy::PLACEMENT_STORAGE)); no API returns the secret key back.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StorageLocation {
    /// A directory on the server's disk (created when the first file is written).
    Local { dir: String },
    /// An S3 bucket.
    S3(S3Location),
}

/// The S3 bucket of a placement — the same parameters as `AUTOXIDE_FILES_S3_*` / `AWS_*` of the
/// `files` module.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3Location {
    pub bucket: String,
    /// Directory inside the bucket; `None`: the bucket root.
    pub prefix: Option<String>,
    pub region: String,
    /// Address of the S3-compatible service; `None`: Amazon S3.
    pub endpoint: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
}

// `Debug` tự viết: không bao giờ in khoá bí mật (log, thông báo lỗi).
impl std::fmt::Debug for S3Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Location")
            .field("bucket", &self.bucket)
            .field("prefix", &self.prefix)
            .field("region", &self.region)
            .field("endpoint", &self.endpoint)
            .field("access_key_id", &self.access_key_id.as_ref().map(|_| "***"))
            .field("secret_access_key", &self.secret_access_key.as_ref().map(|_| "***"))
            .finish()
    }
}

impl std::fmt::Debug for StorageLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageLocation::Local { dir } => f.debug_struct("Local").field("dir", dir).finish(),
            StorageLocation::S3(s3) => s3.fmt(f),
        }
    }
}

impl StorageLocation {
    /// Normalizes (trims whitespace, `/` at both ends of the prefix, trailing `/` of the address) and then
    /// validates; all invalid fields are reported at once (error `validation`, field names like the flags of
    /// `autoxide placement storage`).
    pub fn checked(self) -> Result<Self, Error> {
        use crate::error::FieldError;
        let mut problems = Vec::new();
        let location = match self {
            StorageLocation::Local { dir } => {
                let dir = dir.trim().to_owned();
                if dir.is_empty() {
                    problems.push(FieldError::new("dir", "required", "must not be empty"));
                }
                StorageLocation::Local { dir }
            }
            StorageLocation::S3(s3) => {
                let text = |value: Option<String>| {
                    value.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())
                };
                let bucket = s3.bucket.trim().to_owned();
                let bucket_ok = (3..=63).contains(&bucket.len())
                    && bucket.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.'
                    });
                if !bucket_ok {
                    problems.push(FieldError::new(
                        "s3_bucket",
                        "invalid",
                        "must be 3 to 63 lower-case letters, digits, `-` and `.`",
                    ));
                }
                let prefix = text(s3.prefix).map(|prefix| prefix.trim_matches('/').to_owned());
                if prefix.as_deref().is_some_and(|prefix| {
                    !prefix.chars().all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
                }) {
                    problems.push(FieldError::new(
                        "s3_prefix",
                        "invalid",
                        "may contain only letters, digits, `-`, `_`, `.` and `/`",
                    ));
                }
                let region = s3.region.trim().to_owned();
                if region.is_empty() {
                    problems.push(FieldError::new("s3_region", "required", "must not be empty"));
                }
                let endpoint = text(s3.endpoint).map(|url| url.trim_end_matches('/').to_owned());
                if endpoint
                    .as_deref()
                    .is_some_and(|url| !url.starts_with("https://") && !url.starts_with("http://"))
                {
                    problems.push(FieldError::new(
                        "s3_endpoint",
                        "invalid",
                        "must be an `https://` (or `http://`) URL",
                    ));
                }
                let access_key_id = text(s3.access_key_id);
                let secret_access_key = s3.secret_access_key.filter(|value| !value.is_empty());
                if access_key_id.is_some() != secret_access_key.is_some() {
                    problems.push(FieldError::new(
                        "s3_secret_access_key",
                        "required",
                        "give both the access key and its secret, or neither",
                    ));
                }
                StorageLocation::S3(S3Location {
                    bucket,
                    prefix: prefix.filter(|prefix| !prefix.is_empty()),
                    region,
                    endpoint,
                    access_key_id,
                    secret_access_key,
                })
            }
        };
        if problems.is_empty() { Ok(location) } else { Err(Error::validation(problems)) }
    }

    /// Whether two storages point to the same place (same directory; same address, bucket and directory in
    /// the bucket — different access keys are still the same place).
    pub fn same_place(&self, other: &StorageLocation) -> bool {
        match (self, other) {
            (StorageLocation::Local { dir: a }, StorageLocation::Local { dir: b }) => {
                a.trim_end_matches('/') == b.trim_end_matches('/')
            }
            (StorageLocation::S3(a), StorageLocation::S3(b)) => {
                (&a.endpoint, &a.bucket, &a.prefix) == (&b.endpoint, &b.bucket, &b.prefix)
            }
            _ => false,
        }
    }
}

/// Opens a storage for a [`StorageLocation`] — the module that implements the real storage (`files`)
/// publishes it (`context.provide::<dyn StorageOpener>(…)`); the database registry uses it to open a
/// placement's own storage.
pub trait StorageOpener: Send + Sync {
    /// The storage at `location`. Makes no network call (building the client is enough): configuration errors
    /// surface on the first read / write.
    fn open(&self, location: &StorageLocation) -> Result<std::sync::Arc<dyn Storage>, Error>;
}

/// A storage that cannot be opened (broken configuration, no module opens storage): every operation returns
/// an error giving the reason. Tenants on that placement can still be served for everything that does not
/// touch files.
pub(crate) struct UnavailableStorage(pub(crate) String);

impl UnavailableStorage {
    fn error(&self) -> Error {
        Error::internal(format!("file storage unavailable: {}", self.0))
    }
}

#[async_trait::async_trait]
impl Storage for UnavailableStorage {
    async fn put(&self, _key: &str, _content: ByteStream) -> Result<u64, Error> {
        Err(self.error())
    }

    async fn get(&self, _key: &str) -> Result<Option<StoredFile>, Error> {
        Err(self.error())
    }

    async fn delete(&self, _key: &str) -> Result<(), Error> {
        Err(self.error())
    }

    async fn keys(&self) -> Result<Vec<String>, Error> {
        Err(self.error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::TryStreamExt as _;

    #[test]
    fn noi_luu_rieng_duoc_chuan_hoa_va_kiem() {
        let s3 = |bucket: &str, key: Option<&str>, secret: Option<&str>| {
            StorageLocation::S3(S3Location {
                bucket: bucket.to_owned(),
                prefix: Some("/tenants-eu/".to_owned()),
                region: "us-east-1".to_owned(),
                endpoint: Some("http://127.0.0.1:9000/".to_owned()),
                access_key_id: key.map(str::to_owned),
                secret_access_key: secret.map(str::to_owned),
            })
        };
        let StorageLocation::S3(ok) =
            s3(" files ", Some("id"), Some("TOPSECRET-1")).checked().unwrap()
        else {
            panic!("expected S3")
        };
        assert_eq!(ok.bucket, "files");
        assert_eq!(ok.prefix.as_deref(), Some("tenants-eu"));
        assert_eq!(ok.endpoint.as_deref(), Some("http://127.0.0.1:9000"));
        assert!(!format!("{ok:?}").contains("TOPSECRET"), "{ok:?}");

        let error = s3("A", Some("id"), None).checked().unwrap_err();
        let fields: Vec<&str> = error.fields().iter().map(|e| e.field.as_str()).collect();
        assert_eq!(fields, ["s3_bucket", "s3_secret_access_key"]);
        assert!(StorageLocation::Local { dir: " ".to_owned() }.checked().is_err());

        let a = s3("files", Some("one"), Some("x")).checked().unwrap();
        let b = s3("files", Some("two"), Some("y")).checked().unwrap();
        assert!(a.same_place(&b));
        assert!(!a.same_place(&s3("other", None, None).checked().unwrap()));
        let dir = StorageLocation::Local { dir: "/data/eu/".to_owned() };
        assert!(dir.same_place(&StorageLocation::Local { dir: "/data/eu".to_owned() }));
        assert!(!dir.same_place(&a));
        let json = serde_json::to_string(&dir).unwrap();
        assert_eq!(json, r#"{"kind":"local","dir":"/data/eu/"}"#);
        assert_eq!(serde_json::from_str::<StorageLocation>(&json).unwrap(), dir);
    }

    #[tokio::test]
    async fn cat_doan_qua_nhieu_khuc() {
        for (range, expected) in
            [(0..5, "hello"), (4..8, "o wo"), (6..9, "wor"), (9..12, "ld!"), (11..12, "!")]
        {
            let chunks =
                ["hello ", "wor", "ld!"].map(|text| Ok(Bytes::from_static(text.as_bytes())));
            let content = futures_util::stream::iter(chunks).boxed();
            let parts: Vec<Bytes> = slice(content, range.clone()).try_collect().await.unwrap();
            assert_eq!(parts.concat(), expected.as_bytes(), "{range:?}");
        }
    }
}
