//! Diễn tập restore (bước 39, A2): dựng dữ liệu qua REST trên database gate đang chọn (SQLite hay
//! Postgres), `autoxide backup`, `autoxide restore` vào một database SQLite trống khác (chép chéo
//! khi gate chạy Postgres), rồi chương trình khởi động lại được: `/health/ready` 200, quản trị viên
//! cũ đăng nhập được bằng mật khẩu cũ, bài viết còn nguyên, nội dung ảnh khớp từng byte.
//! Chiều ngược lại (SQLite → database của gate) có ở `commands::backup`.

use autoxide_kernel::config::ConfigSource;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::json;

use super::media::{admin_token, call, png, raw, start};
use crate::commands::backup::{run_backup, run_restore};
use crate::runtime::{self, Migrations};

const PASSWORD: &str = "correct horse battery staple";

fn settings(database_url: &str, files: &std::path::Path) -> crate::settings::Settings {
    let files = files.display().to_string();
    let source = ConfigSource::from_pairs([
        ("DATABASE_URL", database_url),
        ("AUTOXIDE_FILES_DIR", files.as_str()),
    ]);
    crate::settings::read(&source).unwrap()
}

#[tokio::test]
async fn dien_tap_backup_restore_roi_khoi_dong() {
    TestDatabase::run(async |test| {
        let scratch = std::env::temp_dir().join(format!("autoxide-drill-{}", uuid::Uuid::now_v7()));
        let files_a = scratch.join("files-a");

        // 1. Dữ liệu thật qua REST: quản trị viên, một bài viết, một ảnh.
        let runtime =
            start(test.url(), &[("AUTOXIDE_FILES_DIR", files_a.display().to_string())]).await;
        let token = admin_token(&runtime, "drill@example.com").await;
        let auth = format!("Bearer {token}");
        let json_headers = [("authorization", auth.as_str()), ("content-type", "application/json")];
        let entry = json!({"kind": "post", "title": "Diễn tập restore"}).to_string().into_bytes();
        let (status, _, created) =
            call(&runtime, "POST", "/api/v1/content/entries", &json_headers, entry).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let entry_id = created["data"]["id"].as_str().unwrap().to_owned();
        let image = png(9, 4);
        let image_headers = [("authorization", auth.as_str()), ("content-type", "image/png")];
        let (status, _, uploaded) =
            call(&runtime, "POST", "/api/v1/media?name=drill.png", &image_headers, image.clone())
                .await;
        assert_eq!(status, StatusCode::CREATED, "{uploaded}");
        let media_id = uploaded["data"]["id"].as_str().unwrap().to_owned();
        runtime.database.close().await.unwrap();

        // 2. Backup như người vận hành chạy `autoxide backup <thư mục>`.
        let backup = scratch.join("backup");
        let report = run_backup(settings(test.url(), &files_a), backup.clone()).await.unwrap();
        assert!(report.starts_with("Backed up "), "{report}");

        // 3. Restore vào một database trống và một nơi lưu file trống.
        let restored = format!("sqlite://{}?mode=rwc", scratch.join("restored.sqlite").display());
        let files_b = scratch.join("files-b");
        run_restore(settings(&restored, &files_b), backup).await.unwrap();

        // 4. Chương trình khởi động trên bản đã restore và dùng được như trước.
        let runtime = runtime::start(settings(&restored, &files_b), Migrations::RequireApplied)
            .await
            .unwrap();
        let (status, _, ready) = call(&runtime, "GET", "/health/ready", &[], Vec::new()).await;
        assert_eq!(status, StatusCode::OK, "{ready}");
        let login = json!({"email": "drill@example.com", "password": PASSWORD}).to_string();
        let (status, _, signed_in) = call(
            &runtime,
            "POST",
            "/api/v1/auth/login",
            &[("content-type", "application/json")],
            login.into_bytes(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{signed_in}");
        let token = signed_in["data"]["access_token"].as_str().unwrap();
        let auth = format!("Bearer {token}");
        let headers = [("authorization", auth.as_str())];
        let (status, _, entry) = call(
            &runtime,
            "GET",
            &format!("/api/v1/content/entries/{entry_id}"),
            &headers,
            Vec::new(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{entry}");
        assert_eq!(entry["data"]["title"], "Diễn tập restore");
        let (status, _, bytes) = raw(
            &runtime,
            "GET",
            &format!("/api/v1/media/{media_id}/content"),
            &headers,
            Vec::new(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bytes, image, "the image comes back byte for byte");
        runtime.database.close().await.unwrap();
        let _ = std::fs::remove_dir_all(&scratch);
    })
    .await
    .unwrap();
}
