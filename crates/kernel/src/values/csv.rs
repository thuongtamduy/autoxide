//! Reading and writing CSV (RFC 4180) with protection against CSV injection — written in-house, no library
//! needed (D-20261118-step41-kernel-prep). Shared by every export / import: `forms`, the kernel's lists (step
//! 51 P16), `catalog` and the commerce modules (steps 41–49).
//!
//! Writing:
//! - a cell starting with `=`, `+`, `-`, `@`, tab or CR gets a `'` prepended (per OWASP: Excel /
//!   LibreOffice then no longer treat it as a formula);
//! - every cell is quoted, and quotes inside are doubled;
//! - the file starts with a UTF-8 BOM so Excel reads Vietnamese text correctly; line breaks are CRLF.
//!
//! Reading: strips the BOM, accepts mixed CRLF and LF, cells with or without quotes, `""` inside quotes as
//! one quote, and line breaks inside quoted cells; errors give the line number (counted from 1, by the file's
//! lines).

/// Start of file: the UTF-8 BOM.
pub const BOM: &str = "\u{feff}";

/// One CSV row (with CRLF).
pub fn line(row: &[String]) -> String {
    let cells: Vec<String> = row.iter().map(|cell| cell_text(cell)).collect();
    let mut out = cells.join(",");
    out.push_str("\r\n");
    out
}

/// The full CSV content: BOM, header row, then each row.
pub fn write(header: &[String], rows: &[Vec<String>]) -> String {
    let mut out = String::from(BOM);
    for row in std::iter::once(header).chain(rows.iter().map(Vec::as_slice)) {
        out.push_str(&line(row));
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

/// Error when reading CSV. `line` is the file's line number (counted from 1) where the error occurred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsvError {
    /// An opening quote that is not closed by the end of the file.
    UnterminatedQuote { line: usize },
    /// A character right after the closing quote that is not `,` or a line break.
    UnexpectedCharacter { line: usize },
    /// More than `max` records (including the header row).
    TooManyRows { max: usize },
}

impl std::fmt::Display for CsvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CsvError::UnterminatedQuote { line } => {
                write!(f, "line {line}: a quoted cell is never closed")
            }
            CsvError::UnexpectedCharacter { line } => {
                write!(f, "line {line}: unexpected character after a closing quote")
            }
            CsvError::TooManyRows { max } => write!(f, "the file has more than {max} rows"),
        }
    }
}

impl std::error::Error for CsvError {}

/// Reads the whole `text` into records (the header row is the first record). Blank lines are skipped. Past
/// `max_rows` records it stops and reports [`CsvError::TooManyRows`] — the caller sets the limit so one
/// large file cannot take up all the memory.
pub fn read(text: &str, max_rows: usize) -> Result<Vec<Vec<String>>, CsvError> {
    let text = text.strip_prefix(BOM).unwrap_or(text);
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut line_number = 1;
    // `true` khi bản ghi đang dở đã có ít nhất một ô hay một ký tự: phân biệt dòng trống.
    let mut started = false;
    let mut chars = text.chars().peekable();

    let finish_row = |row: &mut Vec<String>, rows: &mut Vec<Vec<String>>| {
        let done = std::mem::take(row);
        if done.len() == 1 && done.first().is_some_and(String::is_empty) {
            return Ok(());
        }
        if rows.len() >= max_rows {
            return Err(CsvError::TooManyRows { max: max_rows });
        }
        rows.push(done);
        Ok(())
    };

    while let Some(c) = chars.next() {
        match c {
            '"' if cell.is_empty() => {
                let start_line = line_number;
                loop {
                    match chars.next() {
                        None => return Err(CsvError::UnterminatedQuote { line: start_line }),
                        Some('"') if chars.peek() == Some(&'"') => {
                            chars.next();
                            cell.push('"');
                        }
                        Some('"') => break,
                        Some(other) => {
                            if other == '\n' {
                                line_number += 1;
                            }
                            cell.push(other);
                        }
                    }
                }
                started = true;
                match chars.peek() {
                    None | Some(',' | '\r' | '\n') => {}
                    Some(_) => return Err(CsvError::UnexpectedCharacter { line: line_number }),
                }
            }
            ',' => {
                row.push(std::mem::take(&mut cell));
                started = true;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                row.push(std::mem::take(&mut cell));
                finish_row(&mut row, &mut rows)?;
                started = false;
                line_number += 1;
            }
            other => {
                cell.push(other);
                started = true;
            }
        }
    }
    if started {
        row.push(cell);
        finish_row(&mut row, &mut rows)?;
    }
    Ok(rows)
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

    #[test]
    fn doc_ca_lf_crlf_o_khong_ngoac_va_dong_trong() {
        let text = "\u{feff}sku,name,price\nA-1,Áo \"thun\",100\r\n\r\n\"B,2\",\"hai\ndòng\",200";
        assert_eq!(
            read(text, 10).unwrap(),
            vec![
                vec!["sku".to_owned(), "name".to_owned(), "price".to_owned()],
                vec!["A-1".to_owned(), "Áo \"thun\"".to_owned(), "100".to_owned()],
                vec!["B,2".to_owned(), "hai\ndòng".to_owned(), "200".to_owned()],
            ]
        );
    }

    #[test]
    fn doc_bao_loi_kem_so_dong() {
        assert_eq!(read("a\n\"b\nc", 10), Err(CsvError::UnterminatedQuote { line: 2 }));
        assert_eq!(read("a\nb\n\"c\"x", 10), Err(CsvError::UnexpectedCharacter { line: 3 }));
        assert_eq!(read("a\nb\nc\n", 2), Err(CsvError::TooManyRows { max: 2 }));
        assert_eq!(read("a,\n", 10).unwrap(), vec![vec!["a".to_owned(), String::new()]]);
    }

    // Property test (đặc tả 31 B3, đặc tả 41): ghi rồi đọc lại ra đúng số dòng, số ô và nội
    // dung; không ô nào mở đầu bằng ký tự công thức mà thiếu `'`.
    proptest::proptest! {
        #[test]
        fn csv_doc_ghi_khu_hoi(
            rows in proptest::collection::vec(proptest::collection::vec("\\PC{0,20}|[=+@\t\r-].{0,10}", 3), 0..8),
        ) {
            let header = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
            let text = write(&header, &rows);
            let read_back = read(&text, 100).unwrap();
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
