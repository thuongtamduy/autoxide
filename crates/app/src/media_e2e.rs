//! Test đầu-cuối của module `media` trên chương trình thật: nơi lưu thật (ổ đĩa, và S3
//! khi `TEST_S3_ENDPOINT` được đặt — gate luôn đặt), database gate đang chọn, đăng nhập
//! thật, idempotency, audit log.

use autoxide_auth::api::{Accounts, NewAccount};
use autoxide_kernel::config::ConfigSource;
use autoxide_kernel::database::Database;
use autoxide_kernel::storage::Storage;
use autoxide_testkit::TestDatabase;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};
use tower::ServiceExt as _;

use crate::runtime::{self, Migrations, Runtime};

/// PNG nhỏ nhất mà đúng định dạng, `width` × `height`.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
    bytes.extend(width.to_be_bytes());
    bytes.extend(height.to_be_bytes());
    bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
    bytes.extend(b"\x00\x00\x00\x00IEND\xaeB`\x82");
    bytes
}

/// Các bộ cấu hình nơi lưu cần thử: một thư mục tạm, và S3 nếu có.
fn backends() -> Vec<Vec<(&'static str, String)>> {
    let dir = std::env::temp_dir().join(format!("autoxide-media-{}", uuid::Uuid::now_v7()));
    let mut all = vec![vec![("AUTOXIDE_FILES_DIR", dir.display().to_string())]];
    all.extend(autoxide_testkit::s3_settings());
    all
}

pub(crate) async fn start(database_url: &str, storage: &[(&'static str, String)]) -> Runtime {
    let mut pairs = vec![("DATABASE_URL", database_url.to_owned())];
    pairs.extend(storage.iter().cloned());
    let source =
        ConfigSource::from_pairs(pairs.iter().map(|(name, value)| (*name, value.as_str())));
    let settings = crate::settings::read(&source).unwrap();
    let database = Database::connect(&settings.config.database).await.unwrap();
    settings.modules.migrator().unwrap().up(&database).await.unwrap();
    database.close().await.unwrap();
    runtime::start(settings, Migrations::RequireApplied).await.unwrap()
}

/// Quản trị viên mới và access token của họ.
pub(crate) async fn admin_token(runtime: &Runtime, email: &str) -> String {
    let accounts = runtime.modules.api::<dyn Accounts>().unwrap();
    let password = "correct horse battery staple";
    let account = NewAccount {
        email: email.to_owned(),
        name: "Admin".to_owned(),
        password: Some(password.to_owned()),
        admin: true,
    };
    accounts.create(account).await.unwrap();
    let login = json!({"email": email, "password": password}).to_string();
    let (_, _, body) = call(
        runtime,
        "POST",
        "/api/v1/auth/login",
        &[("content-type", "application/json")],
        login.into_bytes(),
    )
    .await;
    body["data"]["access_token"].as_str().unwrap().to_owned()
}

pub(crate) async fn call(
    runtime: &Runtime,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Vec<u8>,
) -> (StatusCode, HeaderMap, Value) {
    let (status, headers, bytes) = raw(runtime, method, uri, headers, body).await;
    (status, headers, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn raw(
    runtime: &Runtime,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Vec<u8>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut request = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response =
        runtime.api.router.clone().oneshot(request.body(Body::from(body)).unwrap()).await.unwrap();
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap().to_vec();
    (parts.status, parts.headers, bytes)
}

/// Trên ổ đĩa và S3: tải lên (gửi lại cùng `Idempotency-Key` không tạo hai item), tải về
/// cả file và một khoảng, thay nội dung (nội dung cũ bị xoá khỏi nơi lưu), event vào audit
/// log với đúng người làm.
#[tokio::test]
async fn media_tren_o_dia_va_s3() {
    TestDatabase::run(async |test| {
        for storage in backends() {
            let backend = if storage.len() == 1 { "local" } else { "s3" };
            let runtime = start(test.url(), &storage).await;
            let token = admin_token(&runtime, &format!("{backend}@example.com")).await;
            let auth = format!("Bearer {token}");
            let content = png(30, 20);
            let idempotency_key = format!("upload-once-{backend}");

            let headers = [
                ("authorization", auth.as_str()),
                ("content-type", "image/png"),
                ("idempotency-key", idempotency_key.as_str()),
            ];
            let (status, _, first) =
                call(&runtime, "POST", "/api/v1/media?name=a.png", &headers, content.clone()).await;
            assert_eq!(status, StatusCode::CREATED, "{backend}: {first}");
            let (status, replay, second) =
                call(&runtime, "POST", "/api/v1/media?name=a.png", &headers, content.clone()).await;
            assert_eq!(status, StatusCode::CREATED);
            assert_eq!(replay["idempotent-replayed"], "true");
            assert_eq!(first["data"]["id"], second["data"]["id"]);
            // Thân lớn hơn giới hạn mặc định (1 MiB): lớp idempotency không đọc trước vào bộ
            // nhớ mà vẫn nhận ra lần gửi lại (D-149).
            let mut large = png(30, 20);
            large.resize(1536 * 1024, 0);
            let large_key = format!("upload-large-{backend}");
            let large_headers = [
                ("authorization", auth.as_str()),
                ("content-type", "image/png"),
                ("idempotency-key", large_key.as_str()),
            ];
            let (status, _, big) =
                call(&runtime, "POST", "/api/v1/media?name=big.png", &large_headers, large.clone()).await;
            assert_eq!(status, StatusCode::CREATED, "{backend}: {big}");
            let (status, replay, again) =
                call(&runtime, "POST", "/api/v1/media?name=big.png", &large_headers, large).await;
            assert_eq!(status, StatusCode::CREATED, "{backend}: {again}");
            assert_eq!(replay["idempotent-replayed"], "true");
            assert_eq!(big["data"]["id"], again["data"]["id"]);
            let big_id = big["data"]["id"].as_str().unwrap().to_owned();
            let (_, _, list) = call(&runtime, "GET", "/api/v1/media", &[("authorization", auth.as_str())], Vec::new()).await;
            assert_eq!(list["data"].as_array().unwrap().len(), 2, "{backend}");
            let big_path = format!("/api/v1/media/{big_id}");
            call(&runtime, "DELETE", &big_path, &[("authorization", auth.as_str())], Vec::new()).await;
            let (status, _, _) = call(&runtime, "POST", &format!("{big_path}/purge"), &[("authorization", auth.as_str())], Vec::new()).await;
            assert_eq!(status, StatusCode::NO_CONTENT);

            let item = &first["data"];
            let id = item["id"].as_str().unwrap();
            let url = item["url"].as_str().unwrap();
            let (status, _, bytes) = raw(&runtime, "GET", url, &[("authorization", auth.as_str())], Vec::new()).await;
            assert_eq!((status, bytes), (StatusCode::OK, content.clone()), "{backend}");
            let range = [("authorization", auth.as_str()), ("range", "bytes=8-15")];
            let (status, _, bytes) = raw(&runtime, "GET", url, &range, Vec::new()).await;
            assert_eq!((status, bytes.as_slice()), (StatusCode::PARTIAL_CONTENT, &content[8..16]), "{backend}");

            let (_, got, _) = call(&runtime, "GET", &format!("/api/v1/media/{id}"), &[("authorization", auth.as_str())], Vec::new()).await;
            let etag = got[header::ETAG].to_str().unwrap().to_owned();
            let replacement = png(60, 40);
            let headers =
                [("authorization", auth.as_str()), ("content-type", "image/png"), ("if-match", etag.as_str())];
            let path = format!("/api/v1/media/{id}/content");
            let (status, _, replaced) = call(&runtime, "PUT", &path, &headers, replacement.clone()).await;
            assert_eq!(status, StatusCode::OK, "{backend}: {replaced}");
            let files = runtime.modules.api::<dyn Storage>().unwrap();
            let version = replaced["data"]["version_id"].as_str().unwrap();
            assert_eq!(files.keys_with_prefix("media/").await.unwrap(), [format!("media/{id}/{version}")], "{backend}");
            let (_, _, bytes) = raw(&runtime, "GET", &path, &[("authorization", auth.as_str())], Vec::new()).await;
            assert_eq!(bytes, replacement, "{backend}");

            while runtime.dispatcher.run_once().await.unwrap() > 0 {}
            let backend_kind = runtime.database.read().get_database_backend();
            let sql = format!(
                "SELECT action, actor_user_id FROM audit_entry WHERE subject = 'media.item:{id}' ORDER BY id"
            );
            let rows = runtime.database.read().query_all_raw(Statement::from_string(backend_kind, sql)).await.unwrap();
            let actions: Vec<String> = rows.iter().map(|row| row.try_get("", "action").unwrap()).collect();
            assert_eq!(actions, ["media.item_uploaded", "media.item_content_replaced"], "{backend}");
            let actor: Option<uuid::Uuid> = rows[0].try_get("", "actor_user_id").unwrap();
            assert!(actor.is_some());

            // Dọn để lượt nơi lưu sau bắt đầu từ thư viện trống.
            let trash = format!("/api/v1/media/{id}");
            call(&runtime, "DELETE", &trash, &[("authorization", auth.as_str())], Vec::new()).await;
            let (status, _, _) = call(&runtime, "POST", &format!("{trash}/purge"), &[("authorization", auth.as_str())], Vec::new()).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            assert!(files.keys_with_prefix("media/").await.unwrap().is_empty(), "{backend}");
            runtime.database.close().await.unwrap();
        }
    })
    .await
    .unwrap();
}
