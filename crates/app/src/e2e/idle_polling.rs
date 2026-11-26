//! Việc nền khi rảnh (D-20261126-idle-polling): mỗi giây bộ phát event và worker job hỏi mọi bảng `<module>_events` /
//! `<module>_jobs`. Một câu `SELECT EXISTS(…), EXISTS(…)…` hỏi trước bảng nào có việc, nên một lượt rảnh chỉ còn một câu
//! mỗi database (trước đây một câu mỗi module có bảng — hàng chục câu mỗi giây khi máy không làm gì).

use autoxide_testkit::{TestApp, TestDatabase};

#[tokio::test]
async fn luot_ranh_mot_cau_moi_database() {
    TestDatabase::run(async |test| {
        let dir = std::env::temp_dir().join(format!("autoxide-idle-{}", uuid::Uuid::now_v7()));
        let mut builder = TestApp::builder(test)
            .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
            .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string());
        for module in crate::modules::all() {
            builder = builder.boxed_module(module);
        }
        let app = builder.build().await.unwrap();
        let dispatcher = app.modules().dispatcher().unwrap();
        // Lượt đầu còn dọn event cũ (mỗi giờ một lần) và giao event lúc khởi động; lượt sau là lượt rảnh.
        while dispatcher.run_once().await.unwrap() > 0 {}
        let (handled, statements) = app.statements(dispatcher.run_once()).await;
        assert_eq!(handled.unwrap(), 0);
        assert_eq!(statements.len(), 1, "{statements:#?}");
    })
    .await
    .unwrap();
}
