//! Link giao diện trong thư phải theo khách (D-20261125-tenant-link-templates).
//!
//! Luật `site-links`: mỗi biến cấu hình `AUTOXIDE_…_URL` khai bằng `Setting::new(` trong `crates/` hay `modules/`
//! hoặc nằm trong [`NOT_SITE_LINKS`] (địa chỉ API đối tác, trang của nền tảng, gốc website đã đổi host qua
//! `SiteLinks::rewrite`), hoặc được nêu tên trong mô tả của một `TenantSetting::link(` cùng crate — tức khách
//! đặt được trang riêng thay nó. Thiếu là email của khách B có thể trỏ về tên miền chung.
//!
//! Giới hạn: luật đoán theo hậu tố `_URL` của tên biến; một biến trỏ trang giao diện đặt tên khác (`…_PAGE`)
//! không bị bắt — người viết module vẫn phải tự dùng `SiteLinks`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::boundaries::Violation;

/// Biến `*_URL` không phải trang giao diện của từng khách, kèm lý do.
const NOT_SITE_LINKS: &[(&str, &str)] = &[
    ("AUTOXIDE_HTTP_PUBLIC_URL", "địa chỉ của máy chủ API; khách thay bằng `kernel.public_url`"),
    ("AUTOXIDE_MAIL_SMTP_URL", "máy chủ gửi thư; khách thay bằng `mail.connection.smtp.url`"),
    ("AUTOXIDE_AI_BASE_URL", "API đối tác"),
    ("AUTOXIDE_EMBEDDING_BASE_URL", "API đối tác"),
    ("AUTOXIDE_SEPAY_API_URL", "API đối tác"),
    ("AUTOXIDE_MOMO_API_URL", "API đối tác"),
    ("AUTOXIDE_MOMO_SANDBOX_API_URL", "API đối tác"),
    ("AUTOXIDE_GHN_API_URL", "API đối tác"),
    ("AUTOXIDE_GHN_SANDBOX_API_URL", "API đối tác"),
    ("AUTOXIDE_ESMS_API_URL", "API đối tác"),
    ("AUTOXIDE_EINVOICE_SEPAY_API_URL", "API đối tác"),
    ("AUTOXIDE_EINVOICE_SEPAY_SANDBOX_API_URL", "API đối tác"),
    ("AUTOXIDE_PLANS_SIGNUP_URL", "trang đăng ký của nền tảng, không của khách nào"),
    ("AUTOXIDE_PLANS_SIGNUP_RETURN_URL", "trang của nền tảng, không của khách nào"),
    (
        "AUTOXIDE_COMMENTS_SITE_URL",
        "gốc website, đổi host theo `kernel.public_url` (SiteLinks::rewrite)",
    ),
    (
        "AUTOXIDE_NEWSLETTER_SITE_URL",
        "gốc website, đổi host theo `kernel.public_url` (SiteLinks::rewrite)",
    ),
];

pub fn check(root: &Path) -> Result<Vec<Violation>, String> {
    // Gom văn bản theo crate (`modules/<tên>` hay `crates/<tên>`).
    let mut crates: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for file in crate::lengths::rust_files(root)? {
        let parts: Vec<&str> = file.split('/').collect();
        if parts.len() < 3 || !matches!(parts[0], "modules" | "crates") || parts[2] != "src" {
            continue;
        }
        let Ok(text) = fs::read_to_string(root.join(&file)) else {
            continue;
        };
        crates.entry(format!("{}/{}", parts[0], parts[1])).or_default().push((file, text));
    }
    let mut violations = Vec::new();
    for files in crates.values() {
        violations.extend(check_crate(files));
    }
    Ok(violations)
}

/// Phần không đọc đĩa của [`check`]: các file `(đường dẫn, nội dung)` của một crate.
fn check_crate(files: &[(String, String)]) -> Vec<Violation> {
    let linked: Vec<String> = files.iter().flat_map(|(_, text)| link_blocks(text)).collect();
    let mut violations = Vec::new();
    for (file, text) in files {
        for (line, name) in url_settings(text) {
            let exempt = NOT_SITE_LINKS.iter().any(|(listed, _)| *listed == name);
            if exempt || linked.iter().any(|block| block.contains(&name)) {
                continue;
            }
            violations.push(Violation {
                rule: "site-links",
                location: format!("{file}:{line}"),
                message: format!(
                    "`{name}` looks like a page of the interface linked from mail: declare a \
                     `TenantSetting::link(…)` in this crate whose description names `{name}` and \
                     read it with `SiteLinks::link`, so each tenant can point it at its own site; \
                     if it is not such a page, list it in `NOT_SITE_LINKS` of \
                     `xtask/src/site_links.rs` with the reason"
                ),
            });
        }
    }
    violations
}

/// `(số dòng, tên biến)` của mỗi `Setting::new(` mà tên biến kết thúc bằng `_URL`.
fn url_settings(text: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !line.contains("Setting::new(") || line.contains("TenantSetting::new(") {
            continue;
        }
        let call: String = lines.iter().skip(index).take(2).copied().collect();
        if let Some(name) = quoted_env(&call).filter(|name| name.ends_with("_URL")) {
            found.push((index + 1, name));
        }
    }
    found
}

/// Chuỗi `"AUTOXIDE_…"` đầu tiên trong `text`.
fn quoted_env(text: &str) -> Option<String> {
    let start = text.find("\"AUTOXIDE_")? + 1;
    let end = text[start..].find('"')? + start;
    Some(text[start..end].to_owned())
}

/// Văn bản (12 dòng) từ mỗi `TenantSetting::link(`.
fn link_blocks(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("TenantSetting::link("))
        .map(|(index, _)| lines.iter().skip(index).take(12).copied().collect::<Vec<_>>().join("\n"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(text: &str) -> Vec<(String, String)> {
        vec![("modules/x/src/config.rs".to_owned(), text.to_owned())]
    }

    #[test]
    fn bien_url_giao_dien_phai_co_link_theo_khach() {
        let bare = "pub const PAGE: Setting = Setting::new(\n    \"AUTOXIDE_X_PAGE_URL\",\n    \"Page\",\n);\n";
        let found = check_crate(&files(bare));
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].location, "modules/x/src/config.rs:1");

        let linked = format!(
            "{bare}pub const LINK: TenantSetting = TenantSetting::link(\n    \"x.link.page\",\n    \
             \"When unset, the server's AUTOXIDE_X_PAGE_URL is used.\",\n    &[\"{{token}}\"],\n);\n"
        );
        assert!(check_crate(&files(&linked)).is_empty());

        let partner = "const A: Setting = Setting::new(\"AUTOXIDE_GHN_API_URL\", \"x\");\n";
        assert!(check_crate(&files(partner)).is_empty());
        let other = "const A: Setting = Setting::new(\"AUTOXIDE_X_LIMIT\", \"x\");\n";
        assert!(check_crate(&files(other)).is_empty());
    }
}
