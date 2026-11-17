//! Ghi CSV (RFC 4180) có chặn CSV injection (bước 51 P16) — vài chục dòng tự viết, không cần thư
//! viện. Cùng quy ước với bản xuất của `forms`:
//!
//! - ô bắt đầu bằng `=`, `+`, `-`, `@`, tab hay CR thì thêm `'` vào đầu (theo OWASP: Excel /
//!   LibreOffice không coi là công thức nữa);
//! - mọi ô nằm trong ngoặc kép, ngoặc kép bên trong viết đôi;
//! - đầu file có BOM UTF-8 để Excel đọc đúng chữ tiếng Việt; xuống dòng là CRLF.

/// Đầu file: BOM UTF-8.
pub const BOM: &str = "\u{feff}";

/// Một dòng CSV (kèm CRLF).
pub fn line(row: &[String]) -> String {
    let cells: Vec<String> = row.iter().map(|cell| cell_text(cell)).collect();
    let mut out = cells.join(",");
    out.push_str("\r\n");
    out
}

fn cell_text(value: &str) -> String {
    let guarded = if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{value}")
    } else {
        value.to_owned()
    };
    format!("\"{}\"", guarded.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chan_csv_injection_va_thoat_ngoac_kep() {
        let row = ["=HYPERLINK(\"x\")", "+1", "-2", "@A1", "dòng 1\ndòng 2, \"trích\""]
            .map(str::to_owned);
        assert_eq!(
            line(&row),
            "\"'=HYPERLINK(\"\"x\"\")\",\"'+1\",\"'-2\",\"'@A1\",\"dòng 1\ndòng 2, \"\"trích\"\"\"\r\n"
        );
    }
}
