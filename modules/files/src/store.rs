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
use object_store::{GetOptions, GetRange, ObjectStore, ObjectStoreExt as _};
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
        // giữ cả file trong bộ nhớ. Bọc trong `Upload`: request bị huỷ giữa chừng (hết giờ,
        // client ngắt) thì phần đã gửi lên S3 cũng được huỷ (D-157).
        let mut upload =
            Upload(Some(BufWriter::with_capacity(self.store.clone(), path, PART_SIZE)));
        let mut written: u64 = 0;
        while let Some(chunk) = content.next().await {
            // Lỗi của luồng (vd quá lớn) trả lại nguyên vẹn; `upload` bị bỏ thì tự huỷ.
            let chunk = chunk?;
            written += chunk.len() as u64;
            upload.writer()?.put(chunk).await.map_err(storage_error)?;
        }
        upload.finish().await?;
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

    async fn get_range(
        &self,
        key: &str,
        range: std::ops::Range<u64>,
    ) -> Result<Option<StoredFile>, Error> {
        let path = self.path(key)?;
        let options = GetOptions { range: Some(GetRange::Bounded(range)), ..Default::default() };
        match self.store.get_opts(&path, options).await {
            Ok(result) => {
                let size = result.range.end - result.range.start;
                let content = result.into_stream().map_err(storage_error).boxed();
                Ok(Some(StoredFile { size, content }))
            }
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(storage_error(error)),
        }
    }

    async fn keys(&self) -> Result<Vec<String>, Error> {
        self.list_under("").await
    }

    async fn keys_with_prefix(&self, prefix: &str) -> Result<Vec<String>, Error> {
        // `object_store` liệt kê theo thư mục: liệt kê thư mục chứa `prefix` rồi lọc tiếp.
        let directory = prefix.rsplit_once('/').map_or("", |(directory, _)| directory);
        let mut keys = self.list_under(directory).await?;
        keys.retain(|key| key.starts_with(prefix));
        Ok(keys)
    }

    async fn delete(&self, key: &str) -> Result<(), Error> {
        let path = self.path(key)?;
        match self.store.delete(&path).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(error) => Err(storage_error(error)),
        }
    }
}

impl ObjectStorage {
    /// Khoá của mọi file trong thư mục `directory` (rỗng: mọi file), theo thứ tự tên.
    async fn list_under(&self, directory: &str) -> Result<Vec<String>, Error> {
        let full = [self.prefix.as_deref().unwrap_or_default(), directory]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        let prefix = (!full.is_empty()).then(|| Path::from(full.as_str()));
        let listed: Vec<_> = match self.store.list(prefix.as_ref()).try_collect().await {
            Ok(listed) => listed,
            // Thư mục cục bộ chưa có (chưa ghi file nào): không có file.
            Err(object_store::Error::NotFound { .. }) => Vec::new(),
            Err(error) => return Err(storage_error(error)),
        };
        let start = self.prefix.as_ref().map_or(0, |prefix| prefix.len() + 1);
        let mut keys: Vec<String> = listed
            .into_iter()
            .map(|object| object.location.as_ref().get(start..).unwrap_or_default().to_owned())
            .filter(|key| !key.is_empty())
            .collect();
        keys.sort();
        Ok(keys)
    }
}

/// Một lần ghi đang dở. Bị bỏ trước [`finish`](Self::finish) — lỗi, hay future của request bị
/// huỷ (hết giờ, client ngắt) — thì huỷ phần đã ghi: `object_store` không tự huỷ multipart
/// upload của S3 khi bị bỏ, các phần đã gửi nằm lại (vô hình, vẫn tính tiền) (D-157).
struct Upload(Option<BufWriter>);

impl Upload {
    fn writer(&mut self) -> Result<&mut BufWriter, Error> {
        self.0.as_mut().ok_or_else(|| Error::internal("file storage: upload already finished"))
    }

    async fn finish(mut self) -> Result<(), Error> {
        let mut writer = self
            .0
            .take()
            .ok_or_else(|| Error::internal("file storage: upload already finished"))?;
        if let Err(error) = writer.shutdown().await {
            abort(writer).await;
            return Err(Error::internal(format!("file storage: {error}")));
        }
        Ok(())
    }
}

impl Drop for Upload {
    fn drop(&mut self) {
        // `Drop` không chờ được: giao việc huỷ cho runtime (nếu còn).
        if let Some(writer) = self.0.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(abort(writer));
        }
    }
}

async fn abort(mut writer: BufWriter) {
    if let Err(error) = writer.abort().await {
        tracing::warn!(error = %error, "cannot abort a partial upload");
    }
}
