//! Nơi lưu file trong bộ nhớ cho test của module (đặc tả 32 B3): cài `dyn Storage` của kernel
//! mà không cần ổ đĩa hay S3 (nơi lưu thật được thử ở test đầu-cuối của `crates/app`).

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use autoxide_kernel::error::Error;
use autoxide_kernel::storage::{ByteStream, Storage, StoredFile};
use bytes::Bytes;
use futures_util::{StreamExt as _, TryStreamExt as _};

/// File giữ trong bộ nhớ, theo khoá.
///
/// - `get` trả nội dung thành nhiều khúc 3 byte, để thử code đọc qua nhiều khúc.
/// - Không cài lại `get_range` / `keys_with_prefix`: dùng luôn cài đặt mặc định của kernel.
/// - [`MemoryStorage::fail_writes`] làm `put` thất bại, để thử đường lỗi của nơi lưu.
#[derive(Debug, Default)]
pub struct MemoryStorage {
    files: Mutex<BTreeMap<String, Bytes>>,
    failing: AtomicBool,
}

impl MemoryStorage {
    /// Nơi lưu trống.
    pub fn new() -> Self {
        Self::default()
    }

    /// `true`: mọi `put` sau đó trả lỗi (nội dung không được lưu); `false`: chạy lại bình thường.
    pub fn fail_writes(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }

    /// Nội dung của `key`, nếu có.
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
