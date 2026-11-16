//! Lệnh `autoxide task media.refresh_variants` (D-169): sau khi đổi
//! `AUTOXIDE_MEDIA_IMAGE_WIDTHS`, ảnh đã tải lên trước đó vẫn giữ bộ ảnh thu nhỏ cũ. Lệnh này tìm
//! các ảnh có bộ ảnh thu nhỏ không còn khớp cấu hình — hay thiếu bản WebP (ảnh làm trước D-196)
//! — và xếp job làm lại (job sẵn có, D-093).
//!
//! Theo khách (bước 33): lệnh duyệt mọi database máy chủ này chạy job cho, và trên mỗi database
//! từng khách có ảnh; job xếp vào hàng đợi của đúng khách.

use std::collections::BTreeSet;
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::job::EnqueueOptions;
use autoxide_kernel::task::{Task, TaskArgs, TaskArgument, TaskRun};
use autoxide_kernel::tenancy::{DatabaseRegistry, ScopeExt, TenantContext, TenantId};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use crate::entity::item;
use crate::images::variants::{MakeVariants, SOURCES, variants_of, wanted_widths};
use crate::service::{Item, MediaService};

/// Số item đọc mỗi lượt: đủ lớn để ít câu query, đủ nhỏ để transaction ghi ngắn.
const BATCH: u64 = 200;

pub(crate) fn task(service: Arc<MediaService>, databases: DatabaseRegistry) -> Task {
    Task::new(
        "refresh_variants",
        "Queue new smaller copies for every image whose copies no longer match \
         AUTOXIDE_MEDIA_IMAGE_WIDTHS (run it after changing that setting) or have no WebP copy \
         yet (images processed before WebP copies were added). The worker makes them in the background; \
         nothing else changes.",
        Arc::new(Refresh { service, databases }),
    )
    .argument(TaskArgument::optional(
        "dry_run",
        "`true` to only count the images that need new copies.",
    ))
}

struct Refresh {
    service: Arc<MediaService>,
    databases: DatabaseRegistry,
}

#[async_trait::async_trait]
impl TaskRun for Refresh {
    async fn run(&self, args: &TaskArgs) -> Result<String, Error> {
        let dry_run = args.flag("dry_run", false)?;
        let (checked, stale) = self.service.refresh_variants(&self.databases, dry_run).await?;
        Ok(if dry_run {
            format!(
                "{stale} of {checked} image(s) need new smaller copies; run again without \
                 `dry_run=true` to queue them."
            )
        } else {
            format!("Queued new smaller copies for {stale} of {checked} image(s).")
        })
    }
}

impl MediaService {
    /// Duyệt mọi ảnh làm được ảnh thu nhỏ của mọi khách trên mọi database; xếp job cho ảnh lệch
    /// cấu hình (trừ khi `dry_run`). Trả (số ảnh đã xem, số ảnh lệch).
    pub(crate) async fn refresh_variants(
        &self,
        databases: &DatabaseRegistry,
        dry_run: bool,
    ) -> Result<(usize, usize), Error> {
        let (mut checked, mut stale) = (0, 0);
        for (_, name, database) in databases.databases().await? {
            // Khách có ảnh trên database này.
            let tenants: Vec<Uuid> = item::Entity::find()
                .select_only()
                .column(item::Column::TenantId)
                .distinct()
                .into_tuple::<Uuid>()
                .all(database.read())
                .await?;
            for id in tenants {
                match databases.tenant(TenantId::from_uuid(id)).await {
                    Ok(tenant) => {
                        let (seen, outdated) = self.refresh_tenant(&tenant, dry_run).await?;
                        checked += seen;
                        stale += outdated;
                    }
                    Err(error) => {
                        tracing::warn!(tenant_id = %id, database = name, error = %error, "media variants refresh skipped a tenant");
                    }
                }
            }
        }
        Ok((checked, stale))
    }

    /// Phần của khách `tenant`: duyệt ảnh theo lô, xếp job cho ảnh lệch cấu hình.
    async fn refresh_tenant(
        &self,
        tenant: &TenantContext,
        dry_run: bool,
    ) -> Result<(usize, usize), Error> {
        let mut checked = 0;
        let mut stale = 0;
        let mut after = None;
        loop {
            let mut query = item::Entity::find()
                .scoped(tenant)
                .filter(item::Column::ContentType.is_in(SOURCES))
                .order_by_asc(item::Column::Id)
                .limit(BATCH);
            if let Some(after) = after {
                query = query.filter(item::Column::Id.gt(after));
            }
            let items = query.all(tenant.database().read()).await?;
            let Some(last) = items.last() else {
                break;
            };
            after = Some(last.id);
            let made = variants_of(tenant.database().read(), tenant, &items).await?;
            let outdated: Vec<&Item> = items
                .iter()
                .filter(|item| {
                    let rows = made.get(&item.id).map(Vec::as_slice).unwrap_or_default();
                    let saved: BTreeSet<i32> =
                        rows.iter().filter(|row| !row.is_webp()).map(|row| row.width).collect();
                    let webp: BTreeSet<i32> =
                        rows.iter().filter(|row| row.is_webp()).map(|row| row.width).collect();
                    let wanted = wanted_widths(&self.config.image_widths, item);
                    // Ảnh làm trước khi có bản WebP (D-196): cỡ nào thiếu WebP thì làm lại cả bộ.
                    needs_refresh(&wanted, &saved, item) || saved != webp
                })
                .collect();
            checked += items.len();
            stale += outdated.len();
            if !dry_run && !outdated.is_empty() {
                // Một transaction ngắn mỗi lô: chỉ ghi dòng job.
                let transaction = tenant.database().begin_write().await?;
                for item in outdated {
                    let job = MakeVariants { item_id: item.id, version_id: item.version_id };
                    // Chạy lệnh hai lần trước khi worker kịp làm: không xếp trùng.
                    let options = EnqueueOptions::default()
                        .unique_key(format!("refresh:{}:{}", item.id, item.version_id));
                    self.jobs.enqueue(&transaction, tenant, &job, options).await?;
                }
                transaction.commit().await?;
            }
        }
        Ok((checked, stale))
    }
}

/// Ảnh cần làm lại khi: còn cỡ cần làm, và có ảnh thu nhỏ ở cỡ không còn trong cấu hình, hoặc
/// thiếu một cỡ chắc chắn làm được (nhỏ hơn cả hai cạnh — xoay theo EXIF cũng vẫn nhỏ hơn
/// chiều rộng thật). Cỡ nằm giữa hai cạnh có thể bị bỏ khi làm, nên không tính là thiếu.
fn needs_refresh(wanted: &[u32], saved: &BTreeSet<i32>, item: &Item) -> bool {
    if wanted.is_empty() {
        return false;
    }
    let wanted_set: BTreeSet<i32> =
        wanted.iter().filter_map(|width| i32::try_from(*width).ok()).collect();
    if saved.iter().any(|width| !wanted_set.contains(width)) {
        return true;
    }
    let shortest = item.width.unwrap_or(0).min(item.height.unwrap_or(0));
    wanted_set.iter().any(|width| *width < shortest && !saved.contains(width))
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::event::Outbox;
    use autoxide_kernel::job::JobQueue;
    use autoxide_testkit::TestDatabase;
    use sea_orm::{ConnectionTrait, Statement};

    use super::*;
    use image::ImageFormat;

    use crate::images::variants::samples::encoded;
    use crate::library::upload::NewUpload;
    use crate::testing::{body, editor, limits, service};

    /// Cùng database và nơi lưu, cấu hình cỡ khác (như máy chủ khởi động lại sau khi đổi
    /// `AUTOXIDE_MEDIA_IMAGE_WIDTHS`).
    fn with_widths(media: &MediaService, widths: &[u32]) -> MediaService {
        let mut config = media.config.clone();
        config.image_widths = widths.to_vec();
        MediaService::new(
            media.storage.clone(),
            Outbox::standalone("media"),
            JobQueue::new("media"),
            media.ai.clone(),
            config,
        )
    }

    async fn queued(database: &autoxide_kernel::database::Database) -> i64 {
        let backend = database.read().get_database_backend();
        let sql = "SELECT COUNT(*) AS n FROM media_jobs WHERE kind = 'media.make_variants' \
                   AND completed_at IS NULL";
        let row = database.read().query_one_raw(Statement::from_string(backend, sql));
        let row = row.await.unwrap().unwrap();
        row.try_get::<i64>("", "n").unwrap()
    }

    fn upload(name: &str, content_type: &str) -> NewUpload {
        NewUpload {
            org_id: None,
            name: name.to_owned(),
            folder_id: None,
            content_type: Some(content_type.to_owned()),
            tags: Vec::new(),
        }
    }

    /// Đổi cấu hình cỡ → lệnh thấy ảnh lệch, xếp đúng một job (chạy lại không xếp trùng); làm
    /// xong thì không còn gì lệch. Ảnh quá nhỏ không tính — trên cả hai database.
    #[tokio::test]
    async fn doi_co_anh_thi_xep_lam_lai() {
        TestDatabase::run(async |test| {
            let (media, _, database) = service(test, limits()).await;
            let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
            let registry = DatabaseRegistry::single(database.clone());
            let ada = editor();
            let photo = media
                .upload(
                    &tenant,
                    &ada,
                    upload("photo.png", "image/png"),
                    body(&encoded(300, 240, ImageFormat::Png, false)),
                )
                .await
                .unwrap()
                .item;
            let icon = upload("icon.gif", "image/gif");
            media
                .upload(&tenant, &ada, icon, body(&encoded(32, 16, ImageFormat::Gif, false)))
                .await
                .unwrap();
            // Cấu hình của `testing::service`: 100 và 200.
            assert_eq!(media.make_variants(&tenant, photo.id, photo.version_id).await.unwrap(), 2);
            let before = queued(&database).await;
            assert_eq!(media.refresh_variants(&registry, true).await.unwrap(), (2, 0));

            let wider = with_widths(&media, &[100, 150, 200]);
            assert_eq!(wider.refresh_variants(&registry, true).await.unwrap(), (2, 1));
            assert_eq!(queued(&database).await, before, "dry run queues nothing");
            assert_eq!(wider.refresh_variants(&registry, false).await.unwrap(), (2, 1));
            assert_eq!(wider.refresh_variants(&registry, false).await.unwrap(), (2, 1));
            assert_eq!(queued(&database).await, before + 1, "the same image is queued once");

            assert_eq!(wider.make_variants(&tenant, photo.id, photo.version_id).await.unwrap(), 3);
            assert_eq!(wider.refresh_variants(&registry, true).await.unwrap(), (2, 0));

            // Bỏ bớt cỡ: ảnh thu nhỏ ở cỡ đã bỏ là lệch.
            let narrower = with_widths(&media, &[100]);
            assert_eq!(narrower.refresh_variants(&registry, true).await.unwrap(), (2, 1));
        })
        .await
        .unwrap();
    }

    /// Ảnh làm trước khi có bản WebP (chỉ có JPEG / PNG): lệnh thấy thiếu, xếp làm lại; làm
    /// xong thì đủ — trên cả hai database.
    #[tokio::test]
    async fn anh_chua_co_webp_thi_xep_lam_lai() {
        TestDatabase::run(async |test| {
            let (media, _, database) = service(test, limits()).await;
            let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
            let registry = DatabaseRegistry::single(database.clone());
            let photo = media
                .upload(
                    &tenant,
                    &editor(),
                    upload("photo.png", "image/png"),
                    body(&encoded(300, 240, ImageFormat::Png, false)),
                )
                .await
                .unwrap()
                .item;
            assert_eq!(media.make_variants(&tenant, photo.id, photo.version_id).await.unwrap(), 2);
            assert_eq!(media.refresh_variants(&registry, true).await.unwrap(), (1, 0));

            let backend = database.write().get_database_backend();
            let sql = "DELETE FROM media_variant WHERE content_type = 'image/webp'";
            let deleted = database.write().execute_raw(Statement::from_string(backend, sql));
            assert_eq!(deleted.await.unwrap().rows_affected(), 2);
            assert_eq!(media.refresh_variants(&registry, true).await.unwrap(), (1, 1));

            assert_eq!(media.make_variants(&tenant, photo.id, photo.version_id).await.unwrap(), 2);
            assert_eq!(media.refresh_variants(&registry, true).await.unwrap(), (1, 0));
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn lenh_bao_ket_qua_bang_loi() {
        TestDatabase::run(async |test| {
            let (media, _, database) = service(test, limits()).await;
            let task = task(media, DatabaseRegistry::single(database));
            let args = task.parse(&["dry_run=true".to_owned()]).unwrap();
            let report = task.run(&args).await.unwrap();
            assert_eq!(
                report,
                "0 of 0 image(s) need new smaller copies; run again without `dry_run=true` to \
                 queue them."
            );
            let args = task.parse(&[]).unwrap();
            assert_eq!(
                task.run(&args).await.unwrap(),
                "Queued new smaller copies for 0 of 0 image(s)."
            );
        })
        .await
        .unwrap();
    }
}
