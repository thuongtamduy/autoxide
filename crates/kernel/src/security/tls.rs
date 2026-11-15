//! TLS của chương trình: rustls với provider mật mã `ring` (D-043, D-066).
//!
//! HTTP client (`reqwest`, dùng bởi exporter OTLP và bởi kho file S3) được build với
//! rustls KHÔNG kèm provider, để không kéo thêm thư viện mật mã thứ hai (aws-lc-rs).
//! Vì vậy provider phải được cài cho cả tiến trình trước khi dựng client đầu tiên —
//! chỗ nào dựng client thì gọi [`install_crypto_provider`] trước.

/// Cài `ring` làm provider mặc định của rustls cho tiến trình. Gọi nhiều lần, từ nhiều
/// thread, đều an toàn; đã có provider thì giữ nguyên.
pub fn install_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        // Lỗi chỉ khi một thread khác vừa cài trước: kết quả vẫn là điều ta cần.
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}
