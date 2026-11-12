//! Ghi CSV (RFC 4180) có chặn CSV injection — vài chục dòng tự viết, không cần thư viện.
//!
//! Chặn injection theo OWASP: ô bắt đầu bằng `=`, `+`, `-`, `@`, tab hay CR thì thêm `'` vào
//! đầu (Excel / LibreOffice không coi là công thức nữa). Mọi ô đều nằm trong ngoặc kép, ngoặc
//! kép bên trong viết đôi. Đầu file có BOM UTF-8 để Excel đọc đúng chữ tiếng Việt; xuống dòng
//! là CRLF.

/// Nội dung CSV của `header` và `rows`.
pub(crate) fn write(header: &[String], rows: &[Vec<String>]) -> String {
    let mut out = String::from('\u{feff}');
    for row in std::iter::once(header).chain(rows.iter().map(Vec::as_slice)) {
        let cells: Vec<String> = row.iter().map(|cell| cell_text(cell)).collect();
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
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
        let header = vec!["a".to_owned(), "b".to_owned()];
        let rows = vec![
            vec!["=HYPERLINK(\"http://evil\")".to_owned(), "+1".to_owned()],
            vec!["-2".to_owned(), "@SUM(A1)".to_owned()],
            vec!["\tx".to_owned(), "dòng 1\ndòng 2, \"trích\"".to_owned()],
        ];
        let text = write(&header, &rows);
        assert_eq!(
            text,
            "\u{feff}\"a\",\"b\"\r\n\
             \"'=HYPERLINK(\"\"http://evil\"\")\",\"'+1\"\r\n\
             \"'-2\",\"'@SUM(A1)\"\r\n\
             \"'\tx\",\"dòng 1\ndòng 2, \"\"trích\"\"\"\r\n"
        );
    }
}
