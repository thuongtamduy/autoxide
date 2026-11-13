//! Ảnh thu nhỏ (D-093): sau khi tải lên hay thay nội dung, một job nền làm các bản nhỏ hơn
//! của ảnh JPEG, PNG, GIF, WebP theo các chiều rộng `AUTOXIDE_MEDIA_IMAGE_WIDTHS` (chỉ những
//! cỡ nhỏ hơn ảnh gốc). Website dùng chúng cho `srcset`.
//!
//! - Ảnh được xoay đúng chiều theo EXIF trước khi thu nhỏ; giữ tỉ lệ.
//! - Ảnh không trong suốt → JPEG (chất lượng 82); ảnh có kênh trong suốt → PNG.
//! - Ảnh quá [`MAX_PIXELS`] pixel không được giải mã (chặn "bom giải nén"); ảnh hỏng thì
//!   bỏ qua, ghi log, không thử lại mãi.
//! - Giải mã / đổi cỡ chạy trên thread riêng (`spawn_blocking`), không chặn thread async.
//! - Khoá ở nơi lưu `media/<id>/<version_id>_w<width>`; gắn với một phiên bản nội dung —
//!   thay nội dung thì bản cũ bị xoá và job làm lại cho phiên bản mới.
//! - AVIF và SVG không có ảnh thu nhỏ (không có bộ giải mã AVIF thuần Rust; SVG đã co giãn
//!   được).

use std::io::Cursor;
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::rows;
use autoxide_kernel::storage::StoredFile;
use bytes::Bytes;
use futures_util::TryStreamExt as _;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder as _, ImageFormat, ImageReader, Limits};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entity::item;
use crate::entity::variant::{self, storage_key};
use crate::service::{Item, MediaService, now};

pub(crate) const MAKE_VARIANTS: &str = "media.make_variants";

/// Ảnh lớn hơn chừng này pixel không có ảnh thu nhỏ (giải mã cần khoảng 4 byte mỗi pixel).
pub(crate) const MAX_PIXELS: u64 = 40_000_000;
/// Loại ảnh làm được ảnh thu nhỏ.
pub(crate) const SOURCES: [&str; 4] = ["image/jpeg", "image/png", "image/gif", "image/webp"];
const JPEG_QUALITY: u8 = 82;

/// Một ảnh thu nhỏ.
pub(crate) type Variant = variant::Model;

/// Job: làm ảnh thu nhỏ cho phiên bản `version_id` của item `item_id`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MakeVariants {
    pub item_id: Uuid,
    pub version_id: Uuid,
}

impl Job for MakeVariants {
    const KIND: &'static str = MAKE_VARIANTS;
}

/// Một ảnh thu nhỏ vừa làm, chưa lưu.
struct Rendered {
    width: u32,
    height: u32,
    content_type: &'static str,
    bytes: Vec<u8>,
}

/// File có cỡ hợp lý so với số pixel của nó không: ảnh nén không bao giờ lớn hơn pixel
/// thô (tối đa 8 byte mỗi pixel, ảnh 16-bit RGBA) cộng phần đầu. File lớn hơn thế là ảnh
/// nhét thêm dữ liệu — không đọc cả file vào bộ nhớ để giải mã (D-149).
pub(crate) fn plausible_size(item: &Item) -> bool {
    let (Some(width), Some(height)) = (item.width, item.height) else {
        return false;
    };
    let pixels = u64::try_from(width).unwrap_or(0) * u64::try_from(height).unwrap_or(0);
    let size = u64::try_from(item.size).unwrap_or(u64::MAX);
    size <= pixels.saturating_mul(8).saturating_add(1024 * 1024)
}

/// Đọc hết `stored` vào MỘT vùng nhớ cấp sẵn đúng cỡ (gom từng mảnh rồi nối sẽ giữ file
/// hai lần trong bộ nhớ).
pub(crate) async fn read_all(stored: StoredFile) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::with_capacity(usize::try_from(stored.size).unwrap_or(0));
    let mut content = stored.content;
    while let Some(chunk) = content.try_next().await? {
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Các chiều rộng cần làm cho `item` (rỗng: không làm).
pub(crate) fn wanted_widths(widths: &[u32], item: &Item) -> Vec<u32> {
    let (Some(width), Some(height)) = (item.width, item.height) else {
        return Vec::new();
    };
    let pixels = u64::try_from(width).unwrap_or(0) * u64::try_from(height).unwrap_or(0);
    if !SOURCES.contains(&item.content_type.as_str())
        || pixels > MAX_PIXELS
        || !plausible_size(item)
    {
        return Vec::new();
    }
    // Ảnh xoay 90° theo EXIF thì chiều rộng thật là `height`; lấy cạnh lớn hơn cho chắc,
    // phần thừa bị bỏ khi làm (không bao giờ phóng to).
    let largest = u32::try_from(width.max(height)).unwrap_or(0);
    widths.iter().copied().filter(|wanted| *wanted < largest).collect()
}

pub(crate) struct Generator(pub Arc<MediaService>);

#[async_trait::async_trait]
impl JobHandler for Generator {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let input: MakeVariants = serde_json::from_value(job.payload.clone())
            .map_err(|error| Error::internal(format!("media variants job payload: {error}")))?;
        self.0.make_variants(input.item_id, input.version_id).await.map(|_| ())
    }
}

impl MediaService {
    /// Đưa job làm ảnh thu nhỏ của `item` vào hàng đợi, trong `transaction` của thay đổi —
    /// nếu có cỡ nào cần làm.
    pub(crate) async fn enqueue_variants<C: ConnectionTrait>(
        &self,
        transaction: &C,
        item: &Item,
    ) -> Result<(), Error> {
        if wanted_widths(&self.config.image_widths, item).is_empty() {
            return Ok(());
        }
        let job = MakeVariants { item_id: item.id, version_id: item.version_id };
        self.jobs.enqueue(transaction, &job, EnqueueOptions::default()).await?;
        Ok(())
    }

    /// Làm ảnh thu nhỏ cho phiên bản `version_id` của item `item_id`; trả số ảnh đã làm.
    /// Item đã đổi nội dung hay không còn thì không làm gì (job của phiên bản mới lo).
    pub(crate) async fn make_variants(
        &self,
        item_id: Uuid,
        version_id: Uuid,
    ) -> Result<usize, Error> {
        let Some(item) = item::Entity::find_by_id(item_id).one(self.database.write()).await? else {
            return Ok(0);
        };
        let widths = wanted_widths(&self.config.image_widths, &item);
        if item.version_id != version_id || widths.is_empty() {
            return Ok(0);
        }
        let original = read_all(self.read(&item, None).await?).await?;
        // Giải mã và đổi cỡ tốn CPU: chạy trên thread riêng của tokio cho việc chặn.
        let rendered = tokio::task::spawn_blocking(move || render(&original, &widths))
            .await
            .map_err(Error::internal)?;
        let rendered = match rendered {
            Ok(rendered) => rendered,
            Err(problem) => {
                // Ảnh hỏng không lành lại khi thử lại: ghi log rồi thôi.
                tracing::warn!(media_item_id = %item_id, problem, "cannot make smaller copies of an image");
                return Ok(0);
            }
        };

        let mut written = Vec::new();
        for copy in &rendered {
            let width = i32::try_from(copy.width).map_err(Error::internal)?;
            let key = storage_key(item_id, version_id, width);
            let body = futures_util::stream::iter([Ok(Bytes::from(copy.bytes.clone()))]);
            self.storage.put(&key, Box::pin(body)).await?;
            written.push(key);
        }
        let saved = self.save_variants(item_id, version_id, &rendered).await;
        if !matches!(saved, Ok(true)) {
            // Nội dung vừa đổi (hay lỗi): ảnh vừa làm không còn ai dùng.
            for key in &written {
                if let Err(error) = self.storage.delete(key).await {
                    tracing::warn!(key, error = %error, "cannot remove an unused smaller copy");
                }
            }
        }
        saved?;
        tracing::info!(media_item_id = %item_id, copies = rendered.len(), "smaller copies of an image made");
        Ok(rendered.len())
    }

    /// Lưu dòng của các ảnh thu nhỏ nếu item còn ở phiên bản `version_id`. `false`: đã đổi.
    async fn save_variants(
        &self,
        item_id: Uuid,
        version_id: Uuid,
        rendered: &[Rendered],
    ) -> Result<bool, Error> {
        let transaction = self.database.begin_write().await?;
        // Khoá dòng item (`rows::lock_one`) để lệnh thay nội dung chạy cùng lúc phải chờ.
        let current =
            item::Entity::find_by_id(item_id).filter(item::Column::VersionId.eq(version_id));
        if rows::lock_one(&transaction, current).await?.is_none() {
            transaction.rollback().await?;
            return Ok(false);
        }
        // Làm lại (job chạy lần hai): thay các dòng cũ của cùng phiên bản.
        variant::Entity::delete_many()
            .filter(variant::Column::ItemId.eq(item_id))
            .filter(variant::Column::VersionId.eq(version_id))
            .exec(&transaction)
            .await?;
        let now = now();
        for copy in rendered {
            variant::Entity::insert(variant::ActiveModel {
                id: Set(Uuid::now_v7()),
                item_id: Set(item_id),
                version_id: Set(version_id),
                width: Set(i32::try_from(copy.width).map_err(Error::internal)?),
                height: Set(i32::try_from(copy.height).map_err(Error::internal)?),
                content_type: Set(copy.content_type.to_owned()),
                size: Set(i64::try_from(copy.bytes.len()).map_err(Error::internal)?),
                created_at: Set(now),
            })
            .exec_without_returning(&transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(true)
    }

    /// Ảnh thu nhỏ rộng `width` của nội dung hiện tại của `item`, nếu đã làm.
    pub(crate) async fn variant(&self, item: &Item, width: u32) -> Result<Option<Variant>, Error> {
        let Ok(width) = i32::try_from(width) else {
            return Ok(None);
        };
        Ok(variant::Entity::find()
            .filter(variant::Column::ItemId.eq(item.id))
            .filter(variant::Column::VersionId.eq(item.version_id))
            .filter(variant::Column::Width.eq(width))
            .one(self.database.read())
            .await?)
    }
}

/// Ảnh thu nhỏ của nội dung hiện tại của từng item trong `items`, theo chiều rộng tăng dần.
/// Một câu query.
pub(crate) async fn variants_of<C: ConnectionTrait>(
    connection: &C,
    items: &[Item],
) -> Result<std::collections::HashMap<Uuid, Vec<Variant>>, Error> {
    let mut found: std::collections::HashMap<Uuid, Vec<Variant>> = std::collections::HashMap::new();
    if items.is_empty() {
        return Ok(found);
    }
    let rows = variant::Entity::find()
        .filter(variant::Column::ItemId.is_in(items.iter().map(|item| item.id).collect::<Vec<_>>()))
        .order_by_asc(variant::Column::Width)
        .all(connection)
        .await?;
    for row in rows {
        let current =
            items.iter().any(|item| item.id == row.item_id && item.version_id == row.version_id);
        if current {
            found.entry(row.item_id).or_default().push(row);
        }
    }
    Ok(found)
}

/// Giải mã `original` (có giới hạn chống "bom giải nén"), xoay theo EXIF.
fn decode(original: &[u8]) -> Result<DynamicImage, String> {
    let mut reader =
        ImageReader::new(Cursor::new(original)).with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = Limits::default();
    let side = u32::try_from(MAX_PIXELS).unwrap_or(u32::MAX);
    limits.max_image_width = Some(side);
    limits.max_image_height = Some(side);
    limits.max_alloc = Some(MAX_PIXELS * 4 + 64 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!("{width}x{height} is more than {MAX_PIXELS} pixels"));
    }
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    image.apply_orientation(orientation);
    Ok(image)
}

/// Mã hoá: ảnh có kênh trong suốt → PNG, còn lại → JPEG.
fn encode(image: &DynamicImage, transparent: bool) -> Result<(Vec<u8>, &'static str), String> {
    let mut bytes = Vec::new();
    if transparent {
        image
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        Ok((bytes, "image/png"))
    } else {
        let encoder = JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY);
        DynamicImage::ImageRgb8(image.to_rgb8())
            .write_with_encoder(encoder)
            .map_err(|e| e.to_string())?;
        Ok((bytes, "image/jpeg"))
    }
}

/// Giải mã `original`, xoay theo EXIF, làm một bản cho mỗi chiều rộng nhỏ hơn ảnh.
fn render(original: &[u8], widths: &[u32]) -> Result<Vec<Rendered>, String> {
    let image = decode(original)?;
    let transparent = image.color().has_alpha();
    let mut rendered = Vec::new();
    for &wanted in widths.iter().filter(|wanted| **wanted < image.width()) {
        // `resize` giữ tỉ lệ, vừa trong khung `wanted` × (rất cao).
        let smaller = image.resize(wanted, u32::MAX, FilterType::CatmullRom);
        let (bytes, content_type) = encode(&smaller, transparent)?;
        rendered.push(Rendered {
            width: smaller.width(),
            height: smaller.height(),
            content_type,
            bytes,
        });
    }
    Ok(rendered)
}

/// Một bản tạm (không lưu) vừa trong khung `side` × `side`, giữ tỉ lệ — cho gợi ý alt text
/// khi chưa có ảnh thu nhỏ phù hợp. Trả nội dung và loại.
pub(crate) fn shrink(original: &[u8], side: u32) -> Result<(Vec<u8>, &'static str), String> {
    let image = decode(original)?;
    let transparent = image.color().has_alpha();
    let smaller = if image.width() > side || image.height() > side {
        image.resize(side, side, FilterType::CatmullRom)
    } else {
        image
    };
    encode(&smaller, transparent)
}

#[cfg(test)]
pub(crate) mod samples {
    //! Ảnh thật (giải mã được) cho test.

    use std::io::Cursor;

    use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};

    /// Ảnh `format` thật, `width` × `height`, có màu thay đổi theo vị trí.
    pub fn encoded(width: u32, height: u32, format: ImageFormat, transparent: bool) -> Vec<u8> {
        let image = if transparent {
            DynamicImage::ImageRgba8(RgbaImage::from_fn(width, height, |x, y| {
                Rgba([(x % 256) as u8, (y % 256) as u8, 90, 128])
            }))
        } else {
            DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |x, y| {
                Rgb([(x % 256) as u8, (y % 256) as u8, 90])
            }))
        };
        let mut bytes = Vec::new();
        image.write_to(&mut Cursor::new(&mut bytes), format).unwrap();
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lam_anh_thu_nho_giu_ti_le() {
        let png = samples::encoded(1000, 500, ImageFormat::Png, false);
        let copies = render(&png, &[200, 640, 1000, 2000]).unwrap();
        let sizes: Vec<(u32, u32, &str)> =
            copies.iter().map(|c| (c.width, c.height, c.content_type)).collect();
        assert_eq!(sizes, [(200, 100, "image/jpeg"), (640, 320, "image/jpeg")]);
        let decoded = image::load_from_memory(&copies[0].bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (200, 100));

        let transparent = samples::encoded(400, 400, ImageFormat::Png, true);
        let copies = render(&transparent, &[100]).unwrap();
        assert_eq!(copies[0].content_type, "image/png");
        assert!(render(b"not an image", &[100]).is_err());
    }

    #[test]
    fn chon_co_can_lam() {
        let mut item = Item {
            id: Uuid::nil(),
            folder_id: Uuid::nil(),
            name: "a".into(),
            name_key: "a".into(),
            content_type: "image/png".into(),
            size: 1,
            sha256: String::new(),
            width: Some(1000),
            height: Some(800),
            alt: String::new(),
            title: String::new(),
            caption: String::new(),
            visibility: "private".into(),
            version_id: Uuid::nil(),
            uploaded_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            trashed_at: None,
        };
        assert_eq!(wanted_widths(&[320, 640, 1280], &item), [320, 640]);
        item.content_type = "image/avif".into();
        assert!(wanted_widths(&[320], &item).is_empty());
        item.content_type = "image/jpeg".into();
        item.width = Some(10_000);
        item.height = Some(10_000);
        assert!(wanted_widths(&[320], &item).is_empty(), "too many pixels");
        // File lớn hơn mọi cách nén của số pixel đó (dữ liệu nhét thêm): không giải mã.
        item.width = Some(1000);
        item.height = Some(800);
        item.size = 1000 * 800 * 8 + 1024 * 1024;
        assert_eq!(wanted_widths(&[320], &item), [320]);
        item.size += 1;
        assert!(wanted_widths(&[320], &item).is_empty(), "padded file");
    }
}
