//! `autoxide task media.bench_seed` (bước 39 B1): sinh ảnh công khai kèm bản nhỏ cho bài đo trên
//! dữ liệu cỡ thật.
//!
//! Chỉ biên dịch khi bật cargo feature `bench-seed` (và trong test). Vài ảnh mẫu được làm MỘT lần
//! (mã hoá JPEG, rồi làm bản nhỏ JPEG + WebP bằng đúng hàm `render` của job ảnh thu nhỏ); mỗi ảnh
//! giả là một bản chép của một ảnh mẫu: file gốc và file bản nhỏ ghi thật vào Storage theo đúng khoá
//! của module, dòng `media_item` / `media_variant` ghi theo lô. Không xếp job, không phát event:
//! bản nhỏ đã có sẵn, dữ liệu giả không cần audit.

use std::io::Cursor;
use std::sync::Arc;

use autoxide_kernel::bench_seed::{self, Pseudo, SeedArgs};
use autoxide_kernel::error::Error;
use autoxide_kernel::storage::Storage as _;
use autoxide_kernel::task::{Task, TaskArgs, TaskRun};
use autoxide_kernel::tenancy::{DatabaseRegistry, ScopeExt, TenantContext, root_of};
use bytes::Bytes;
use chrono::{DateTime, Duration, SubsecRound, Utc};
use futures_util::{StreamExt as _, TryStreamExt as _};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use sea_orm::{EntityTrait, PaginatorTrait, Set};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::entity::{ROOT, item, variant};
use crate::images::variants::{Rendered, render, wanted_widths};
use crate::service::{MediaService, Visibility};

/// Khoá trong file ID.
pub(crate) const IDS_KEY: &str = bench_seed::MEDIA_KEY;

/// Cỡ ảnh mẫu: nhỏ để 50 nghìn bản chép không chiếm nhiều đĩa, vẫn lớn hơn cỡ nhỏ nhất mặc định
/// (320) nên có bản nhỏ.
const SAMPLE_WIDTH: u32 = 480;
const SAMPLE_HEIGHT: u32 = 360;
const SAMPLES: u8 = 4;

/// Số file ghi song song vào Storage.
const PARALLEL_WRITES: usize = 32;

pub(crate) fn task(service: Arc<MediaService>, databases: DatabaseRegistry) -> Task {
    bench_seed::arguments().into_iter().fold(
        Task::new(
            "bench_seed",
            "Make `count` made-up public images (default 50000) with their smaller copies for load \
             tests: files are written to the storage, rows in batches, and the ids to the `ids` \
             file. Refuses to run when the customer already has media.",
            Arc::new(Seed { service, databases }),
        ),
        Task::argument,
    )
}

struct Seed {
    service: Arc<MediaService>,
    databases: DatabaseRegistry,
}

/// Một ảnh mẫu đã mã hoá cùng các bản nhỏ của nó.
struct Sample {
    original: Bytes,
    sha256: String,
    copies: Vec<(Rendered, Bytes)>,
}

#[async_trait::async_trait]
impl TaskRun for Seed {
    async fn run(&self, args: &TaskArgs) -> Result<String, Error> {
        let args = SeedArgs::parse(args, 50_000)?;
        let tenant = args.tenant(&self.databases).await?;
        let existing = item::Entity::find().scoped(&tenant).count(tenant.database().read()).await?;
        if existing > 0 {
            return Err(bench_seed::refused(&format!(
                "the customer already has {existing} media items; seed only a fresh database"
            )));
        }
        let widths = self.service.config.image_widths.clone();
        let samples = tokio::task::spawn_blocking(move || samples(&widths))
            .await
            .map_err(Error::internal)??;
        let ids = self.seed(&tenant, &samples, args.count).await?;
        bench_seed::write_ids(&args.ids, IDS_KEY, &ids)?;
        Ok(format!(
            "Made {} public images with smaller copies; ids written to `{}`.",
            ids.len(),
            args.ids.display()
        ))
    }
}

impl Seed {
    async fn seed(
        &self,
        tenant: &TenantContext,
        samples: &[Sample],
        count: u64,
    ) -> Result<Vec<Uuid>, Error> {
        let mut random = Pseudo::new(0x3ED1A);
        let now = Utc::now().trunc_subsecs(6);
        let org = root_of(tenant.id()).as_uuid();
        let (mut items, mut copies, mut files) = (Vec::new(), Vec::new(), Vec::new());
        let mut ids = Vec::new();
        for index in 0..count {
            let sample_index = usize::try_from(index % u64::from(SAMPLES)).unwrap_or(0);
            let Some(sample) = samples.get(sample_index) else { continue };
            let (id, version) = (Uuid::now_v7(), Uuid::now_v7());
            let at = now - Duration::minutes(i64::try_from(random.below(1_051_200)).unwrap_or(0));
            let name = format!("{}-{index}.jpg", random.sentence(2).replace(' ', "-"));
            items.push(item::ActiveModel {
                id: Set(id),
                tenant_id: Set(tenant.id().as_uuid()),
                folder_id: Set(ROOT),
                name_key: Set(autoxide_kernel::search::fold(&name)),
                name: Set(name),
                content_type: Set("image/jpeg".to_owned()),
                size: Set(i64::try_from(sample.original.len()).map_err(Error::internal)?),
                sha256: Set(sample.sha256.clone()),
                width: Set(Some(i32::try_from(SAMPLE_WIDTH).map_err(Error::internal)?)),
                height: Set(Some(i32::try_from(SAMPLE_HEIGHT).map_err(Error::internal)?)),
                alt: Set(random.sentence(5)),
                title: Set(String::new()),
                caption: Set(String::new()),
                decorative: Set(false),
                alt_ai_assisted: Set(false),
                visibility: Set(Visibility::Public.as_str().to_owned()),
                version_id: Set(version),
                uploaded_by: Set(None),
                org_id: Set(Some(org)),
                created_at: Set(at),
                updated_at: Set(at),
                trashed_at: Set(None),
            });
            files.push((item::storage_key(id, version), sample.original.clone()));
            for (copy, bytes) in &sample.copies {
                let width = i32::try_from(copy.width).map_err(Error::internal)?;
                copies.push(variant::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    item_id: Set(id),
                    version_id: Set(version),
                    width: Set(width),
                    height: Set(i32::try_from(copy.height).map_err(Error::internal)?),
                    content_type: Set(copy.content_type.to_owned()),
                    size: Set(i64::try_from(bytes.len()).map_err(Error::internal)?),
                    created_at: Set(at),
                });
                files.push((
                    variant::storage_key(id, version, width, copy.content_type),
                    bytes.clone(),
                ));
            }
            ids.push(id);
            // Ghi từng lô: file trước, dòng sau — dòng không bao giờ trỏ tới file chưa có.
            if items.len() >= bench_seed::ROWS_PER_TRANSACTION {
                self.flush(tenant, &mut items, &mut copies, &mut files).await?;
            }
        }
        self.flush(tenant, &mut items, &mut copies, &mut files).await?;
        Ok(ids)
    }

    async fn flush(
        &self,
        tenant: &TenantContext,
        items: &mut Vec<item::ActiveModel>,
        copies: &mut Vec<variant::ActiveModel>,
        files: &mut Vec<(String, Bytes)>,
    ) -> Result<(), Error> {
        let store = self.service.store(tenant);
        futures_util::stream::iter(std::mem::take(files))
            .map(|(key, bytes)| {
                let store = &store;
                async move {
                    let body = futures_util::stream::iter([Ok(bytes)]);
                    store.put(&key, Box::pin(body)).await.map(|_| ())
                }
            })
            .buffer_unordered(PARALLEL_WRITES)
            .try_collect::<Vec<()>>()
            .await?;
        bench_seed::insert_all(tenant.database(), std::mem::take(items)).await?;
        bench_seed::insert_all(tenant.database(), std::mem::take(copies)).await
    }
}

/// Các ảnh mẫu, mỗi ảnh một kiểu màu, kèm bản nhỏ theo `widths`.
fn samples(widths: &[u32]) -> Result<Vec<Sample>, Error> {
    (0..SAMPLES)
        .map(|index| {
            let image =
                DynamicImage::ImageRgb8(RgbImage::from_fn(SAMPLE_WIDTH, SAMPLE_HEIGHT, |x, y| {
                    let x = u8::try_from(x * 255 / SAMPLE_WIDTH).unwrap_or(0);
                    let y = u8::try_from(y * 255 / SAMPLE_HEIGHT).unwrap_or(0);
                    Rgb([x, y, index.wrapping_mul(60)])
                }));
            let mut original = Vec::new();
            image
                .write_to(&mut Cursor::new(&mut original), ImageFormat::Jpeg)
                .map_err(Error::internal)?;
            let probe = item::Model {
                width: Some(i32::try_from(SAMPLE_WIDTH).map_err(Error::internal)?),
                height: Some(i32::try_from(SAMPLE_HEIGHT).map_err(Error::internal)?),
                content_type: "image/jpeg".to_owned(),
                size: i64::try_from(original.len()).map_err(Error::internal)?,
                ..probe_item()
            };
            let copies = render(&original, &wanted_widths(widths, &probe))
                .map_err(Error::internal)?
                .into_iter()
                .map(|copy| {
                    let bytes = Bytes::from(copy.bytes.clone());
                    (copy, bytes)
                })
                .collect();
            let sha256 =
                Sha256::digest(&original).iter().map(|byte| format!("{byte:02x}")).collect();
            Ok(Sample { original: Bytes::from(original), sha256, copies })
        })
        .collect()
}

/// Dòng item rỗng chỉ để hỏi `wanted_widths` cỡ nào cần làm.
fn probe_item() -> item::Model {
    let epoch = DateTime::<Utc>::UNIX_EPOCH;
    item::Model {
        id: Uuid::nil(),
        tenant_id: Uuid::nil(),
        folder_id: ROOT,
        name: String::new(),
        name_key: String::new(),
        content_type: String::new(),
        size: 0,
        sha256: String::new(),
        width: None,
        height: None,
        alt: String::new(),
        title: String::new(),
        caption: String::new(),
        decorative: false,
        alt_ai_assisted: false,
        visibility: String::new(),
        version_id: Uuid::nil(),
        uploaded_by: None,
        org_id: None,
        created_at: epoch,
        updated_at: epoch,
        trashed_at: None,
    }
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::bench_seed::CONFIRM;
    use autoxide_testkit::TestDatabase;

    use super::*;

    #[tokio::test]
    async fn sinh_anh_kem_ban_nho_doc_duoc() {
        TestDatabase::run(async |test| {
            let (service, _, database) =
                crate::testing::service(test, crate::testing::limits()).await;
            let tenant = TenantContext::default_on(database.clone());
            let ids = std::env::temp_dir().join(format!("autoxide-bench-{}.json", Uuid::now_v7()));
            let task = task(service.clone(), DatabaseRegistry::single(tenant.database().clone()));
            let raw = [
                format!("confirm={CONFIRM}"),
                "count=1003".to_owned(),
                format!("ids={}", ids.display()),
            ];
            let report = task.run(&task.parse(&raw).unwrap()).await.unwrap();
            assert!(report.starts_with("Made 1003 public images"), "{report}");
            let made = bench_seed::read_ids(&ids, IDS_KEY).unwrap();
            assert_eq!(made.len(), 1003);
            let database = tenant.database().read();
            assert_eq!(item::Entity::find().count(database).await.unwrap(), 1003);
            // Mỗi ảnh có bản nhỏ (JPEG + WebP mỗi cỡ dưới 480).
            let copies = variant::Entity::find().count(database).await.unwrap();
            assert_eq!(copies % 1003, 0);
            assert!(copies > 0);
            // File gốc và bản nhỏ đọc lại được qua Storage của module.
            let last = item::Entity::find_by_id(made[1002]).one(database).await.unwrap().unwrap();
            let mut body =
                service.store(&tenant).get(&last.storage_key()).await.unwrap().unwrap().content;
            let mut read = 0;
            while let Some(chunk) = body.next().await {
                read += chunk.unwrap().len();
            }
            assert_eq!(i64::try_from(read).unwrap(), last.size);
            // Lần hai: đã có ảnh → từ chối.
            let error = task.run(&task.parse(&raw).unwrap()).await.unwrap_err();
            assert_eq!(error.code(), "bench_seed_refused");
            std::fs::remove_file(ids).unwrap();
        })
        .await
        .unwrap();
    }
}
