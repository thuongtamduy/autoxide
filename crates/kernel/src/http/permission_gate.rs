//! Lớp kiểm quyền chung (PT-14, D-20261126-permission-gate): trước mọi extractor của handler (thân JSON, `If-Match`,
//! tham số) và trước khi service tra bản ghi, người gọi phải có ít nhất một permission trong `x-permissions` của
//! operation — không thì 403 `permission_denied`. Người không có quyền không học được endpoint cần `ETag`, trường nào
//! sai hay bản ghi nào tồn tại.
//!
//! Nguồn duy nhất là tài liệu OpenAPI của module (như `approvals::rules`): `x-permissions: []` (chỉ cần đăng nhập)
//! hay operation công khai không bị lớp này chặn. Lớp chỉ kiểm "có permission ở bất kỳ nút nào"; phạm vi, bản ghi và
//! xác nhận lại (step-up) vẫn do service kiểm như trước. Không xác thực được thì để handler trả 401 như thường.

use std::sync::Arc;

use axum::extract::{FromRequestParts, MatchedPath, Request};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use utoipa::openapi::OpenApi;

use crate::access::Principal;

/// `(method, full path template)` → permission names (one of them is enough).
#[derive(Debug, Default)]
pub(super) struct Rules {
    operations: Vec<(Method, String, Vec<String>)>,
}

impl Rules {
    pub(super) fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    fn find(&self, method: &Method, path: &str) -> Option<&[String]> {
        self.operations
            .iter()
            .find(|(known, template, _)| known == method && template == path)
            .map(|(_, _, permissions)| permissions.as_slice())
    }
}

/// Operations of module `module` (full path `/api/v1/<module>…`) that list at least one permission.
pub(super) fn rules(module: &str, openapi: &OpenApi) -> Rules {
    let mut operations = Vec::new();
    for (path, item) in &openapi.paths.paths {
        let methods = [
            (Method::GET, item.get.as_ref()),
            (Method::POST, item.post.as_ref()),
            (Method::PUT, item.put.as_ref()),
            (Method::PATCH, item.patch.as_ref()),
            (Method::DELETE, item.delete.as_ref()),
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
            // Đường gốc của module là `/` trong tài liệu của nó, `/api/v1/<module>` sau khi ghép (không có `/` cuối).
            let tail = if path == "/" { "" } else { path.as_str() };
            operations.push((method, format!("{}/{module}{tail}", super::API_PREFIX), permissions));
        }
    }
    Rules { operations }
}

/// The layer (see the top of the file).
pub(super) async fn gate(rules: Arc<Rules>, request: Request, next: Next) -> Response {
    let Some(permissions) = request
        .extensions()
        .get::<MatchedPath>()
        .and_then(|matched| rules.find(request.method(), matched.as_str()))
        .map(<[String]>::to_vec)
    else {
        return next.run(request).await;
    };
    let (mut parts, body) = request.into_parts();
    // Xác thực một lần; kết quả được giữ trong `extensions`, extractor `Principal` của handler dùng lại.
    if let Ok(principal) = Principal::from_request_parts(&mut parts, &()).await
        && let Err(error) = principal.require_any_named(&permissions)
    {
        return error.into_response();
    }
    next.run(Request::from_parts(parts, body)).await
}
