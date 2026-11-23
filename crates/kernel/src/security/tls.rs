//! The program's TLS: rustls with the `ring` crypto provider (D-043, D-066).
//!
//! The HTTP client (`reqwest`, used by the OTLP exporter and the S3 file store) is built with rustls WITHOUT a
//! provider, so that a second crypto library (aws-lc-rs) is not pulled in. Therefore the provider must be
//! installed for the whole process before the first client is built; wherever a client is built, call
//! [`install_crypto_provider`] first.

/// Installs `ring` as rustls's default provider for the process. Safe to call many times, from many
/// threads; when a provider already exists it is left unchanged.
pub fn install_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        // Lỗi chỉ khi một thread khác vừa cài trước: kết quả vẫn là điều ta cần.
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}
