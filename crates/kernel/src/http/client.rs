//! Người gọi là ai, theo mạng: địa chỉ IP thật (kể cả sau reverse proxy), user agent,
//! `Origin`, `Referer`. Một cách tính duy nhất, dùng chung cho rate limiting và cho handler
//! (extractor [`ClientInfo`]).
//!
//! Sau proxy, địa chỉ nằm trong header cấu hình ở `AUTOXIDE_HTTP_CLIENT_IP_HEADER` (thường là
//! `X-Forwarded-For`). Mỗi proxy THÊM địa chỉ của bên vừa kết nối tới nó vào cuối danh sách;
//! phần đầu danh sách là do client tự gửi, giả được. Với `N` proxy tin cậy
//! (`AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT`), địa chỉ thật là phần tử thứ `N` tính từ phải:
//!
//! ```text
//! trình duyệt ─▶ CDN ─▶ Next.js ─▶ API        X-Forwarded-For: <giả>, <trình duyệt>, <CDN>
//!                                              N = 2 ─────────────────────▲
//! ```
//!
//! Nhiều dòng header được gộp thành một danh sách theo thứ tự (RFC 9110). Danh sách ngắn hơn
//! `N` (request không đi qua đủ proxy) thì lấy phần tử đầu. Giá trị không phải địa chỉ IP thì
//! dùng địa chỉ kết nối.
//!
//! Đếm proxy chỉ đúng khi mọi request đi qua đủ `N` proxy: client gọi thẳng vào proxy cuối thì
//! tự chọn được địa chỉ. Khai dải địa chỉ của proxy (`AUTOXIDE_HTTP_TRUSTED_PROXIES`, D-152) thì
//! thay cho cách đếm: header chỉ được đọc khi KẾT NỐI đến từ một proxy tin cậy, rồi đi từ phải
//! sang trái, bỏ qua địa chỉ của proxy tin cậy; địa chỉ đầu tiên không thuộc dải là client.
//!
//! ```text
//! trình duyệt ─▶ CDN ─▶ Next.js ─▶ API     X-Forwarded-For: <giả>, <trình duyệt>, <CDN>
//! dải: CDN, Next.js                         bỏ qua <CDN> ──────────────▲  (kết nối: Next.js)
//! trình duyệt ─────────▶ Next.js ─▶ API     X-Forwarded-For: <giả>, <trình duyệt>
//!                                           ──────────────────────────▲
//! ```

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderName, header};
use axum::middleware::Next;
use axum::response::Response;

use crate::config::IpRange;

/// Độ dài tối đa giữ lại của user agent, `Origin`, `Referer` (phần dài hơn bị cắt).
const MAX_TEXT: usize = 1024;

/// Cách tìm địa chỉ client, theo cấu hình.
#[derive(Debug, Clone)]
pub(super) struct ClientAddress {
    header: Option<HeaderName>,
    trusted_proxies: usize,
    /// Dải địa chỉ của proxy tin cậy; không rỗng thì dùng thay cho `trusted_proxies`.
    trusted_ranges: Vec<IpRange>,
}

/// Địa chỉ client đã giải, gắn vào extensions của request (lớp [`attach`]).
#[derive(Debug, Clone, Copy)]
pub(super) struct ClientIp(pub(super) Option<IpAddr>);

impl ClientAddress {
    pub(super) fn new(
        header: Option<HeaderName>,
        trusted_proxies: usize,
        trusted_ranges: Vec<IpRange>,
    ) -> Self {
        ClientAddress { header, trusted_proxies: trusted_proxies.max(1), trusted_ranges }
    }

    fn is_trusted(&self, address: IpAddr) -> bool {
        self.trusted_ranges.iter().any(|range| range.contains(address))
    }

    /// Địa chỉ client của request có `headers`, đến từ kết nối `connection`.
    pub(super) fn resolve(
        &self,
        headers: &HeaderMap,
        connection: Option<IpAddr>,
    ) -> Option<IpAddr> {
        let Some(name) = &self.header else {
            return connection;
        };
        // Có dải proxy: kết nối không đến từ proxy tin cậy thì header là do client tự viết.
        if !self.trusted_ranges.is_empty() && !connection.is_some_and(|ip| self.is_trusted(ip)) {
            return connection;
        }
        // Mọi dòng của header, nối theo thứ tự thành một danh sách.
        let entries: Vec<&str> = headers
            .get_all(name)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|line| line.split(','))
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .collect();
        if entries.is_empty() {
            return connection;
        }
        let chosen = if self.trusted_ranges.is_empty() {
            entries[entries.len().saturating_sub(self.trusted_proxies)]
        } else {
            // Từ phải sang: bỏ qua proxy tin cậy, dừng ở địa chỉ đầu tiên không thuộc dải (hay
            // không đọc được — phần trước nó không đáng tin). Toàn proxy tin cậy: phần tử đầu.
            entries
                .iter()
                .rev()
                .copied()
                .find(|entry| !parse_address(entry).is_some_and(|ip| self.is_trusted(ip)))
                .unwrap_or(entries[0])
        };
        match parse_address(chosen) {
            Some(address) => Some(address),
            None => {
                tracing::warn!(
                    header = %name,
                    value = chosen,
                    "cannot read the client address from the configured header; using the connection address"
                );
                connection
            }
        }
    }
}

/// Địa chỉ IP trong header của proxy: `203.0.113.9`, `203.0.113.9:4711`, `2001:db8::1`
/// hoặc `[2001:db8::1]:4711` (vài load balancer ghi kèm port).
pub(super) fn parse_address(text: &str) -> Option<IpAddr> {
    text.parse::<IpAddr>()
        .ok()
        .or_else(|| text.parse::<SocketAddr>().ok().map(|address| address.ip()))
}

/// Lớp HTTP: giải địa chỉ client một lần cho mỗi request, gắn vào extensions.
pub(super) async fn attach(
    resolver: Arc<ClientAddress>,
    mut request: Request,
    next: Next,
) -> Response {
    let connection = request.extensions().get::<ConnectInfo<SocketAddr>>().map(|info| info.0.ip());
    let address = resolver.resolve(request.headers(), connection);
    request.extensions_mut().insert(ClientIp(address));
    next.run(request).await
}

/// Thông tin mạng của người gọi. Handler khai `client: ClientInfo` là có; không bao giờ lỗi.
///
/// Đừng lưu `ip` thô: băm nó trước (xem `autoxide_kernel::http::ip_hash` khi có).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientInfo {
    /// Địa chỉ đã giải theo số proxy tin cậy; `None` khi không biết (vd request không qua
    /// socket, trong test).
    pub ip: Option<IpAddr>,
    pub user_agent: Option<String>,
    /// Header `Origin` (trình duyệt gửi khi gọi từ trang khác hoặc gửi form).
    pub origin: Option<String>,
    pub referer: Option<String>,
}

impl<S: Send + Sync> FromRequestParts<S> for ClientInfo {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let text = |name: header::HeaderName| {
            parts.headers.get(name).and_then(|value| value.to_str().ok()).map(|value| {
                let value = value.trim();
                // Cắt theo ký tự (không cắt giữa một ký tự nhiều byte).
                value
                    .char_indices()
                    .nth(MAX_TEXT)
                    .map_or(value, |(end, _)| &value[..end])
                    .to_owned()
            })
        };
        let ip = match parts.extensions.get::<ClientIp>() {
            Some(resolved) => resolved.0,
            // Router không đi qua lớp `attach` (vd test dựng router riêng): địa chỉ kết nối.
            None => parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|info| info.0.ip()),
        };
        Ok(ClientInfo {
            ip,
            user_agent: text(header::USER_AGENT),
            origin: text(header::ORIGIN),
            referer: text(header::REFERER),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    fn headers(lines: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for line in lines {
            map.append("x-forwarded-for", HeaderValue::from_str(line).unwrap());
        }
        map
    }

    fn proxies(count: usize) -> ClientAddress {
        ClientAddress::new(Some(HeaderName::from_static("x-forwarded-for")), count, Vec::new())
    }

    #[test]
    fn khong_cau_hinh_header_thi_dung_dia_chi_ket_noi() {
        let direct = ClientAddress::new(None, 1, Vec::new());
        assert_eq!(
            direct.resolve(&headers(&["1.2.3.4"]), Some(ip("10.0.0.1"))),
            Some(ip("10.0.0.1"))
        );
    }

    #[test]
    fn mot_proxy_lay_phan_tu_cuoi_hai_proxy_lay_ke_cuoi() {
        let connection = Some(ip("10.0.0.1"));
        let list = headers(&["198.51.100.66, 203.0.113.9, 192.0.2.50"]);
        assert_eq!(proxies(1).resolve(&list, connection), Some(ip("192.0.2.50")));
        assert_eq!(proxies(2).resolve(&list, connection), Some(ip("203.0.113.9")));
        // Danh sách ngắn hơn số proxy: phần tử đầu.
        assert_eq!(proxies(5).resolve(&list, connection), Some(ip("198.51.100.66")));
        // Không có header: địa chỉ kết nối.
        assert_eq!(proxies(2).resolve(&HeaderMap::new(), connection), connection);
    }

    #[test]
    fn nhieu_dong_header_gop_thanh_mot_danh_sach() {
        // Client tự gửi một dòng giả, proxy thêm dòng mới: dòng của proxy ở cuối.
        let forged = headers(&["1.2.3.4", "203.0.113.10"]);
        assert_eq!(proxies(1).resolve(&forged, None), Some(ip("203.0.113.10")));
        let split = headers(&["1.2.3.4, 198.51.100.7", "203.0.113.10"]);
        assert_eq!(proxies(2).resolve(&split, None), Some(ip("198.51.100.7")));
    }

    #[test]
    fn gia_tri_khong_phai_ip_thi_dung_dia_chi_ket_noi() {
        let connection = Some(ip("10.0.0.1"));
        assert_eq!(proxies(1).resolve(&headers(&["unknown"]), connection), connection);
        assert_eq!(
            proxies(1).resolve(&headers(&["203.0.113.9:4711"]), connection),
            Some(ip("203.0.113.9"))
        );
        assert_eq!(
            proxies(1).resolve(&headers(&["[2001:db8::1]:4711"]), connection),
            Some(ip("2001:db8::1"))
        );
    }

    #[tokio::test]
    async fn client_info_doc_header_va_cat_bot() {
        let long = "a".repeat(5000);
        let request = axum::http::Request::builder()
            .header(header::USER_AGENT, long.as_str())
            .header(header::ORIGIN, "https://example.com")
            .body(())
            .unwrap();
        let (mut parts, ()) = request.into_parts();
        parts.extensions.insert(ClientIp(Some(ip("203.0.113.9"))));
        let info = ClientInfo::from_request_parts(&mut parts, &()).await.unwrap();
        assert_eq!(info.ip, Some(ip("203.0.113.9")));
        assert_eq!(info.user_agent.unwrap().len(), MAX_TEXT);
        assert_eq!(info.origin.as_deref(), Some("https://example.com"));
        assert_eq!(info.referer, None);
    }

    fn ranges(list: &str) -> ClientAddress {
        let source = crate::config::ConfigSource::from_pairs([
            ("AUTOXIDE_HTTP_CLIENT_IP_HEADER", "X-Forwarded-For"),
            ("AUTOXIDE_HTTP_TRUSTED_PROXIES", list),
        ]);
        let mut reader = crate::config::ConfigReader::new(&source);
        let config = crate::config::HttpConfig::read(&mut reader);
        let (config, _) = reader.finish(config).unwrap();
        ClientAddress::new(
            config.client_ip_header,
            config.trusted_proxy_count,
            config.trusted_proxies,
        )
    }

    #[test]
    fn dai_proxy_bo_qua_proxy_tin_cay_tu_phai_sang() {
        // CDN (198.51.100.0/24) → website (10.0.0.5) → API.
        let resolver = ranges("198.51.100.0/24, 10.0.0.5");
        let website = Some(ip("10.0.0.5"));
        let via_cdn = headers(&["6.6.6.6, 203.0.113.9, 198.51.100.20"]);
        assert_eq!(resolver.resolve(&via_cdn, website), Some(ip("203.0.113.9")));
        // Trình duyệt gọi thẳng website, bỏ qua CDN: vẫn là địa chỉ trình duyệt — đếm proxy
        // (N = 2) sẽ lấy địa chỉ giả `6.6.6.6`.
        let direct = headers(&["6.6.6.6, 203.0.113.9"]);
        assert_eq!(resolver.resolve(&direct, website), Some(ip("203.0.113.9")));
        assert_eq!(proxies(2).resolve(&direct, website), Some(ip("6.6.6.6")));
        // Kết nối không đến từ proxy tin cậy: header bị bỏ qua.
        let stranger = Some(ip("203.0.113.50"));
        assert_eq!(resolver.resolve(&via_cdn, stranger), stranger);
        assert_eq!(resolver.resolve(&via_cdn, None), None);
        // Toàn proxy tin cậy: phần tử đầu. Không có header: địa chỉ kết nối.
        assert_eq!(
            resolver.resolve(&headers(&["198.51.100.1, 10.0.0.5"]), website),
            Some(ip("198.51.100.1"))
        );
        assert_eq!(resolver.resolve(&HeaderMap::new(), website), website);
        // Giá trị không đọc được ngay sau proxy tin cậy: dừng ở đó, dùng địa chỉ kết nối.
        assert_eq!(resolver.resolve(&headers(&["203.0.113.9, unknown"]), website), website);
        // Kết nối IPv4 viết dạng IPv6 (socket hai giao thức) vẫn được nhận ra.
        assert_eq!(
            resolver.resolve(&via_cdn, Some(ip("::ffff:10.0.0.5"))),
            Some(ip("203.0.113.9"))
        );
    }
}
