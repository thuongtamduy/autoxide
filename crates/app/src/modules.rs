//! Nơi DUY NHẤT đăng ký module vào chương trình.
//!
//! Thêm một module = thêm một dòng vào [`all`] (và một dòng phụ thuộc trong
//! `Cargo.toml` của crate này). Không chỗ nào khác trong core phải sửa.

use autoxide_kernel::module::Module;

/// Mọi module của chương trình, theo thứ tự đăng ký.
pub fn all() -> Vec<Box<dyn Module>> {
    // Mỗi module một dòng. Module cần trait của module khác đăng ký SAU module đó.
    vec![
        // Gửi mail: các module gửi mail cần nó, nên đứng trước chúng.
        Box::new(autoxide_mail::module()),
        // Người dùng, mật khẩu (và sau này đăng nhập, role, permission): mọi module
        // khác có thể cần, nên đứng đầu.
        Box::new(autoxide_auth::module()),
        // Audit log: nhận event của mọi module; không module nào cần trait của nó.
        Box::new(autoxide_audit::module()),
        // Chỗ lưu idempotency key cho lớp HTTP của kernel.
        Box::new(autoxide_idempotency::module()),
    ]
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::config::ConfigSource;
    use autoxide_kernel::database::Database;
    use autoxide_testkit::{TestDatabase, schema_ownership_violations};

    /// Chạy mọi migration của chương trình trên một database trống rồi hỏi chính
    /// database: mỗi bảng có thuộc đúng một module không, có foreign key nào xuyên
    /// module không. Gate chạy bài này trên cả SQLite và Postgres.
    #[tokio::test]
    async fn moi_bang_thuoc_mot_module_va_khong_foreign_key_xuyen_module() {
        TestDatabase::run(async |database| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
            let settings = crate::settings::read(&source).unwrap();
            let db = Database::connect(&settings.config.database).await.unwrap();
            let names = settings.modules.names();
            let migrator = settings.modules.migrator().unwrap();

            migrator.up(&db).await.unwrap();

            let violations = schema_ownership_violations(db.write(), &names).await.unwrap();
            assert_eq!(violations, Vec::<String>::new());

            // Migration nào cũng phải gỡ được: xuống hết rồi lên lại.
            for name in names.iter().rev() {
                migrator.down(&db, name, u32::MAX).await.unwrap();
            }
            migrator.up(&db).await.unwrap();
            assert!(migrator.pending(&db).await.unwrap().is_empty());
            db.close().await.unwrap();
        })
        .await
        .unwrap();
    }

    /// Xuyên module: `auth` ghi event cùng transaction, bộ phát giao, `audit` ghi
    /// đúng một dòng — trên database gate đang chọn.
    #[tokio::test]
    async fn thay_doi_cua_auth_vao_audit_log() {
        use autoxide_auth::api::{Accounts, NewAccount};
        use sea_orm::{ConnectionTrait, Statement};

        TestDatabase::run(async |database| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
            let mut settings = crate::settings::read(&source).unwrap();
            let db = Database::connect(&settings.config.database).await.unwrap();
            settings.modules.migrator().unwrap().up(&db).await.unwrap();
            settings.modules.start(&db).unwrap();
            let accounts = settings.modules.api::<dyn Accounts>().unwrap();
            let dispatcher = settings.modules.dispatcher(&db).unwrap();

            let account = NewAccount {
                email: "ada@example.com".to_owned(),
                name: "Ada".to_owned(),
                password: None,
                admin: true,
            };
            let ada = accounts.create(account).await.unwrap();
            assert_eq!(dispatcher.run_once().await.unwrap(), 1);
            assert_eq!(dispatcher.run_once().await.unwrap(), 0);

            let backend = db.read().get_database_backend();
            let sql = "SELECT action, subject, actor_user_id, after FROM audit_entry";
            let rows = db.read().query_all_raw(Statement::from_string(backend, sql)).await.unwrap();
            assert_eq!(rows.len(), 1);
            let action: String = rows[0].try_get("", "action").unwrap();
            let subject: Option<String> = rows[0].try_get("", "subject").unwrap();
            let actor: Option<uuid::Uuid> = rows[0].try_get("", "actor_user_id").unwrap();
            let after: Option<String> = rows[0].try_get("", "after").unwrap();
            assert_eq!(action, "auth.user_created");
            assert_eq!(subject, Some(format!("auth.user:{}", ada.id)));
            assert_eq!(actor, None, "created from the command line, by the system");
            assert!(after.unwrap().contains(r#""admin":true"#));
            db.close().await.unwrap();
        })
        .await
        .unwrap();
    }

    /// Lịch định kỳ của `auth` được worker đưa vào hàng đợi và chạy — đúng một lần dù
    /// hai worker cùng chạy.
    #[tokio::test]
    async fn worker_chay_lich_dinh_ky_cua_module() {
        TestDatabase::run(async |database| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
            let mut settings = crate::settings::read(&source).unwrap();
            let db = Database::connect(&settings.config.database).await.unwrap();
            settings.modules.migrator().unwrap().up(&db).await.unwrap();
            settings.modules.start(&db).unwrap();
            let first = settings.modules.worker(&db, 4).unwrap();
            let second = settings.modules.worker(&db, 4).unwrap();

            let (a, b) = tokio::join!(first.run_once(), second.run_once());

            // Mỗi lịch (của `auth` và của `idempotency`) đúng một job.
            assert_eq!(a.unwrap() + b.unwrap(), 2);
            let done =
                first.list("auth", autoxide_kernel::job::JobStatus::Completed, 10).await.unwrap();
            assert_eq!(done.len(), 1);
            assert_eq!(done[0].kind, "auth.cleanup");
            db.close().await.unwrap();
        })
        .await
        .unwrap();
    }
}
