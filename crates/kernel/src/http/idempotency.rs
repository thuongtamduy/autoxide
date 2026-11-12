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
const KEPT_HEADERS: [HeaderName; 6] = [
    header::CONTENT_TYPE,
    header::ETAG,
    header::LOCATION,
    header::RETRY_AFTER,
    header::WWW_AUTHENTICATE,
    header::CONTENT_LANGUAGE,
];

/// Đánh dấu trên response: request có thể đã làm xong việc dù trả lỗi (hết giờ, panic giữa
/// chừng — transaction có thể đã commit). Lớp idempotency không cho chạy lại bằng cùng key
/// (D-157).
#[derive(Debug, Clone, Copy)]
pub(super) struct OutcomeUnknown;

/// Response lớn hơn chừng này không được lưu (thay bằng lời giải thích): kho idempotency không
/// phình theo response lớn (đọc file qua MCP…).
const MAX_STORED_BODY: usize = 256 * 1024;

pub(super) struct Settings {
    pub store: Arc<dyn IdempotencyStore>,
    /// Xác thực người gọi TRƯỚC khi đọc thân request: người lạ không làm máy chủ giữ thân
    /// request trong bộ nhớ hay ghi dòng nào vào kho (đo được khi rà soát, D-079).
    pub authenticator: Option<Arc<dyn crate::access::Authenticator>>,
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
    // Người gọi = danh tính đã xác thực (người dùng, và API key nếu có) — không phải chuỗi
    // token: access token đổi mỗi lần làm mới, lần gửi lại sau đó vẫn phải nhận ra. Không
    // xác thực được thì bỏ qua idempotency: handler tự trả lỗi, không đọc, không lưu gì.
    let authorization =
        request.headers().get(header::AUTHORIZATION).and_then(|value| value.to_str().ok());
    let principal = match &settings.authenticator {
        Some(authenticator) => authenticator.authenticate(authorization).await.ok(),
        None => None,
    };
    let Some(principal) = principal else {
        return Ok(next.run(request).await);
    };
    let api_key = principal.api_key_id().map(|id| id.as_bytes().to_vec()).unwrap_or_default();
    let scope = hash(&[principal.user_id().as_bytes(), &api_key]);
    let limit = settings.body_limits.for_request(&request);
    let declared = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    // Chỉ giữ trong bộ nhớ thân không lớn hơn giới hạn mặc định. Route nhận thân lớn (tải
    // file lên, tới hàng GB) mà thân thật sự lớn thì KHÔNG đọc trước: thân chảy thẳng vào
    // handler như khi không có key, và dấu của request dùng độ dài thay cho nội dung (D-149).
    // Đọc trước cả trăm MB chỉ để băm sẽ cho người chỉ cần đăng nhập giữ RAM máy chủ.
    let buffer_limit = limit.min(settings.body_limits.default_limit());
    let streamed = streams(limit, buffer_limit, declared);
    let (parts, body) = request.into_parts();
    let content_type =
        parts.headers.get(header::CONTENT_TYPE).map(HeaderValue::as_bytes).unwrap_or_default();
    let target = parts.uri.path_and_query().map(|target| target.as_str()).unwrap_or_default();
    let (body, content) = if streamed {
        let length = declared.map(|length| length.to_string()).unwrap_or_default();
        (body, format!("streamed:{length}").into_bytes())
    } else {
        // Đọc thân trong giới hạn thời gian của request: client gửi nhỏ giọt không giữ được
        // kết nối và bộ nhớ mãi.
        let bytes = match tokio::time::timeout(
            settings.request_timeout,
            axum::body::to_bytes(body, buffer_limit),
        )
        .await
        {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(_)) => return Err(super::body_limit::too_large(buffer_limit)),
            Err(_) => return Err(super::timed_out(settings.request_timeout)),
        };
        let mut content = b"buffered:".to_vec();
        content.extend_from_slice(&bytes);
        (Body::from(bytes), content)
    };
    // Cùng key mà khác đường dẫn, query, loại nội dung, `If-Match` hay thân: là request khác
    // (client đọc lại rồi gửi `If-Match` mới thì không nhận lại lỗi 412 cũ).
    let if_match =
        parts.headers.get(header::IF_MATCH).map(HeaderValue::as_bytes).unwrap_or_default();
    let fingerprint = hash(&[
        parts.method.as_str().as_bytes(),
        target.as_bytes(),
        content_type,
        if_match,
        &content,
    ]);
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

    let response = next.run(Request::from_parts(parts, body)).await;
    // Lưu kết quả mà hỏng (database bận, mất kết nối): việc đã xong, client vẫn nhận đúng
    // response; dòng còn khoá tới `locked_until` rồi lần gửi lại chạy lại (đã ghi log).
    let (response, outcome) = settle(settings, id, response).await;
    if let Err(error) = outcome {
        tracing::error!(code = error.code(), "cannot save the idempotent result");
    }
    Ok(response)
}

/// Quyết định lưu gì cho lần chạy `id` vừa trả `response`, rồi trả response cho client.
async fn settle(
    settings: &Settings,
    id: uuid::Uuid,
    response: Response,
) -> (Response, Result<(), Error>) {
    let status = response.status();
    if response.extensions().get::<OutcomeUnknown>().is_some() {
        // Hết giờ hay panic: việc có thể đã xong. Không bỏ (chạy lại có thể làm hai lần),
        // không lưu lỗi 5xx như kết quả: lần gửi lại nhận lời giải thích (D-157).
        let refusal = not_kept(Error::conflict(
            "idempotency_result_unknown",
            "The first request with this `Idempotency-Key` did not finish in time, so the \
             server cannot tell whether it took effect. Look the result up before repeating \
             it, then use a new key.",
        ));
        let outcome = keep(settings, id, refusal).await;
        return (response, outcome);
    }
    if status.is_server_error() || is_temporary(status) {
        // Lỗi phía máy chủ, hay lỗi tạm (429 vượt giới hạn, 408): không lưu, để client gửi lại
        // được với cùng key khi hết giới hạn (D-157).
        return (response, settings.store.abandon(id).await);
    }
    if status.is_success() && no_store(response.headers()) {
        // Response chứa secret chỉ trả một lần (API key mới, token, mã khôi phục): không lưu
        // nó vào database. Lần gửi lại nhận lời giải thích thay vì chạy lại việc (D-149).
        let refusal = not_kept(Error::conflict(
            "idempotency_result_not_kept",
            "The first request with this `Idempotency-Key` succeeded, but its response held \
             a secret that the server does not keep. Do not repeat it: look the result up \
             instead (for example list your API keys), or use a new key to do it again.",
        ));
        let outcome = keep(settings, id, refusal).await;
        return (response, outcome);
    }
    let (response_parts, body) = response.into_parts();
    let body = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(body) => body,
        Err(error) => {
            let error = Error::internal(error);
            let abandoned = settings.store.abandon(id).await;
            return (error.into_response(), abandoned);
        }
    };
    let outcome = if body.len() > MAX_STORED_BODY {
        let refusal = not_kept(Error::conflict(
            "idempotency_result_too_large",
            "The first request with this `Idempotency-Key` succeeded, but its response was \
             too large to keep. Do not repeat it: look the result up instead, or use a new \
             key to do it again.",
        ));
        keep(settings, id, refusal).await
    } else {
        settings.store.complete(id, stored(&response_parts, &body)).await
    };
    (Response::from_parts(response_parts, Body::from(body)), outcome)
}

/// Lỗi tạm thời: gửi lại sau sẽ khác.
fn is_temporary(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS | StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_EARLY
    )
}

/// Response (409) thay cho kết quả không lưu được.
fn not_kept(error: Error) -> Response {
    error.into_response()
}

/// Lưu `refusal` làm kết quả của lần chạy `id`.
async fn keep(settings: &Settings, id: uuid::Uuid, refusal: Response) -> Result<(), Error> {
    let (parts, body) = refusal.into_parts();
    let body = axum::body::to_bytes(body, usize::MAX).await.map_err(Error::internal)?;
    settings.store.complete(id, stored(&parts, &body)).await
}

/// Bản lưu của một response: status, các header cần giữ, thân.
fn stored(parts: &axum::http::response::Parts, body: &[u8]) -> StoredResponse {
    let headers = KEPT_HEADERS
        .iter()
        .filter_map(|name| {
            let value = parts.headers.get(name)?.to_str().ok()?;
            Some((name.as_str().to_owned(), value.to_owned()))
        })
        .collect();
    StoredResponse { status: parts.status.as_u16(), headers, body: body.to_vec() }
}

/// Response có `Cache-Control: no-store` không.
fn no_store(headers: &axum::http::HeaderMap) -> bool {
    headers.get_all(header::CACHE_CONTROL).iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value.split(',').any(|directive| directive.trim().eq_ignore_ascii_case("no-store"))
        })
    })
}

/// Thân request có chảy thẳng vào handler (không đọc trước để băm) không: chỉ khi route nhận
/// thân lớn hơn mức được giữ trong bộ nhớ và thân không tự khai là nhỏ.
fn streams(limit: usize, buffer_limit: usize, declared: Option<usize>) -> bool {
    limit > buffer_limit && declared.is_none_or(|length| length > buffer_limit)
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

#[cfg(test)]
mod tests {
    use super::streams;

    #[test]
    fn chi_than_lon_cua_route_than_lon_moi_khong_doc_truoc() {
        const MIB: usize = 1024 * 1024;
        // Route thường: luôn đọc trước (giới hạn = mức giữ trong bộ nhớ).
        assert!(!streams(MIB, MIB, None));
        assert!(!streams(MIB, MIB, Some(10 * MIB)));
        // Route tải lên: thân nhỏ thì đọc trước, lớn hoặc không khai độ dài thì không.
        assert!(!streams(100 * MIB, MIB, Some(1000)));
        assert!(streams(100 * MIB, MIB, Some(2 * MIB)));
        assert!(streams(100 * MIB, MIB, None));
    }
}
