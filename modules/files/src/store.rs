//! Cài đặt `Storage` (kernel) bằng `object_store`: một trait chung, hai nơi lưu — thư
//! mục trên ổ đĩa và S3 (hay dịch vụ tương thích S3).

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::storage::{ByteStream, Storage, StoredFile};
use futures_util::{StreamExt as _, TryStreamExt as _};
use object_store::aws::AmazonS3Builder;
use object_store::buffered::BufWriter;
use object_store::local::LocalFileSystem;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt as _};
use tokio::io::AsyncWriteExt as _;

use crate::config::{Backend, S3Config};

/// Ghi lên S3 theo từng phần (multipart) chừng này byte; nhỏ hơn thì ghi một lần.
const PART_SIZE: usize = 8 * 1024 * 1024;

pub(crate) struct ObjectStorage {
    store: Arc<dyn ObjectStore>,
    /// Thêm vào đầu mọi khoá: thư mục lưu file (ổ đĩa), hay thư mục con trong bucket (S3).
    prefix: Option<String>,
}

impl ObjectStorage {
    /// Mở nơi lưu theo cấu hình. Thư mục cục bộ chưa có thì được tạo khi ghi file đầu
    /// tiên (khởi động không tạo gì).
    pub(crate) fn open(backend: &Backend) -> Result<Self, Error> {
        match backend {
            Backend::Local { dir } => {
                let absolute = std::path::absolute(dir).map_err(|error| {
                    Error::internal(format!("file directory {}: {error}", dir.display()))
                })?;
                let base = Path::from_absolute_path(&absolute).map_err(Error::internal)?;
                // `fsync`: ghi xong là nằm trên đĩa thật, như S3 bảo đảm — mất điện
                // không mất file đã báo là lưu xong.
                let store = LocalFileSystem::new().with_fsync(true).with_automatic_cleanup(true);
                Ok(ObjectStorage { store: Arc::new(store), prefix: Some(base.to_string()) })
            }
            Backend::S3(s3) => {
                Ok(ObjectStorage { store: Arc::new(s3_store(s3)?), prefix: s3.prefix.clone() })
            }
        }
    }

    fn path(&self, key: &str) -> Result<Path, Error> {
        let valid = !key.is_empty()
            && key.split('/').all(|part| {
                !part.is_empty()
                    && part != "."
                    && part != ".."
                    && part.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            });
        if !valid {
            return Err(Error::internal(format!("invalid storage key `{key}`")));
        }
        let full = match &self.prefix {
            Some(prefix) => format!("{prefix}/{key}"),
            None => key.to_owned(),
        };
        Path::parse(full).map_err(Error::internal)
    }
}

fn s3_store(config: &S3Config) -> Result<object_store::aws::AmazonS3, Error> {
    // HTTP client của S3 dùng rustls với provider chung của chương trình.
    autoxide_kernel::tls::install_crypto_provider();
    let mut builder =
        AmazonS3Builder::new().with_bucket_name(&config.bucket).with_region(&config.region);
    if let Some(endpoint) = &config.endpoint {
        builder = builder.with_endpoint(endpoint).with_allow_http(endpoint.starts_with("http://"));
    }
    if let Some(key) = &config.access_key_id {
        builder = builder.with_access_key_id(key);
    }
    if let Some(secret) = &config.secret_access_key {
        builder = builder.with_secret_access_key(secret);
    }
    builder.build().map_err(|error| Error::internal(format!("cannot set up S3: {error}")))
}

/// Lỗi của `object_store` thành lỗi chung (500; nguyên nhân chỉ nằm trong log).
fn storage_error(error: object_store::Error) -> Error {
    Error::internal(format!("file storage: {error}"))
}

#[async_trait::async_trait]
impl Storage for ObjectStorage {
    async fn put(&self, key: &str, mut content: ByteStream) -> Result<u64, Error> {
        let path = self.path(key)?;
        // `BufWriter`: file nhỏ ghi một lần; file lớn tự chuyển sang ghi từng phần, không
        // giữ cả file trong bộ nhớ.
        let mut writer = BufWriter::with_capacity(self.store.clone(), path, PART_SIZE);
        let mut written: u64 = 0;
        while let Some(chunk) = content.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    // Huỷ phần đã ghi dở; lỗi của luồng (vd quá lớn) trả lại nguyên vẹn.
                    if let Err(abort) = writer.abort().await {
                        tracing::warn!(error = %abort, "cannot abort a partial upload");
                    }
                    return Err(error);
                }
            };
            written += chunk.len() as u64;
            if let Err(error) = writer.put(chunk).await {
                if let Err(abort) = writer.abort().await {
                    tracing::warn!(error = %abort, "cannot abort a partial upload");
                }
                return Err(storage_error(error));
            }
        }
        writer
            .shutdown()
            .await
            .map_err(|error| Error::internal(format!("file storage: {error}")))?;
        Ok(written)
    }

    async fn get(&self, key: &str) -> Result<Option<StoredFile>, Error> {
        let path = self.path(key)?;
        match self.store.get(&path).await {
            Ok(result) => {
                let size = result.meta.size;
                let content = result.into_stream().map_err(storage_error).boxed();
                Ok(Some(StoredFile { size, content }))
            }
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(storage_error(error)),
        }
    }

    async fn delete(&self, key: &str) -> Result<(), Error> {
        let path = self.path(key)?;
        match self.store.delete(&path).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(error) => Err(storage_error(error)),
        }
    }
}
