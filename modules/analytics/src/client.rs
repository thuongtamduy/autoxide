//! Đọc thông tin của trình duyệt mà KHÔNG giữ lại: nhận ra bot, rút user agent thành ba nhãn thô
//! (trình duyệt, hệ điều hành, loại máy) để làm khoá phiên, lấy tên miền từ URL. Không có dấu vân
//! tay trình duyệt (fingerprinting — cấm): chỉ ba nhãn thô, không phiên bản, không độ phân giải.

/// Mẩu chữ (chữ thường) có trong user agent của bot, trình thu thập, công cụ dòng lệnh và trình
/// duyệt không giao diện đã biết (D-20261118-step57-analytics).
const BOT_MARKERS: &[&str] = &[
    "bot",
    "crawl",
    "spider",
    "slurp",
    "scrape",
    "fetch",
    "monitor",
    "preview",
    "lighthouse",
    "pagespeed",
    "headless",
    "phantomjs",
    "puppeteer",
    "playwright",
    "selenium",
    "curl/",
    "wget/",
    "python-requests",
    "python-urllib",
    "aiohttp",
    "httpx",
    "go-http-client",
    "java/",
    "okhttp",
    "node-fetch",
    "axios/",
    "libwww-perl",
    "facebookexternalhit",
    "embedly",
    "quora link preview",
    "bitlybot",
    "uptime",
];

/// `user_agent` là bot đã biết. Không có user agent: coi là bot (trình duyệt luôn gửi).
pub(crate) fn is_bot(user_agent: Option<&str>) -> bool {
    let Some(agent) = user_agent.map(str::trim).filter(|agent| !agent.is_empty()) else {
        return true;
    };
    let agent = agent.to_ascii_lowercase();
    // Trình duyệt thật luôn mở đầu bằng `Mozilla/` (hay `Opera/` rất cũ).
    if !agent.starts_with("mozilla/") && !agent.starts_with("opera/") {
        return true;
    }
    BOT_MARKERS.iter().any(|marker| agent.contains(marker))
}

/// Nhãn thô `trình duyệt|hệ điều hành|loại máy`, vd `chrome|android|mobile`. Không có phiên bản.
pub(crate) fn short_agent(user_agent: Option<&str>) -> String {
    let agent = user_agent.unwrap_or_default().to_ascii_lowercase();
    let has = |marker: &str| agent.contains(marker);
    // Thứ tự quan trọng: Edge và Opera cũng ghi `chrome`, Chrome cũng ghi `safari`.
    let browser = if has("edg/") {
        "edge"
    } else if has("opr/") || has("opera") {
        "opera"
    } else if has("coc_coc") {
        "coccoc"
    } else if has("samsungbrowser") {
        "samsung"
    } else if has("firefox/") || has("fxios") {
        "firefox"
    } else if has("chrome/") || has("crios") {
        "chrome"
    } else if has("safari/") {
        "safari"
    } else {
        "other"
    };
    let os = if has("android") {
        "android"
    } else if has("iphone") || has("ipad") || has("ipod") {
        "ios"
    } else if has("windows") {
        "windows"
    } else if has("mac os") || has("macintosh") {
        "macos"
    } else if has("cros") {
        "chromeos"
    } else if has("linux") {
        "linux"
    } else {
        "other"
    };
    let device = if has("ipad") || has("tablet") {
        "tablet"
    } else if has("mobi") || has("iphone") {
        "mobile"
    } else {
        "desktop"
    };
    format!("{browser}|{os}|{device}")
}

/// Tên miền (chữ thường, không cổng, không `www.` đầu) của một URL `http(s)://…`; `None` khi
/// không phải URL như vậy.
pub(crate) fn host_of(url: &str) -> Option<String> {
    let rest = url.trim();
    let rest = rest
        .strip_prefix("https://")
        .or_else(|| rest.strip_prefix("http://"))
        .or_else(|| rest.strip_prefix("HTTPS://"))
        .or_else(|| rest.strip_prefix("HTTP://"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // Bỏ phần `user:pass@` nếu có.
    let authority = authority.rsplit('@').next().unwrap_or_default();
    let host = if let Some(inner) = authority.strip_prefix('[') {
        inner.split(']').next().unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host.len() > 253 { None } else { Some(host) }
}

/// Bỏ `www.` đầu để `www.shop.example` và `shop.example` là một nguồn.
pub(crate) fn without_www(host: &str) -> &str {
    host.strip_prefix("www.").unwrap_or(host)
}

/// Tên miền của header `Host` (chữ thường, không cổng).
pub(crate) fn request_host(raw: &str) -> String {
    let raw = raw.trim();
    let host = if let Some(inner) = raw.strip_prefix('[') {
        inner.split(']').next().unwrap_or_default()
    } else {
        raw.split(':').next().unwrap_or_default()
    };
    host.trim_end_matches('.').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHROME_ANDROID: &str = "Mozilla/5.0 (Linux; Android 14; SM-S918B) AppleWebKit/537.36 \
                                  (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36";
    const SAFARI_MAC: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_6) AppleWebKit/605.1.15 \
                              (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
    const EDGE_WINDOWS: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                                (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0";

    #[test]
    fn bot_bi_nhan_ra() {
        for agent in [
            "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)",
            "curl/8.5.0",
            "python-requests/2.32",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 HeadlessChrome/130.0.0.0",
            "facebookexternalhit/1.1",
            "",
        ] {
            assert!(is_bot(Some(agent)), "{agent}");
        }
        assert!(is_bot(None));
        for agent in [CHROME_ANDROID, SAFARI_MAC, EDGE_WINDOWS] {
            assert!(!is_bot(Some(agent)), "{agent}");
        }
    }

    #[test]
    fn user_agent_rut_gon_khong_co_phien_ban() {
        assert_eq!(short_agent(Some(CHROME_ANDROID)), "chrome|android|mobile");
        assert_eq!(short_agent(Some(SAFARI_MAC)), "safari|macos|desktop");
        assert_eq!(short_agent(Some(EDGE_WINDOWS)), "edge|windows|desktop");
        assert_eq!(short_agent(None), "other|other|desktop");
    }

    #[test]
    fn ten_mien_cua_url() {
        assert_eq!(host_of("https://Shop.Example:8443/a?b#c").as_deref(), Some("shop.example"));
        assert_eq!(host_of("http://u:p@www.google.com/search").as_deref(), Some("www.google.com"));
        assert_eq!(host_of("android-app://com.example"), None);
        assert_eq!(host_of(""), None);
        assert_eq!(without_www("www.google.com"), "google.com");
        assert_eq!(request_host("Shop.Example.:443"), "shop.example");
    }
}
