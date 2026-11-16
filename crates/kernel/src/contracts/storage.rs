//! Lưu file: trait chung để module nào cũng lưu và đọc được file mà không phụ thuộc
//! nơi lưu (ổ đĩa cục bộ, S3 hay dịch vụ tương thích S3).
//!
//! Một module (`files`) công bố `dyn Storage`; module cần lưu file lấy nó bằng
//! `context.require::<dyn Storage>()` (đăng ký SAU module `files`). Nội dung đi dưới
//! dạng luồng byte ([`ByteStream`]): file lớn không bao giờ nằm trọn trong bộ nhớ.

use std::ops::Range;
use std::pin::Pin;

use bytes::Bytes;
use futures_util::{Stream, StreamExt as _};
use serde::{Deserialize, Serialize};

use crate::error::Error;

/// Luồng byte của một file, đọc dần từng đoạn.
//
// `Pin<Box<dyn Stream…>>`: kiểu cụ thể của luồng khác nhau theo nơi lưu; đóng hộp để
// mọi cài đặt trả cùng một kiểu. `Pin`: luồng async có thể tự trỏ vào mình, nên không
// được di chuyển sau khi bắt đầu đọc.
pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, Error>> + Send>>;

/// Một file đọc ra: kích thước và nội dung.
#[non_exhaustive]
pub struct StoredFile {
    /// Số byte.
    pub size: u64,
    pub content: ByteStream,
}

impl StoredFile {
    /// A file of `size` bytes, read from `content`.
    pub fn new(size: u64, content: ByteStream) -> Self {
        StoredFile { size, content }
    }
}

/// Nơi lưu file của chương trình. Khoá (`key`) là đường dẫn tương đối gồm chữ, số,
/// `-`, `_`, `.` và `/` (vd `notes/0193…`); module nên đặt tên module của mình ở đầu.
#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    /// Ghi `content` vào `key` (đè nếu đã có) và trả số byte đã ghi. Luồng báo lỗi giữa
    /// chừng thì không có gì được ghi và lỗi đó được trả lại nguyên vẹn.
    async fn put(&self, key: &str, content: ByteStream) -> Result<u64, Error>;

    /// File ở `key`, hoặc `None` nếu không có.
    async fn get(&self, key: &str) -> Result<Option<StoredFile>, Error>;

    /// Xoá file ở `key`. Không có file thì coi như xong.
    async fn delete(&self, key: &str) -> Result<(), Error>;

    /// Khoá của mọi file đang lưu, theo thứ tự tên (dùng cho backup).
    async fn keys(&self) -> Result<Vec<String>, Error>;

    /// Đoạn byte `range` (`start..end`, không gồm `end`) của file ở `key`, hoặc `None` nếu
    /// không có file. `size` của kết quả là số byte của đoạn. Bên gọi bảo đảm
    /// `start < end <= kích thước file` (vd từ header `Range` đã kiểm).
    ///
    /// Cài đặt mặc định đọc từ đầu file rồi bỏ phần ngoài đoạn — đúng nhưng tốn; nơi lưu
    /// đọc được từng đoạn (ổ đĩa, S3) nên cài lại.
    async fn get_range(&self, key: &str, range: Range<u64>) -> Result<Option<StoredFile>, Error> {
        let Some(file) = self.get(key).await? else {
            return Ok(None);
        };
        let size = range.end.min(file.size).saturating_sub(range.start);
        Ok(Some(StoredFile { size, content: slice(file.content, range) }))
    }

    /// Khoá bắt đầu bằng `prefix` (vd `media/`), theo thứ tự tên. Module dùng để dọn file
    /// của riêng mình mà không phải đọc khoá của module khác.
    ///
    /// Cài đặt mặc định lọc từ [`keys`](Self::keys); nơi lưu liệt kê được theo tiền tố nên
    /// cài lại.
    async fn keys_with_prefix(&self, prefix: &str) -> Result<Vec<String>, Error> {
        let mut keys = self.keys().await?;
        keys.retain(|key| key.starts_with(prefix));
        Ok(keys)
    }
}

/// Chỉ giữ các byte trong `range` của luồng `content` (đọc từ đầu file).
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

/// Nơi lưu file của **một khách** (bước 33, D-259): bọc [`Storage`] chung, thêm tiền tố
/// [`TenantContext::storage_prefix`](crate::tenancy::TenantContext::storage_prefix) vào mọi khoá
/// (khách mặc định không có tiền tố — file cũ không phải dời; khách khác `tenants/<id>/`) và từ chối
/// khoá có `..`, bắt đầu bằng `/` hay có đoạn rỗng: một khách không với tới file của khách khác.
/// Liệt kê chỉ trả file của khách, khoá đã bỏ tiền tố.
///
/// Khách trên placement có nơi lưu riêng ([`StorageLocation`], `autoxide placement storage`) dùng
/// nơi lưu đó thay cho nơi lưu chung (sổ database gắn nó vào ngữ cảnh khách,
/// [`TenantContext::own_storage`](crate::tenancy::TenantContext::own_storage)) — module vẫn chỉ gọi
/// `TenantStorage::new(nơi_lưu_chung, khách)`.
pub struct TenantStorage {
    inner: std::sync::Arc<dyn Storage>,
    prefix: String,
}

impl TenantStorage {
    /// Nơi lưu của khách `tenant`: nơi lưu riêng của placement của nó nếu có, không thì `inner`
    /// (nơi lưu chung).
    pub fn new(inner: std::sync::Arc<dyn Storage>, tenant: &crate::tenancy::TenantContext) -> Self {
        let inner = tenant.own_storage().cloned().unwrap_or(inner);
        TenantStorage { inner, prefix: tenant.storage_prefix() }
    }

    /// Khoá thật của `key` (đã kiểm).
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
}

/// Nơi lưu file riêng của một placement (bước 33): một thư mục trên máy chủ hay một bucket S3 (hay
/// dịch vụ tương thích S3). Lưu mã hoá trong `kernel_placements.storage_sealed` (mục đích
/// [`PLACEMENT_STORAGE`](crate::tenancy::PLACEMENT_STORAGE)); không API nào trả lại khoá bí mật.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StorageLocation {
    /// Thư mục trên ổ đĩa của máy chủ (tạo khi ghi file đầu tiên).
    Local { dir: String },
    /// Bucket S3.
    S3(S3Location),
}

/// Bucket S3 của một placement — cùng các thông số với `AUTOXIDE_FILES_S3_*` / `AWS_*` của module
/// `files`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3Location {
    pub bucket: String,
    /// Thư mục trong bucket; `None`: gốc bucket.
    pub prefix: Option<String>,
    pub region: String,
    /// Địa chỉ dịch vụ tương thích S3; `None`: Amazon S3.
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
    /// Chuẩn hoá (bỏ khoảng trắng, `/` ở hai đầu tiền tố, `/` cuối địa chỉ) rồi kiểm; mọi trường sai
    /// báo cùng lúc (lỗi `validation`, tên trường như cờ của `autoxide placement storage`).
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

    /// Hai nơi lưu chỉ cùng một chỗ (cùng thư mục; cùng địa chỉ, bucket và thư mục trong bucket —
    /// khoá truy cập khác nhau vẫn là cùng chỗ).
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

/// Mở nơi lưu theo [`StorageLocation`] — module cài nơi lưu thật (`files`) công bố
/// (`context.provide::<dyn StorageOpener>(…)`); sổ database dùng để mở nơi lưu riêng của placement.
pub trait StorageOpener: Send + Sync {
    /// Nơi lưu ở `location`. Không gọi mạng (dựng client là đủ): lỗi cấu hình hiện ra ở lần đọc / ghi
    /// đầu tiên.
    fn open(&self, location: &StorageLocation) -> Result<std::sync::Arc<dyn Storage>, Error>;
}

/// Nơi lưu không mở được (cấu hình hỏng, không module nào mở nơi lưu): mọi thao tác trả lỗi nêu
/// lý do. Khách trên placement đó vẫn phục vụ được phần không chạm file.
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
