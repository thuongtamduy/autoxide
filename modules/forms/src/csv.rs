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

    /// Bộ đọc RFC 4180 tối giản: ô trong ngoặc kép, `""` là một ngoặc kép, dòng ngăn bằng CRLF.
    fn read(text: &str) -> Vec<Vec<String>> {
        let text = text.strip_prefix('\u{feff}').unwrap();
        let mut rows = vec![Vec::new()];
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' => {
                    let mut cell = String::new();
                    loop {
                        match chars.next().unwrap() {
                            '"' if chars.peek() == Some(&'"') => {
                                chars.next();
                                cell.push('"');
                            }
                            '"' => break,
                            other => cell.push(other),
                        }
                    }
                    rows.last_mut().unwrap().push(cell);
                }
                ',' => {}
                '\r' => {
                    assert_eq!(chars.next(), Some('\n'));
                    rows.push(Vec::new());
                }
                other => panic!("unexpected {other:?} outside a quoted cell"),
            }
        }
        assert!(rows.pop().unwrap().is_empty(), "the file ends with CRLF");
        rows
    }

    // Property test (đặc tả 31 B3): đọc lại ra đúng số dòng, số ô và nội dung; không ô nào mở
    // đầu bằng ký tự công thức mà thiếu `'`.
    proptest::proptest! {
        #[test]
        fn csv_doc_lai_dung_va_khong_co_cong_thuc(
            rows in proptest::collection::vec(proptest::collection::vec("\\PC{0,20}|[=+@\t\r-].{0,10}", 3), 0..8),
        ) {
            let header = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
            let text = write(&header, &rows);
            let read_back = read(&text);
            proptest::prop_assert_eq!(read_back.len(), rows.len() + 1);
            for (written, read) in rows.iter().zip(read_back.iter().skip(1)) {
                proptest::prop_assert_eq!(read.len(), written.len());
                for (value, cell) in written.iter().zip(read) {
                    proptest::prop_assert!(!cell.starts_with(['=', '+', '-', '@', '\t', '\r']), "{cell:?}");
                    let original = if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
                        format!("'{value}")
                    } else {
                        value.clone()
                    };
                    proptest::prop_assert_eq!(cell, &original);
                }
            }
        }
    }
}
