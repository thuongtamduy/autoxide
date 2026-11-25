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

// Origin chuẩn hoá `scheme://host:port` của một URL http(s) không có user/mật khẩu; `None` nếu không đọc
// được hay không phải http(s).
fn origin(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let port = url.port_or_known_default()?;
    Some(format!("{}://{host}:{port}", url.scheme()))
}

// Một origin hợp lệ để khai: chỉ scheme + host (+ port), không đường dẫn hay tham số.
fn listed_origin(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw.trim()).ok()?;
    let bare = url.path() == "/" && url.query().is_none() && url.fragment().is_none();
    bare.then(|| origin(raw)).flatten()
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
            "must be origins such as `https://shop.example.com`, separated by commas (no path); \
             not valid: {items}",
        )
        .with_param("items", bad.join(", ")))
    }
}

/// Kiểm `url` cho lượt thu của khách `tenant`: lỗi ô `return_url` nếu sai dạng hay không thuộc khách.
pub(crate) async fn check(
    tenant: &TenantContext,
    url: &str,
    domains: Option<&autoxide_kernel::tenancy::TenantDomains>,
    settings: Option<&dyn TenantSettings>,
) -> Result<Option<FieldError>, Error> {
    let Some(wanted) = origin(url).filter(|_| url.len() <= MAX_LEN) else {
        return Ok(Some(FieldError::new(
            "return_url",
            "format",
            "must be an http(s) URL of at most 2000 characters",
        )));
    };
    if let Some(domains) = domains {
        let hosts = domains.verified_hosts(tenant).await?;
        if hosts.iter().any(|host| wanted == format!("https://{host}:443")) {
            return Ok(None);
        }
    }
    if let Some(settings) = settings
        && let Some(listed) = settings.get(tenant, &RETURN_ORIGINS).await?
        && listed.as_str().split(',').filter_map(listed_origin).any(|origin| origin == wanted)
    {
        return Ok(None);
    }
    Ok(Some(FieldError::new(
        "return_url",
        "ledger.return_url_not_allowed",
        "must be on one of this shop's own addresses: a verified domain (https) or an origin listed \
         in the tenant setting `ledger.payments.return_origins`",
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn origin_khai_phai_tran() {
        assert!(check_origins("https://shop.example, http://localhost:3000").is_ok());
        assert!(check_origins("https://shop.example/thanks").is_err());
        assert!(check_origins("shop.example").is_err());
    }
}
