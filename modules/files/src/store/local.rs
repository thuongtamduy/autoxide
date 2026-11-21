//! Nơi lưu trên ổ đĩa (thư mục `AUTOXIDE_FILES_DIR`), tự viết trên `tokio::fs` (D-20261121-s3-client): mỗi
//! khoá là một file dưới thư mục gốc, thư mục con theo `/` của khoá.
//!
//! Ghi an toàn như S3: ghi vào file tạm cùng thư mục, `fsync`, rồi đổi tên — người đọc không bao giờ thấy file
//! ghi dở, mất điện không mất file đã báo là lưu xong. Ghi hỏng hay bị huỷ giữa chừng thì file tạm bị xoá.

use std::io::SeekFrom;
use std::path::{Path, PathBuf};

use autoxide_kernel::error::Error;
use autoxide_kernel::storage::{ByteStream, ObjectInfo, StoredFile};
use bytes::Bytes;
use futures_util::StreamExt as _;
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _, AsyncWriteExt as _};

/// Đọc file theo từng đoạn chừng này byte.
const CHUNK: usize = 64 * 1024;

/// Đuôi của file tạm khi đang ghi. Khoá chỉ có chữ, số, `-`, `_`, `.` nên không bao giờ trùng.
const STAGING_MARK: &str = "#staging-";

pub(super) struct LocalStore {
    root: PathBuf,
}

fn io_error(what: &str, error: std::io::Error) -> Error {
    Error::internal(format!("file storage: {what}: {error}"))
}

impl LocalStore {
    pub(super) fn new(root: PathBuf) -> Self {
        LocalStore { root }
    }

    fn path(&self, key: &str) -> PathBuf {
        // `key` đã được kiểm (chỉ chữ, số, `-`, `_`, `.`, không `..`): nối thẳng an toàn.
        key.split('/').fold(self.root.clone(), |path, part| path.join(part))
    }

    pub(super) async fn put(&self, key: &str, mut content: ByteStream) -> Result<u64, Error> {
        let path = self.path(key);
        let parent = path.parent().unwrap_or(&self.root).to_path_buf();
        tokio::fs::create_dir_all(&parent)
            .await
            .map_err(|error| io_error("create directory", error))?;
        let staging = Staging::new(&path);
        let mut file = tokio::fs::File::create(&staging.path)
            .await
            .map_err(|error| io_error("create file", error))?;
        let mut written: u64 = 0;
        while let Some(chunk) = content.next().await {
            // Lỗi của luồng (vd quá lớn) trả lại nguyên vẹn; `staging` bị bỏ thì file tạm bị xoá.
            let chunk = chunk?;
            written += chunk.len() as u64;
            file.write_all(&chunk).await.map_err(|error| io_error("write", error))?;
        }
        file.sync_all().await.map_err(|error| io_error("sync", error))?;
        drop(file);
        staging.commit(&path).await?;
        // Đổi tên chỉ bền khi thư mục cha cũng được ghi xuống đĩa (POSIX). Windows không mở được thư mục:
        // bỏ qua ở đó.
        if let Ok(directory) = tokio::fs::File::open(&parent).await {
            let _ = directory.sync_all().await;
        }
        Ok(written)
    }

    pub(super) async fn get(&self, key: &str) -> Result<Option<StoredFile>, Error> {
        let Some((file, size)) = self.open(key).await? else {
            return Ok(None);
        };
        Ok(Some(StoredFile::new(size, read_stream(file, size))))
    }

    pub(super) async fn get_range(
        &self,
        key: &str,
        range: std::ops::Range<u64>,
    ) -> Result<Option<StoredFile>, Error> {
        let Some((mut file, size)) = self.open(key).await? else {
            return Ok(None);
        };
        let end = range.end.min(size);
        let start = range.start.min(end);
        file.seek(SeekFrom::Start(start)).await.map_err(|error| io_error("seek", error))?;
        Ok(Some(StoredFile::new(end - start, read_stream(file, end - start))))
    }

    pub(super) async fn head(&self, key: &str) -> Result<Option<ObjectInfo>, Error> {
        match tokio::fs::metadata(self.path(key)).await {
            Ok(metadata) if metadata.is_file() => Ok(Some(ObjectInfo::new(metadata.len()))),
            Ok(_) => Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_error("read metadata", error)),
        }
    }

    pub(super) async fn delete(&self, key: &str) -> Result<(), Error> {
        match tokio::fs::remove_file(self.path(key)).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error("delete", error)),
        }
    }

    /// Khoá của mọi file dưới thư mục `directory` của khoá (rỗng: mọi file), theo thứ tự tên.
    pub(super) async fn list_under(&self, directory: &str) -> Result<Vec<String>, Error> {
        let start = if directory.is_empty() { self.root.clone() } else { self.path(directory) };
        let mut keys = Vec::new();
        let mut pending = vec![(start, directory.to_owned())];
        // Duyệt bằng ngăn xếp thay vì đệ quy: hàm `async` đệ quy phải đóng hộp từng tầng.
        while let Some((dir, key_prefix)) = pending.pop() {
            let mut entries = match tokio::fs::read_dir(&dir).await {
                Ok(entries) => entries,
                // Thư mục chưa có (chưa ghi file nào): không có file.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(io_error("list", error)),
            };
            while let Some(entry) =
                entries.next_entry().await.map_err(|error| io_error("list", error))?
            {
                let Ok(name) = entry.file_name().into_string() else { continue };
                if name.contains(STAGING_MARK) {
                    continue;
                }
                let key = if key_prefix.is_empty() { name } else { format!("{key_prefix}/{name}") };
                let kind = entry.file_type().await.map_err(|error| io_error("list", error))?;
                if kind.is_dir() {
                    pending.push((entry.path(), key));
                } else if kind.is_file() {
                    keys.push(key);
                }
            }
        }
        keys.sort();
        Ok(keys)
    }

    async fn open(&self, key: &str) -> Result<Option<(tokio::fs::File, u64)>, Error> {
        let file = match tokio::fs::File::open(self.path(key)).await {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error("open", error)),
        };
        let metadata = file.metadata().await.map_err(|error| io_error("read metadata", error))?;
        if !metadata.is_file() {
            return Ok(None);
        }
        Ok(Some((file, metadata.len())))
    }
}

/// Đọc tối đa `remaining` byte của `file` thành luồng từng đoạn.
fn read_stream(file: tokio::fs::File, remaining: u64) -> ByteStream {
    let stream =
        futures_util::stream::unfold((file, remaining), |(mut file, remaining)| async move {
            if remaining == 0 {
                return None;
            }
            let want = usize::try_from(remaining).map_or(CHUNK, |remaining| remaining.min(CHUNK));
            let mut buffer = vec![0; want];
            match file.read(&mut buffer).await {
                // File ngắn hơn đã đọc lúc mở (bị ghi đè giữa chừng): dừng.
                Ok(0) => None,
                Ok(read) => {
                    buffer.truncate(read);
                    Some((Ok(Bytes::from(buffer)), (file, remaining - read as u64)))
                }
                Err(error) => Some((Err(io_error("read", error)), (file, 0))),
            }
        });
    Box::pin(stream)
}

/// File tạm đang ghi. Bị bỏ trước [`Staging::commit`] (lỗi, request bị huỷ) thì bị xoá.
struct Staging {
    path: PathBuf,
    committed: bool,
}

impl Staging {
    fn new(target: &Path) -> Self {
        let name = target.file_name().and_then(|name| name.to_str()).unwrap_or("file");
        let path =
            target.with_file_name(format!("{name}{STAGING_MARK}{}", uuid::Uuid::now_v7().simple()));
        Staging { path, committed: false }
    }

    async fn commit(mut self, target: &Path) -> Result<(), Error> {
        tokio::fs::rename(&self.path, target).await.map_err(|error| io_error("rename", error))?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.committed {
            // `Drop` không chờ được: xoá đồng bộ (nhanh, một file).
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
