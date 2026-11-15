//! Extractor `Principal` qua router thật: bộ xác thực được gắn vào request, handler
//! kiểm permission, thiếu bộ xác thực thì báo rõ.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use autoxide_kernel::access::{Authenticator, Permission, Principal};
use autoxide_kernel::config::{ConfigReader, ConfigSource, HttpConfig};
use autoxide_kernel::error::Error;
use autoxide_kernel::http;
use autoxide_kernel::response::Data;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

const NOTES_WRITE: Permission = Permission::new("notes.write", "Write notes.");

/// Bộ xác thực giả: token `writer` có `notes.write`, `reader` không có gì.
struct FakeAuthenticator;

#[async_trait::async_trait]
impl Authenticator for FakeAuthenticator {
    async fn authenticate(
        &self,
        authorization: Option<&str>,
        _: &autoxide_kernel::tenancy::TenantHint,
    ) -> Result<Principal, Error> {
        match authorization {
            Some("Bearer writer") => Ok(Principal::new(Uuid::nil(), ["notes.write".to_owned()])),
            Some("Bearer reader") => Ok(Principal::new(Uuid::nil(), [])),
            _ => Err(Error::unauthenticated("auth.missing_token", "Sign in.")),
        }
    }
}

/// Write a note.
#[utoipa::path(post, path = "/", responses((status = 200, description = "Written.")))]
async fn write(principal: Principal) -> Result<Data<bool>, Error> {
    principal.require(&NOTES_WRITE)?;
    Ok(Data { data: true })
}

fn app(authenticator: Option<Arc<dyn Authenticator>>) -> Router {
    let source = ConfigSource::default();
    let mut reader = ConfigReader::new(&source);
    let config = HttpConfig::read(&mut reader);
    let config = reader.finish(config).unwrap().0;
    let routes = OpenApiRouter::new().routes(routes!(write));
    http::build(
        &config,
        OpenApiRouter::new(),
        vec![("notes", routes)],
        http::Extras::default().authenticator(authenticator),
    )
    .router
}

async fn call(app: Router, authorization: Option<&str>) -> (StatusCode, serde_json::Value) {
    let mut request = Request::builder().method("POST").uri("/api/v1/notes");
    if let Some(value) = authorization {
        request = request.header(header::AUTHORIZATION, value);
    }
    let response = app.oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn handler_kiem_permission_cua_ben_goi() {
    let authenticator: Arc<dyn Authenticator> = Arc::new(FakeAuthenticator);
    let app = app(Some(authenticator));

    let (status, body) = call(app.clone(), Some("Bearer writer")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = call(app.clone(), Some("Bearer reader")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "permission_denied");

    let (status, body) = call(app, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "auth.missing_token");
}

#[tokio::test]
async fn thieu_bo_xac_thuc_la_loi_noi_bo() {
    let (status, body) = call(app(None), Some("Bearer writer")).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["code"], "internal_error");
}
