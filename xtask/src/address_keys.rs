//! Khoá giới hạn theo địa chỉ client phải theo khối địa chỉ (D-20261125-address-keys): client IPv6 giữ
//! cả một /64 và đổi địa chỉ mỗi request được, nên khoá dựng từ địa chỉ đúng không giới hạn được ai.
//!
//! Luật `address-key` — mọi cách dựng khoá từ địa chỉ thô hay gặp, kể cả viết trên nhiều dòng
//! (D-20261128-address-key-rule):
//!
//! - lời gọi `format!(…)` dựng KHOÁ (chuỗi mẫu không có khoảng trắng — câu thông báo có) mà có `ip:{`, có chỗ điền
//!   mang tên địa chỉ (`{ip}`, `{client_ip}`, `{peer_addr}`…) hay nhận một biến địa chỉ làm đối số;
//! - đối số của `.hit(…)` (bộ đếm `Throttle`) có `<địa chỉ>.to_string()`;
//! - `let <tên có key>… = <địa chỉ>.to_string()`.
//!
//! Dùng `autoxide_kernel::throttle::address_key(kind, ip)`. Đối số đi qua `ip_hash::…` hay `address_key(…)` là đã
//! gom theo khối. Chuỗi đã dựng từ thứ đã gom theo khối cách khác thì ghi `// address-key: <lý do>` trên dòng đầu,
//! một dòng bên trong lời gọi, hay dòng ngay trên. Luật đọc chữ, không đọc kiểu: bắt cách viết hay gặp, không
//! thay được việc rà soát.

use std::fs;
use std::path::Path;

use crate::boundaries::Violation;

const MARK: &str = "// address-key:";

pub fn check(root: &Path) -> Result<Vec<Violation>, String> {
    let mut violations = Vec::new();
    for file in crate::lengths::rust_files(root)? {
        if file.starts_with("xtask/") {
            continue;
        }
        let Ok(text) = fs::read_to_string(root.join(&file)) else {
            continue;
        };
        violations.extend(check_text(&file, &text));
    }
    Ok(violations)
}

/// Phần không đọc đĩa của [`check`], để test được.
fn check_text(file: &str, text: &str) -> Vec<Violation> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found: Vec<usize> = Vec::new();
    for (start, end) in calls(text, "format!(") {
        if raw_format_key(&text[start..end]) {
            found.push(start);
        }
    }
    for (start, end) in calls(text, ".hit(") {
        if converts_address(&text[start..end]) {
            found.push(start);
        }
    }
    for (index, line) in lines.iter().enumerate() {
        if key_from_address(line) {
            found.push(offset_of_line(text, index));
        }
    }
    found.sort_unstable();
    found.dedup();
    let mut violations = Vec::new();
    let mut reported = Vec::new();
    for start in found {
        let first = line_of(text, start);
        let end = statement_end(text, start);
        let last = line_of(text, end);
        let marked = (first.saturating_sub(1)..=last)
            .any(|index| lines.get(index).is_some_and(|line| line.contains(MARK)));
        if marked || reported.contains(&first) {
            continue;
        }
        reported.push(first);
        violations.push(Violation {
            rule: "address-key",
            location: format!("{file}:{}", first + 1),
            message: "a limit key built from a raw client address: a client rotating addresses inside \
                      its own /64 (IPv6) escapes the limit — build it with \
                      `autoxide_kernel::throttle::address_key(kind, ip)`, or mark a key already grouped \
                      by block with `// address-key: <reason>`"
                .to_owned(),
        });
    }
    violations
}

/// Vùng `(start, end)` của mọi lời gọi `opener…)` trong `text` (start ở đầu `opener`, end sau dấu `)` khớp), đọc
/// qua chuỗi, ký tự và comment để dấu ngoặc trong đó không làm lệch.
fn calls(text: &str, opener: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(position) = text[from..].find(opener) {
        let start = from + position;
        let open = start + opener.len() - 1;
        let end = closing(text, open).unwrap_or(text.len());
        found.push((start, end));
        from = open + 1;
    }
    found
}

/// Vị trí ngay sau dấu `)` khớp với dấu `(` ở `open`.
fn closing(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut index = open;
    while index < bytes.len() {
        match bytes[index] {
            b'(' => depth += 1,
            b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            b'"' => index = skip_string(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            // Ký tự `'('` / `')'`; lifetime (`'a`) không có dấu `'` thứ hai ngay sau nên không bị bỏ qua nhầm.
            b'\'' if bytes.get(index + 2) == Some(&b'\'') => index += 2,
            _ => {}
        }
        index += 1;
    }
    None
}

/// Vị trí của dấu `"` đóng chuỗi mở ở `open` (bỏ qua `\"`).
fn skip_string(bytes: &[u8], open: usize) -> usize {
    let mut index = open + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 1,
            b'"' => return index,
            _ => {}
        }
        index += 1;
    }
    bytes.len()
}

/// Cuối câu lệnh bắt đầu ở `start`: dấu `)` khớp của lời gọi đầu tiên, hay cuối dòng.
fn statement_end(text: &str, start: usize) -> usize {
    let line_end = text[start..].find('\n').map_or(text.len(), |offset| start + offset);
    match text[start..line_end].find('(') {
        Some(offset) => closing(text, start + offset).unwrap_or(line_end),
        None => line_end,
    }
}

fn line_of(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].matches('\n').count()
}

fn offset_of_line(text: &str, index: usize) -> usize {
    text.split_inclusive('\n').take(index).map(str::len).sum()
}

/// Lời gọi `format!(…)` dựng khoá từ địa chỉ thô.
fn raw_format_key(call: &str) -> bool {
    let inner = &call["format!(".len()..];
    let Some(quote) = inner.find('"') else { return false };
    // Chuỗi mẫu phải là đối số đầu tiên.
    if !inner[..quote].trim().is_empty() {
        return false;
    }
    let bytes = inner.as_bytes();
    let close = skip_string(bytes, quote);
    let literal = &inner[quote + 1..close.min(inner.len())];
    let arguments = inner.get(close + 1..).unwrap_or_default();
    // Câu thông báo có khoảng trắng, URL có `://`; khoá thì không.
    if literal.chars().any(char::is_whitespace) || literal.contains("://") {
        return false;
    }
    if literal.contains("ip:{") {
        return true;
    }
    let named = placeholders(literal).any(is_address_name);
    let grouped = arguments.contains("ip_hash::") || arguments.contains("address_key(");
    named || (!grouped && variables(arguments).any(is_address_name))
}

/// Các từ trong `code` không phải lời gọi hàm / method (`local_addr()` là lời gọi, `ip` là biến).
fn variables(code: &str) -> impl Iterator<Item = &str> {
    let mut found = Vec::new();
    let mut rest = code;
    while let Some(start) = rest.find(|c: char| c.is_ascii_alphanumeric() || c == '_') {
        let tail = &rest[start..];
        let end =
            tail.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(tail.len());
        if !tail[end..].starts_with('(') {
            found.push(&tail[..end]);
        }
        rest = &tail[end..];
    }
    found.into_iter()
}

/// Tên trong các chỗ điền `{tên}` / `{tên:…}` của chuỗi mẫu.
fn placeholders(literal: &str) -> impl Iterator<Item = &str> {
    literal.split('{').skip(1).filter_map(|part| {
        let name = part.split(['}', ':']).next()?;
        (!name.is_empty()).then_some(name)
    })
}

/// Tên biến hay gặp của một địa chỉ client: `ip`, `client_ip`, `ip_addr`, `addr`, `peer_addr`… (không gồm
/// `address`: chữ đó hay là địa chỉ email / bưu chính).
fn is_address_name(name: &str) -> bool {
    let name = name.trim();
    name == "ip"
        || name == "addr"
        || name.ends_with("_ip")
        || name.ends_with("_addr")
        || (name.starts_with("ip_") && !name.starts_with("ip_hash"))
}

/// Đối số của `.hit(…)` có `<địa chỉ>.to_string()`.
fn converts_address(call: &str) -> bool {
    call.match_indices(".to_string()").any(|(position, _)| address_chain(&call[..position]))
}

/// Chuỗi gọi ngay trước một vị trí (`peer.addr.ip()`, `ip`) có một đoạn mang tên địa chỉ.
fn address_chain(before: &str) -> bool {
    let start = before
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '(' | ')')))
        .map_or(0, |index| index + 1);
    before[start..].split('.').any(|segment| is_address_name(segment.trim_end_matches("()")))
}

/// `let <tên có key> = <địa chỉ>.to_string()` trên một dòng.
fn key_from_address(line: &str) -> bool {
    let Some(rest) = line.trim_start().strip_prefix("let ") else { return false };
    let Some((name, value)) = rest.split_once('=') else { return false };
    if !name.to_ascii_lowercase().contains("key") {
        return false;
    }
    let value = value.trim().trim_end_matches(';').trim();
    value.strip_suffix(".to_string()").is_some_and(address_chain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn khoa_theo_dia_chi_tho_bi_chan() {
        let bad = "fn f() {\n    throttle.hit(tenant, &format!(\"otp-ip:{ip}\")).await?;\n}\n";
        let found = check_text("modules/x/src/a.rs", bad);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].location, "modules/x/src/a.rs:2");
        assert_eq!(check_text("a.rs", "let k = format!(\"ip:{}\", ip);\n").len(), 1);

        let good = "throttle.hit(tenant, &address_key(\"otp-ip\", ip)).await?;\n";
        assert!(check_text("a.rs", good).is_empty());
        let marked = "// address-key: đã gom theo khối\nlet k = format!(\"cookie-ip:{bucket}\");\n";
        assert!(check_text("a.rs", marked).is_empty());
        assert!(check_text("a.rs", "let k = format!(\"phone:{phone}\");\n").is_empty());
    }

    /// P17: khoá dựng cách khác `format!("…ip:{…")` một dòng cũng bị bắt.
    #[test]
    fn khoa_dung_cach_khac_cung_bi_bat() {
        let cases = [
            // Nhiều dòng.
            "let key = format!(\n    \"otp:{}:{}\",\n    phone,\n    client_ip,\n);\n",
            // Chỗ điền mang tên địa chỉ, không có `ip:`.
            "let key = format!(\"otp-{client_ip}\");\n",
            "let key = format!(\"login:{email}:{peer_addr}\");\n",
            // Đối số là biến địa chỉ.
            "throttle.hit(tenant, &format!(\"otp:{}\", ip)).await?;\n",
            // `to_string()` thẳng vào bộ đếm, kể cả xuống dòng.
            "throttle.hit(tenant, &ip.to_string()).await?;\n",
            "throttle\n    .hit(\n        tenant,\n        &addr.ip().to_string(),\n    )\n    .await?;\n",
            // Biến khoá từ địa chỉ.
            "let key = client_ip.to_string();\n",
        ];
        for case in cases {
            assert_eq!(check_text("a.rs", case).len(), 1, "{case}");
        }
    }

    #[test]
    fn khong_bat_nham() {
        let cases = [
            // Câu thông báo (có khoảng trắng), không phải khoá.
            "let message = format!(\"the address {ip} is not allowed\");\n",
            // Đã gom theo khối.
            "let key = format!(\"{kind}:{}\", crate::ip_hash::client(ip));\n",
            "let key = format!(\"{kind}:{}\", address_key(\"x\", ip));\n",
            // Địa chỉ email / bưu chính, `ip_hash`.
            "let key = format!(\"confirm:{address}\");\n",
            "let key = format!(\"x:{ip_hash}\");\n",
            // URL, lời gọi `local_addr()`.
            "let url = format!(\"http://{}/hook\", listener.local_addr().unwrap());\n",
            "let key = format!(\"bind:{}\", listener.local_addr().unwrap());\n",
            // Ghi địa chỉ để lưu (không phải khoá).
            "let stored = ip.to_string();\n",
            // Dấu ngoặc trong chuỗi không làm lệch.
            "let key = format!(\"user:(\");\nlet x = ip;\n",
            // Đánh dấu trên một dòng bên trong lời gọi.
            "let key = format!(\n    \"bucket:{}\", // address-key: đã gom /24\n    ip,\n);\n",
        ];
        for case in cases {
            assert!(check_text("a.rs", case).is_empty(), "{case}");
        }
    }
}
