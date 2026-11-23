//! In-memory file storage for module tests (spec 32 B3): implements the kernel's `dyn Storage`
//! without needing a disk or S3 (real storage is exercised in the end-to-end tests of `crates/app`).

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use autoxide_kernel::error::Error;
use autoxide_kernel::storage::{
    ByteStream, PresignedUpload, Storage, StorageLocation, StorageOpener, StoredFile,
    UploadConstraints,
};
use bytes::Bytes;
use futures_util::{StreamExt as _, TryStreamExt as _};

/// Files kept in memory, by key.
///
/// - `get` returns the content in 3-byte chunks, to exercise code that reads across several chunks.
/// - Does not reimplement `get_range` / `keys_with_prefix`: it uses the kernel's default implementation.
/// - [`MemoryStorage::fail_writes`] makes `put` fail, to exercise the storage error path.
#[derive(Debug, Default)]
pub struct MemoryStorage {
    files: Mutex<BTreeMap<String, Bytes>>,
    failing: AtomicBool,
    presigning: AtomicBool,
}

impl MemoryStorage {
    /// Empty storage.
    pub fn new() -> Self {
        Self::default()
    }

    /// `true`: every later `put` returns an error (content is not stored); `false`: back to normal.
    pub fn fail_writes(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }

    /// `true`: issues fake direct-upload links (step 40 P4) like S3 — `url` is `memory://<real key>`; tests
    /// "PUT" with [`MemoryStorage::put_presigned`]. Defaults to `false`: like a disk, no link is issued.
    pub fn presign(&self, presigning: bool) {
        self.presigning.store(presigning, Ordering::SeqCst);
    }

    /// Writes `content` as a client would PUT it to an issued link `url`.
    pub fn put_presigned(&self, url: &str, content: impl Into<Bytes>) {
        let key = url.strip_prefix("memory://").unwrap_or(url);
        self.lock().insert(key.to_owned(), content.into());
    }

    /// Content of `key`, if any.
    pub fn bytes(&self, key: &str) -> Option<Bytes> {
        self.lock().get(key).cloned()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Bytes>> {
        // Mutex chỉ bị "nhiễm độc" khi một test đã panic lúc giữ nó; dữ liệu vẫn dùng được.
        self.files.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[async_trait::async_trait]
impl Storage for MemoryStorage {
    async fn put(&self, key: &str, content: ByteStream) -> Result<u64, Error> {
        if self.failing.load(Ordering::SeqCst) {
            return Err(Error::internal("MemoryStorage: writes are set to fail"));
        }
        let chunks: Vec<Bytes> = content.try_collect().await?;
        let bytes = Bytes::from(chunks.concat());
        let size = bytes.len() as u64;
        self.lock().insert(key.to_owned(), bytes);
        Ok(size)
    }

    async fn get(&self, key: &str) -> Result<Option<StoredFile>, Error> {
        let bytes = self.bytes(key);
        Ok(bytes.map(|bytes| {
            let chunks: Vec<Result<Bytes, Error>> =
                bytes.chunks(3).map(|chunk| Ok(Bytes::copy_from_slice(chunk))).collect();
            StoredFile::new(bytes.len() as u64, futures_util::stream::iter(chunks).boxed())
        }))
    }

    async fn delete(&self, key: &str) -> Result<(), Error> {
        self.lock().remove(key);
        Ok(())
    }

    async fn keys(&self) -> Result<Vec<String>, Error> {
        Ok(self.lock().keys().cloned().collect())
    }

    async fn presign_upload(
        &self,
        key: &str,
        constraints: &UploadConstraints,
        ttl: std::time::Duration,
    ) -> Result<Option<PresignedUpload>, Error> {
        if !self.presigning.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let headers = vec![
            ("content-length".to_owned(), constraints.size.to_string()),
            ("content-type".to_owned(), constraints.content_type.clone()),
        ];
        let ttl = SignedDuration::try_from(ttl).map_err(Error::internal)?;
        let expires_at = Timestamp::now() + ttl;
        Ok(Some(PresignedUpload::new("PUT", format!("memory://{key}"), headers, expires_at)))
    }
}

/// Opener of per-placement storage for tests (step 33): one [`MemoryStorage`] per place (`dir` of local
/// storage, `bucket` of S3), created the first time it is opened. Pass it to the app with
/// `.provide::<dyn StorageOpener>(opener)`; tests inspect content with [`MemoryStorageOpener::store`].
#[derive(Clone, Default)]
pub struct MemoryStorageOpener {
    stores: std::sync::Arc<Mutex<std::collections::HashMap<String, std::sync::Arc<MemoryStorage>>>>,
}

impl std::fmt::Debug for MemoryStorageOpener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryStorageOpener").finish_non_exhaustive()
    }
}

impl MemoryStorageOpener {
    /// Opener with no storage yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Storage at place `place` (created if missing).
    pub fn store(&self, place: &str) -> std::sync::Arc<MemoryStorage> {
        let mut stores = self.stores.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        stores.entry(place.to_owned()).or_default().clone()
    }

    /// Makes place `place` point to an existing `store` (e.g. the app's shared storage): two different
    /// configurations that resolve to the same place.
    pub fn alias(&self, place: &str, store: std::sync::Arc<MemoryStorage>) {
        let mut stores = self.stores.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        stores.insert(place.to_owned(), store);
    }
}

impl StorageOpener for MemoryStorageOpener {
    fn open(&self, location: &StorageLocation) -> Result<std::sync::Arc<dyn Storage>, Error> {
        let place = match location {
            StorageLocation::Local { dir } => dir.clone(),
            StorageLocation::S3(s3) => s3.bucket.clone(),
        };
        let store: std::sync::Arc<dyn Storage> = self.store(&place);
        Ok(store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(bytes: &'static [u8]) -> ByteStream {
        futures_util::stream::iter([Ok(Bytes::from_static(bytes))]).boxed()
    }

    #[tokio::test]
    async fn luu_doc_xoa_va_gia_loi_ghi() {
        let storage = MemoryStorage::new();
        assert_eq!(storage.put("a/1", body(b"hello")).await.unwrap(), 5);
        let file = storage.get("a/1").await.unwrap().unwrap();
        let chunks: Vec<Bytes> = file.content.try_collect().await.unwrap();
        assert_eq!((file.size, chunks.len(), chunks.concat()), (5, 2, b"hello".to_vec()));
        assert_eq!(storage.keys_with_prefix("a/").await.unwrap(), ["a/1"]);
        storage.fail_writes(true);
        assert!(storage.put("a/2", body(b"x")).await.is_err());
        assert!(storage.bytes("a/2").is_none());
        storage.fail_writes(false);
        storage.delete("a/1").await.unwrap();
        assert!(storage.get("a/1").await.unwrap().is_none());
    }
}
