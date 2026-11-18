//! Cài đặt `Storage` (kernel) bằng `object_store`: một trait chung, hai nơi lưu — thư
//! mục trên ổ đĩa và S3 (hay dịch vụ tương thích S3).

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::storage::{
    ByteStream, ObjectInfo, PresignedUpload, Storage, StorageLocation, StorageOpener, StoredFile,
    UploadConstraints,
};
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
    /// Bộ ký link của S3 (cùng client với `store`); `None` với ổ đĩa: không tải thẳng được.
    signer: Option<Arc<object_store::aws::AmazonS3>>,
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
                Ok(ObjectStorage {
                    store: Arc::new(store),
                    signer: None,
                    prefix: Some(base.to_string()),
                })
            }
            Backend::S3(s3) => {
                let store = Arc::new(s3_store(s3)?);
                Ok(ObjectStorage {
                    store: store.clone(),
                    signer: Some(store),
                    prefix: s3.prefix.clone(),
                })
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

/// Mở nơi lưu file riêng của placement (bước 33) bằng đúng cài đặt của nơi lưu chung. Module công
/// bố `dyn StorageOpener`; sổ database của kernel gọi khi khách nằm trên placement có nơi lưu riêng.
pub(crate) struct Opener;

impl StorageOpener for Opener {
    fn open(&self, location: &StorageLocation) -> Result<Arc<dyn Storage>, Error> {
        let backend = match location {
            StorageLocation::Local { dir } => Backend::Local { dir: std::path::PathBuf::from(dir) },
            StorageLocation::S3(s3) => Backend::S3(S3Config {
                bucket: s3.bucket.clone(),
                prefix: s3.prefix.clone(),
                region: s3.region.clone(),
                endpoint: s3.endpoint.clone(),
                access_key_id: s3.access_key_id.clone(),
                secret_access_key: s3.secret_access_key.clone(),
            }),
        };
        Ok(Arc::new(ObjectStorage::open(&backend)?))
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
        // client ngắt) thì phần đã gửi lên S3 cũng được huỷ (D-158).
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
                Ok(Some(StoredFile::new(size, content)))
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
                Ok(Some(StoredFile::new(size, content)))
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

    async fn head(&self, key: &str) -> Result<Option<ObjectInfo>, Error> {
        let path = self.path(key)?;
        match self.store.head(&path).await {
            Ok(meta) => Ok(Some(ObjectInfo::new(meta.size))),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(storage_error(error)),
        }
    }

    /// Link `PUT` ký SigV4 (bước 40 P4), ký kèm `content-type` và `x-amz-checksum-sha256`: S3
    /// từ chối request thiếu hay sai một trong hai, và từ chối nội dung khác hash — tức khác cả cỡ.
    /// `content-length` không ký được qua `object_store` (header do bộ ký giữ); nó vẫn được trả để
    /// client gửi. Dịch vụ tương thích S3 có thể không kiểm hash: lệnh xác nhận luôn đọc lại, đo cỡ
    /// và băm (D-378).
    async fn presign_upload(
        &self,
        key: &str,
        constraints: &UploadConstraints,
        ttl: std::time::Duration,
    ) -> Result<Option<PresignedUpload>, Error> {
        use base64::Engine as _;
        use object_store::signer::{
            HeaderName, HeaderValue, Method, SignedUrlOptions, Signer as _,
        };
        let Some(signer) = &self.signer else {
            return Ok(None);
        };
        let path = self.path(key)?;
        let checksum = base64::engine::general_purpose::STANDARD.encode(constraints.sha256);
        let signed = [
            ("content-type", constraints.content_type.clone()),
            ("x-amz-checksum-sha256", checksum),
        ];
        let mut options = SignedUrlOptions::new();
        for (name, value) in &signed {
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(Error::internal)?;
            let value = HeaderValue::from_str(value).map_err(Error::internal)?;
            options = options.with_signed_header(name, value);
        }
        let mut headers = vec![("content-length".to_owned(), constraints.size.to_string())];
        headers.extend(signed.into_iter().map(|(name, value)| (name.to_owned(), value)));
        let issued = chrono::Utc::now();
        let url = signer
            .signed_url_opts(Method::PUT, &path, ttl, &options)
            .await
            .map_err(storage_error)?;
        let expires_at = issued + chrono::Duration::from_std(ttl).map_err(Error::internal)?;
        Ok(Some(PresignedUpload::new("PUT", url.to_string(), headers, expires_at)))
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
/// upload của S3 khi bị bỏ, các phần đã gửi nằm lại (vô hình, vẫn tính tiền) (D-158).
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
        // Không gọi `abort` sau `shutdown` hỏng: `shutdown` đã chuyển writer sang trạng thái
        // `Flush` (object_store 0.14, `BufWriter::poll_shutdown`), và `abort` ở trạng thái đó
        // panic ("Already shut down"); upload nhiều phần (nếu có) đã bị `finish` bên trong tiêu
        // thụ, nên không còn gì để huỷ từ đây. Lỗi này lộ ra khi `autoxide doctor` thử ghi vào
        // thư mục không ghi được (D-166).
        writer.shutdown().await.map_err(|error| Error::internal(format!("file storage: {error}")))
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
