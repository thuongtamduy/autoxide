//! Lớp HTTP của idempotency key (xem `crate::idempotency`).

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use crate::error::Error;
use crate::idempotency::{Begin, IdempotencyStore, StoredResponse};

/// Header client gửi.
pub const IDEMPOTENCY_KEY: HeaderName = HeaderName::from_static("idempotency-key");
/// Header đánh dấu response là bản đã lưu.
pub const IDEMPOTENT_REPLAYED: HeaderName = HeaderName::from_static("idempotent-replayed");
/// Độ dài tối đa của key.
const MAX_KEY_LENGTH: usize = 255;
/// Header của response được lưu và trả lại khi gửi lại.
const KEPT_HEADERS: [HeaderName; 5] = [
    header::CONTENT_TYPE,
    header::LOCATION,
    header::RETRY_AFTER,
    header::WWW_AUTHENTICATE,
    header::CONTENT_LANGUAGE,
];

pub(super) struct Settings {
    pub store: Arc<dyn IdempotencyStore>,
    /// Giới hạn thân request (đọc trước để băm).
    pub body_limits: Arc<super::body_limit::BodyLimits>,
    /// Thời gian tối đa của một request: hết thì lần đầu coi như đã chết.
    pub request_timeout: Duration,
}

pub(super) async fn idempotency(
    settings: Option<Arc<Settings>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(settings) = settings else {
        return next.run(request).await;
    };
    let writes =
        matches!(*request.method(), Method::POST | Method::PUT | Method::PATCH | Method::DELETE);
    let Some(key) = request.headers().get(&IDEMPOTENCY_KEY).cloned() else {
        return next.run(request).await;
    };
    if !writes {
        return next.run(request).await;
    }
    match run(&settings, key, request, next).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn run(
    settings: &Settings,
    key: HeaderValue,
    request: Request,
    next: Next,
) -> Result<Response, Error> {
    let key = key
        .to_str()
        .ok()
        .filter(|key| (1..=MAX_KEY_LENGTH).contains(&key.len()))
        .ok_or_else(|| {
            Error::bad_request(
                "idempotency_key_invalid",
                "The `Idempotency-Key` header must be 1 to {max} visible ASCII characters; a \
                 random UUID is a good choice.",
            )
            .with_param("max", MAX_KEY_LENGTH)
        })?
        .to_owned();
    // Người gọi = thông tin xác thực đã băm: key của hai người khác nhau không đụng nhau.
    let authorization =
        request.headers().get(header::AUTHORIZATION).map(HeaderValue::as_bytes).unwrap_or_default();
    let scope = hash(&[authorization]);
    let limit = settings.body_limits.for_request(&request);
    let (parts, body) = request.into_parts();
    let bytes =
        axum::body::to_bytes(body, limit).await.map_err(|_| super::body_limit::too_large(limit))?;
    let fingerprint =
        hash(&[parts.method.as_str().as_bytes(), parts.uri.path().as_bytes(), bytes.as_ref()]);
    let lock = chrono::Duration::from_std(settings.request_timeout + Duration::from_secs(30))
        .map_err(Error::internal)?;
    let begin = settings.store.begin(&scope, &key, &fingerprint, chrono::Utc::now() + lock).await?;
    let id = match begin {
        Begin::Started { id } => id,
        Begin::Replay(stored) => return Ok(replay(stored)),
        Begin::InProgress => {
            return Err(Error::conflict(
                "idempotency_key_in_progress",
                "A request with this `Idempotency-Key` is still running. Retry in a moment to \
                 get its result.",
            )
            .with_retry_after(1));
        }
        Begin::Mismatch => {
            return Err(Error::new(
                crate::error::ErrorKind::Validation,
                "idempotency_key_reused",
                "This `Idempotency-Key` was already used for a different request. Use a new \
                 key for each distinct request.",
            ));
        }
    };

    let response = next.run(Request::from_parts(parts, Body::from(bytes))).await;
    if response.status().is_server_error() {
        // Lỗi phía máy chủ: không lưu, để client gửi lại được với cùng key.
        settings.store.abandon(id).await?;
        return Ok(response);
    }
    let (response_parts, body) = response.into_parts();
    let body = axum::body::to_bytes(body, usize::MAX).await.map_err(Error::internal)?;
    let headers = KEPT_HEADERS
        .iter()
        .filter_map(|name| {
            let value = response_parts.headers.get(name)?.to_str().ok()?;
            Some((name.as_str().to_owned(), value.to_owned()))
        })
        .collect();
    let stored =
        StoredResponse { status: response_parts.status.as_u16(), headers, body: body.to_vec() };
    settings.store.complete(id, stored).await?;
    Ok(Response::from_parts(response_parts, Body::from(body)))
}

/// Response đã lưu, dựng lại.
fn replay(stored: StoredResponse) -> Response {
    let mut response = Response::new(Body::from(stored.body));
    *response.status_mut() = StatusCode::from_u16(stored.status).unwrap_or(StatusCode::OK);
    for (name, value) in stored.headers {
        if let (Ok(name), Ok(value)) = (HeaderName::try_from(name), HeaderValue::from_str(&value)) {
            response.headers_mut().insert(name, value);
        }
    }
    response.headers_mut().insert(IDEMPOTENT_REPLAYED, HeaderValue::from_static("true"));
    response
}

/// SHA-256 của các phần nối nhau (mỗi phần kèm độ dài, để `ab|c` khác `a|bc`).
fn hash(parts: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    URL_SAFE_NO_PAD.encode(digest.finalize())
}
