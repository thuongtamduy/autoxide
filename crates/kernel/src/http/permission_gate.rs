//! The shared permission check of the HTTP layer (D-20261126-permission-gate): before any extractor of the
//! handler (JSON body, `If-Match`, parameters) and before the service looks a record up, the caller must have at
//! least one permission listed in the operation's `x-permissions` — otherwise 403 `permission_denied`. A caller
//! without access does not learn which endpoints need an `ETag`, which fields are wrong or which records exist.
//!
//! The only source is the module's OpenAPI document (like `approvals::rules`): `x-permissions: []` (any signed-in
//! user) needs only a signed-in caller — enforced here even when the handler does not take a `Principal` — and public
//! operations (no `x-permissions`) pass. The layer only checks "has the permission at some unit"; scope, records and
//! step-up are still checked by the service. Every guarded operation needs a signed-in caller, so a caller that
//! cannot be authenticated gets the authentication error here (401), before any extractor of the handler.
//! A route that does not declare its own `HEAD` answers `HEAD` with its `GET` handler (axum), so `HEAD` is guarded
//! like that `GET`; a declared `HEAD` follows its own `x-permissions` (none listed: not guarded).

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

/// A method of a path and its guard (`None`: a declared `HEAD` with no permission).
type MethodGuard = (Method, Option<Arc<Guard>>);

/// Full path template → its methods: a guard, or `None` for a declared `HEAD` that lists no permission (so it does not
/// fall back to the `GET` rule). Looked up once per request by the matched path: a map, not a scan over
/// every operation of the module; the guard is shared (`Arc`), nothing is copied per request.
#[derive(Debug, Default)]
pub(super) struct Rules {
    operations: HashMap<String, Vec<MethodGuard>>,
}

impl Rules {
    pub(super) fn is_empty(&self) -> bool {
        !self.operations.values().flatten().any(|(_, guard)| guard.is_some())
    }

    fn find(&self, method: &Method, path: &str) -> Option<Arc<Guard>> {
        let methods = self.operations.get(path)?;
        let exact = |wanted: &Method| {
            methods.iter().find(|(known, _)| known == wanted).map(|(_, guard)| guard.clone())
        };
        match exact(method) {
            Some(guard) => guard,
            // axum chạy handler `get` cho `HEAD` (bỏ thân trả lời) khi route không khai `head` riêng → kiểm như `GET`.
            None if method == Method::HEAD => exact(&Method::GET).flatten(),
            None => None,
        }
    }
}

/// Operations of module `module` (full path `/api/v1/<module>…`) that list at least one permission.
pub(super) fn rules(module: &str, openapi: &OpenApi) -> Rules {
    let mut operations: HashMap<String, Vec<MethodGuard>> = HashMap::new();
    for (path, item) in &openapi.paths.paths {
        let methods = [
            (Method::GET, item.get.as_ref()),
            (Method::POST, item.post.as_ref()),
            (Method::PUT, item.put.as_ref()),
            (Method::PATCH, item.patch.as_ref()),
            (Method::DELETE, item.delete.as_ref()),
            (Method::HEAD, item.head.as_ref()),
            // `#[endpoint]` nhận cả `options` và `trace`: gate phủ mọi method mà tài liệu khai.
            (Method::OPTIONS, item.options.as_ref()),
            (Method::TRACE, item.trace.as_ref()),
        ];
        for (method, operation) in methods {
            let Some(operation) = operation else { continue };
            // `None`: operation không khai `x-permissions` (công khai). `Some([])`: chỉ cần đăng nhập (`signed_in`).
            let declared = operation
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.get("x-permissions"))
                .and_then(|value| value.as_array());
            let permissions: Vec<String> = declared
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect();
            // Đường gốc của module là `/` trong tài liệu của nó, `/api/v1/<module>` sau khi ghép (không có `/` cuối).
            let tail = if path == "/" { "" } else { path.as_str() };
            let full = format!("{}/{module}{tail}", super::API_PREFIX);
            if declared.is_none() {
                // `HEAD` khai riêng mà không nêu permission: ghi lại để nó không mượn quy tắc của `GET`.
                if method == Method::HEAD {
                    operations.entry(full).or_default().push((method, None));
                }
                continue;
            }
            let buyers = operation
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.get("x-buyers"))
                .is_some_and(|value| value == true);
            let guard = Arc::new(Guard { permissions, buyers });
            operations.entry(full).or_default().push((method, Some(guard)));
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
    // `x-permissions: []` (`signed_in`): đăng nhập là đủ — kernel bảo đảm, kể cả khi handler không nhận `Principal`.
    if !guard.permissions.is_empty()
        && !(guard.buyers && principal.realm() == Realm::Customer)
        && let Err(error) = principal.require_any_named(&guard.permissions)
    {
        return error.into_response();
    }
    next.run(Request::from_parts(parts, body)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard(name: &str) -> Option<Arc<Guard>> {
        Some(Arc::new(Guard { permissions: vec![name.to_owned()], buyers: false }))
    }

    fn names(guard: Option<Arc<Guard>>) -> Option<Vec<String>> {
        guard.map(|guard| guard.permissions.clone())
    }

    #[test]
    fn head_theo_khai_bao_rieng_hay_muon_get() {
        let mut operations = HashMap::new();
        // Không khai `head`: `HEAD` mượn quy tắc của `GET`.
        operations.insert("/api/v1/m/items".to_owned(), vec![(Method::GET, guard("m.read"))]);
        // Khai `head` riêng có permission: theo nó.
        operations.insert(
            "/api/v1/m/files".to_owned(),
            vec![(Method::GET, guard("m.read")), (Method::HEAD, guard("m.peek"))],
        );
        // Khai `head` riêng KHÔNG có permission (công khai): không mượn `GET`.
        operations.insert(
            "/api/v1/m/public".to_owned(),
            vec![(Method::GET, guard("m.read")), (Method::HEAD, None)],
        );
        let rules = Rules { operations };
        let read = Some(vec!["m.read".to_owned()]);
        assert_eq!(names(rules.find(&Method::HEAD, "/api/v1/m/items")), read);
        assert_eq!(
            names(rules.find(&Method::HEAD, "/api/v1/m/files")),
            Some(vec!["m.peek".to_owned()])
        );
        assert_eq!(names(rules.find(&Method::HEAD, "/api/v1/m/public")), None);
        assert_eq!(names(rules.find(&Method::GET, "/api/v1/m/public")), read);
        assert_eq!(names(rules.find(&Method::POST, "/api/v1/m/items")), None);
        assert!(!rules.is_empty());
        let only_public =
            Rules { operations: HashMap::from([("/x".to_owned(), vec![(Method::HEAD, None)])]) };
        assert!(only_public.is_empty());
    }
}
