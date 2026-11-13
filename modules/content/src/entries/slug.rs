//! Slug: phần cuối của đường dẫn (`duong-den-da-lat`). Sinh từ tiêu đề bằng crate `slug`
//! (chuyển mọi chữ sang ASCII: bỏ dấu, `đ` → `d`); slug người dùng gửi phải đúng dạng
//! `[a-z0-9]+(-[a-z0-9]+)*`, dài tối đa [`MAX_LENGTH`] ký tự (D-102).

use uuid::Uuid;

/// Độ dài tối đa của slug.
pub(crate) const MAX_LENGTH: usize = 100;

/// Slug có đúng dạng không.
pub(crate) fn is_valid(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= MAX_LENGTH
        && slug.split('-').all(|part| {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// Slug sinh từ `title` (bỏ emoji, ký hiệu); tiêu đề không cho ra chữ nào (vd chỉ có emoji)
/// thì dùng đoạn đầu của `id`.
pub(crate) fn from_title(title: &str, id: Uuid) -> String {
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

/// `base` kèm hậu tố thứ `n` (`n` ≥ 2): `slug-2`, cắt bớt `base` cho vừa giới hạn.
pub(crate) fn with_suffix(base: &str, n: u32) -> String {
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
}
