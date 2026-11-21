//! Kết nối đối tác do khách tự đặt (D-20261121-ssrf-targets), cả SQLite và Postgres: máy chủ SMTP
//! riêng của khách phải là `smtp://` / `smtps://` trên Internet công cộng — địa chỉ mạng riêng hay
//! máy này bị từ chối ngay lúc lưu (người vận hành chưa bật `AUTOXIDE_MAIL_ALLOW_PRIVATE_NETWORKS`).

use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

async fn put(app: &TestApp, value: &str) -> (StatusCode, Value) {
    let answer = app
        .request(Method::PUT, "/api/v1/kernel/settings/mail.connection.smtp.url")
        .token("owner")
        .json(json!({"value": value}))
        .send()
        .await
        .unwrap();
    (answer.status(), answer.json())
}

#[tokio::test]
async fn smtp_rieng_cua_khach_khong_tro_vao_mang_noi_bo() {
    TestDatabase::run(async |test| {
        let dir = std::env::temp_dir().join(format!("autoxide-conn-{}", uuid::Uuid::now_v7()));
        let app = TestApp::builder(test)
            .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
            .tenant("shop")
            .user_in("shop", "owner", 1, &["kernel.settings.manage"])
            .module(autoxide_mail::module())
            .build()
            .await
            .unwrap();
        for raw in [
            "smtp://127.0.0.1:6379",
            "smtp://localhost:25",
            "smtps://[::1]:465",
            "smtp://169.254.169.254:80",
            "smtp://10.0.0.8:2525",
        ] {
            let (status, body) = put(&app, raw).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{raw}: {body}");
            assert_eq!(body["errors"][0]["code"], "private_address", "{raw}: {body}");
        }
        for raw in ["http://smtp.example.com", "redis://smtp.example.com:6379"] {
            let (status, body) = put(&app, raw).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{raw}: {body}");
            assert_eq!(body["errors"][0]["code"], "invalid", "{raw}: {body}");
        }
        let (status, body) = put(&app, "smtps://user:pass@smtp.example.com:465").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let _ = std::fs::remove_dir_all(&dir);
    })
    .await
    .unwrap();
}
