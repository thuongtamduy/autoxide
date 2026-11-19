//! Nén response (gzip, brotli) ngay trong app (bước 39 B4, D-20261119-step39-compression).
//!
//! Đo trên JSON thật (`docs/BENCHMARKS.md`, mục "Nén response"): trang 20 bài công khai 136 KB còn
//! ~26–32 KB, `openapi.json` 3 MB còn ~370 KB, ở mức nén nhanh nhất mỗi trang dưới 1 ms CPU. Mức cao
//! (brotli 11) nhỏ thêm ~25% nhưng tốn hàng trăm lần CPU, nên dùng mức 1.
//!
//! Chỉ nén **chữ**: JSON (`application/json`, `application/problem+json`, `…+json`), `text/*` trừ
//! `text/event-stream` (stream MCP, sự kiện — nén làm chậm từng sự kiện tới), XML (sitemap, feed),
//! JavaScript. File người dùng tải lên (ảnh, PDF, zip…) đã nén sẵn hay là nhị phân: không nén. Thân
//! dưới 1 KiB không đáng nén. Response một khúc (`206`, có `Content-Range`) thì `tower-http` tự bỏ
//! qua. `ETag` giữ nguyên: so `If-None-Match` vẫn đúng, `If-Match` của lệnh sửa không đổi
//! (`Vary: Accept-Encoding` tách bản lưu của cache).
//!
//! Đặt NGOÀI lớp idempotency: bản lưu để trả lại là bản chưa nén (lần gọi lại có thể xin cách nén
//! khác). Tắt bằng `AUTOXIDE_HTTP_COMPRESSION=false` khi reverse proxy / CDN đã nén.

use axum::http::{Extensions, HeaderMap, StatusCode, Version, header};
use tower_http::compression::predicate::{Predicate, SizeAbove};
use tower_http::compression::{CompressionLayer, CompressionLevel};

/// Thân nhỏ hơn chừng này byte không nén: phần đầu gzip / brotli và CPU không đáng.
const MIN_BYTES: u64 = 1024;

/// Lớp nén của API.
pub(super) fn layer() -> CompressionLayer<impl Predicate> {
    CompressionLayer::new()
        .gzip(true)
        .br(true)
        .quality(CompressionLevel::Precise(1))
        .compress_when(SizeAbove::new(MIN_BYTES).and(is_text))
}

/// Response là chữ nén được (xem đầu file).
fn is_text(_: StatusCode, _: Version, headers: &HeaderMap, _: &Extensions) -> bool {
    let Some(value) = headers.get(header::CONTENT_TYPE).and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let essence = value.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    match essence.split_once('/') {
        Some(("text", "event-stream")) => false,
        Some(("text", _)) => true,
        Some(("application", subtype)) => {
            matches!(subtype, "json" | "xml" | "javascript" | "ld+json")
                || subtype.ends_with("+json")
                || subtype.ends_with("+xml")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(content_type: &str) -> bool {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, content_type.parse().unwrap());
        is_text(StatusCode::OK, Version::HTTP_11, &headers, &Extensions::new())
    }

    #[test]
    fn chi_nen_chu() {
        for yes in [
            "application/json",
            "application/problem+json",
            "application/json; charset=utf-8",
            "application/rss+xml",
            "application/xml",
            "text/plain; charset=utf-8",
            "text/html",
            "Application/JSON",
        ] {
            assert!(text(yes), "{yes}");
        }
        for no in [
            "image/jpeg",
            "image/webp",
            "application/pdf",
            "application/zip",
            "application/octet-stream",
            "text/event-stream",
            "video/mp4",
        ] {
            assert!(!text(no), "{no}");
        }
        assert!(!is_text(StatusCode::OK, Version::HTTP_11, &HeaderMap::new(), &Extensions::new()));
    }
}
