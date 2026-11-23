//! A fake server for a provider (step 70 P7, D-20261121-provider-accounts): a connector tests requests,
//! signatures and webhooks exactly per the provider's documentation without going on the network.
//!
//! ```ignore
//! let server = FakeProviderServer::start().await;
//! server.reply("POST", "/v1/orders", 200, json!({"code": 0, "id": "A1"}));
//! // … point the connector at `server.url()` (variable `AUTOXIDE_<NAME>_API_URL`), call the connector …
//! let seen = server.take_requests();
//! assert_eq!(seen[0].header("authorization"), Some("Bearer tenant-a-key"));
//! ```
//!
//! Answers are queued by (method, path): each request takes the answer at the head of the queue; when one
//! answer is left it is reused forever (the connector calling again gets the same answer). A request with no
//! answer → 501 and is still recorded.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::Value;

/// A request the fake server has received.
#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub method: String,
    pub path: String,
    /// Raw query string (without the `?`), empty when absent.
    pub query: String,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl ProviderRequest {
    /// Value of header `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    /// Body read as JSON; `Value::Null` when it is not JSON.
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    /// Value of parameter `name` in the query string (not percent-decoded).
    pub fn query_param(&self, name: &str) -> Option<&str> {
        self.query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find_map(|(key, value)| (key == name).then_some(value))
    }
}

#[derive(Debug, Clone)]
struct Canned {
    status: u16,
    content_type: &'static str,
    body: Bytes,
}

#[derive(Default)]
struct Shared {
    replies: Mutex<HashMap<(String, String), VecDeque<Canned>>>,
    requests: Mutex<Vec<ProviderRequest>>,
}

/// A fake HTTP server on `127.0.0.1`, random port. Shuts down when the test process ends.
#[derive(Clone)]
pub struct FakeProviderServer {
    url: String,
    shared: Arc<Shared>,
}

impl FakeProviderServer {
    /// Opens the server. `Err` when the port cannot be opened.
    pub async fn start() -> std::io::Result<Self> {
        let shared = Arc::new(Shared::default());
        // Một handler nhận mọi request (đây không phải API của Autoxide nên không qua `routes!`).
        let service = axum::handler::Handler::with_state(answer, shared.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        tokio::spawn(async move { axum::serve(listener, service.into_make_service()).await });
        Ok(FakeProviderServer { url: format!("http://{address}"), shared })
    }

    /// Base address, e.g. `http://127.0.0.1:41873` (no trailing `/`).
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Queues a JSON answer for `method` `path`.
    pub fn reply(&self, method: &str, path: &str, status: u16, body: Value) {
        self.push(method, path, status, "application/json", body.to_string().into_bytes().into());
    }

    /// Queues a raw-body answer (PDF, XML, text…) for `method` `path`.
    pub fn reply_raw(
        &self,
        method: &str,
        path: &str,
        status: u16,
        content_type: &'static str,
        body: impl Into<Bytes>,
    ) {
        self.push(method, path, status, content_type, body.into());
    }

    fn push(&self, method: &str, path: &str, status: u16, content_type: &'static str, body: Bytes) {
        self.shared
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry((method.to_ascii_uppercase(), path.to_owned()))
            .or_default()
            .push_back(Canned { status, content_type, body });
    }

    /// All requests received since the previous take, in order of arrival.
    pub fn take_requests(&self) -> Vec<ProviderRequest> {
        std::mem::take(&mut *self.shared.requests.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

async fn answer(
    State(shared): State<Arc<Shared>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let key = (method.as_str().to_owned(), uri.path().to_owned());
    shared.requests.lock().unwrap_or_else(PoisonError::into_inner).push(ProviderRequest {
        method: key.0.clone(),
        path: key.1.clone(),
        query: uri.query().unwrap_or_default().to_owned(),
        headers,
        body,
    });
    let canned = {
        let mut replies = shared.replies.lock().unwrap_or_else(PoisonError::into_inner);
        replies.get_mut(&key).and_then(|queue| {
            if queue.len() > 1 { queue.pop_front() } else { queue.front().cloned() }
        })
    };
    let Some(canned) = canned else {
        return (StatusCode::NOT_IMPLEMENTED, "no reply was set for this request").into_response();
    };
    let status = StatusCode::from_u16(canned.status).unwrap_or(StatusCode::OK);
    (status, [(axum::http::header::CONTENT_TYPE, canned.content_type)], canned.body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn ghi_request_va_tra_cau_da_xep() {
        let server = FakeProviderServer::start().await.unwrap();
        server.reply("POST", "/v1/orders", 200, json!({"id": "A1"}));
        server.reply("POST", "/v1/orders", 409, json!({"id": "A1", "again": true}));
        let client = autoxide_kernel::provider::http_client(
            std::time::Duration::from_secs(5),
            autoxide_kernel::provider::Target::Fixed,
        )
        .unwrap();
        let call = |path: &str| {
            let request = client
                .post(format!("{}{path}", server.url()))
                .header("authorization", "Bearer k")
                .body("{\"a\":1}");
            autoxide_kernel::provider::send(request, "fake", "test")
        };
        let first = call("/v1/orders?x=1").await.unwrap();
        assert_eq!((first.status, first.json().unwrap()["id"].as_str()), (200, Some("A1")));
        // Câu cuối hàng được dùng lại.
        assert_eq!(call("/v1/orders").await.unwrap().status, 409);
        assert_eq!(call("/v1/orders").await.unwrap().status, 409);
        assert_eq!(call("/unknown").await.unwrap().status, 501);
        let seen = server.take_requests();
        assert_eq!(seen.len(), 4);
        assert_eq!(seen[0].header("Authorization"), Some("Bearer k"));
        assert_eq!(seen[0].query_param("x"), Some("1"));
        assert_eq!(seen[0].json(), json!({"a": 1}));
        assert!(server.take_requests().is_empty());
    }
}
