//! Lỗi của module `ai` (tiền tố `ai.`, bản dịch ở `locales/vi.json`). Mã lỗi là hợp đồng: module
//! khác (và client) dựa vào chúng.

use autoxide_kernel::error::{Error, ErrorKind};
use chrono::{Duration, Timelike, Utc};

pub(crate) fn disabled() -> Error {
    Error::unavailable(
        "ai.disabled",
        "AI features are off on this server. The operator turns them on with \
         AUTOXIDE_AI_PROVIDER and AUTOXIDE_AI_API_KEY.",
    )
}

pub(crate) fn embedding_disabled() -> Error {
    Error::unavailable(
        "ai.embedding_disabled",
        "Embeddings are off on this server. The operator turns them on with \
         AUTOXIDE_EMBEDDING_PROVIDER.",
    )
}

/// Nhà cung cấp từ chối thông tin xác thực hay cấu hình (401, 402, 403, 404 mô hình).
pub(crate) fn misconfigured() -> Error {
    Error::unavailable(
        "ai.misconfigured",
        "The AI provider refused this server's key or settings. The operator should check the \
         AUTOXIDE_AI_* (or AUTOXIDE_EMBEDDING_*) settings, for example with `autoxide ai check`.",
    )
}

/// Hết hạn mức token của ngày (UTC): chờ tới nửa đêm UTC.
pub(crate) fn quota_exceeded() -> Error {
    let now = Utc::now();
    let elapsed = i64::from(now.num_seconds_from_midnight());
    let until_midnight = (Duration::days(1).num_seconds() - elapsed).max(1);
    Error::too_many_requests(
        "ai.quota_exceeded",
        "Today's AI token limit (AUTOXIDE_AI_DAILY_TOKEN_LIMIT) is used up. Try again after \
         midnight UTC, or ask the operator to raise the limit.",
    )
    .with_retry_after(u64::try_from(until_midnight).unwrap_or(1))
}

/// Tạm thời: quá tải, rate limit, lỗi mạng, hết giờ chờ. Thử lại được.
pub(crate) fn provider_unavailable(retry_after: Option<u64>) -> Error {
    let error = Error::unavailable(
        "ai.provider_unavailable",
        "The AI provider is busy or cannot be reached. Retry in a moment.",
    );
    match retry_after {
        Some(seconds) => error.with_retry_after(seconds),
        None => error,
    }
}

pub(crate) fn refused() -> Error {
    Error::new(
        ErrorKind::Validation,
        "ai.refused",
        "The AI model declined this content. Change the content or do this step by hand.",
    )
}

pub(crate) fn output_truncated() -> Error {
    Error::new(
        ErrorKind::Validation,
        "ai.output_truncated",
        "The AI answer was cut off because it was too long. Send less content at once.",
    )
}

pub(crate) fn output_invalid() -> Error {
    Error::unavailable(
        "ai.output_invalid",
        "The AI model gave an answer in the wrong shape, twice. Retry later, or use a more \
         capable model.",
    )
}

pub(crate) fn unsupported_input() -> Error {
    Error::new(
        ErrorKind::Validation,
        "ai.unsupported_input",
        "The configured AI model does not read images (AUTOXIDE_AI_IMAGES is `false`).",
    )
}

/// Nhà cung cấp từ chối hẳn request (400, 413, 422…): thử lại không giúp.
pub(crate) fn request_rejected() -> Error {
    Error::unavailable(
        "ai.request_rejected",
        "The AI provider rejected the request, for example because the content is too long for \
         the model or a setting is not accepted by it. The server log has the provider's reason.",
    )
}
