//! Slug: the last part of a path (`duong-den-da-lat`), shared by every module (`content`, `catalog`, ...;
//! moved from `content` in step 41). Generated from a title with the `slug` crate (converts every letter to
//! ASCII: strips diacritics, `đ` -> `d`); a slug submitted by a user must have the form
//! `[a-z0-9]+(-[a-z0-9]+)*` and be at most [`MAX_LENGTH`] characters long (D-102).

use uuid::Uuid;

/// Maximum length of a slug.
pub const MAX_LENGTH: usize = 100;

/// Whether the slug has the right shape.
pub fn is_valid(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= MAX_LENGTH
        && slug.split('-').all(|part| {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// Slug generated from `title` (emoji and symbols dropped); when the title yields no letters at all (e.g. only
/// emoji), the first part of `id` is used.
pub fn from_title(title: &str, id: Uuid) -> String {
    // Chỉ giữ chữ, số và dấu ngăn từ: emoji và ký hiệu bị bỏ (không thì thành chữ tiếng Anh).
    let kept: String = title.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    let full = slug::slugify(kept);
    let mut cut = String::new();
    for part in full.split('-').filter(|part| !part.is_empty()) {
        let extra = if cut.is_empty() { part.len() } else { part.len() + 1 };
        if cut.len() + extra > MAX_LENGTH {
            break;
        }
        if !cut.is_empty() {
            cut.push('-');
        }
        cut.push_str(part);
    }
    if cut.is_empty() {
        // Một từ dài hơn cả giới hạn thì cắt cứng; không có chữ nào thì dùng ID.
        let fallback: String =
            full.chars().filter(char::is_ascii_alphanumeric).take(MAX_LENGTH).collect();
        if fallback.is_empty() { id.simple().to_string()[..12].to_owned() } else { fallback }
    } else {
        cut
    }
}

/// `base` with the `n`th suffix (`n` >= 2): `slug-2`, with `base` truncated to fit the limit.
pub fn with_suffix(base: &str, n: u32) -> String {
    let suffix = format!("-{n}");
    let room = MAX_LENGTH.saturating_sub(suffix.len());
    let trimmed = base.get(..room.min(base.len())).unwrap_or(base).trim_end_matches('-');
    format!("{trimmed}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_tieng_viet_va_gioi_han() {
        let id = Uuid::now_v7();
        assert_eq!(from_title("Đường đến Đà Lạt", id), "duong-den-da-lat");
        assert_eq!(from_title("  Hello, World!  ", id), "hello-world");
        assert_eq!(from_title("🎉🎉", id).len(), 12);
        assert_eq!(from_title("Tết 🎉 2026", id), "tet-2026");
        let long = "word ".repeat(40);
        let slug = from_title(&long, id);
        assert!(slug.len() <= MAX_LENGTH && is_valid(&slug), "{slug}");
        assert_eq!(from_title(&"a".repeat(150), id).len(), MAX_LENGTH);
        assert_eq!(with_suffix("abc", 2), "abc-2");
        assert_eq!(with_suffix(&"a".repeat(100), 12).len(), MAX_LENGTH);
        for good in ["a", "a-b", "ab1-2c"] {
            assert!(is_valid(good), "{good}");
        }
        for bad in ["", "A", "a--b", "-a", "a-", "a b", "đ", &"a".repeat(101)] {
            assert!(!is_valid(bad), "{bad}");
        }
    }

    // Property test (đặc tả 31 B3): slug sinh từ tiêu đề bất kỳ (kể cả emoji, chữ lạ) luôn hợp
    // lệ; thêm hậu tố vào slug hợp lệ vẫn hợp lệ.
    proptest::proptest! {
        #[test]
        fn slug_tu_tieu_de_bat_ky_luon_hop_le(title in "\\PC{0,300}", id in proptest::prelude::any::<u128>()) {
            let slug = from_title(&title, Uuid::from_u128(id));
            proptest::prop_assert!(is_valid(&slug), "{slug:?} from {title:?}");
        }

        #[test]
        fn them_hau_to_van_hop_le(base in "[a-z0-9]{1,30}(-[a-z0-9]{1,30}){0,6}", n in proptest::prelude::any::<u32>()) {
            proptest::prop_assume!(is_valid(&base));
            let slug = with_suffix(&base, n);
            proptest::prop_assert!(is_valid(&slug), "{slug:?}");
            let suffix = format!("-{n}");
            proptest::prop_assert!(slug.ends_with(&suffix));
        }
    }
}
