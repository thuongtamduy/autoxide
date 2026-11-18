//! Tìm theo chữ trong danh sách: "chứa chuỗi này", không phân biệt hoa thường, không phân
//! biệt dấu ("da lat" ra "Đà Lạt"), cho cùng kết quả trên SQLite và Postgres.
//!
//! Cách làm chung cho mọi module:
//!
//! 1. Lưu sẵn một cột "khoá tìm" là chữ đã qua [`fold`] (vd `search_key`), cập nhật cùng
//!    lúc với cột gốc. Làm trong Rust chứ không bằng `LOWER()` / `unaccent()` của database:
//!    `LOWER()` của SQLite chỉ biết chữ ASCII, của Postgres theo locale; `unaccent` chỉ
//!    Postgres có — hai database sẽ cho hai kết quả.
//! 2. Lọc bằng [`contains`] trên cột đó: `LIKE '%…%' ESCAPE '\'`, nên `%` hay `_` người
//!    dùng gõ là chữ thường, không phải ký tự đại diện.
//!
//! [`normalize`] (chỉ viết thường) là khoá để chặn trùng tên và để sắp: "Anh" và "Ảnh" là
//! hai từ khác nhau nên không được coi là trùng. Đổi cách [`fold`] làm thì phải thêm
//! migration tính lại mọi cột khoá tìm đã lưu.
//!
//! "Chứa" không dùng được index: database quét các dòng còn lại sau những bộ lọc khác.
//! Đủ cho bảng vài trăm nghìn dòng; lớn hơn thì dùng chỉ mục tìm kiếm thay được ([`SearchIndex`],
//! bước 40 P7): module khai loại tài liệu, bộ máy mặc định [`DatabaseSearch`] hay bộ máy ngoài.

mod backend;
pub(crate) mod database;
mod index;

pub use backend::{
    OrgScope, SearchBackend, SearchDoc, SearchHit, SearchHits, SearchKind, SearchRequest,
    SearchSource,
};
pub use database::DatabaseSearch;
pub use index::{DATABASE_BACKEND, SearchIndex};
pub(crate) use index::{SEARCH_BACKEND, SEARCH_SYNC, SyncRunner, reindex_task};

use sea_orm::ColumnTrait;
use sea_orm::sea_query::{Expr, LikeExpr};
use unicode_normalization::UnicodeNormalization as _;
use unicode_normalization::char::is_combining_mark;

/// Khoá để chặn trùng và để sắp theo chữ: bỏ khoảng trắng hai đầu, viết thường.
pub fn normalize(text: &str) -> String {
    text.trim().to_lowercase()
}

/// Dạng đem so khi tìm: như [`normalize`], rồi bỏ dấu — tách chữ khỏi dấu (Unicode NFD), bỏ
/// dấu, `đ` → `d`. "Đường đến Đà Lạt" → "duong den da lat".
pub fn fold(text: &str) -> String {
    normalize(text)
        .nfd()
        .filter(|character| !is_combining_mark(*character))
        .map(|character| if character == 'đ' { 'd' } else { character })
        .collect()
}

/// Dạng đem so khi tìm "khớp đúng dấu": như [`normalize`], rồi gộp chữ và dấu về một dạng
/// (Unicode NFC) — "Ảnh" gõ trên máy này và máy kia ra cùng chuỗi, còn "anh" thì khác. Dùng làm
/// tầng xếp trước khi tìm (D-202): tiếng Việt "dấu / dầu / dâu" khác nghĩa.
pub fn exact_key(text: &str) -> String {
    normalize(text).nfc().collect()
}

/// Điều kiện "cột `column` (đã lưu dạng [`exact_key`]) chứa `text` đúng dấu".
pub fn contains_exact<C: ColumnTrait>(column: C, text: &str) -> Expr {
    let pattern = format!("%{}%", escape_like(&exact_key(text)));
    column.like(LikeExpr::new(pattern).escape('\\'))
}

/// Điều kiện "cột `column` (đã lưu dạng [`fold`]) chứa `text`" — `text` có dấu hay không
/// đều được.
pub fn contains<C: ColumnTrait>(column: C, text: &str) -> Expr {
    let pattern = format!("%{}%", escape_like(&fold(text)));
    column.like(LikeExpr::new(pattern).escape('\\'))
}

/// Thoát `\`, `%` và `_` để dùng trong mẫu `LIKE … ESCAPE '\'`.
fn escape_like(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thoat_ky_tu_dai_dien() {
        assert_eq!(escape_like(r"50%_a\b"), r"50\%\_a\\b");
        assert_eq!(normalize("  Ảnh ĐẸP "), "ảnh đẹp");
        assert_eq!(fold(" Đường đến Đà Lạt "), "duong den da lat");
        assert_eq!(fold("Ảnh ĐẸP, Crème brûlée"), "anh dep, creme brulee");
        assert_eq!(fold("東京"), "東京", "scripts without accents stay as they are");
        // "Ả" viết sẵn một ký tự hay "A" + dấu hỏi rời: cùng khoá.
        assert_eq!(exact_key(" A\u{0309}nh ĐẸP "), exact_key("ảnh đẹp"));
        assert_ne!(exact_key("ảnh"), exact_key("anh"));
    }
}
