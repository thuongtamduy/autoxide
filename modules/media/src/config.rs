//! Cấu hình của module `media`, tiền tố `AUTOXIDE_MEDIA_`. Nơi lưu nội dung là nơi lưu
//! chung của chương trình (module `files`, `AUTOXIDE_FILES_…`).

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

/// Loại được tải lên khi không cấu hình: ảnh raster, SVG, video MP4 / WebM, âm thanh, PDF.
pub(crate) const DEFAULT_TYPES: &str = "image/jpeg,image/png,image/gif,image/webp,image/avif,\
image/svg+xml,video/mp4,video/webm,audio/*,application/pdf";

pub(crate) const MAX_BYTES: Setting = Setting::new(
    "AUTOXIDE_MEDIA_MAX_BYTES",
    "Largest media item that can be uploaded, in bytes (1 to 5368709120). Larger uploads are \
     answered with 413. Big files also need enough time: raise \
     AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS for slow clients.",
)
.with_default("104857600");

pub(crate) const ALLOWED_TYPES: Setting = Setting::new(
    "AUTOXIDE_MEDIA_ALLOWED_TYPES",
    "Media types that may be uploaded to the media library, comma-separated; `audio/*` allows \
     every audio type. Other types are answered with 415. Whatever the list, the content must \
     really be of the declared type.",
)
.with_default(DEFAULT_TYPES);

pub(crate) const TRASH_DAYS: Setting = Setting::new(
    "AUTOXIDE_MEDIA_TRASH_DAYS",
    "Days an item stays in the trash before it is deleted for good (1 to 3650).",
)
.with_default("30");

pub(crate) const IMAGE_WIDTHS: Setting = Setting::new(
    "AUTOXIDE_MEDIA_IMAGE_WIDTHS",
    "Widths in pixels of the smaller copies made of every JPEG, PNG, GIF and WebP image \
     (comma-separated, each 16 to 8192, at most 10); only widths smaller than the image are \
     made. `off`: no smaller copies.",
)
.with_default("320,640,1280,1920");

pub(crate) const AI_SEND_PRIVATE: Setting = Setting::new(
    "AUTOXIDE_MEDIA_AI_SEND_PRIVATE",
    "Allow sending private images to the AI provider for alt text suggestions (`true` or \
     `false`). Public images are already visible to anyone; private ones leave the server only \
     when this is `true`.",
)
.with_default("false");

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MediaConfig {
    pub limits: Limits,
    pub trash_days: u32,
    /// Chiều rộng các ảnh thu nhỏ, tăng dần; rỗng: không tạo (D-093).
    pub image_widths: Vec<u32>,
    /// Được gửi ảnh riêng tư cho nhà cung cấp AI để gợi ý alt text (D-109).
    pub ai_send_private: bool,
}

/// Giới hạn của nội dung tải lên.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Limits {
    pub max_bytes: u64,
    /// Mỗi mục là `loại/kiểu` hoặc `loại/*`, chữ thường.
    pub allowed_types: Vec<String>,
}

impl Limits {
    /// `content_type` (đã chuẩn hoá) có được phép không.
    pub(crate) fn allows(&self, content_type: &str) -> bool {
        self.allowed_types.iter().any(|pattern| match pattern.strip_suffix("/*") {
            Some(kind) => content_type.split('/').next() == Some(kind),
            None => pattern == content_type,
        })
    }
}

/// Loại nội dung dạng `loại/kiểu` (RFC 6838: chữ, số và `!#$&-^_.+`), chữ thường, bỏ
/// tham số (`; charset=…`). `None` nếu sai dạng.
pub(crate) fn normalize_type(raw: &str) -> Option<String> {
    let essence = raw.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    let (kind, subtype) = essence.split_once('/')?;
    let token = |part: &str| {
        (1..=127).contains(&part.len())
            && part.chars().all(|c| c.is_ascii_alphanumeric() || "!#$&-^_.+".contains(c))
    };
    (token(kind) && token(subtype)).then_some(essence)
}

impl MediaConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let max_bytes =
            reader.get_with(&MAX_BYTES, |raw| parse_in_range(raw, 1u64, 5 * 1024 * 1024 * 1024));
        let allowed_types = reader.get_with(&ALLOWED_TYPES, |raw| {
            let types = raw
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| {
                    let wildcard = item.strip_suffix("/*").map(|kind| format!("{kind}/x"));
                    match (normalize_type(wildcard.as_deref().unwrap_or(item)), wildcard.is_some())
                    {
                        (Some(_), true) => Ok(item.to_ascii_lowercase()),
                        (Some(normalized), false) => Ok(normalized),
                        (None, _) => Err(format!(
                            "`{item}` is not a media type like `image/png` or `audio/*`"
                        )),
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            if types.is_empty() {
                Err("must list at least one media type".to_owned())
            } else {
                Ok(types)
            }
        });
        let trash_days = reader.get_with(&TRASH_DAYS, |raw| parse_in_range(raw, 1u32, 3650));
        let image_widths = reader.get_with(&IMAGE_WIDTHS, |raw| {
            if raw.trim().eq_ignore_ascii_case("off") {
                return Ok(Vec::new());
            }
            let mut widths = raw
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| parse_in_range(item, 16u32, 8192))
                .collect::<Result<Vec<_>, _>>()?;
            widths.sort_unstable();
            widths.dedup();
            if widths.len() > 10 {
                Err("must list at most 10 widths".to_owned())
            } else {
                Ok(widths)
            }
        });
        let ai_send_private = reader.get_with(&AI_SEND_PRIVATE, |raw| match raw.trim() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("must be `true` or `false`".to_owned()),
        });
        Ok(MediaConfig {
            limits: Limits { max_bytes: max_bytes?, allowed_types: allowed_types? },
            trash_days: trash_days?,
            image_widths: image_widths?,
            ai_send_private: ai_send_private?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::ConfigSource;

    fn read(pairs: &[(&str, &str)]) -> Result<MediaConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = MediaConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mac_dinh_hop_ly() {
        let config = read(&[]).unwrap();
        assert_eq!(config.limits.max_bytes, 104_857_600);
        assert_eq!(config.trash_days, 30);
        assert_eq!(config.image_widths, [320, 640, 1280, 1920]);
        let none = read(&[("AUTOXIDE_MEDIA_IMAGE_WIDTHS", "off")]).unwrap();
        assert!(none.image_widths.is_empty());
        let custom = read(&[("AUTOXIDE_MEDIA_IMAGE_WIDTHS", "800, 200,800")]).unwrap();
        assert_eq!(custom.image_widths, [200, 800]);
        assert!(read(&[("AUTOXIDE_MEDIA_IMAGE_WIDTHS", "8")]).is_err());
        for allowed in ["image/png", "image/avif", "image/svg+xml", "audio/ogg", "application/pdf"]
        {
            assert!(config.limits.allows(allowed), "{allowed}");
        }
        for refused in ["text/html", "application/zip", "video/x-matroska"] {
            assert!(!config.limits.allows(refused), "{refused}");
        }
    }

    #[test]
    fn gia_tri_sai_bi_tu_choi_cung_luc() {
        let error = read(&[
            ("AUTOXIDE_MEDIA_ALLOWED_TYPES", "png"),
            ("AUTOXIDE_MEDIA_TRASH_DAYS", "0"),
            ("AUTOXIDE_MEDIA_MAX_BYTES", "0"),
        ])
        .unwrap_err();
        for name in ["ALLOWED_TYPES", "TRASH_DAYS", "MAX_BYTES"] {
            assert!(error.contains(&format!("AUTOXIDE_MEDIA_{name}")), "{error}");
        }
        assert!(read(&[("AUTOXIDE_MEDIA_ALLOWED_TYPES", " , ")]).is_err());
        let custom = read(&[("AUTOXIDE_MEDIA_ALLOWED_TYPES", "Image/*, text/plain")]).unwrap();
        assert_eq!(custom.limits.allowed_types, ["image/*", "text/plain"]);
    }
}
