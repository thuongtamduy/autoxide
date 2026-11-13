//! Bước kiểm của `files` cho `autoxide doctor` (D-166): ghi, đọc lại và xoá một file thử ở nơi lưu
//! đang cấu hình (thư mục cục bộ hay bucket S3). Sai quyền thư mục, sai key S3, bucket không có —
//! đều lộ ra ở đây thay vì ở lần tải file đầu tiên của người dùng.

use std::sync::Arc;

use autoxide_kernel::doctor::{Finding, Probe};
use autoxide_kernel::storage::Storage;
use bytes::Bytes;
use futures_util::{StreamExt as _, stream};
use uuid::Uuid;

/// Nội dung file thử.
const PROBE: &[u8] = b"autoxide doctor";

pub(crate) struct StorageProbe {
    pub storage: Arc<dyn Storage>,
    /// `local disk at <dir>` hay `S3 bucket <name>`, để thông báo nói rõ nơi nào.
    pub location: String,
}

#[async_trait::async_trait]
impl Probe for StorageProbe {
    async fn probe(&self) -> Finding {
        let key = format!("files/doctor-{}", Uuid::now_v7());
        let outcome = round_trip(self.storage.as_ref(), &key).await;
        // Dọn cả khi bước giữa hỏng; lỗi khi dọn không che lỗi chính.
        let removed =
            self.storage.delete(&key).await.map_err(|error| format!("delete failed: {error}"));
        match (outcome, removed) {
            (Ok(()), Ok(())) => Finding::ok(format!(
                "Wrote, read back and deleted a test file on {}.",
                self.location
            )),
            (Err(error), _) | (Ok(()), Err(error)) => Finding::failed(format!(
                "file storage on {} does not work: {error}",
                self.location
            ))
            .with_hint(
                "Check AUTOXIDE_FILES_* settings: the directory must be writable by this server, \
                 or the S3 bucket, endpoint and keys must be correct.",
            ),
        }
    }
}

async fn round_trip(storage: &dyn Storage, key: &str) -> Result<(), String> {
    let content = stream::once(async { Ok(Bytes::from_static(PROBE)) }).boxed();
    storage.put(key, content).await.map_err(|error| format!("write failed: {error}"))?;
    let file = storage
        .get(key)
        .await
        .map_err(|error| format!("read failed: {error}"))?
        .ok_or("the file just written was not found")?;
    let mut read = Vec::new();
    let mut content = file.content;
    while let Some(chunk) = content.next().await {
        read.extend_from_slice(&chunk.map_err(|error| format!("read failed: {error}"))?);
    }
    if read != PROBE {
        return Err("the file read back differs from the file written".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::doctor::Status;

    use super::*;
    use crate::store::ObjectStorage;

    #[tokio::test]
    async fn ghi_doc_xoa_duoc_thi_dat_va_khong_de_lai_file() {
        let dir = std::env::temp_dir().join(format!("autoxide-files-doctor-{}", Uuid::now_v7()));
        let backend = crate::config::Backend::Local { dir: dir.clone() };
        let storage: Arc<dyn Storage> = Arc::new(ObjectStorage::open(&backend).unwrap());
        let probe = StorageProbe { storage: storage.clone(), location: "local disk".to_owned() };

        let finding = probe.probe().await;
        assert_eq!(finding.status, Status::Ok, "{finding:?}");
        assert!(storage.keys().await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn noi_luu_khong_ghi_duoc_thi_hong_kem_goi_y() {
        // Một file thường đứng đúng chỗ thư mục lưu phải nằm: không tạo được thư mục con.
        let blocker =
            std::env::temp_dir().join(format!("autoxide-files-blocker-{}", Uuid::now_v7()));
        std::fs::write(&blocker, b"not a directory").unwrap();
        let backend = crate::config::Backend::Local { dir: blocker.clone() };
        let storage: Arc<dyn Storage> = Arc::new(ObjectStorage::open(&backend).unwrap());
        let probe = StorageProbe { storage, location: "local disk".to_owned() };

        let finding = probe.probe().await;
        assert_eq!(finding.status, Status::Failed, "{finding:?}");
        assert!(finding.hint.unwrap().contains("AUTOXIDE_FILES_"));
        let _ = std::fs::remove_file(blocker);
    }
}
