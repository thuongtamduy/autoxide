//! Che liên hệ trong tin chat khi nút bật `conversations.mask_contacts` (sàn giữ giao dịch trong sàn):
//! số điện thoại (từ 9 chữ số, kể cả viết cách bằng dấu cách, chấm, gạch), email, đường link / tên
//! miền và các từ nút tự khai. Chữ bị che thay bằng `•••`. Che lúc lưu: bản gốc không giữ lại — module
//! không đọc tin để quảng cáo hay việc gì khác.

const HIDDEN: &str = "•••";

/// Chữ `text` sau khi che.
pub(crate) fn mask(text: &str, terms: &[String]) -> String {
    let phones = mask_phones(text);
    let words: String = phones
        .split_inclusive(char::is_whitespace)
        .map(|word| {
            let core = word.trim_end();
            let tail = &word[core.len()..];
            let bare = core.trim_matches(|c: char| ",;:!?()[]{}\"'".contains(c));
            if is_email(bare) || is_link(bare) {
                format!("{}{tail}", core.replace(bare, HIDDEN))
            } else {
                word.to_owned()
            }
        })
        .collect();
    terms.iter().fold(words, |text, term| mask_term(&text, term))
}

/// Che mọi dãy 9–15 chữ số bắt đầu bằng `+`, `0` hay `84` (dạng số điện thoại), chỉ ngăn bởi dấu
/// cách, `.`, `-`, `(`, `)` — giá tiền như `1.500.000.000` không bị che.
fn mask_phones(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        let starts = chars[index].is_ascii_digit()
            || (chars[index] == '+' && chars.get(index + 1).is_some_and(char::is_ascii_digit));
        if !starts {
            out.push(chars[index]);
            index += 1;
            continue;
        }
        let mut end = index + 1;
        let mut last_digit = index;
        let mut digits = usize::from(chars[index].is_ascii_digit());
        while end < chars.len() && (chars[end].is_ascii_digit() || " .-()".contains(chars[end])) {
            if chars[end].is_ascii_digit() {
                digits += 1;
                last_digit = end;
            }
            end += 1;
        }
        let run: String = chars[index..=last_digit].iter().filter(|c| c.is_ascii_digit()).collect();
        let phone_like = chars[index] == '+' || run.starts_with('0') || run.starts_with("84");
        if (9..=15).contains(&digits) && phone_like {
            out.push_str(HIDDEN);
            index = last_digit + 1;
        } else {
            out.extend(&chars[index..=last_digit]);
            index = last_digit + 1;
        }
    }
    out
}

fn is_email(word: &str) -> bool {
    match word.split_once('@') {
        Some((name, domain)) => !name.is_empty() && domain.contains('.'),
        None => false,
    }
}

/// Link (`http://…`, `www.…`) hay tên miền (`shop.vn`, `zalo.me/abc`).
fn is_link(word: &str) -> bool {
    let lower = word.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.") {
        return true;
    }
    let host = lower.split('/').next().unwrap_or_default();
    let Some((name, tld)) = host.rsplit_once('.') else {
        return false;
    };
    (2..=6).contains(&tld.len())
        && tld.chars().all(|c| c.is_ascii_alphabetic())
        && name.chars().any(|c| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

/// Che `term` (không phân biệt hoa thường) khi nó đứng thành từ riêng.
fn mask_term(text: &str, term: &str) -> String {
    let term = term.trim().to_lowercase();
    if term.is_empty() {
        return text.to_owned();
    }
    let lower = text.to_lowercase();
    // Chữ thường hoá có thể đổi độ dài byte (vài ký tự Unicode): khi đó bỏ qua để không cắt sai.
    if lower.len() != text.len() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    while let Some(found) = lower[from..].find(&term) {
        let start = from + found;
        let end = start + term.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        let alone = before.is_none_or(|c| !c.is_alphanumeric())
            && after.is_none_or(|c| !c.is_alphanumeric());
        out.push_str(&text[from..start]);
        out.push_str(if alone { HIDDEN } else { &text[start..end] });
        from = end;
    }
    out.push_str(&text[from..]);
    out
}

#[cfg(test)]
mod tests {
    use super::mask;

    #[test]
    fn che_so_dien_thoai_email_va_link() {
        let terms = vec!["Zalo".to_owned()];
        assert_eq!(
            mask("Gọi 090 123 4567 hoặc +84.901.234.567 nhé", &terms),
            "Gọi ••• hoặc ••• nhé"
        );
        assert_eq!(mask("mail an@example.com, ok", &terms), "mail •••, ok");
        assert_eq!(mask("xem https://x.vn/a và shop.vn/b", &terms), "xem ••• và •••");
        assert_eq!(
            mask("nhắn zalo nhé, zalopay thì không", &terms),
            "nhắn ••• nhé, zalopay thì không"
        );
    }

    #[test]
    fn khong_che_so_luong_hay_gia() {
        let none: Vec<String> = Vec::new();
        let text = "Size 42, giá 1.500.000.000đ, nặng 1.5kg";
        assert_eq!(mask(text, &none), text);
        assert_eq!(mask("Đơn DH000123 ngày 19.11", &none), "Đơn DH000123 ngày 19.11");
    }
}
