//! The shared permission check of the HTTP layer (D-20261126-permission-gate): before any extractor of the
//! handler (JSON body, `If-Match`, parameters) and before the service looks a record up, the caller must have at
//! least one permission listed in the operation's `x-permissions` — otherwise 403 `permission_denied`. A caller
//! without access does not learn which endpoints need an `ETag`, which fields are wrong or which records exist.
//!
//! The only source is the module's OpenAPI document (like `approvals::rules`): `x-permissions: []` (any signed-in
//! user) and public operations pass. The layer only checks "has the permission at some unit"; scope, records and
//! step-up are still checked by the service. Every guarded operation needs a signed-in caller, so a caller that
//! cannot be authenticated gets the authentication error here (401), before any extractor of the handler.
//! `HEAD` is guarded like `GET` (axum answers `HEAD` with the `GET` handler).

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{FromRequestParts, MatchedPath, Request};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use utoipa::openapi::OpenApi;

use crate::access::{Principal, Realm};

/// One guarded operation: permission names (one of them is enough), and whether buyer accounts pass
/// (`x-buyers: true`: a buyer account has no permissions; the service decides what it may do).
#[derive(Debug)]
struct Guard {
    permissions: Vec<String>,
    buyers: bool,
}

/// Full path template → its guarded methods. Looked up once per request by the matched path: a map, not a scan over
/// every operation of the module; the guard is shared (`Arc`), nothing is copied per request.
#[derive(Debug, Default)]
pub(super) struct Rules {
    operations: HashMap<String, Vec<(Method, Arc<Guard>)>>,
}

impl Rules {
    pub(super) fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    fn find(&self, method: &Method, path: &str) -> Option<Arc<Guard>> {
        let methods = self.operations.get(path)?;
        let exact = |wanted: &Method| {
            methods.iter().find(|(known, _)| known == wanted).map(|(_, guard)| guard.clone())
        };
        // axum chạy handler `get` cho `HEAD` (bỏ thân trả lời) khi route không khai `head` riêng → kiểm như `GET`.
        exact(method).or_else(|| if method == Method::HEAD { exact(&Method::GET) } else { None })
    }
}

/// Operations of module `module` (full path `/api/v1/<module>…`) that list at least one permission.
pub(super) fn rules(module: &str, openapi: &OpenApi) -> Rules {
    let mut operations: HashMap<String, Vec<(Method, Arc<Guard>)>> = HashMap::new();
    for (path, item) in &openapi.paths.paths {
        let methods = [
            (Method::GET, item.get.as_ref()),
            (Method::POST, item.post.as_ref()),
            (Method::PUT, item.put.as_ref()),
            (Method::PATCH, item.patch.as_ref()),
            (Method::DELETE, item.delete.as_ref()),
            (Method::HEAD, item.head.as_ref()),
        ];
        for (method, operation) in methods {
            let Some(operation) = operation else { continue };
            let permissions: Vec<String> = operation
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.get("x-permissions"))
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect();
            if permissions.is_empty() {
                continue;
            }
            let buyers = operation
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.get("x-buyers"))
                .is_some_and(|value| value == true);
            // Đường gốc của module là `/` trong tài liệu của nó, `/api/v1/<module>` sau khi ghép (không có `/` cuối).
            let tail = if path == "/" { "" } else { path.as_str() };
            let guard = Arc::new(Guard { permissions, buyers });
            operations
                .entry(format!("{}/{module}{tail}", super::API_PREFIX))
                .or_default()
                .push((method, guard));
        }
    }
    Rules { operations }
}

/// The layer (see the top of the file).
pub(super) async fn gate(rules: Arc<Rules>, request: Request, next: Next) -> Response {
    let Some(guard) = request
        .extensions()
        .get::<MatchedPath>()
        .and_then(|matched| rules.find(request.method(), matched.as_str()))
    else {
        return next.run(request).await;
    };
    let (mut parts, body) = request.into_parts();
    // Xác thực một lần; kết quả được giữ trong `extensions`, extractor `Principal` của handler dùng lại. Operation có
    // permission luôn cần đăng nhập: xác thực hỏng thì trả lỗi ngay, không để extractor khác của handler (thân, header)
    // trả lời trước cho người ẩn danh, và không xác thực (đếm giới hạn API key) lần thứ hai.
    let principal = match Principal::from_request_parts(&mut parts, &()).await {
        Ok(principal) => principal,
        Err(error) => return error.into_response(),
    };
    if !(guard.buyers && principal.realm() == Realm::Customer)
        && let Err(error) = principal.require_any_named(&guard.permissions)
    {
        return error.into_response();
    }
    next.run(Request::from_parts(parts, body)).await
}
