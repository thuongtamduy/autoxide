//! Địa chỉ quay về sau khi trả (`return_url`, P-5 của rà soát bảo mật 2026-11,
//! D-20261125-return-url-allowlist): đối tác thanh toán chuyển TRÌNH DUYỆT của người mua tới địa chỉ
//! này sau khi trả, nên nhận mọi địa chỉ http(s) là một open redirect — kẻ gian gửi link thanh toán thật
//! của cửa hàng mà người trả xong bị đưa sang trang giả. Chỉ nhận địa chỉ thuộc khách của lượt thu:
//!
//! - `https://` trên một tên miền khách đã xác minh (`kernel` — `TenantDomains`, chế độ nhiều khách);
//! - origin khách tự khai ở cấu hình `ledger.payments.return_origins` (website ở tên miền chưa gắn vào
//!   Autoxide, chế độ một khách, app di động qua universal / app link, máy phát triển `http://localhost:3000`).
//!
//! So theo origin (scheme + host + port), đọc bằng crate `url` như trình duyệt (WHATWG): `user@host`,
//! dấu `\` hay chữ hoa không lừa được phép so.

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::tenancy::{TenantContext, TenantSetting, TenantSettings};

/// Origin khách cho phép làm địa chỉ quay về, ngoài tên miền đã xác minh.
pub(crate) const RETURN_ORIGINS: TenantSetting = TenantSetting::new(
    "ledger.payments.return_origins",
    "Addresses a buyer may be sent back to after paying (`return_url` of a payment), besides this \
     tenant's verified domains: origins separated by commas, such as `https://shop.example.com, \
     https://app.example.com, http://localhost:3000`. Only the scheme, host and port count.",
)
.checked(check_origins);

/// Độ dài tối đa của `return_url`.
const MAX_LEN: usize = 2000;

// URL http(s) đã đọc như trình duyệt, không có user/mật khẩu, không có khoảng trắng hay ký tự điều khiển ở hai
// đầu (trình duyệt bỏ chúng đi, nên chuỗi chứa chúng là chuỗi khác với chuỗi được kiểm); `None` nếu không phải.
fn parse(raw: &str) -> Option<url::Url> {
    if raw.is_empty() || raw.chars().any(char::is_control) || raw.trim() != raw {
        return None;
    }
    let url = url::Url::parse(raw).ok()?;
    let plain = matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some();
    plain.then_some(url)
}

// Origin chuẩn hoá `scheme://host:port` của `url` (tên miền chữ thường, IDN thành punycode, bỏ dấu chấm cuối).
fn origin_of(url: &url::Url) -> Option<String> {
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.strip_suffix('.').unwrap_or(&host).to_owned();
    let port = url.port_or_known_default()?;
    Some(format!("{}://{host}:{port}", url.scheme()))
}

// Một origin hợp lệ để khai: chỉ scheme + host (+ port), không đường dẫn, tham số hay ký tự đại diện `*` (so khớp
// chính xác; `https://*.example.com` sẽ không bao giờ khớp nên từ chối ngay lúc lưu để quản trị viên khỏi hiểu lầm).
fn listed_origin(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.contains('*') {
        return None;
    }
    let url = parse(raw)?;
    let bare = url.path() == "/" && url.query().is_none() && url.fragment().is_none();
    bare.then(|| origin_of(&url)).flatten()
}

fn check_origins(raw: &str) -> Result<(), FieldError> {
    let bad: Vec<&str> =
        raw.split(',').map(str::trim).filter(|item| listed_origin(item).is_none()).collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(FieldError::new(
            "value",
            "ledger.return_origins_invalid",
            "must be origins such as `https://shop.example.com`, separated by commas (no path, no `*`: \
             list each host); not valid: {items}",
        )
        .with_param("items", bad.join(", ")))
    }
}

/// Kiểm `url` cho lượt thu của khách `tenant`: `Ok(chuỗi đã chuẩn hoá)` để lưu và gửi cho đối tác, hay lỗi ô
/// `return_url` nếu sai dạng hoặc không thuộc khách.
pub(crate) async fn check(
    tenant: &TenantContext,
    url: &str,
    domains: Option<&autoxide_kernel::tenancy::TenantDomains>,
    settings: Option<&dyn TenantSettings>,
) -> Result<Result<String, FieldError>, Error> {
    let parsed = parse(url).filter(|_| url.len() <= MAX_LEN);
    let Some((parsed, wanted)) = parsed.and_then(|parsed| origin_of(&parsed).map(|o| (parsed, o)))
    else {
        return Ok(Err(FieldError::new(
            "return_url",
            "format",
            "must be an http(s) URL of at most 2000 characters",
        )));
    };
    let normalized = parsed.as_str().to_owned();
    if let Some(domains) = domains {
        let hosts = domains.verified_hosts(tenant).await?;
        if hosts.iter().any(|host| wanted == format!("https://{host}:443")) {
            return Ok(Ok(normalized));
        }
    }
    if let Some(settings) = settings
        && let Some(listed) = settings.get(tenant, &RETURN_ORIGINS).await?
        && listed.as_str().split(',').filter_map(listed_origin).any(|origin| origin == wanted)
    {
        return Ok(Ok(normalized));
    }
    Ok(Err(FieldError::new(
        "return_url",
        "ledger.return_url_not_allowed",
        "must be on one of this shop's own addresses: a verified domain (https) or an origin listed \
         in the tenant setting `ledger.payments.return_origins`",
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(raw: &str) -> Option<String> {
        parse(raw).as_ref().and_then(origin_of)
    }

    #[test]
    fn origin_doc_nhu_trinh_duyet() {
        assert_eq!(
            origin("https://Shop.Example/thank-you?x=1").as_deref(),
            Some("https://shop.example:443")
        );
        assert_eq!(origin("http://localhost:3000/done").as_deref(), Some("http://localhost:3000"));
        // `user@host`: trình duyệt tới `evil.example`, không phải `shop.example`.
        assert_eq!(origin("https://shop.example@evil.example/"), None);
        assert_eq!(
            origin("https://evil.example\\@shop.example/").as_deref(),
            Some("https://evil.example:443")
        );
        assert_eq!(
            origin("https://evil.example\\.shop.example/").as_deref(),
            Some("https://evil.example:443")
        );
        assert_eq!(origin("javascript:alert(1)"), None);
        assert_eq!(origin("myapp://done"), None);
    }

    // Các lớp lách (kiểm độc lập #508): mỗi dòng là (URL, origin mà trình duyệt thật sự tới, `None` = bị từ chối).
    #[test]
    fn cac_lop_lach_khong_lot() {
        for (raw, expected) in [
            // IDN: chữ Unicode thành punycode, khác hẳn tên ASCII giống mặt chữ.
            ("https://xn--shp-qma.example/", Some("https://xn--shp-qma.example:443")),
            ("https://shöp.example/", Some("https://xn--shp-tna.example:443")),
            // Dấu chấm cuối: cùng máy chủ với không có dấu chấm.
            ("https://shop.example./done", Some("https://shop.example:443")),
            // IPv6 literal: không phải tên miền nào của cửa hàng.
            ("https://[::1]/done", Some("https://[::1]:443")),
            // Host mã hoá `%`: trình duyệt giải mã trước khi tới.
            ("https://shop%2eexample/done", Some("https://shop.example:443")),
            ("https://evil%2eexample/done", Some("https://evil.example:443")),
            // Khoảng trắng, ký tự điều khiển, URL thiếu scheme.
            (" https://shop.example/", None),
            ("https://shop.example/\n", None),
            ("https://shop.exa\tmple/", None),
            ("//shop.example/done", None),
            ("/done", None),
        ] {
            assert_eq!(origin(raw).as_deref(), expected, "{raw:?}");
        }
    }

    #[test]
    fn origin_khai_phai_tran() {
        assert!(check_origins("https://shop.example, http://localhost:3000").is_ok());
        assert!(check_origins("https://shop.example/thanks").is_err());
        assert!(check_origins("shop.example").is_err());
        assert!(check_origins("https://*.example.com").is_err(), "ký tự đại diện bị từ chối");
        assert!(check_origins("https://shop.example, https://*.shop.example").is_err());
    }
}
