//! Job dọn hằng ngày: nội dung dưới `files/` mà không còn dòng nào trong `files_file` trỏ tới
//! (D-160) — request bị huỷ giữa lúc ghi nội dung và lúc lưu dòng, xoá nội dung hỏng sau khi
//! dòng đã xoá, upload kết thúc sau khi tài khoản đã bị xoá.
//!
//! Theo khách (bước 33): nội dung của khách mặc định ở `files/<id>`, của khách khác ở
//! `tenants/<khách>/files/<id>` (D-259). Mỗi khoá được đối chiếu với **database của chính khách đó**
//! (tra qua sổ database): một khoá không bao giờ bị coi là mồ côi chỉ vì job đang chạy trên database
//! khác. Khoá của khách không còn trong sổ thì để nguyên.

use std::collections::HashSet;
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler};
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use uuid::Uuid;

use autoxide_kernel::tenancy::{DatabaseRegistry, ScopeExt, TenantId};

use crate::entity::{Column, Entity};
use crate::service::FileService;

pub(crate) const CLEAN_ORPHANS: &str = "files.clean_orphans";
/// Nội dung mới hơn chừng này không bị đụng tới: có thể đang được tải lên, chưa kịp lưu dòng.
const GRACE_HOURS: i64 = 24;
/// Số khoá kiểm mỗi câu truy vấn.
const BATCH: usize = 200;
const PREFIX: &str = "files/";

pub(crate) struct CleanOrphans {
    pub service: Arc<FileService>,
    pub databases: DatabaseRegistry,
}

#[async_trait::async_trait]
impl JobHandler for CleanOrphans {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        self.service.clean_orphans(&self.databases, Utc::now()).await.map(|_| ())
    }
}

/// Khách và ID file của khoá `key` ở nơi lưu chung; `None` khi không phải khoá nội dung file.
fn parse_key(key: &str) -> Option<(TenantId, Uuid)> {
    if let Some(id) = key.strip_prefix(PREFIX) {
        return Some((autoxide_kernel::tenancy::DEFAULT_TENANT, Uuid::parse_str(id).ok()?));
    }
    let rest = key.strip_prefix("tenants/")?;
    let (tenant, id) = rest.split_once('/')?;
    let id = id.strip_prefix(PREFIX)?;
    Some((TenantId::from_uuid(Uuid::parse_str(tenant).ok()?), Uuid::parse_str(id).ok()?))
}

impl FileService {
    /// Xoá nội dung mồ côi (của mọi khách) cũ hơn `now - GRACE_HOURS`. Trả số khoá đã xoá.
    pub(crate) async fn clean_orphans(
        &self,
        databases: &DatabaseRegistry,
        now: DateTime<Utc>,
    ) -> Result<u64, Error> {
        let cutoff = now - chrono::Duration::hours(GRACE_HOURS);
        let mut keys = self.storage().keys_with_prefix(PREFIX).await?;
        keys.extend(
            self.storage()
                .keys_with_prefix("tenants/")
                .await?
                .into_iter()
                .filter(|key| key.contains("/files/")),
        );
        // Theo khách: (khoá, ID file) của nội dung đủ cũ.
        let mut by_tenant: std::collections::BTreeMap<TenantId, Vec<(String, Uuid)>> =
            std::collections::BTreeMap::new();
        for key in keys {
            match parse_key(&key) {
                Some((tenant, id)) if created_at(id).is_some_and(|time| time < cutoff) => {
                    by_tenant.entry(tenant).or_default().push((key, id));
                }
                Some(_) => {}
                None => tracing::warn!(key, "unexpected key under files/; left alone"),
            }
        }
        let mut removed = 0;
        for (tenant_id, candidates) in by_tenant {
            let Ok(tenant) = databases.tenant(tenant_id).await else {
                // Khách không còn trong sổ (hay chưa mở được database của nó): không đoán.
                continue;
            };
            for chunk in candidates.chunks(BATCH) {
                let ids: Vec<Uuid> = chunk.iter().map(|(_, id)| *id).collect();
                // Kết nối ghi: dòng vừa lưu chưa tới read replica thì vẫn thấy.
                let known: HashSet<Uuid> = Entity::find()
                    .select_only()
                    .column(Column::Id)
                    .filter(Column::Id.is_in(ids))
                    .scoped(&tenant)
                    .into_tuple::<Uuid>()
                    .all(tenant.database().write())
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
            let (files, database) = service(test, no_limits()).await;
            let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
            let ada = Principal::with_all_permissions(uuid::Uuid::now_v7());
            let kept = files
                .upload(&tenant, &ada, "a.txt", Some("text/plain"), content("kept"))
                .await
                .unwrap();
            let storage = files.storage().clone();
            let orphan = format!("files/{}", uuid::Uuid::now_v7());
            storage.put(&orphan, content("orphan")).await.unwrap();
            storage.put("files/not-an-id", content("odd")).await.unwrap();

            // Mới: chưa đụng tới. Một ngày sau: chỉ nội dung mồ côi bị xoá.
            assert_eq!(
                files
                    .clean_orphans(
                        &autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone()),
                        chrono::Utc::now()
                    )
                    .await
                    .unwrap(),
                0
            );
            let later = chrono::Utc::now() + chrono::Duration::hours(25);
            assert_eq!(
                files
                    .clean_orphans(
                        &autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone()),
                        later
                    )
                    .await
                    .unwrap(),
                1
            );
            assert!(storage.get(&orphan).await.unwrap().is_none());
            assert!(storage.get(&kept.storage_key()).await.unwrap().is_some());
            assert!(storage.get("files/not-an-id").await.unwrap().is_some());
        })
        .await
        .unwrap();
    }
}
