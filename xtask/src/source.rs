//! Đọc mã nguồn Rust đủ để các luật dạng văn bản không bị comment, chuỗi hay code test đánh lừa,
//! mà không cần bộ phân tích cú pháp (`syn`) — xtask không kéo thêm thư viện cho việc này.
//!
//! [`Source::new`] giữ hai bản cùng độ dài, cùng vị trí xuống dòng:
//! - `raw`: nguyên văn (để đọc chú thích miễn trừ);
//! - `code`: comment, chuỗi và ký tự đơn thay bằng dấu cách, phần `#[cfg(test)]` cũng thay bằng
//!   dấu cách — đếm ngoặc và tìm token trên bản này.
//!
//! Đây là lưới chặn lỗi vô ý dựa vào code đã qua `cargo fmt`, không phải trình biên dịch.

pub struct Source {
    pub raw: String,
    pub code: String,
}

impl Source {
    pub fn new(raw: &str) -> Self {
        let masked = mask(raw);
        let code = blank_test_items(&masked);
        Self { raw: raw.to_owned(), code }
    }

    /// Số dòng (từ 1) của vị trí byte `offset`.
    pub fn line_of(&self, offset: usize) -> usize {
        self.raw[..offset].bytes().filter(|byte| *byte == b'\n').count() + 1
    }

    /// Dòng thứ `line` (từ 1) của bản nguyên văn.
    pub fn raw_line(&self, line: usize) -> &str {
        self.raw.lines().nth(line.saturating_sub(1)).unwrap_or("")
    }

    /// Chú thích miễn trừ `// <marker> <lý do>` ở dòng `line` hay trong khối comment liền trên nó.
    /// Lý do phải có chữ: chỉ ghi `marker` không đủ.
    pub fn has_marker(&self, line: usize, marker: &str) -> bool {
        let accepted = |text: &str| {
            text.split_once("//")
                .and_then(|(_, comment)| comment.split_once(marker))
                .is_some_and(|(_, reason)| reason.trim().len() >= 3)
        };
        if accepted(self.raw_line(line)) {
            return true;
        }
        let mut above = line;
        while above > 1 {
            above -= 1;
            let text = self.raw_line(above).trim_start();
            if !text.starts_with("//") {
                return false;
            }
            if accepted(text) {
                return true;
            }
        }
        false
    }
}

/// Thay comment, chuỗi (kể cả `r#"…"#`, `b"…"`) và ký tự đơn bằng dấu cách, giữ xuống dòng.
fn mask(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = bytes.to_vec();
    let blank = |out: &mut [u8], from: usize, to: usize| {
        for byte in &mut out[from..to] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    };
    let mut index = 0;
    while index < bytes.len() {
        let rest = &bytes[index..];
        if rest.starts_with(b"//") {
            let end = rest.iter().position(|b| *b == b'\n').map_or(bytes.len(), |p| index + p);
            blank(&mut out, index, end);
            index = end;
        } else if rest.starts_with(b"/*") {
            // Comment khối lồng nhau được trong Rust.
            let (mut depth, mut at) = (0usize, index);
            while at < bytes.len() {
                if bytes[at..].starts_with(b"/*") {
                    depth += 1;
                    at += 2;
                } else if bytes[at..].starts_with(b"*/") {
                    depth -= 1;
                    at += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    at += 1;
                }
            }
            blank(&mut out, index, at);
            index = at;
        } else if let Some((hashes, open)) = raw_string_start(bytes, index) {
            let mut close = vec![b'"'];
            close.extend(std::iter::repeat_n(b'#', hashes));
            let body = index + open;
            let end = bytes[body..]
                .windows(close.len())
                .position(|window| window == close.as_slice())
                .map_or(bytes.len(), |p| body + p + close.len());
            blank(&mut out, index, end);
            index = end;
        } else if bytes[index] == b'"' {
            let mut at = index + 1;
            while at < bytes.len() && bytes[at] != b'"' {
                at += if bytes[at] == b'\\' { 2 } else { 1 };
            }
            let end = (at + 1).min(bytes.len());
            blank(&mut out, index, end);
            index = end;
        } else if bytes[index] == b'\'' {
            // Ký tự đơn `'x'`, `'\n'`, `'{'`; lifetime (`'a`) không có dấu đóng ngay sau.
            let end = char_literal_end(raw, index);
            if let Some(end) = end {
                blank(&mut out, index, end);
                index = end;
            } else {
                index += 1;
            }
        } else {
            index += 1;
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

/// `r"`, `r#"`, `br##"`… bắt đầu ở `index` (không nằm giữa một tên): số `#` và độ dài phần mở.
fn raw_string_start(bytes: &[u8], index: usize) -> Option<(usize, usize)> {
    if index > 0 && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_') {
        return None;
    }
    let mut at = index;
    if bytes.get(at) == Some(&b'b') {
        at += 1;
    }
    if bytes.get(at) != Some(&b'r') {
        return None;
    }
    at += 1;
    let hashes = bytes[at..].iter().take_while(|b| **b == b'#').count();
    at += hashes;
    (bytes.get(at) == Some(&b'"')).then_some((hashes, at + 1 - index))
}

fn char_literal_end(raw: &str, index: usize) -> Option<usize> {
    let rest = &raw[index + 1..];
    let mut chars = rest.char_indices();
    let (_, first) = chars.next()?;
    if first == '\\' {
        // `'\n'`, `'\''`, `'\u{1F600}'`: dấu đóng trong vài ký tự tới.
        let close = rest[1..].find('\'')?;
        return (close <= 10).then_some(index + 1 + 1 + close + 1);
    }
    let (next, second) = chars.next()?;
    (second == '\'').then_some(index + 1 + next + 1)
}

/// Thay bằng dấu cách mỗi mục gắn `#[cfg(test)]` (thường là `mod tests { … }` cuối file).
fn blank_test_items(code: &str) -> String {
    let mut out = code.as_bytes().to_vec();
    let mut search = 0;
    while let Some(found) = code[search..].find("#[cfg(test)]") {
        let start = search + found;
        let Some(end) = item_end(code, start + "#[cfg(test)]".len()) else { break };
        for byte in &mut out[start..end] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
        search = end;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Cuối mục bắt đầu sau `from`: `;` trước mọi `{`, hoặc `}` khớp với `{` đầu tiên.
fn item_end(code: &str, from: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, byte) in code.as_bytes()[from..].iter().enumerate() {
        match byte {
            b';' if depth == 0 => return Some(from + offset + 1),
            b'{' => depth += 1,
            b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(from + offset + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Đuôi biểu thức bắt đầu ở `start` (đã ở trong biểu thức): dừng ở `;` hay `,` cùng cấp, hay ở
/// ngoặc đóng của ngoặc bao ngoài.
pub fn expression_end(code: &str, start: usize) -> usize {
    let mut depth = 0i32;
    for (offset, byte) in code.as_bytes()[start..].iter().enumerate() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth < 0 {
                    return start + offset;
                }
            }
            b';' | b',' if depth == 0 => return start + offset,
            _ => {}
        }
    }
    code.len()
}

/// Cuối khối `{ … }` đang bao vị trí `at`.
pub fn enclosing_block_end(code: &str, at: usize) -> usize {
    let mut depth = 0i32;
    for (offset, byte) in code.as_bytes()[at..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth < 0 {
                    return at + offset;
                }
            }
            _ => {}
        }
    }
    code.len()
}

pub fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Vị trí mỗi lần `word` xuất hiện như một tên trọn vẹn (không dính chữ hai bên).
pub fn find_word(text: &str, word: &str) -> Vec<usize> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut search = 0;
    while let Some(offset) = text[search..].find(word) {
        let at = search + offset;
        let end = at + word.len();
        let before = at == 0 || !is_ident_byte(bytes[at - 1]);
        let after = end >= bytes.len() || !is_ident_byte(bytes[end]);
        if before && after {
            found.push(at);
        }
        search = end;
    }
    found
}

/// File trong `src/` chỉ chứa test (`src/tests/…`, `…/tests.rs`, `…_tests.rs`).
pub fn is_test_file(path: &str) -> bool {
    path.contains("/tests/") || path.ends_with("/tests.rs") || path.ends_with("_tests.rs")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn che_comment_chuoi_va_phan_test() {
        let raw = "let a = \"x { y\"; // z {\nlet c = '{'; let r = r#\"q\"#;\nfn f<'a>() {}\n\
                   #[cfg(test)]\nmod tests {\n    fn t() { find() }\n}\nfn after() {}\n";
        let source = Source::new(raw);
        assert_eq!(source.code.len(), raw.len());
        assert!(!source.code.contains("x {"));
        assert!(!source.code.contains("z {"));
        assert!(!source.code.contains("'{'"));
        assert!(!source.code.contains("\"q\""));
        assert!(source.code.contains("fn f<'a>() {}"));
        assert!(!source.code.contains("find()"));
        assert!(source.code.contains("fn after() {}"));
        assert_eq!(source.line_of(raw.find("fn after").unwrap()), 8);
    }

    #[test]
    fn chu_thich_mien_tru_can_ly_do() {
        let source =
            Source::new("// x-ok: tool đọc cả khách\n// thêm\nlet q = 1;\nlet r = 2; // x-ok:\n");
        assert!(source.has_marker(3, "x-ok:"));
        assert!(!source.has_marker(4, "x-ok:"));
    }

    #[test]
    fn duoi_bieu_thuc() {
        let code = "f(a::find().b(c, d), e);";
        let start = code.find("a::").unwrap();
        assert_eq!(&code[start..expression_end(code, start)], "a::find().b(c, d)");
    }
}
