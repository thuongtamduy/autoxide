//! Job dọn hằng ngày: nội dung dưới `files/` mà không còn dòng nào trong `files_file` trỏ tới
//! (D-160) — request bị huỷ giữa lúc ghi nội dung và lúc lưu dòng, xoá nội dung hỏng sau khi
//! dòng đã xoá, upload kết thúc sau khi tài khoản đã bị xoá.

use std::collections::HashSet;
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler};
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use uuid::Uuid;

use crate::entity::{Column, Entity};
use crate::service::FileService;

pub(crate) const CLEAN_ORPHANS: &str = "files.clean_orphans";
/// Nội dung mới hơn chừng này không bị đụng tới: có thể đang được tải lên, chưa kịp lưu dòng.
const GRACE_HOURS: i64 = 24;
/// Số khoá kiểm mỗi câu truy vấn.
const BATCH: usize = 200;
const PREFIX: &str = "files/";

pub(crate) struct CleanOrphans(pub Arc<FileService>);

#[async_trait::async_trait]
impl JobHandler for CleanOrphans {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        self.0.clean_orphans(Utc::now()).await.map(|_| ())
    }
}

impl FileService {
    /// Xoá nội dung mồ côi dưới `files/` cũ hơn `now - GRACE_HOURS`. Trả số khoá đã xoá.
    pub(crate) async fn clean_orphans(&self, now: DateTime<Utc>) -> Result<u64, Error> {
        let cutoff = now - chrono::Duration::hours(GRACE_HOURS);
        let candidates: Vec<(String, Uuid)> = self
            .storage()
            .keys_with_prefix(PREFIX)
            .await?
            .into_iter()
            .filter_map(|key| {
                let id = key.strip_prefix(PREFIX).and_then(|id| Uuid::parse_str(id).ok());
                if id.is_none() {
                    tracing::warn!(key, "unexpected key under files/; left alone");
                }
                id.map(|id| (key, id))
            })
            .filter(|(_, id)| created_at(*id).is_some_and(|time| time < cutoff))
            .collect();
        let mut removed = 0;
        for chunk in candidates.chunks(BATCH) {
            let ids: Vec<Uuid> = chunk.iter().map(|(_, id)| *id).collect();
            // Kết nối ghi: dòng vừa lưu chưa tới read replica thì vẫn thấy.
            let known: HashSet<Uuid> = Entity::find()
                .select_only()
                .column(Column::Id)
                .filter(Column::Id.is_in(ids))
                .into_tuple::<Uuid>()
                .all(self.database().write())
                .await?
                .into_iter()
                .collect();
            for (key, id) in chunk {
                if known.contains(id) {
                    continue;
                }
                self.storage().delete(key).await?;
                removed += 1;
            }
        }
        if removed > 0 {
            tracing::info!(keys = removed, "orphaned file content removed");
        }
        Ok(removed)
    }
}

/// Thời điểm tạo ghi trong UUIDv7 (ID của file cũng là tên nội dung của nó).
fn created_at(id: Uuid) -> Option<DateTime<Utc>> {
    let (seconds, nanos) = id.get_timestamp()?.to_unix();
    DateTime::from_timestamp(i64::try_from(seconds).ok()?, nanos)
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::access::Principal;
    use autoxide_testkit::TestDatabase;
    use bytes::Bytes;
    use futures_util::StreamExt as _;

    use crate::testing::{no_limits, service};

    fn content(text: &'static str) -> autoxide_kernel::storage::ByteStream {
        futures_util::stream::iter([Ok(Bytes::from_static(text.as_bytes()))]).boxed()
    }

    /// Chỉ nội dung cũ hơn 24 giờ mà không có dòng nào trỏ tới mới bị xoá (D-160).
    #[tokio::test]
    async fn chi_xoa_noi_dung_mo_coi_da_cu() {
        TestDatabase::run(async |test| {
            let (files, _) = service(test, no_limits()).await;
            let ada = Principal::with_all_permissions(uuid::Uuid::now_v7());
            let kept =
                files.upload(&ada, "a.txt", Some("text/plain"), content("kept")).await.unwrap();
            let storage = files.storage().clone();
            let orphan = format!("files/{}", uuid::Uuid::now_v7());
            storage.put(&orphan, content("orphan")).await.unwrap();
            storage.put("files/not-an-id", content("odd")).await.unwrap();

            // Mới: chưa đụng tới. Một ngày sau: chỉ nội dung mồ côi bị xoá.
            assert_eq!(files.clean_orphans(chrono::Utc::now()).await.unwrap(), 0);
            let later = chrono::Utc::now() + chrono::Duration::hours(25);
            assert_eq!(files.clean_orphans(later).await.unwrap(), 1);
            assert!(storage.get(&orphan).await.unwrap().is_none());
            assert!(storage.get(&kept.storage_key()).await.unwrap().is_some());
            assert!(storage.get("files/not-an-id").await.unwrap().is_some());
        })
        .await
        .unwrap();
    }
}
