//! Khoá giới hạn theo địa chỉ client phải theo khối địa chỉ (D-20261125-address-keys): client IPv6 giữ
//! cả một /64 và đổi địa chỉ mỗi request được, nên khoá dựng từ địa chỉ đúng không giới hạn được ai.
//!
//! Luật `address-key`: chuỗi `format!` có `ip:{` (khoá kiểu `otp-ip:{ip}`) bị chặn — dùng
//! `autoxide_kernel::throttle::address_key(kind, ip)`. Chuỗi đã dựng từ thứ đã gom theo khối (vd
//! `ip_hash::bucket`) thì ghi `// address-key: <lý do>` trên cùng dòng hay dòng ngay trên.
//! Luật rẻ, bắt cách viết hay gặp; không thay được việc rà soát.

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
    let mut violations = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !raw_address_key(line) {
            continue;
        }
        let marked = line.contains(MARK)
            || index.checked_sub(1).is_some_and(|before| lines[before].contains(MARK));
        if marked {
            continue;
        }
        violations.push(Violation {
            rule: "address-key",
            location: format!("{file}:{}", index + 1),
            message: "a limit key built from a raw client address: a client rotating addresses inside \
                      its own /64 (IPv6) or /24 escapes the limit — build it with \
                      `autoxide_kernel::throttle::address_key(kind, ip)`, or mark a key already grouped \
                      by block with `// address-key: <reason>`"
                .to_owned(),
        });
    }
    violations
}

/// Dòng có chuỗi `format!` chứa `ip:{`.
fn raw_address_key(line: &str) -> bool {
    let Some(start) = line.find("format!(\"") else {
        return false;
    };
    let rest = &line[start + "format!(\"".len()..];
    let literal = rest.split('"').next().unwrap_or_default();
    literal.contains("ip:{")
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
}
