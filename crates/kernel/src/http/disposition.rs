//! Header `Content-Disposition` của response tải file (RFC 6266).

/// Trình duyệt mở file ngay trong trang (`Inline`) hay tải về (`Attachment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Inline,
    Attachment,
}

/// Giá trị `Content-Disposition` kèm tên file: `filename` chỉ chữ ASCII an toàn (cho
/// client cũ), `filename*` là tên đầy đủ mã hoá UTF-8 (RFC 8187).
pub fn content_disposition(disposition: Disposition, name: &str) -> String {
    let kind = match disposition {
        Disposition::Inline => "inline",
        Disposition::Attachment => "attachment",
    };
    let fallback: String = name
        .chars()
        .map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' })
        .collect();
    let encoded: String = name
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    format!("{kind}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_file_trong_content_disposition() {
        assert_eq!(
            content_disposition(Disposition::Attachment, "báo cáo \"Q1\".pdf"),
            "attachment; filename=\"b_o c_o _Q1_.pdf\"; \
             filename*=UTF-8''b%C3%A1o%20c%C3%A1o%20%22Q1%22.pdf"
        );
        assert!(content_disposition(Disposition::Inline, "a.png").starts_with("inline; "));
    }
}
