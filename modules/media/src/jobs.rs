//! Job định kỳ của module `media` (chạy trên job queue của core, mỗi ngày một lần):
//!
//! - [`PURGE_TRASH`]: xoá hẳn item đã nằm trong thùng rác quá `AUTOXIDE_MEDIA_TRASH_DAYS`;
//! - [`CLEAN_ORPHANS`]: xoá nội dung dưới `media/` mà không item nào trỏ tới (xoá hỏng giữa
//!   chừng, máy chủ tắt giữa lúc tải lên). Chỉ xoá khoá cũ hơn [`ORPHAN_GRACE_HOURS`] giờ —
//!   tuổi đọc từ `version_id` (UUIDv7 mang thời điểm tạo) — để không xoá upload đang dở.
//!   Không bao giờ đọc hay xoá khoá ngoài `media/`.

use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{JobContext, JobHandler};
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

pub(crate) struct PurgeTrash(pub Arc<MediaService>);

#[async_trait::async_trait]
impl JobHandler for PurgeTrash {
    async fn run(&self, _: &JobContext) -> Result<(), Error> {
        self.0.purge_expired(now()).await.map(|_| ())
    }
}

pub(crate) struct CleanOrphans(pub Arc<MediaService>);

#[async_trait::async_trait]
impl JobHandler for CleanOrphans {
    async fn run(&self, _: &JobContext) -> Result<(), Error> {
        self.0.clean_orphans(now()).await.map(|_| ())
    }
}

impl MediaService {
    /// Xoá hẳn item vào thùng rác trước `now - trash_days`. Trả số item đã xoá.
    pub(crate) async fn purge_expired(&self, now: DateTime<Utc>) -> Result<u64, Error> {
        let before = now - chrono::Duration::days(i64::from(self.config.trash_days));
        let mut purged = 0;
        loop {
            let items = Entity::find()
                .filter(Column::TrashedAt.lt(before))
                .order_by_asc(Column::TrashedAt)
                .limit(BATCH as u64)
                .all(self.database.write())
                .await?;
            if items.is_empty() {
                break;
            }
            for item in &items {
                if self.purge_item(Actor::system(), item).await? {
                    purged += 1;
                }
            }
            if items.len() < BATCH {
                break;
            }
        }
        if purged > 0 {
            tracing::info!(items = purged, "expired media items purged from the trash");
        }
        Ok(purged)
    }

    /// Xoá nội dung mồ côi dưới `media/` cũ hơn `now - ORPHAN_GRACE_HOURS`. Trả số khoá đã xoá.
    pub(crate) async fn clean_orphans(&self, now: DateTime<Utc>) -> Result<u64, Error> {
        let cutoff = now - chrono::Duration::hours(ORPHAN_GRACE_HOURS);
        let keys = self.storage.keys_with_prefix(KEY_PREFIX).await?;
        let candidates: Vec<(String, Uuid, Uuid)> = keys
            .into_iter()
            .filter_map(|key| {
                let parsed = parse_key(&key);
                if parsed.is_none() {
                    tracing::warn!(key, "unexpected key under media/; left alone");
                }
                parsed.map(|(item, version)| (key, item, version))
            })
            .filter(|(_, _, version)| created_at(*version).is_some_and(|time| time < cutoff))
            .collect();
        let mut removed = 0;
        for chunk in candidates.chunks(BATCH) {
            let ids: Vec<Uuid> = chunk.iter().map(|(_, item, _)| *item).collect();
            let current: HashMap<Uuid, Uuid> = Entity::find()
                .select_only()
                .column(Column::Id)
                .column(Column::VersionId)
                .filter(Column::Id.is_in(ids))
                .into_tuple::<(Uuid, Uuid)>()
                .all(self.database.write())
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
        if removed > 0 {
            tracing::info!(keys = removed, "orphaned media content removed");
        }
        Ok(removed)
    }
}

/// `media/<item_id>/<version_id>` (bản gốc) hay `media/<item_id>/<version_id>_w<rộng>[.webp]`
/// (ảnh thu nhỏ) → ID item và phiên bản.
fn parse_key(key: &str) -> Option<(Uuid, Uuid)> {
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
        assert_eq!(parse_key(&format!("media/{item}/{version}")), Some((item, version)));
        assert_eq!(parse_key(&format!("media/{item}/{version}_w640")), Some((item, version)));
        assert_eq!(parse_key(&format!("media/{item}/{version}_w640.webp")), Some((item, version)));
        assert_eq!(parse_key(&format!("media/{item}/{version}_wx")), None);
        assert_eq!(parse_key("files/x"), None);
        assert_eq!(parse_key("media/x/y"), None);
        let age = Utc::now() - created_at(version).unwrap();
        assert!(age.num_seconds().abs() < 5);
    }
}
