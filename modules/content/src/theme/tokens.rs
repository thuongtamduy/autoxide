//! Token của bộ giao diện website (bước 67, D-229, D-20261122-theme-tokens): tên → giá trị phẳng,
//! để giao diện gán thẳng thành biến CSS. Không CSS, không mã màu tự do trong khối — khối chỉ nêu tên
//! token (`primary`…), đổi token là mọi trang đổi theo.
//!
//! Mỗi nút của cây tổ chức chỉ lưu phần GHI ĐÈ của mình; token hiệu lực của một website = mặc định
//! của hệ thống, rồi lần lượt ghi đè của nút gốc → … → nút của website (chuỗi đặt, cửa hàng đè).

use std::collections::BTreeMap;

use autoxide_kernel::error::FieldError;
use serde_json::{Map, Value};
use uuid::Uuid;

/// Font cho phép: có đủ dấu tiếng Việt, tải được từ nguồn font mở; `system` là font của máy.
pub(crate) const FONTS: [&str; 12] = [
    "system",
    "Inter",
    "Roboto",
    "Open Sans",
    "Noto Sans",
    "Noto Serif",
    "Be Vietnam Pro",
    "Montserrat",
    "Lora",
    "Merriweather",
    "Playfair Display",
    "Source Sans 3",
];

/// Màu (`#rrggbb`).
pub(crate) const COLORS: [&str; 10] = [
    "color.primary",
    "color.on_primary",
    "color.secondary",
    "color.on_secondary",
    "color.background",
    "color.surface",
    "color.text",
    "color.muted",
    "color.success",
    "color.error",
];

/// Cặp chữ / nền phải đạt WCAG AA (tỉ lệ tương phản 4,5:1 cho chữ thường).
pub(crate) const CONTRAST_PAIRS: [(&str, &str); 5] = [
    ("color.text", "color.background"),
    ("color.text", "color.surface"),
    ("color.muted", "color.background"),
    ("color.on_primary", "color.primary"),
    ("color.on_secondary", "color.secondary"),
];
/// Tỉ lệ tối thiểu của WCAG 2.2 mức AA cho chữ thường.
pub(crate) const AA_RATIO: f64 = 4.5;

const ENUMS: [(&str, &[&str]); 3] = [
    ("radius", &["none", "small", "medium", "large", "full"]),
    ("spacing", &["compact", "normal", "relaxed"]),
    ("button.style", &["solid", "outline", "soft"]),
];
const MEDIA: [&str; 2] = ["logo_media_id", "background_media_id"];

/// Token mặc định của hệ thống: website chưa chỉnh gì vẫn đủ token, đạt tương phản AA.
pub(crate) fn defaults() -> BTreeMap<String, Value> {
    let pairs: [(&str, Value); 19] = [
        ("color.primary", "#1d4ed8".into()),
        ("color.on_primary", "#ffffff".into()),
        ("color.secondary", "#0f766e".into()),
        ("color.on_secondary", "#ffffff".into()),
        ("color.background", "#ffffff".into()),
        ("color.surface", "#f4f4f5".into()),
        ("color.text", "#18181b".into()),
        ("color.muted", "#52525b".into()),
        ("color.success", "#15803d".into()),
        ("color.error", "#b91c1c".into()),
        ("font.heading", "Be Vietnam Pro".into()),
        ("font.body", "Be Vietnam Pro".into()),
        ("font.size_base", 16.into()),
        ("radius", "medium".into()),
        ("spacing", "normal".into()),
        ("button.style", "solid".into()),
        ("logo_media_id", Value::Null),
        ("background_media_id", Value::Null),
        ("content_width", 1200.into()),
    ];
    pairs.into_iter().map(|(key, value)| (key.to_owned(), value)).collect()
}

/// Kiểm phần ghi đè `tokens` của một nút (`null` = bỏ ghi đè, dùng của nút trên). Mọi lỗi một lần,
/// trường `tokens.<tên>`. Trả phần ghi đè đã chuẩn hoá (màu viết thường, bỏ `null`).
pub(crate) fn check(
    tokens: &Map<String, Value>,
) -> Result<BTreeMap<String, Value>, Vec<FieldError>> {
    let known = defaults();
    let mut errors = Vec::new();
    let mut kept = BTreeMap::new();
    for (key, value) in tokens {
        let field = format!("tokens.{key}");
        if !known.contains_key(key) {
            errors.push(
                FieldError::new(field, "unknown", "is not a theme token; known: {known}")
                    .with_param("known", known.keys().cloned().collect::<Vec<_>>().join(", ")),
            );
            continue;
        }
        if value.is_null() {
            continue;
        }
        match normalize(key, value, &field) {
            Ok(value) => {
                kept.insert(key.clone(), value);
            }
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty() { Ok(kept) } else { Err(errors) }
}

fn normalize(key: &str, value: &Value, field: &str) -> Result<Value, FieldError> {
    if COLORS.contains(&key) {
        let color = value.as_str().filter(|raw| parse_color(raw).is_some()).ok_or_else(|| {
            FieldError::new(field, "invalid", "must be a color written `#rrggbb`")
        })?;
        return Ok(Value::String(color.to_ascii_lowercase()));
    }
    if key == "font.heading" || key == "font.body" {
        return value
            .as_str()
            .and_then(|font| FONTS.iter().find(|allowed| allowed.eq_ignore_ascii_case(font)))
            .map(|font| Value::String((*font).to_owned()))
            .ok_or_else(|| {
                FieldError::new(field, "not_allowed", "must be one of the allowed fonts: {fonts}")
                    .with_param("fonts", FONTS.join(", "))
            });
    }
    if MEDIA.contains(&key) {
        return value
            .as_str()
            .and_then(|raw| Uuid::parse_str(raw).ok())
            .map(|id| Value::String(id.to_string()))
            .ok_or_else(|| FieldError::new(field, "invalid", "must be the id of a media item"));
    }
    let range = match key {
        "font.size_base" => Some((12, 24)),
        "content_width" => Some((640, 1920)),
        _ => None,
    };
    if let Some((min, max)) = range {
        return value
            .as_i64()
            .filter(|size| (min..=max).contains(size))
            .map(Value::from)
            .ok_or_else(|| {
                FieldError::new(field, "range", "must be a whole number in {range}")
                    .with_param("range", format!("{min}..={max}"))
            });
    }
    let allowed =
        ENUMS.iter().find(|(name, _)| *name == key).map_or(&[][..], |(_, values)| *values);
    value
        .as_str()
        .filter(|raw| allowed.contains(raw))
        .map(|raw| Value::String(raw.to_owned()))
        .ok_or_else(|| {
            FieldError::new(field, "invalid", "must be one of {allowed}")
                .with_param("allowed", allowed.join(", "))
        })
}

/// Ghép: mặc định, rồi từng phần ghi đè theo thứ tự (gốc trước, nút của website sau cùng).
pub(crate) fn merge<'a>(
    layers: impl IntoIterator<Item = &'a BTreeMap<String, Value>>,
) -> BTreeMap<String, Value> {
    let mut tokens = defaults();
    for layer in layers {
        for (key, value) in layer {
            tokens.insert(key.clone(), value.clone());
        }
    }
    tokens
}

// Một cặp màu không đạt tương phản AA (cảnh báo, không chặn).
/// A text and background color pair below the WCAG AA contrast (a warning; the theme is saved).
#[derive(Debug, Clone, PartialEq, serde::Serialize, utoipa::ToSchema)]
pub(crate) struct ContrastWarning {
    /// Token of the text color, e.g. `color.text`.
    pub foreground: String,
    /// Token of the background color, e.g. `color.background`.
    pub background: String,
    /// Contrast ratio, rounded to two decimals; WCAG AA asks at least 4.5 for normal text.
    pub ratio: f64,
}

/// Cặp màu của token hiệu lực `tokens` dưới mức AA.
pub(crate) fn contrast_warnings(tokens: &BTreeMap<String, Value>) -> Vec<ContrastWarning> {
    let color = |key: &str| tokens.get(key).and_then(Value::as_str).and_then(parse_color);
    CONTRAST_PAIRS
        .iter()
        .filter_map(|(fore, back)| {
            let ratio = contrast(color(fore)?, color(back)?);
            (ratio < AA_RATIO).then(|| ContrastWarning {
                foreground: (*fore).to_owned(),
                background: (*back).to_owned(),
                ratio: (ratio * 100.0).round() / 100.0,
            })
        })
        .collect()
}

/// `#rrggbb` → (r, g, b).
fn parse_color(raw: &str) -> Option<(u8, u8, u8)> {
    let hex = raw.strip_prefix('#').filter(|hex| hex.len() == 6)?;
    let channel = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
    Some((channel(0)?, channel(2)?, channel(4)?))
}

/// Tỉ lệ tương phản WCAG 2.x giữa hai màu (1 tới 21). Số thực chỉ dùng để tính cảnh báo hiển thị,
/// không lưu và không phải tiền.
fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (light, dark) = if la > lb { (la, lb) } else { (lb, la) };
    (light + 0.05) / (dark + 0.05)
}

fn luminance((r, g, b): (u8, u8, u8)) -> f64 {
    let linear = |channel: u8| {
        let c = f64::from(channel) / 255.0;
        if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mac_dinh_dat_tuong_phan_aa() {
        assert!(contrast_warnings(&defaults()).is_empty());
    }

    #[test]
    fn tuong_phan_theo_cong_thuc_wcag() {
        let ratio = contrast((0, 0, 0), (255, 255, 255));
        assert!((ratio - 21.0).abs() < 0.01, "{ratio}");
        let mut tokens = defaults();
        tokens.insert("color.text".to_owned(), json!("#aaaaaa"));
        let warnings = contrast_warnings(&tokens);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert_eq!(warnings[0].foreground, "color.text");
        assert!(warnings[0].ratio < AA_RATIO);
    }

    #[test]
    fn font_la_mau_sai_bi_tu_choi_mot_lan() {
        let input = json!({
            "color.primary": "red", "font.body": "Comic Sans", "radius": "huge",
            "font.size_base": 40, "nope": 1, "logo_media_id": "x", "color.text": null,
            "font.heading": "inter"
        });
        let errors = check(input.as_object().unwrap()).unwrap_err();
        let fields: Vec<&str> = errors.iter().map(|e| e.field.as_str()).collect();
        assert_eq!(fields.len(), 6, "{fields:?}");
        let ok = check(
            json!({"font.heading": "inter", "color.primary": "#AA0000"}).as_object().unwrap(),
        )
        .unwrap();
        assert_eq!(ok["font.heading"], "Inter");
        assert_eq!(ok["color.primary"], "#aa0000");
    }

    #[test]
    fn ghi_de_theo_thu_tu_goc_truoc() {
        let root: BTreeMap<String, Value> = [("color.primary".to_owned(), json!("#111111"))].into();
        let shop: BTreeMap<String, Value> = [("color.primary".to_owned(), json!("#222222"))].into();
        assert_eq!(merge([&root, &shop])["color.primary"], "#222222");
        assert_eq!(merge([&root])["color.primary"], "#111111");
        assert_eq!(merge([])["radius"], "medium");
    }
}
