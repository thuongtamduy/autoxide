//! Vị trí hiển thị và banner cho giao diện cửa hàng (bước 55, D-219, D-236,
//! D-20261119-step55-banners).
//!
//! - `placements`: vị trí theo khoá (`home_hero`…) của cả khách: kiểu hiển thị, cài đặt theo kiểu,
//!   loại banner nhận, cỡ ảnh, thiết bị, loại trang. Cần `content.manage`.
//! - `items`: banner của một vị trí, theo nút (`org_id`), ngôn ngữ, khung giờ, thứ tự; link tới
//!   địa chỉ hay đối tượng của sổ đối tượng. Cần `content.banners` (theo nút).
//! - `public`: đọc công khai nhiều vị trí của một trang trong một lời gọi, lọc theo trang, cache tới
//!   mốc bắt đầu / kết thúc gần nhất.
//! - `presets`: bộ vị trí chuẩn gieo cho khách mới.
//! - `promotions`: chương trình khuyến mại kết thúc thì banner trỏ tới nó vào lưu trữ.
//!
//! Thứ tự banner cố định theo `position` — không đảo ngẫu nhiên (phá cache, người xem thấy nhảy).

pub(crate) mod checks;
pub(crate) mod items;
pub(crate) mod placements;
pub(crate) mod presets;
pub(crate) mod promotions;
pub(crate) mod public;
pub(crate) mod tools;

use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use serde_json::{Map, Value};

/// Kiểu hiển thị của vị trí.
pub(crate) const DISPLAYS: [&str; 6] = ["single", "carousel", "grid", "stack", "strip", "popup"];
/// Loại banner vị trí nhận.
pub(crate) const ACCEPTS: [&str; 3] = ["image", "text", "any"];
/// Thiết bị.
pub(crate) const DEVICES: [&str; 3] = ["all", "desktop", "mobile"];
/// Loại trang (D-236). `any`: mọi loại.
pub(crate) const PAGE_KINDS: [&str; 10] = [
    "home",
    "category",
    "collection",
    "product",
    "search",
    "cart",
    "content_entry",
    "content_type",
    "landing",
    "any",
];
/// Token màu của theme cho banner chữ — không nhận mã màu tự do.
pub(crate) const TONES: [&str; 4] = ["primary", "secondary", "accent", "neutral"];
/// Tần suất của popup.
const FREQUENCIES: [&str; 3] = ["once_per_session", "once_per_day", "once_per_week"];

/// Số banner tối đa một vị trí nhận (`max_items`).
pub(crate) const MAX_ITEMS: i32 = 50;
/// Số vị trí tối đa mỗi lần đọc công khai.
pub(crate) const MAX_KEYS: usize = 10;
/// Số trang cụ thể tối đa một banner chọn.
pub(crate) const MAX_PAGE_TARGETS: usize = 50;
/// Sai lệch tỉ lệ ảnh cho phép: 2%.
const RATIO_TOLERANCE: f64 = 0.02;

/// Khoá vị trí: 1 tới 64 chữ thường, số, `_` (không dấu chấm: `seller_store.` để dành, D-233).
pub(crate) fn is_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 64
        && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// `value` có trong `allowed` không; không thì thêm lỗi `field`.
pub(crate) fn check_choice(
    field: &str,
    value: &str,
    allowed: &[&str],
    problems: &mut Vec<FieldError>,
) -> bool {
    if allowed.contains(&value) {
        return true;
    }
    problems.push(
        FieldError::new(field, "invalid", "must be one of {allowed}")
            .with_param("allowed", allowed.join(", ")),
    );
    false
}

/// Kiểm `display_settings` theo kiểu `display` (D-236). Lỗi mang mã `settings_invalid` để gom thành
/// `content.placement_settings_invalid`. Trả bản đã chuẩn hoá (chỉ khoá đã biết).
pub(crate) fn check_display_settings(
    display: &str,
    settings: &Map<String, Value>,
    problems: &mut Vec<FieldError>,
) -> Map<String, Value> {
    let allowed: &[&str] = match display {
        "carousel" => &["autoplay_seconds"],
        "grid" => &["columns"],
        "popup" => &["frequency", "delay_seconds"],
        _ => &[],
    };
    for key in settings.keys() {
        if !allowed.contains(&key.as_str()) {
            problems.push(FieldError::new(
                format!("display_settings.{key}"),
                "settings_invalid",
                "is not a setting of this display",
            ));
        }
    }
    let int = |key: &str| settings.get(key).map(Value::as_i64);
    let mut out = Map::new();
    match display {
        "carousel" => match int("autoplay_seconds") {
            None => {
                out.insert("autoplay_seconds".into(), Value::from(0));
            }
            // Tự trượt nhanh hơn 4 giây thì người xem không kịp đọc (WCAG 2.2.2).
            Some(Some(seconds @ (0 | 4..=60))) => {
                out.insert("autoplay_seconds".into(), Value::from(seconds));
            }
            Some(_) => problems.push(FieldError::new(
                "display_settings.autoplay_seconds",
                "settings_invalid",
                "must be 0 (off) or 4 to 60 seconds",
            )),
        },
        "grid" => match int("columns") {
            None => {
                out.insert("columns".into(), Value::from(3));
            }
            Some(Some(columns @ 2..=4)) => {
                out.insert("columns".into(), Value::from(columns));
            }
            Some(_) => problems.push(FieldError::new(
                "display_settings.columns",
                "settings_invalid",
                "must be 2 to 4",
            )),
        },
        "popup" => {
            match settings.get("frequency").map(Value::as_str) {
                None => {
                    out.insert("frequency".into(), Value::from("once_per_day"));
                }
                Some(Some(frequency)) if FREQUENCIES.contains(&frequency) => {
                    out.insert("frequency".into(), Value::from(frequency));
                }
                Some(_) => {
                    problems.push(FieldError::new(
                        "display_settings.frequency",
                        "settings_invalid",
                        "must be once_per_session, once_per_day or once_per_week",
                    ));
                }
            }
            match int("delay_seconds") {
                None => {
                    out.insert("delay_seconds".into(), Value::from(5));
                }
                Some(Some(seconds @ 0..=60)) => {
                    out.insert("delay_seconds".into(), Value::from(seconds));
                }
                Some(_) => problems.push(FieldError::new(
                    "display_settings.delay_seconds",
                    "settings_invalid",
                    "must be 0 to 60",
                )),
            }
        }
        _ => {}
    }
    out
}

/// Loại trang: không rỗng, không trùng, trong [`PAGE_KINDS`]; `any` thì đứng một mình.
pub(crate) fn check_page_kinds(
    field: &str,
    kinds: &[String],
    allow_empty: bool,
    problems: &mut Vec<FieldError>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (index, kind) in kinds.iter().enumerate() {
        if check_choice(&format!("{field}[{index}]"), kind, &PAGE_KINDS, problems)
            && !out.contains(kind)
        {
            out.push(kind.clone());
        }
    }
    if out.is_empty() && !allow_empty {
        problems.push(FieldError::new(field, "required", "must name at least one page kind"));
    }
    if out.len() > 1 && out.iter().any(|kind| kind == "any") {
        problems.push(FieldError::new(field, "invalid", "`any` must be the only page kind"));
    }
    out
}

/// Ảnh `width`×`height` có hợp tỉ lệ `ratio` (sai lệch ≤ 2%) và cỡ tối thiểu không.
pub(crate) fn image_fits(
    width: Option<i32>,
    height: Option<i32>,
    ratio: Option<(i32, i32)>,
    min: (Option<i32>, Option<i32>),
) -> bool {
    let (Some(width), Some(height)) = (width, height) else {
        // Không biết cỡ (vd SVG): chỉ đạt khi vị trí không đòi gì.
        return ratio.is_none() && min.0.is_none() && min.1.is_none();
    };
    if min.0.is_some_and(|min| width < min) || min.1.is_some_and(|min| height < min) {
        return false;
    }
    match ratio {
        Some((w, h)) if w > 0 && h > 0 && height > 0 => {
            let wanted = f64::from(w) / f64::from(h);
            let actual = f64::from(width) / f64::from(height);
            ((actual - wanted) / wanted).abs() <= RATIO_TOLERANCE
        }
        _ => true,
    }
}

/// Đọc mảng JSON chữ đã lưu (`page_kinds`); hỏng thì rỗng.
pub(crate) fn strings(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

/// Lỗi kiểm dữ liệu của vị trí và banner: mọi lỗi cùng một loại thì mã riêng của loại đó (máy nhận
/// ra ngay, D-083), lẫn nhiều loại thì `validation_failed`. Lỗi từng chỗ luôn nằm trong `errors`.
pub(crate) fn invalid(problems: Vec<FieldError>) -> Error {
    let all = |code: &str| problems.iter().all(|problem| problem.code == code);
    let count = problems.len();
    let first = |name: &str| {
        problems
            .first()
            .and_then(|problem| {
                problem.params().find(|(key, _)| *key == name).map(|(_, value)| value.to_owned())
            })
            .unwrap_or_default()
    };
    // `ErrorKind` đứng ngay trước mã để `cargo xtask check` nhận ra đây là chỗ tạo lỗi.
    let error = if all("settings_invalid") {
        Error::new(
            ErrorKind::Validation,
            "content.placement_settings_invalid",
            "The display settings of the placement are not valid: {count} problems (see `errors`).",
        )
        .with_param("count", count)
    } else if all("image_size_invalid") {
        Error::new(
            ErrorKind::Validation,
            "content.banner_image_size_invalid",
            "The picture for placement `{key}` needs ratio {ratio} and at least {min_width}×{min_height} px; see `errors`.",
        )
        .with_param("key", first("key"))
        .with_param("ratio", first("ratio"))
        .with_param("min_width", first("min_width"))
        .with_param("min_height", first("min_height"))
    } else if all("kind_not_allowed") {
        Error::new(
            ErrorKind::Validation,
            "content.banner_kind_not_allowed",
            "Placement `{key}` only takes {accepts} banners.",
        )
        .with_param("key", first("key"))
        .with_param("accepts", first("accepts"))
    } else if all("page_kind_not_allowed") {
        Error::new(
            ErrorKind::Validation,
            "content.page_kind_not_allowed",
            "Placement `{key}` is not used on pages of kind `{page_kind}`.",
        )
        .with_param("key", first("key"))
        .with_param("page_kind", first("page_kind"))
    } else if all("page_target_not_found") {
        Error::new(
            ErrorKind::Validation,
            "content.page_target_not_found",
            "There is no `{kind}` page `{id}` to show the banner on.",
        )
        .with_param("kind", first("kind"))
        .with_param("id", first("id"))
    } else if all("link_kind_unavailable") {
        Error::new(
            ErrorKind::Validation,
            "content.link_kind_unavailable",
            "No installed module provides links of kind `{kind}`.",
        )
        .with_param("kind", first("kind"))
    } else if all("link_target_missing") {
        Error::new(
            ErrorKind::Validation,
            "content.link_target_missing",
            "The object the banner links to does not exist.",
        )
    } else if all("window_invalid") {
        Error::new(
            ErrorKind::Validation,
            "content.banner_window_invalid",
            "The end time must be after the start time.",
        )
    } else if all("media_not_found") {
        Error::new(
            ErrorKind::Validation,
            "content.media_not_found",
            "A media item used by the entry does not exist (or is in the trash); see `errors`.",
        )
    } else if all("media_not_public") {
        Error::new(
            ErrorKind::Validation,
            "content.media_not_public",
            "A media item used by the entry is not public, so readers could not see it. Make it public first; see `errors`.",
        )
    } else {
        return Error::validation(problems);
    };
    problems.into_iter().fold(error, Error::with_field)
}

pub(crate) fn placement_not_found(key: &str) -> Error {
    Error::not_found("content.placement_not_found", "There is no display placement `{key}`.")
        .with_param("key", key)
}

pub(crate) fn banner_not_found(id: uuid::Uuid) -> Error {
    Error::not_found("content.banner_not_found", "There is no banner `{id}`.").with_param("id", id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ti_le_sai_lech_toi_da_hai_phan_tram() {
        let ratio = Some((3, 1));
        assert!(image_fits(Some(1800), Some(600), ratio, (Some(1800), Some(600))));
        assert!(image_fits(Some(1830), Some(600), ratio, (None, None)));
        assert!(!image_fits(Some(1900), Some(600), ratio, (None, None)));
        assert!(!image_fits(Some(1500), Some(500), ratio, (Some(1800), None)));
        assert!(!image_fits(None, None, ratio, (None, None)));
        assert!(image_fits(None, None, None, (None, None)));
    }

    #[test]
    fn cai_dat_carousel_duoi_4_giay_bi_tu_choi() {
        let mut problems = Vec::new();
        let settings = serde_json::json!({"autoplay_seconds": 3});
        check_display_settings("carousel", settings.as_object().unwrap(), &mut problems);
        assert_eq!(problems.len(), 1);
        assert_eq!(invalid(problems).code(), "content.placement_settings_invalid");
        let mut problems = Vec::new();
        let settings = serde_json::json!({"autoplay_seconds": 0, "columns": 2});
        check_display_settings("carousel", settings.as_object().unwrap(), &mut problems);
        assert_eq!(problems.len(), 1, "khoá của kiểu khác bị từ chối");
    }
}
