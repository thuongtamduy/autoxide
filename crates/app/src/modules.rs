//! Nơi DUY NHẤT đăng ký module vào chương trình.
//!
//! Thêm một module = thêm một dòng vào [`all`] (và một dòng phụ thuộc trong
//! `Cargo.toml` của crate này). Không chỗ nào khác trong core phải sửa.

use autoxide_kernel::module::Module;

/// Mọi module của chương trình, theo thứ tự đăng ký.
pub fn all() -> Vec<Box<dyn Module>> {
    // Chưa có module nào. Khi có, mỗi module một dòng:
    //     Box::new(autoxide_<tên>::module()),
    Vec::new()
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
}
