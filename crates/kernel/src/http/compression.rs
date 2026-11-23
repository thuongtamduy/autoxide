//! Response compression (gzip, brotli) inside the app (step 39 B4, D-20261119-step39-compression).
//!
//! Measured on real JSON (`docs/BENCHMARKS.md`, section "Response compression"): a 136 KB public page of
//! 20 posts shrinks to ~26–32 KB, the 3 MB `openapi.json` to ~370 KB, and at the fastest level each page
//! costs under 1 ms of CPU. A high level (brotli 11) is ~25% smaller still but costs hundreds of times
//! more CPU, so level 1 is used.
//!
//! Only **text** is compressed: JSON (`application/json`, `application/problem+json`, `…+json`), `text/*`
//! except `text/event-stream` (MCP stream, events - compression delays each event), XML (sitemap, feed),
//! JavaScript. User-uploaded files (images, PDF, zip...) are already compressed or binary: not compressed.
//! Bodies under 1 KiB are not worth compressing. A partial response (`206`, with `Content-Range`) is
//! skipped by `tower-http`. `ETag` is kept: comparing `If-None-Match` stays correct, and `If-Match` on
//! update requests is unchanged (`Vary: Accept-Encoding` separates the cache entries).
//!
//! Placed OUTSIDE the idempotency layer: the stored copy for replay is the uncompressed one (a replayed
//! call may ask for a different encoding). Turn it off with `AUTOXIDE_HTTP_COMPRESSION=false` when the
//! reverse proxy / CDN already compresses.

use axum::http::{Extensions, HeaderMap, StatusCode, Version, header};
use tower_http::compression::predicate::{Predicate, SizeAbove};
use tower_http::compression::{CompressionLayer, CompressionLevel};

/// A body smaller than this many bytes is not compressed: the gzip / brotli header and CPU are not worth
/// it.
const MIN_BYTES: u64 = 1024;

/// The API's compression layer.
pub(super) fn layer() -> CompressionLayer<impl Predicate> {
    CompressionLayer::new()
        .gzip(true)
        .br(true)
        .quality(CompressionLevel::Precise(1))
        .compress_when(SizeAbove::new(MIN_BYTES).and(is_text))
}

/// The response is compressible text (see the top of the file).
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
