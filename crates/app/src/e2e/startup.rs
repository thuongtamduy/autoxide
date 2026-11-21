//! `autoxide serve` tự chuẩn bị lúc khởi động (D-20261121-migrate-on-start): migrate, tạo quản trị
//! viên đầu tiên, chạy task nạp dữ liệu — trên database trống, hai bản khởi động cùng lúc, cả hai
//! database.

use autoxide_auth::api::TenantAdmins;
use autoxide_kernel::config::ConfigSource;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::TestDatabase;
use serde_json::json;

use crate::commands::serve::serve;
use crate::e2e::media::call;
use crate::runtime::{self, Migrations};
use crate::settings::Settings;

const EMAIL: &str = "first-admin@example.com";
const PASSWORD: &str = "correct horse battery staple";

fn settings(url: &str, extra: &[(&str, &str)]) -> Settings {
    settings_in(&scratch(), url, extra)
}

/// Thư mục tạm riêng cho một lần khởi động.
fn scratch() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("autoxide-startup-{}", uuid::Uuid::now_v7()))
}

/// Mặc định của `files`, `mail` và file mật khẩu tự sinh là đường dẫn tương đối trong thư mục chạy
/// test (thư mục của crate, trong cây mã nguồn): mọi thứ đặt vào `scratch`.
fn settings_in(scratch: &std::path::Path, url: &str, extra: &[(&str, &str)]) -> Settings {
    let files = scratch.join("files").display().to_string();
    let mail = scratch.join("mail").display().to_string();
    let password_out = scratch.join("initial-admin-password").display().to_string();
    let mut pairs = vec![
        ("AUTOXIDE_FILES_DIR", files.as_str()),
        ("AUTOXIDE_MAIL_FILE_DIR", mail.as_str()),
        ("AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD_OUT", password_out.as_str()),
        ("DATABASE_URL", url),
        ("AUTOXIDE_HTTP_ADDRESS", "127.0.0.1:0"),
        ("AUTOXIDE_HTTP_DRAIN_SECONDS", "0"),
        ("AUTOXIDE_MIGRATE_ON_START", "true"),
        ("AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL", EMAIL),
    ];
    pairs.extend_from_slice(extra);
    crate::settings::read(&ConfigSource::from_pairs(pairs)).unwrap()
}

/// Khởi động rồi dừng ngay: `serve` chỉ trả `Ok` khi phần chuẩn bị xong và máy chủ đã nghe.
async fn start_and_stop(settings: Settings) -> Result<(), String> {
    serve(settings, std::future::ready(())).await
}

#[tokio::test]
async fn hai_ban_khoi_dong_cung_luc_tren_database_trong() {
    TestDatabase::run(async |test| {
        let extra = [
            ("AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD", PASSWORD),
            ("AUTOXIDE_STARTUP_TASKS", "address.import"),
        ];
        // Hai bản cùng lúc: khoá migration cho một bản làm trước, bản kia thấy không còn gì.
        let (first, second) = tokio::join!(
            start_and_stop(settings(test.url(), &extra)),
            start_and_stop(settings(test.url(), &extra)),
        );
        assert_eq!((first, second), (Ok(()), Ok(())));

        // `RequireApplied`: còn migration chờ thì không khởi động được.
        let runtime =
            runtime::start(settings(test.url(), &[]), Migrations::RequireApplied).await.unwrap();
        let admins = runtime.modules.api::<dyn TenantAdmins>().unwrap();
        let tenant = TenantContext::default_on(runtime.database.clone());
        let found = admins.admins(&tenant).await.unwrap();
        assert_eq!(found.iter().map(|user| user.email.as_str()).collect::<Vec<_>>(), [EMAIL]);

        let login = json!({"email": EMAIL, "password": PASSWORD}).to_string();
        let (status, _, body) = call(
            &runtime,
            "POST",
            "/api/v1/auth/login",
            &[("content-type", "application/json")],
            login.into_bytes(),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert!(body["data"]["access_token"].is_string());
        runtime.database.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn da_co_quan_tri_vien_thi_khong_tao_them() {
    TestDatabase::run(async |test| {
        // Lần đầu: không có mật khẩu, máy chủ tự sinh và ghi vào file đã cấu hình (không vào log,
        // không vào cây mã nguồn — D-20261121-admin-password-file).
        let first = scratch();
        start_and_stop(settings_in(&first, test.url(), &[])).await.unwrap();
        let written = std::fs::read_to_string(first.join("initial-admin-password")).unwrap();
        assert!(written.starts_with(&format!("{EMAIL}\n")), "{written}");
        let in_tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        assert!(!in_tree.join("initial-admin-password").exists(), "written into the source tree");
        let _ = std::fs::remove_dir_all(&first);
        // Lần sau, email khác: đã có quản trị viên nên không tạo ai.
        let other = [("AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL", "someone-else@example.com")];
        start_and_stop(settings(test.url(), &other)).await.unwrap();

        let runtime =
            runtime::start(settings(test.url(), &[]), Migrations::RequireApplied).await.unwrap();
        let admins = runtime.modules.api::<dyn TenantAdmins>().unwrap();
        let tenant = TenantContext::default_on(runtime.database.clone());
        let found = admins.admins(&tenant).await.unwrap();
        assert_eq!(found.iter().map(|user| user.email.as_str()).collect::<Vec<_>>(), [EMAIL]);
        runtime.database.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn task_khong_co_thi_tu_choi_khoi_dong() {
    TestDatabase::run(async |test| {
        let extra = [("AUTOXIDE_STARTUP_TASKS", "address.no_such_task")];

        let error = start_and_stop(settings(test.url(), &extra)).await.unwrap_err();

        assert!(error.contains("there is no task `address.no_such_task`"), "{error}");
    })
    .await
    .unwrap();
}
