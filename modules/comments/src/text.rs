//! Chữ của bình luận: lọc, đếm link, dựng HTML an toàn, so từ cấm, email, tên.
//!
//! Bản đầu chỉ có **chữ thường** (D-112 mục 11 của khảo sát): máy chủ lưu đúng chữ người viết
//! (đã lọc ký tự điều khiển) và dựng HTML khi trả về: escape mọi ký tự đặc biệt, xuống dòng thành
//! `<br>`, chỉ địa chỉ `http://` / `https://` thành link, có `rel="nofollow ugc noopener"`. Tự
//! viết vài chục dòng thay vì thêm thư viện (Markdown + sanitizer để sau).

use autoxide_kernel::error::FieldError;

/// Giá trị `rel` của mọi link trong bình luận (Google khuyên `ugc` cho nội dung người dùng).
pub(crate) const LINK_REL: &str = "nofollow ugc noopener";

/// Lọc nội dung: xuống dòng kiểu Windows / Mac cũ thành `\n`, bỏ khoảng trắng cuối dòng và đầu
/// / cuối bài, gộp hơn hai dòng trống. Ký tự điều khiển khác (trừ tab) là lỗi — website gửi chữ
/// người gõ, không bao giờ có chúng.
pub(crate) fn clean_body(raw: &str, max: usize, problems: &mut Vec<FieldError>) -> String {
    let unified = raw.replace("\r\n", "\n").replace('\r', "\n");
    if unified.chars().any(|c| (c.is_control() && c != '\n' && c != '\t') || is_invisible(c)) {
        problems.push(FieldError::new(
            "body",
            "invalid",
            "must not contain control or invisible formatting characters",
        ));
    }
    let mut lines: Vec<&str> = Vec::new();
    let mut blank = 0;
    for line in unified.split('\n').map(str::trim_end) {
        if line.is_empty() {
            blank += 1;
            if blank > 2 {
                continue;
            }
        } else {
            blank = 0;
        }
        lines.push(line);
    }
    let body = lines.join("\n").trim().to_owned();
    let length = body.chars().count();
    if length == 0 {
        problems.push(FieldError::new("body", "required", "must not be empty"));
    } else if length > max {
        problems.push(FieldError::new(
            "body",
            "length",
            format!("must be at most {max} characters long"),
        ));
    }
    body
}

/// Ký tự định dạng vô hình dùng để giấu hay đảo chữ (bidi override, zero-width…).
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

/// Ký tự định dạng (nhóm Unicode `Cf`, theo Unicode 15): không hiện ra nhưng đổi cách chữ được
/// hiện / so (gạch nối mềm, dấu hướng chữ, ký tự thẻ…). Tự liệt kê thay vì thêm thư viện bảng
/// Unicode: nhóm này ít và hiếm khi đổi.
fn is_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061C}'
            | '\u{06DD}'
            | '\u{070F}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08E2}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{110BD}'
            | '\u{110CD}'
            | '\u{13430}'..='\u{1343F}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// Ký tự trông như khoảng trống mà không phải khoảng trắng (nên `split_whitespace` không gộp):
/// chữ đệm Hangul, dấu nối tổ hợp, nguyên âm câm Khmer, bộ chọn biến thể Mông Cổ, ô chữ nổi
/// trống. Dùng để làm tên "trống" hay giả tên người khác.
fn is_blank_lookalike(c: char) -> bool {
    matches!(
        c,
        '\u{034F}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}' | '\u{180B}'
            ..='\u{180D}' | '\u{180F}' | '\u{2800}' | '\u{3164}' | '\u{FFA0}'
    )
}

/// Bảng chữ của một chữ cái, cho ba bảng có nhiều chữ trông giống nhau.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Script {
    Latin,
    Greek,
    Cyrillic,
}

fn script(c: char) -> Option<Script> {
    if !c.is_alphabetic() {
        return None;
    }
    match c {
        'A'..='Z' | 'a'..='z' | '\u{00C0}'..='\u{024F}' | '\u{1E00}'..='\u{1EFF}' => {
            Some(Script::Latin)
        }
        '\u{0370}'..='\u{03FF}' | '\u{1F00}'..='\u{1FFF}' => Some(Script::Greek),
        '\u{0400}'..='\u{052F}' => Some(Script::Cyrillic),
        _ => None,
    }
}

/// Có từ nào trộn chữ Latin, Hy Lạp, Kirin không — cách giả tên quen thuộc (`аdmin` với `а`
/// Kirin qua mặt danh sách tên dành riêng). Một từ viết trọn một bảng chữ thì được.
fn mixes_scripts(name: &str) -> bool {
    name.split_whitespace().any(|word| {
        let mut seen = None;
        word.chars().filter_map(script).any(|found| *seen.get_or_insert(found) != found)
    })
}

/// Escape chữ để đặt vào HTML (cả trong giá trị thuộc tính có ngoặc kép).
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Một đoạn của bài: chữ thường hoặc link.
#[derive(Debug, PartialEq, Eq)]
enum Piece<'a> {
    Text(&'a str),
    Link(&'a str),
}

/// Tách bài thành chữ và link `http(s)://…`. Link kết thúc ở khoảng trắng hay ký tự không thể
/// nằm trong URL viết tay (`<`, `>`, `"`, `'`); dấu câu cuối (`.`, `,`, `)`…) không thuộc link.
fn pieces(body: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(start) = find_scheme(rest) {
        let candidate = &rest[start..];
        let end = candidate
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\''))
            .unwrap_or(candidate.len());
        let mut link = &candidate[..end];
        loop {
            let trimmed = link.trim_end_matches(['.', ',', ';', ':', '!', '?', ']', '}']);
            // `)` cuối thuộc link chỉ khi link có `(` (vd trang Wikipedia).
            let trimmed = match trimmed.strip_suffix(')') {
                Some(inner) if !inner.contains('(') => inner,
                _ => trimmed,
            };
            if trimmed.len() == link.len() {
                break;
            }
            link = trimmed;
        }
        // `find_scheme` chỉ trả vị trí của `http://` / `https://` (ASCII): cắt theo byte an toàn.
        let scheme = if candidate.get(..8).is_some_and(|head| head.eq_ignore_ascii_case("https://"))
        {
            8
        } else {
            7
        };
        if link.len() <= scheme {
            // Chỉ có `http://`: không phải link.
            out.push(Piece::Text(&rest[..start + scheme]));
            rest = &rest[start + scheme..];
            continue;
        }
        if start > 0 {
            out.push(Piece::Text(&rest[..start]));
        }
        out.push(Piece::Link(link));
        rest = &rest[start + link.len()..];
    }
    if !rest.is_empty() {
        out.push(Piece::Text(rest));
    }
    out
}

/// Vị trí `http://` hay `https://` (không phân biệt hoa thường) đầu tiên đứng ở đầu một từ.
fn find_scheme(text: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(offset) = lower[from..].find("http") {
        let at = from + offset;
        let tail = &lower[at..];
        let starts_word =
            at == 0 || !text[..at].chars().next_back().is_some_and(char::is_alphanumeric);
        if starts_word && (tail.starts_with("http://") || tail.starts_with("https://")) {
            return Some(at);
        }
        from = at + 4;
    }
    None
}

/// Số link trong bài.
pub(crate) fn link_count(body: &str) -> usize {
    pieces(body).iter().filter(|piece| matches!(piece, Piece::Link(_))).count()
}

/// HTML an toàn của bài: mọi chữ đã escape, xuống dòng thành `<br>`, link `http(s)` có
/// `rel="nofollow ugc noopener"`.
pub(crate) fn to_html(body: &str) -> String {
    let mut html = String::with_capacity(body.len() + 32);
    for piece in pieces(body) {
        match piece {
            Piece::Text(text) => html.push_str(&escape(text).replace('\n', "<br>\n")),
            Piece::Link(link) => {
                let link = escape(link);
                html.push_str(&format!("<a href=\"{link}\" rel=\"{LINK_REL}\">{link}</a>"));
            }
        }
    }
    html
}

/// Dạng đem so của từ / tên: không hoa thường, không dấu, mỗi từ cách một khoảng trắng.
pub(crate) fn words_key(text: &str) -> String {
    autoxide_kernel::search::fold(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Chữ Kirin / Hy Lạp trông giống một chữ Latin thì đổi thành chữ Latin đó; chữ khác giữ nguyên.
/// Xét theo hoa / thường (`Н` Kirin hoa giống `H`, `н` thường thì không). Bảng tự liệt kê, chỉ gồm
/// những chữ dễ nhầm trong phông chữ thường gặp (theo tinh thần "skeleton" của Unicode TS #39),
/// thay vì thêm thư viện bảng confusables đầy đủ.
fn latin_lookalike(c: char) -> char {
    match c {
        // Kirin hoa.
        'А' => 'A',
        'В' => 'B',
        'Е' => 'E',
        'Ѕ' => 'S',
        'І' | 'Ӏ' => 'I',
        'Ј' => 'J',
        'К' => 'K',
        'М' => 'M',
        'Н' => 'H',
        'О' => 'O',
        'Р' => 'P',
        'С' => 'C',
        'Т' => 'T',
        'У' | 'Ү' => 'Y',
        'Х' => 'X',
        'Ԛ' => 'Q',
        'Ԝ' => 'W',
        // Kirin thường.
        'а' => 'a',
        'ԁ' => 'd',
        'е' => 'e',
        'һ' => 'h',
        'і' => 'i',
        'ј' => 'j',
        'к' => 'k',
        'ӏ' => 'l',
        'м' => 'm',
        'п' => 'n',
        'о' => 'o',
        'р' => 'p',
        'ԛ' => 'q',
        'ѕ' => 's',
        'с' => 'c',
        'у' | 'ү' => 'y',
        'ԝ' => 'w',
        'х' => 'x',
        'ь' => 'b',
        // Hy Lạp hoa.
        'Α' => 'A',
        'Β' => 'B',
        'Ε' => 'E',
        'Ζ' => 'Z',
        'Η' => 'H',
        'Ι' => 'I',
        'Κ' => 'K',
        'Μ' => 'M',
        'Ν' => 'N',
        'Ο' => 'O',
        'Ρ' => 'P',
        'Τ' => 'T',
        'Υ' => 'Y',
        'Χ' => 'X',
        // Hy Lạp thường.
        'α' => 'a',
        'ϲ' => 'c',
        'ι' => 'i',
        'κ' => 'k',
        'ν' => 'v',
        'ο' => 'o',
        'ρ' => 'p',
        'υ' => 'u',
        'χ' => 'x',
        other => other,
    }
}

/// Dạng đem so của TÊN: như [`words_key`], rồi chữ Kirin / Hy Lạp trông như chữ Latin thành chữ
/// Latin. "Аdmin" viết trọn chữ Kirin (`Аԁміп`) ra cùng khoá với "admin": không qua được danh
/// sách tên dành riêng, không chiếm được tên người khác. Chỉ dùng cho tên — đem dùng cho nội
/// dung thì từ cấm tiếng Nga sẽ khớp nhầm chữ Latin.
pub(crate) fn name_key(name: &str) -> String {
    // Đổi hai lần: trước khi viết thường (để phân biệt `Н` hoa với `н` thường) và sau khi bỏ dấu
    // (chữ có dấu như `ё` tách thành `е` + dấu).
    let before: String = name.chars().map(latin_lookalike).collect();
    let folded: String = words_key(&before).chars().map(latin_lookalike).collect();
    folded
}

/// Tên `name` có chứa trọn một tên / cụm tên nào trong `list` (đã qua [`words_key`]) không, so
/// theo [`name_key`] ở cả hai phía.
pub(crate) fn contains_name(name: &str, list: &[String]) -> bool {
    if list.is_empty() {
        return false;
    }
    let haystack = format!(" {} ", name_key(name));
    list.iter().any(|word| haystack.contains(&format!(" {} ", name_key(word))))
}

/// `body` có chứa trọn một từ / cụm từ nào trong `list` (đã qua [`words_key`]) không.
pub(crate) fn contains_word(body: &str, list: &[String]) -> bool {
    if list.is_empty() {
        return false;
    }
    let haystack = format!(" {} ", words_key(body));
    list.iter().any(|word| haystack.contains(&format!(" {word} ")))
}

/// Tên hiển thị (khách hay hồ sơ người dùng): 1–50 ký tự; không ký tự điều khiển, ký tự định
/// dạng vô hình (`Cf`) hay ký tự trông như khoảng trống; không trộn chữ Latin / Hy Lạp / Kirin
/// trong một từ. Trả tên đã bỏ khoảng trắng thừa.
pub(crate) fn clean_name(raw: &str, field: &str, problems: &mut Vec<FieldError>) -> String {
    let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let length = name.chars().count();
    if length == 0 {
        problems.push(FieldError::new(field, "required", "must not be empty"));
    } else if length > 50 {
        problems.push(FieldError::new(field, "length", "must be at most 50 characters long"));
    } else if name.chars().any(|c| c.is_control() || is_format(c) || is_blank_lookalike(c)) {
        problems.push(FieldError::new(
            field,
            "invalid",
            "must not contain control, invisible or blank-looking characters",
        ));
    } else if mixes_scripts(&name) {
        problems.push(FieldError::new(
            field,
            "invalid",
            "must not mix Latin, Greek and Cyrillic letters in one word",
        ));
    }
    name
}

/// ID của đích do module sở hữu cấp: 1–100 ký tự chữ, số, `-`, `_`, `.`, `:`.
pub(crate) fn is_target_id(id: &str) -> bool {
    (1..=100).contains(&id.len())
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(raw: &str) -> (String, Vec<String>) {
        let mut problems = Vec::new();
        let body = clean_body(raw, 50, &mut problems);
        (body, problems.into_iter().map(|p| p.code).collect())
    }

    #[test]
    fn loc_noi_dung() {
        assert_eq!(clean("  Xin chào \r\n\r\n\r\n\r\nbạn  \n").0, "Xin chào\n\n\nbạn");
        assert_eq!(clean("   \n ").1, ["required"]);
        assert_eq!(clean(&"x".repeat(51)).1, ["length"]);
        assert_eq!(clean("a\u{0007}b").1, ["invalid"]);
        assert_eq!(clean("a\u{202E}b").1, ["invalid"]);
        assert!(clean("tab\tok").1.is_empty());
    }

    #[test]
    fn xss_bi_chan() {
        let html = to_html("<script>alert(1)</script> <img src=x onerror=alert(1)>");
        assert_eq!(
            html,
            "&lt;script&gt;alert(1)&lt;/script&gt; &lt;img src=x onerror=alert(1)&gt;"
        );
        // `javascript:` và scheme khác không bao giờ thành link.
        for text in
            ["javascript:alert(1)", "JaVaScRiPt:alert(1)", "data:text/html,<b>x</b>", "vbscript:x"]
        {
            assert!(!to_html(text).contains("<a "), "{text}");
        }
        // Link có dấu ngoặc kép / thuộc tính chèn vào thì bị cắt trước ký tự đó và escape.
        let html = to_html("https://example.com/\"onmouseover=\"alert(1)");
        assert_eq!(
            html,
            "<a href=\"https://example.com/\" rel=\"nofollow ugc noopener\">https://example.com/</a>&quot;onmouseover=&quot;alert(1)"
        );
        let html = to_html("x https://a.example/?q=<b>&r=1");
        assert!(html.contains("href=\"https://a.example/?q=\""), "{html}");
        assert!(html.contains("&lt;b&gt;&amp;r=1"), "{html}");
        assert!(!html.contains("<b>"), "{html}");
    }

    #[test]
    fn link_va_xuong_dong() {
        let html = to_html("Xem https://example.com/a_(b), và HTTP://Example.com.\nHết");
        assert_eq!(
            html,
            "Xem <a href=\"https://example.com/a_(b)\" rel=\"nofollow ugc noopener\">https://example.com/a_(b)</a>, và <a href=\"HTTP://Example.com\" rel=\"nofollow ugc noopener\">HTTP://Example.com</a>.<br>\nHết"
        );
        assert_eq!(
            link_count("a https://x.example b http://y.example/ c http:// d xhttp://z.example"),
            2
        );
        assert_eq!(
            to_html("(see https://x.example)"),
            "(see <a href=\"https://x.example\" rel=\"nofollow ugc noopener\">https://x.example</a>)"
        );
    }

    #[test]
    fn tu_cam_theo_tu_tron() {
        let list = vec![words_key("Mua bán"), words_key("casino")];
        assert!(contains_word("Chỗ này MUA BÁN đồ", &list));
        assert!(contains_word("CASINO!", &list));
        assert!(!contains_word("casinos are a word apart", &list));
        assert!(!contains_word("muaban", &list));
    }

    #[test]
    fn email_va_ten() {
        assert!(autoxide_kernel::email::is_valid("ada@example.com"));
        assert!(!autoxide_kernel::email::is_valid("ada@example"));
        let mut problems = Vec::new();
        assert_eq!(clean_name("  Ada   Lovelace ", "name", &mut problems), "Ada Lovelace");
        assert!(problems.is_empty());
        assert!(is_target_id("0190c2a4-1b2c-7d3e-8f40-123456789abc"));
    }

    #[test]
    fn ten_khong_co_ky_tu_vo_hinh_hay_tron_bang_chu() {
        let check = |raw: &str| {
            let mut problems = Vec::new();
            clean_name(raw, "name", &mut problems);
            problems.into_iter().map(|p| p.code).collect::<Vec<_>>()
        };
        for hidden in [
            '\u{00AD}',
            '\u{034F}',
            '\u{061C}',
            '\u{115F}',
            '\u{1160}',
            '\u{180E}',
            '\u{200B}',
            '\u{202E}',
            '\u{2066}',
            '\u{3164}',
            '\u{FEFF}',
            '\u{FFA0}',
            '\u{E0001}',
            '\u{E0041}',
            '\u{E007F}',
        ] {
            assert_eq!(check(&format!("Ad{hidden}a")), ["invalid"], "U+{:04X}", hidden as u32);
        }
        // Tên chỉ có chữ đệm: không còn "trống" để giả người khác.
        assert_eq!(check("\u{3164}"), ["invalid"]);
        // `а` Kirin trong chữ Latin: giả `admin`.
        assert_eq!(check("\u{0430}dmin"), ["invalid"]);
        assert_eq!(check("Ad\u{03B1}"), ["invalid"]);
        // Một bảng chữ mỗi từ thì được, kể cả tiếng Việt, Nga, Hy Lạp, Nhật.
        for name in ["Nguyễn Thị Ánh", "Анна Каренина", "Σωκράτης", "山田 太郎", "Ада Lovelace"]
        {
            assert!(check(name).is_empty(), "{name}");
        }
        assert!(!is_target_id("a/b"));
    }

    #[test]
    fn ten_gia_chu_latin_ra_cung_khoa() {
        // Viết trọn chữ Kirin (không trộn bảng chữ nên lọt `clean_name`) mà trông như "admin".
        let fake_admin = "\u{0430}\u{0501}\u{043C}\u{0456}\u{043F}";
        let mut problems = Vec::new();
        clean_name(fake_admin, "name", &mut problems);
        assert!(problems.is_empty());
        assert_eq!(name_key(fake_admin), "admin");
        // Hoa: `С`, `О` Kirin; `Μ`, `Ο` Hy Lạp.
        assert_eq!(name_key("\u{0421}\u{041E}\u{0421}\u{041E}"), name_key("COCO"));
        assert_eq!(name_key("\u{039C}\u{039F}"), "mo");
        // `Н` hoa giống `H`, `н` thường thì không.
        assert_eq!(name_key("\u{041D}"), "h");
        assert_eq!(name_key("\u{043D}"), "\u{043D}");
        // Có dấu: `ё` tách thành `е` + dấu, rồi thành `e`.
        assert_eq!(name_key("\u{0451}"), "e");
        // Chữ Latin giữ nguyên như `words_key`.
        assert_eq!(name_key("  Ada   Lovelace "), "ada lovelace");

        let reserved = ["admin".to_owned(), "moderator".to_owned()];
        assert!(contains_name(fake_admin, &reserved));
        assert!(contains_name(&format!("{fake_admin} team"), &reserved));
        // Tên tiếng Nga, Hy Lạp thật không bị coi là tên dành riêng.
        assert!(!contains_name("Анна Каренина", &reserved));
        assert!(!contains_name("Σωκράτης", &reserved));
    }

    // Property test (đặc tả 31 B3, chống XSS): với chữ bất kỳ, HTML ra chỉ có `<a href="http(s)://…"
    // rel="nofollow ugc noopener">`, `</a>`, `<br>`; `escape` không để lọt ký tự HTML thô; số link
    // đếm được bằng số thẻ `<a`.
    fn tags_are_safe(html: &str) -> Result<(), String> {
        let open = format!("\" rel=\"{LINK_REL}\">");
        let mut rest = html;
        while let Some(at) = rest.find('<') {
            let tail = &rest[at..];
            if let Some(after) = tail.strip_prefix("<a href=\"") {
                let end = after.find('"').ok_or("unterminated href")?;
                let href = after[..end].to_ascii_lowercase();
                if !(href.starts_with("http://") || href.starts_with("https://")) {
                    return Err(format!("href `{href}` is not http(s)"));
                }
                if !after[end..].starts_with(&open) {
                    return Err(format!("link without rel: {tail}"));
                }
                rest = &after[end + open.len()..];
            } else if let Some(after) =
                tail.strip_prefix("</a>").or_else(|| tail.strip_prefix("<br>"))
            {
                rest = after;
            } else {
                return Err(format!(
                    "unexpected tag at `{}`",
                    tail.chars().take(20).collect::<String>()
                ));
            }
        }
        Ok(())
    }

    proptest::proptest! {
        #[test]
        fn html_cua_binh_luan_chi_co_the_an_toan(
            raw in "\\PC{0,120}|([a-z <>\"'&]{0,10}(https?|HTTP)://[a-z0-9./?=&()<>\"'-]{0,30}[ .,)\n]{0,3}){1,4}",
        ) {
            let mut problems = Vec::new();
            let body = clean_body(&raw, 10_000, &mut problems);
            for text in [raw.as_str(), body.as_str()] {
                let html = to_html(text);
                proptest::prop_assert!(tags_are_safe(&html).is_ok(), "{:?} from {text:?}", tags_are_safe(&html));
                proptest::prop_assert_eq!(link_count(text), html.matches("<a ").count());
                let escaped = escape(text);
                proptest::prop_assert!(!escaped.contains(['<', '>', '"', '\'']), "{escaped:?}");
                let without_entities = escaped
                    .replace("&amp;", "")
                    .replace("&lt;", "")
                    .replace("&gt;", "")
                    .replace("&quot;", "")
                    .replace("&#39;", "");
                proptest::prop_assert!(!without_entities.contains('&'), "{escaped:?}");
            }
        }
    }
}
