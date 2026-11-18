//! Backup theo lịch (bước 39, A1) trên database gate đang chọn: ghi vào nơi lưu file, giữ đúng N
//! bản, hai worker cùng lúc chỉ ra một bản, lượt chết bị đánh hỏng, `autoxide doctor` cảnh báo.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use autoxide_kernel::backup::scheduled::{BackupOutcome, ScheduledBackups, latest_successes};
use autoxide_kernel::backup::{BACKUP_PREFIX, MANIFEST};
use autoxide_kernel::doctor::Status;
use autoxide_kernel::storage::Storage;
use autoxide_testkit::{MemoryStorage, TestApp, TestDatabase};
use sea_orm::ConnectionTrait as _;
use sea_orm::Statement;
use sea_orm::sea_query::{Alias, Query};

async fn app(test: &TestDatabase, settings: &[(&str, &str)]) -> (TestApp, Arc<MemoryStorage>) {
    let storage = Arc::new(MemoryStorage::new());
    let shared: Arc<dyn Storage> = storage.clone();
    let mut builder = TestApp::builder(test).provide::<dyn Storage>(shared);
    for (name, value) in settings {
        builder = builder.setting(name, value);
    }
    (builder.build().await.unwrap(), storage)
}

async fn rows(app: &TestApp) -> Vec<(String, String)> {
    let backend = app.database().write().get_database_backend();
    let sql = "SELECT status, storage_prefix FROM kernel_backups ORDER BY started_at, id";
    app.database()
        .read()
        .query_all_raw(Statement::from_string(backend, sql))
        .await
        .unwrap()
        .iter()
        .map(|row| (row.try_get("", "status").unwrap(), row.try_get("", "storage_prefix").unwrap()))
        .collect()
}

async fn doctor_status(app: &TestApp) -> (Status, String) {
    let check = app
        .modules()
        .checks()
        .into_iter()
        .find(|(module, check)| *module == "kernel" && check.name == "backup_age")
        .unwrap()
        .1;
    let finding = check.run().await;
    (finding.status, finding.message)
}

#[tokio::test]
async fn backup_theo_lich_ghi_vao_storage_va_giu_n_ban() {
    TestDatabase::run(async |test| {
        let (app, storage) = app(test, &[("AUTOXIDE_BACKUP_KEEP", "2")]).await;
        // Chưa có bản nào: doctor cảnh báo.
        let (status, message) = doctor_status(&app).await;
        assert_eq!(status, Status::Warning, "{message}");
        assert!(message.contains("main (none yet)"), "{message}");

        let backups = app.api::<ScheduledBackups>().unwrap();
        let mut ids = Vec::new();
        for _ in 0..3 {
            let outcomes = backups.run().await.unwrap();
            assert_eq!(outcomes.len(), 1);
            let (name, outcome) = &outcomes[0];
            assert_eq!(name, "main");
            let BackupOutcome::Succeeded { id, size_bytes } = outcome else {
                panic!("{outcome:?}")
            };
            assert!(*size_bytes > 0);
            ids.push(*id);
        }
        // Còn đúng hai bản mới nhất, cả dòng lẫn file.
        let kept = rows(&app).await;
        assert_eq!(kept.len(), 2, "{kept:?}");
        assert!(kept.iter().all(|(status, _)| status == "succeeded"));
        let keys = storage.keys().await.unwrap();
        for (index, id) in ids.iter().enumerate() {
            let manifest = format!("{BACKUP_PREFIX}main/{id}/{MANIFEST}");
            assert_eq!(keys.contains(&manifest), index > 0, "{manifest} in {keys:?}");
        }
        // Bản backup không chép file của bản trước (mặc định không chép file nào).
        assert!(keys.iter().all(|key| !key.contains("/files/")), "{keys:?}");

        let latest = latest_successes(app.database()).await.unwrap();
        assert_eq!(latest.len(), 1);
        let (status, message) = doctor_status(&app).await;
        assert_eq!(status, Status::Ok, "{message}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_lan_chay_cung_luc_chi_mot_ban_backup() {
    TestDatabase::run(async |test| {
        let (app, _storage) = app(test, &[]).await;
        let backups = app.api::<ScheduledBackups>().unwrap();
        let (first, second) = tokio::join!(backups.run(), backups.run());
        let outcomes: Vec<BackupOutcome> =
            first.unwrap().into_iter().chain(second.unwrap()).map(|(_, outcome)| outcome).collect();
        let succeeded =
            outcomes.iter().filter(|o| matches!(o, BackupOutcome::Succeeded { .. })).count();
        assert!(succeeded >= 1, "{outcomes:?}");
        // Hai lượt chồng nhau: lượt sau nhận "đang chạy"; hai lượt nối nhau: cả hai xong. Không bao
        // giờ có hai dòng `running` cùng lúc (unique `active_key`), và không lượt nào hỏng.
        assert!(
            outcomes.iter().all(|o| !matches!(o, BackupOutcome::Failed { .. })),
            "{outcomes:?}"
        );
        assert!(rows(&app).await.iter().all(|(status, _)| status == "succeeded"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn backup_dang_running_ma_tien_trinh_chet_thi_danh_failed() {
    TestDatabase::run(async |test| {
        let (app, _storage) = app(test, &[]).await;
        let backups = app.api::<ScheduledBackups>().unwrap();
        backups.run().await.unwrap();
        // Giả một lượt đang chạy của tiến trình đã chết từ lâu: nhịp cũ, vẫn giữ `active_key`.
        let backend = app.database().write().get_database_backend();
        let sql = "UPDATE kernel_backups SET status = 'running', active_key = 'main', \
                   heartbeat_at = started_at";
        app.database().write().execute_raw(Statement::from_string(backend, sql)).await.unwrap();
        // Nhịp cuối cách đây một giờ (quá 15 phút): lượt kia đã chết, lượt mới chạy được ngay.
        let old = Query::update()
            .table(Alias::new("kernel_backups"))
            .value(Alias::new("heartbeat_at"), chrono::Utc::now() - chrono::Duration::hours(1))
            .to_owned();
        app.database().write().execute(&old).await.unwrap();

        let outcomes = backups.run().await.unwrap();
        assert!(matches!(outcomes[0].1, BackupOutcome::Succeeded { .. }), "{outcomes:?}");
        let statuses: Vec<String> =
            rows(&app).await.into_iter().map(|(status, _)| status).collect();
        assert_eq!(statuses, ["failed", "succeeded"]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doctor_bao_khong_co_backup_tu_dong() {
    TestDatabase::run(async |test| {
        let (app, _storage) = app(test, &[("AUTOXIDE_BACKUP_INTERVAL_HOURS", "0")]).await;
        let (status, message) = doctor_status(&app).await;
        assert_eq!(status, Status::Warning);
        assert!(message.contains("AUTOXIDE_BACKUP_INTERVAL_HOURS=0"), "{message}");
    })
    .await
    .unwrap();
}
