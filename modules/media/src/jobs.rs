//! Job định kỳ của module `media` (chạy trên job queue của core, mỗi ngày một lần):
//!
//! - [`PURGE_TRASH`]: xoá hẳn item đã nằm trong thùng rác quá `AUTOXIDE_MEDIA_TRASH_DAYS`;
//! - [`CLEAN_ORPHANS`]: xoá nội dung dưới `media/` mà không item nào trỏ tới (xoá hỏng giữa
//!   chừng, máy chủ tắt giữa lúc tải lên). Chỉ xoá khoá cũ hơn [`ORPHAN_GRACE_HOURS`] giờ —
//!   tuổi đọc từ `version_id` (UUIDv7 mang thời điểm tạo) — để không xoá upload đang dở.
//!   Không bao giờ đọc hay xoá khoá ngoài `media/` (của từng khách).
//!
//! Theo khách (bước 33): job theo lịch chạy một lần cho mỗi database. Dọn thùng rác tìm các khách
//! có item quá hạn trên database đó (index quét `media_item_trashed_sweep_idx`) rồi xoá theo từng
//! khách — mỗi lần xoá ghi event của đúng khách. Dọn mồ côi đọc khoá ở nơi lưu chung: nội dung của
//! khách mặc định ở `media/…`, của khách khác ở `tenants/<khách>/media/…` (D-259); mỗi khoá được
//! đối chiếu với **database của chính khách đó** (tra qua sổ database). Khoá của khách không còn
//! trong sổ thì để nguyên.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::tenancy::{
    DEFAULT_TENANT, DatabaseRegistry, ScopeExt, TenantContext, TenantId,
};
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use crate::entity::item::{Column, Entity, KEY_PREFIX};
use crate::service::{MediaService, now};

pub(crate) const PURGE_TRASH: &str = "media.purge_trash";
pub(crate) const CLEAN_ORPHANS: &str = "media.clean_orphans";

/// Nội dung trẻ hơn chừng này giờ không bao giờ bị coi là mồ côi.
pub(crate) const ORPHAN_GRACE_HOURS: i64 = 24;

/// Số dòng / khoá xử lý mỗi lượt.
const BATCH: usize = 200;

pub(crate) struct PurgeTrash {
    pub service: Arc<MediaService>,
    /// Sổ database: tra ngữ cảnh của từng khách có item quá hạn trên database của job.
    pub databases: DatabaseRegistry,
}

#[async_trait::async_trait]
impl JobHandler for PurgeTrash {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.service.purge_expired(&self.databases, &job.database, now()).await.map(|_| ())
    }
}

pub(crate) struct CleanOrphans {
    pub service: Arc<MediaService>,
    /// Sổ database: mỗi khoá được đối chiếu với database của khách sở hữu nó.
    pub databases: DatabaseRegistry,
}

#[async_trait::async_trait]
impl JobHandler for CleanOrphans {
    async fn run(&self, _: &JobContext) -> Result<(), Error> {
        self.service.clean_orphans(&self.databases, now()).await.map(|_| ())
    }
}

impl MediaService {
    /// Xoá hẳn item vào thùng rác trước `now - trash_days` trên `database`, theo từng khách. Trả
    /// số item đã xoá.
    pub(crate) async fn purge_expired(
        &self,
        databases: &DatabaseRegistry,
        database: &Database,
        now: DateTime<Utc>,
    ) -> Result<u64, Error> {
        let before = now - chrono::Duration::days(i64::from(self.config.trash_days));
        // Khách có item quá hạn trên database này (quét cả database qua index `_sweep_idx`).
        let tenants: Vec<Uuid> = Entity::find()
            .select_only()
            .column(Column::TenantId)
            .distinct()
            .filter(Column::TrashedAt.lt(before))
            .into_tuple::<Uuid>()
            .all(database.read())
            .await?;
        let mut purged = 0;
        for id in tenants {
            match databases.tenant(TenantId::from_uuid(id)).await {
                Ok(tenant) => purged += self.purge_expired_of(&tenant, before).await?,
                // Khách đã bị xoá khỏi sổ: dữ liệu còn lại do việc xoá khách lo, không đoán ở đây.
                Err(error) => {
                    tracing::warn!(tenant_id = %id, error = %error, "media trash purge skipped a tenant");
                }
            }
        }
        if purged > 0 {
            tracing::info!(items = purged, "expired media items purged from the trash");
        }
        Ok(purged)
    }

    /// Phần của khách `tenant`: xoá hẳn item vào thùng rác trước `before`.
    async fn purge_expired_of(
        &self,
        tenant: &TenantContext,
        before: DateTime<Utc>,
    ) -> Result<u64, Error> {
        let mut purged = 0;
        loop {
            let items = Entity::find()
                .scoped(tenant)
                .filter(Column::TrashedAt.lt(before))
                .order_by_asc(Column::TrashedAt)
                .limit(BATCH as u64)
                .all(tenant.database().write())
                .await?;
            if items.is_empty() {
                break;
            }
            for item in &items {
                if self.purge_item(tenant, Actor::system(), item).await? {
                    purged += 1;
                }
            }
            if items.len() < BATCH {
                break;
            }
        }
        Ok(purged)
    }

    /// Xoá nội dung mồ côi (của mọi khách) cũ hơn `now - ORPHAN_GRACE_HOURS`. Trả số khoá đã xoá.
    pub(crate) async fn clean_orphans(
        &self,
        databases: &DatabaseRegistry,
        now: DateTime<Utc>,
    ) -> Result<u64, Error> {
        let cutoff = now - chrono::Duration::hours(ORPHAN_GRACE_HOURS);
        // Khoá thật ở nơi lưu chung (không qua tiền tố khách): của khách mặc định và của mọi khách
        // khác.
        let mut keys = self.storage.keys_with_prefix(KEY_PREFIX).await?;
        keys.extend(
            self.storage
                .keys_with_prefix("tenants/")
                .await?
                .into_iter()
                .filter(|key| key.contains("/media/")),
        );
        // Theo khách: (khoá, item, phiên bản) của nội dung đủ cũ.
        let mut by_tenant: BTreeMap<TenantId, Vec<(String, Uuid, Uuid)>> = BTreeMap::new();
        for key in keys {
            match parse_key(&key) {
                Some((tenant, item, version))
                    if created_at(version).is_some_and(|time| time < cutoff) =>
                {
                    by_tenant.entry(tenant).or_default().push((key, item, version));
                }
                Some(_) => {}
                None => tracing::warn!(key, "unexpected key under media/; left alone"),
            }
        }
        let mut removed = 0;
        for (tenant_id, candidates) in by_tenant {
            let Ok(tenant) = databases.tenant(tenant_id).await else {
                // Khách không còn trong sổ (hay chưa mở được database của nó): không đoán.
                continue;
            };
            for chunk in candidates.chunks(BATCH) {
                let ids: Vec<Uuid> = chunk.iter().map(|(_, item, _)| *item).collect();
                // Kết nối ghi: dòng vừa lưu chưa tới read replica thì vẫn thấy.
                let current: HashMap<Uuid, Uuid> = Entity::find()
                    .scoped(&tenant)
                    .select_only()
                    .column(Column::Id)
                    .column(Column::VersionId)
                    .filter(Column::Id.is_in(ids))
                    .into_tuple::<(Uuid, Uuid)>()
                    .all(tenant.database().write())
                    .await?
                    .into_iter()
                    .collect();
                for (key, item, version) in chunk {
                    if current.get(item) == Some(version) {
                        continue;
                    }
                    self.storage.delete(key).await?;
                    removed += 1;
                }
            }
        }
        if removed > 0 {
            tracing::info!(keys = removed, "orphaned media content removed");
        }
        Ok(removed)
    }
}

/// Khoá thật ở nơi lưu chung → khách, ID item và phiên bản. Khoá của khách mặc định:
/// `media/<item_id>/<version_id>` (bản gốc) hay `media/<item_id>/<version_id>_w<rộng>[.webp]`
/// (ảnh thu nhỏ); của khách khác: cùng dạng sau `tenants/<khách>/`.
fn parse_key(key: &str) -> Option<(TenantId, Uuid, Uuid)> {
    if key.starts_with(KEY_PREFIX) {
        let (item, version) = parse_media_key(key)?;
        return Some((DEFAULT_TENANT, item, version));
    }
    let (tenant, rest) = key.strip_prefix("tenants/")?.split_once('/')?;
    let (item, version) = parse_media_key(rest)?;
    Some((TenantId::from_uuid(Uuid::parse_str(tenant).ok()?), item, version))
}

/// Khoá `media/…` (không tiền tố khách) → ID item và phiên bản.
fn parse_media_key(key: &str) -> Option<(Uuid, Uuid)> {
    let (item, rest) = key.strip_prefix(KEY_PREFIX)?.split_once('/')?;
    let rest = rest.strip_suffix(".webp").unwrap_or(rest);
    let version = match rest.split_once("_w") {
        Some((version, width))
            if !width.is_empty() && width.bytes().all(|b| b.is_ascii_digit()) =>
        {
            version
        }
        Some(_) => return None,
        None => rest,
    };
    Some((Uuid::parse_str(item).ok()?, Uuid::parse_str(version).ok()?))
}

/// Thời điểm tạo ghi trong UUIDv7.
fn created_at(id: Uuid) -> Option<DateTime<Utc>> {
    let (seconds, nanos) = id.get_timestamp()?.to_unix();
    DateTime::from_timestamp(i64::try_from(seconds).ok()?, nanos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_khoa_va_tuoi() {
        let (item, version) = (Uuid::now_v7(), Uuid::now_v7());
        let main = Some((DEFAULT_TENANT, item, version));
        assert_eq!(parse_key(&format!("media/{item}/{version}")), main);
        assert_eq!(parse_key(&format!("media/{item}/{version}_w640")), main);
        assert_eq!(parse_key(&format!("media/{item}/{version}_w640.webp")), main);
        assert_eq!(parse_key(&format!("media/{item}/{version}_wx")), None);
        assert_eq!(parse_key("files/x"), None);
        assert_eq!(parse_key("media/x/y"), None);
        let other = Uuid::now_v7();
        assert_eq!(
            parse_key(&format!("tenants/{other}/media/{item}/{version}_w640.webp")),
            Some((TenantId::from_uuid(other), item, version))
        );
        assert_eq!(parse_key(&format!("tenants/{other}/files/{item}")), None);
        assert_eq!(parse_key(&format!("tenants/x/media/{item}/{version}")), None);
        let age = Utc::now() - created_at(version).unwrap();
        assert!(age.num_seconds().abs() < 5);
    }
}
