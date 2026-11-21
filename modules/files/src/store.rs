//! Cài đặt `Storage` (kernel): hai nơi lưu — thư mục trên ổ đĩa (`local.rs`) và S3 hay dịch vụ tương thích
//! S3 (`s3.rs`, qua `rusty-s3`; thay `object_store` — D-20261121-s3-client).

mod local;
mod s3;

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::storage::{
    ByteStream, ObjectInfo, PresignedUpload, Storage, StorageLocation, StorageOpener, StoredFile,
    UploadConstraints,
};

use self::local::LocalStore;
use self::s3::S3Store;
use crate::config::{Backend, S3Config};

pub(crate) struct ObjectStorage {
    backend: Place,
    /// Thêm vào đầu mọi khoá trong bucket (thư mục con), chỉ với S3.
    prefix: Option<String>,
}

enum Place {
    Local(LocalStore),
    S3(S3Store),
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
                Ok(ObjectStorage { backend: Place::Local(LocalStore::new(absolute)), prefix: None })
            }
            Backend::S3(s3) => Ok(ObjectStorage {
                backend: Place::S3(S3Store::open(s3)?),
                prefix: s3.prefix.clone().filter(|prefix| !prefix.is_empty()),
            }),
        }
    }

    /// Khoá đã kiểm (chỉ chữ, số, `-`, `_`, `.`, các đoạn nối bằng `/`, không `.` hay `..`).
    fn checked(key: &str) -> Result<&str, Error> {
        let valid = !key.is_empty()
            && key.split('/').all(|part| {
                !part.is_empty()
                    && part != "."
                    && part != ".."
                    && part.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            });
        if valid { Ok(key) } else { Err(Error::internal(format!("invalid storage key `{key}`"))) }
    }

    /// Tên đối tượng trong bucket của khoá `key` (thêm tiền tố nếu có).
    fn object(&self, key: &str) -> Result<String, Error> {
        let key = Self::checked(key)?;
        Ok(match &self.prefix {
            Some(prefix) => format!("{prefix}/{key}"),
            None => key.to_owned(),
        })
    }

    /// Khoá của mọi file trong thư mục `directory` (rỗng: mọi file), theo thứ tự tên.
    async fn list_under(&self, directory: &str) -> Result<Vec<String>, Error> {
        match &self.backend {
            Place::Local(local) => local.list_under(directory).await,
            Place::S3(s3) => {
                // Liệt kê theo thư mục (kết thúc bằng `/`): `media` không lẫn `media2/…`.
                let full: Vec<&str> = [self.prefix.as_deref().unwrap_or_default(), directory]
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect();
                let listed = if full.is_empty() {
                    s3.list("").await?
                } else {
                    s3.list(&format!("{}/", full.join("/"))).await?
                };
                let start = self.prefix.as_ref().map_or(0, |prefix| prefix.len() + 1);
                Ok(listed
                    .into_iter()
                    .filter_map(|object| object.get(start..).map(str::to_owned))
                    .filter(|key| !key.is_empty())
                    .collect())
            }
        }
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

#[async_trait::async_trait]
impl Storage for ObjectStorage {
    async fn put(&self, key: &str, content: ByteStream) -> Result<u64, Error> {
        let object = self.object(key)?;
        match &self.backend {
            Place::Local(local) => local.put(&object, content).await,
            Place::S3(s3) => s3.put(&object, content).await,
        }
    }

    async fn get(&self, key: &str) -> Result<Option<StoredFile>, Error> {
        let object = self.object(key)?;
        match &self.backend {
            Place::Local(local) => local.get(&object).await,
            Place::S3(s3) => s3.get(&object).await,
        }
    }

    async fn get_range(
        &self,
        key: &str,
        range: std::ops::Range<u64>,
    ) -> Result<Option<StoredFile>, Error> {
        let object = self.object(key)?;
        match &self.backend {
            Place::Local(local) => local.get_range(&object, range).await,
            Place::S3(s3) => s3.get_range(&object, range).await,
        }
    }

    async fn keys(&self) -> Result<Vec<String>, Error> {
        self.list_under("").await
    }

    async fn keys_with_prefix(&self, prefix: &str) -> Result<Vec<String>, Error> {
        // Liệt kê thư mục chứa `prefix` rồi lọc tiếp (`media/ab` → thư mục `media`).
        let directory = prefix.rsplit_once('/').map_or("", |(directory, _)| directory);
        let mut keys = self.list_under(directory).await?;
        keys.retain(|key| key.starts_with(prefix));
        Ok(keys)
    }

    async fn delete(&self, key: &str) -> Result<(), Error> {
        let object = self.object(key)?;
        match &self.backend {
            Place::Local(local) => local.delete(&object).await,
            Place::S3(s3) => s3.delete(&object).await,
        }
    }

    async fn head(&self, key: &str) -> Result<Option<ObjectInfo>, Error> {
        let object = self.object(key)?;
        match &self.backend {
            Place::Local(local) => local.head(&object).await,
            Place::S3(s3) => s3.head(&object).await,
        }
    }

    /// Link tải thẳng: chỉ S3 cấp được; ổ đĩa trả `None` (client tải qua máy chủ).
    async fn presign_upload(
        &self,
        key: &str,
        constraints: &UploadConstraints,
        ttl: std::time::Duration,
    ) -> Result<Option<PresignedUpload>, Error> {
        let object = self.object(key)?;
        match &self.backend {
            Place::Local(_) => Ok(None),
            Place::S3(s3) => s3.presign_upload(&object, constraints, ttl).await.map(Some),
        }
    }
}
