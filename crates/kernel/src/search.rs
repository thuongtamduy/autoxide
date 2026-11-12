//! Tìm theo chữ trong danh sách: "chứa chuỗi này", không phân biệt hoa thường, cho cùng
//! kết quả trên SQLite và Postgres.
//!
//! Cách làm chung cho mọi module:
//!
//! 1. Lưu sẵn một cột "khoá tìm" là chữ đã qua [`normalize`] (vd `name_key`), cập nhật
//!    cùng lúc với cột gốc. Viết thường trong Rust chứ không bằng `LOWER()` của database:
//!    `LOWER()` của SQLite chỉ biết chữ ASCII, của Postgres theo locale — hai kết quả khác
//!    nhau với chữ có dấu.
//! 2. Lọc bằng [`contains`] trên cột đó: `LIKE '%…%' ESCAPE '\'`, nên `%` hay `_` người
//!    dùng gõ là chữ thường, không phải ký tự đại diện.
//!
//! "Chứa" không dùng được index: database quét các dòng còn lại sau những bộ lọc khác.
//! Đủ cho bảng vài trăm nghìn dòng; lớn hơn thì cần full-text search riêng.

use sea_orm::ColumnTrait;
use sea_orm::sea_query::{Expr, LikeExpr};

/// Dạng đem so khi tìm và khi sắp theo chữ: bỏ khoảng trắng hai đầu, viết thường.
pub fn normalize(text: &str) -> String {
    text.trim().to_lowercase()
}

/// Điều kiện "cột `column` (đã lưu dạng [`normalize`]) chứa `text`".
pub fn contains<C: ColumnTrait>(column: C, text: &str) -> Expr {
    let pattern = format!("%{}%", escape_like(&normalize(text)));
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
    }
}
