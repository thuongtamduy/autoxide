//! Luật `doc-language` (D-20261123-english-rustdoc): doc comment (`///`, `//!`) của phần code là hợp
//! đồng với người viết module ở repo riêng viết bằng tiếng Anh — người lạ đọc docs.rs / `cargo doc`
//! phải hiểu được. Comment `//` thường (giải thích VÌ SAO cho chủ dự án) vẫn viết tiếng Việt được.
//!
//! Phạm vi ([`in_scope`]): `crates/kernel/src`, `crates/testkit/src`, `crates/app/src/lib.rs`, và
//! `api` của mỗi module (`modules/<tên>/src/api.rs`, `modules/<tên>/src/api/…`). Không có danh sách
//! miễn trừ. Chữ trong `code span` và chuỗi `"…"` trong khối code của doc được bỏ qua: đó là dữ liệu
//! ví dụ (vd từ khoá tìm kiếm `áo thun`), không phải lời văn.

use std::fs;
use std::path::Path;

use crate::boundaries::Violation;

/// Chữ có dấu tiếng Việt (thường và hoa), kể cả `đ`.
const VIETNAMESE: &str = "àáạảãâầấậẩẫăằắặẳẵèéẹẻẽêềếệểễìíịỉĩòóọỏõôồốộổỗơờớợởỡùúụủũưừứựửữỳýỵỷỹđ\
                          ÀÁẠẢÃÂẦẤẬẨẪĂẰẮẶẲẴÈÉẸẺẼÊỀẾỆỂỄÌÍỊỈĨÒÓỌỎÕÔỒỐỘỔỖƠỜỚỢỞỠÙÚỤỦŨƯỪỨỰỬỮỲÝỴỶỸĐ";

pub fn check(root: &Path) -> Result<Vec<Violation>, String> {
    let mut violations = Vec::new();
    for file in crate::lengths::rust_files(root)? {
        if !in_scope(&file) {
            continue;
        }
        let Ok(text) = fs::read_to_string(root.join(&file)) else {
            continue;
        };
        violations.extend(check_text(&file, &text));
    }
    Ok(violations)
}

/// File thuộc phần công khai (hợp đồng) mà luật áp vào.
fn in_scope(file: &str) -> bool {
    if file.starts_with("crates/kernel/src/")
        || file.starts_with("crates/testkit/src/")
        || file == "crates/app/src/lib.rs"
    {
        return true;
    }
    // `modules/<tên>/src/api.rs` hoặc `modules/<tên>/src/api/…`.
    let Some(rest) = file.strip_prefix("modules/") else {
        return false;
    };
    let Some((_, inside)) = rest.split_once('/') else {
        return false;
    };
    inside == "src/api.rs" || inside.starts_with("src/api/")
}

/// Phần không đọc đĩa của [`check`], để test được.
fn check_text(file: &str, text: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    // Đang ở trong khối ``` của doc hay không.
    let mut in_fence = false;
    for (index, line) in text.lines().enumerate() {
        let Some(doc) = doc_text(line) else {
            continue;
        };
        if doc.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        let prose = if in_fence { strip(doc, '"') } else { strip(doc, '`') };
        if prose.chars().any(|letter| VIETNAMESE.contains(letter)) {
            violations.push(Violation {
                rule: "doc-language",
                location: format!("{file}:{}", index + 1),
                message: "doc comment (`///`, `//!`) in Vietnamese: write it in English — this \
                          file is part of the public contract read on docs.rs; an explanation \
                          for maintainers can stay Vietnamese as a plain `//` comment \
                          (D-20261123-english-rustdoc)"
                    .to_owned(),
            });
        }
    }
    violations
}

/// Phần chữ sau `///` hoặc `//!`; `None` nếu dòng không phải doc comment (`////` là comment thường).
fn doc_text(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("////") {
        return None;
    }
    trimmed.strip_prefix("///").or_else(|| trimmed.strip_prefix("//!"))
}

/// Bỏ các đoạn nằm giữa hai dấu `quote` trên dòng (code span hoặc chuỗi).
fn strip(text: &str, quote: char) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut inside = false;
    for letter in text.chars() {
        if letter == quote {
            inside = !inside;
        } else if !inside {
            kept.push(letter);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pham_vi_la_phan_cong_khai() {
        assert!(in_scope("crates/kernel/src/module.rs"));
        assert!(in_scope("crates/testkit/src/lib.rs"));
        assert!(in_scope("crates/app/src/lib.rs"));
        assert!(in_scope("modules/notes/src/api.rs"));
        assert!(in_scope("modules/catalog/src/api/testing.rs"));
        assert!(!in_scope("crates/app/src/modules.rs"));
        assert!(!in_scope("modules/notes/src/service.rs"));
        assert!(!in_scope("modules/notes/src/api_v2.rs"));
        assert!(!in_scope("modules/notes/tests/integration/main.rs"));
        assert!(!in_scope("xtask/src/main.rs"));
    }

    #[test]
    fn doc_tieng_viet_bi_chan() {
        let text =
            "//! Hợp đồng module.\n\n/// Returns the note.\nfn a() {}\n    /// Trả về ghi chú.\n";
        let found = check_text("crates/kernel/src/module.rs", text);
        let locations: Vec<_> = found.iter().map(|found| found.location.as_str()).collect();
        assert_eq!(locations, ["crates/kernel/src/module.rs:1", "crates/kernel/src/module.rs:5"]);
        assert!(found[0].message.contains("English"), "{}", found[0].message);
    }

    #[test]
    fn comment_thuong_va_du_lieu_vi_du_duoc_phep() {
        let text = "// Giải thích vì sao cho chủ dự án.\n\
                    //// Không phải doc.\n\
                    /// Synonyms, e.g. `[[\"áo thun\", \"áo phông\"]]`.\n\
                    /// ```\n\
                    /// let title = \"Đường đến Đà Lạt\";\n\
                    /// ```\n\
                    let s = \"chuỗi\";\n";
        assert!(check_text("crates/kernel/src/a.rs", text).is_empty());
    }

    #[test]
    fn comment_tieng_viet_trong_khoi_code_cua_doc_bi_chan() {
        let text = "/// ```\n/// let x = 1; // giá trị đầu\n/// ```\n/// Đã đóng khối.\n";
        let found = check_text("crates/kernel/src/a.rs", text);
        let locations: Vec<_> = found.iter().map(|found| found.location.as_str()).collect();
        assert_eq!(locations, ["crates/kernel/src/a.rs:2", "crates/kernel/src/a.rs:4"]);
    }
}
