//! Bản AVIF của ảnh thu nhỏ (bước 39 B8, D-20261119-step39-avif; đo trước ở D-232).
//!
//! - Chỉ khi `AUTOXIDE_MEDIA_AVIF=true` (mặc định tắt): job `media.make_avif` được xếp ngay sau khi
//!   `media.make_variants` lưu xong ảnh thu nhỏ — không bao giờ trên đường request tải lên hay trong
//!   job ảnh thu nhỏ (mã hoá AVIF chậm hơn WebP 20–40 lần). `unique_key = "avif:{item}:{version}"`.
//! - Mã hoá từ ảnh gốc đã xoay theo EXIF (không từ bản JPEG đã nén — mất chất lượng hai lần), mỗi cỡ
//!   đã có bản JPEG / PNG một bản AVIF, cùng giới hạn [`MAX_PIXELS`](super::variants::MAX_PIXELS);
//!   chạy trong `spawn_blocking`, một thread mỗi job.
//! - Lưu dòng `media_variant` `image/avif` + file `…_w<rộng>.avif` trong một transaction; chạy lại
//!   (job xếp hai lần, hai worker) thay các dòng AVIF cũ của cùng phiên bản — mỗi cỡ một bản (unique
//!   index của `m0007`). Ảnh hỏng: ghi log, kết thúc `Ok`, không thử lại.
//! - Tắt cấu hình thì không xếp job mới; bản AVIF đã có vẫn phục vụ.

use autoxide_kernel::error::Error;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::rows;
use autoxide_kernel::storage::Storage as _;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use bytes::Bytes;
use image::DynamicImage;
use image::imageops::FilterType;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entity::item;
use crate::entity::variant::{self, AVIF, WEBP, storage_key};
use crate::images::variants::{Rendered, decode, read_all};
use crate::service::{Item, MediaService, now};

pub(crate) const MAKE_AVIF: &str = "media.make_avif";

/// Chất lượng AVIF (0–100 của `ravif`): ở 75, cỡ nhỏ hơn WebP 80 cùng điểm DSSIM (đo ở bước 39).
const AVIF_QUALITY: f32 = 75.0;
/// Tốc độ mã hoá (1 chậm, nén tốt — 10 nhanh); 6 là mức đo trong `docs/BENCHMARKS.md`.
const AVIF_SPEED: u8 = 6;

/// Job: làm bản AVIF cho phiên bản `version_id` của item `item_id`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MakeAvif {
    pub item_id: Uuid,
    pub version_id: Uuid,
}

impl Job for MakeAvif {
    const KIND: &'static str = MAKE_AVIF;
}

pub(crate) struct AvifGenerator(pub std::sync::Arc<MediaService>);

#[async_trait::async_trait]
impl JobHandler for AvifGenerator {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let input: MakeAvif = serde_json::from_value(job.payload.clone())
            .map_err(|error| Error::internal(format!("media avif job payload: {error}")))?;
        let tenant =
            job.tenant.as_ref().ok_or_else(|| Error::internal("media avif job has no tenant"))?;
        self.0.make_avif(tenant, input.item_id, input.version_id).await.map(|_| ())
    }
}

impl MediaService {
    /// Xếp job làm bản AVIF của phiên bản hiện tại của `item` (nếu bật `AUTOXIDE_MEDIA_AVIF`).
    /// Xếp lại khi đã có job cùng khoá thì không thêm job mới.
    pub(crate) async fn enqueue_avif<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        item_id: Uuid,
        version_id: Uuid,
    ) -> Result<(), Error> {
        if !self.config.avif {
            return Ok(());
        }
        let options = EnqueueOptions::default().unique_key(format!("avif:{item_id}:{version_id}"));
        self.jobs.enqueue(connection, tenant, &MakeAvif { item_id, version_id }, options).await?;
        Ok(())
    }

    /// Làm bản AVIF cho mỗi cỡ ảnh thu nhỏ đã có của phiên bản `version_id`; trả số bản đã làm.
    /// Item đã đổi nội dung, không còn, hay chưa có ảnh thu nhỏ: không làm gì.
    pub(crate) async fn make_avif(
        &self,
        tenant: &TenantContext,
        item_id: Uuid,
        version_id: Uuid,
    ) -> Result<usize, Error> {
        let found = item::Entity::find_by_id(item_id).scoped(tenant).one(tenant.database().write());
        let Some(item) = found.await? else {
            return Ok(0);
        };
        if item.version_id != version_id {
            return Ok(0);
        }
        let mut widths: Vec<u32> = variant::Entity::find()
            .scoped(tenant)
            .filter(variant::Column::ItemId.eq(item_id))
            .filter(variant::Column::VersionId.eq(version_id))
            .filter(variant::Column::ContentType.is_not_in([WEBP, AVIF]))
            .all(tenant.database().read())
            .await?
            .into_iter()
            .filter_map(|copy| u32::try_from(copy.width).ok())
            .collect();
        widths.sort_unstable();
        widths.dedup();
        if widths.is_empty() {
            return Ok(0);
        }
        let original = read_all(self.read(tenant, &item, None).await?).await?;
        let rendered = tokio::task::spawn_blocking(move || render_avif(&original, &widths))
            .await
            .map_err(Error::internal)?;
        let rendered = match rendered {
            Ok(rendered) => rendered,
            Err(problem) => {
                // Ảnh hỏng không lành lại khi thử lại: ghi log rồi thôi.
                tracing::warn!(media_item_id = %item_id, problem, "cannot make AVIF copies of an image");
                return Ok(0);
            }
        };
        let store = self.store(tenant);
        let mut written = Vec::new();
        for copy in &rendered {
            let width = i32::try_from(copy.width).map_err(Error::internal)?;
            let key = storage_key(item_id, version_id, width, AVIF);
            let body = futures_util::stream::iter([Ok(Bytes::from(copy.bytes.clone()))]);
            store.put(&key, Box::pin(body)).await?;
            written.push(key);
        }
        let saved = self.save_avif(tenant, &item, &rendered).await;
        if !matches!(saved, Ok(true)) {
            // Nội dung vừa đổi (hay lỗi): bản vừa làm không còn ai dùng.
            for key in &written {
                if let Err(error) = store.delete(key).await {
                    tracing::warn!(key, error = %error, "cannot remove an unused AVIF copy");
                }
            }
        }
        saved?;
        tracing::info!(media_item_id = %item_id, sizes = rendered.len(), "AVIF copies of an image made");
        Ok(rendered.len())
    }

    /// Lưu dòng của các bản AVIF nếu item còn ở phiên bản đó; thay bản AVIF cũ của cùng phiên bản.
    async fn save_avif(
        &self,
        tenant: &TenantContext,
        item: &Item,
        rendered: &[Rendered],
    ) -> Result<bool, Error> {
        let transaction = tenant.database().begin_write().await?;
        // Khoá dòng item: lệnh thay nội dung chạy cùng lúc phải chờ; hai worker cùng job thì lần
        // sau thấy và thay dòng của lần trước.
        let current = item::Entity::find_by_id(item.id)
            .scoped(tenant)
            .filter(item::Column::VersionId.eq(item.version_id));
        if rows::lock_one(&transaction, current).await?.is_none() {
            transaction.rollback().await?;
            return Ok(false);
        }
        variant::Entity::delete_many()
            .scoped(tenant)
            .filter(variant::Column::ItemId.eq(item.id))
            .filter(variant::Column::VersionId.eq(item.version_id))
            .filter(variant::Column::ContentType.eq(AVIF))
            .exec(&transaction)
            .await?;
        let now = now();
        for copy in rendered {
            variant::Entity::insert(variant::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                item_id: Set(item.id),
                version_id: Set(item.version_id),
                width: Set(i32::try_from(copy.width).map_err(Error::internal)?),
                height: Set(i32::try_from(copy.height).map_err(Error::internal)?),
                content_type: Set(AVIF.to_owned()),
                size: Set(i64::try_from(copy.bytes.len()).map_err(Error::internal)?),
                created_at: Set(now),
            })
            .exec_without_returning(&transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(true)
    }
}

/// Giải mã `original` (xoay theo EXIF, có giới hạn), làm một bản AVIF cho mỗi chiều rộng.
fn render_avif(original: &[u8], widths: &[u32]) -> Result<Vec<Rendered>, String> {
    let image = decode(original)?;
    let transparent = image.color().has_alpha();
    let mut rendered = Vec::new();
    for &wanted in widths.iter().filter(|wanted| **wanted < image.width()) {
        let smaller = image.resize(wanted, u32::MAX, FilterType::CatmullRom);
        let bytes = encode_avif(&smaller, transparent)?;
        rendered.push(Rendered {
            width: smaller.width(),
            height: smaller.height(),
            content_type: AVIF,
            bytes,
        });
    }
    Ok(rendered)
}

/// Mã hoá AVIF; ảnh có kênh trong suốt giữ kênh đó.
fn encode_avif(image: &DynamicImage, transparent: bool) -> Result<Vec<u8>, String> {
    let encoder = ravif::Encoder::new()
        .with_quality(AVIF_QUALITY)
        .with_alpha_quality(AVIF_QUALITY)
        .with_speed(AVIF_SPEED)
        .with_num_threads(Some(1));
    let width = usize::try_from(image.width()).map_err(|error| error.to_string())?;
    let height = usize::try_from(image.height()).map_err(|error| error.to_string())?;
    let encoded = if transparent {
        let pixels: Vec<ravif::RGBA8> =
            image.to_rgba8().pixels().map(|p| ravif::RGBA8::new(p[0], p[1], p[2], p[3])).collect();
        encoder.encode_rgba(ravif::Img::new(&pixels[..], width, height))
    } else {
        let pixels: Vec<ravif::RGB8> =
            image.to_rgb8().pixels().map(|p| ravif::RGB8::new(p[0], p[1], p[2])).collect();
        encoder.encode_rgb(ravif::Img::new(&pixels[..], width, height))
    };
    encoded.map(|done| done.avif_file).map_err(|error| format!("avif: {error}"))
}
