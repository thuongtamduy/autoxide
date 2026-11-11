//! Module `auth`: tài khoản người dùng và mật khẩu.
//!
//! Đây là module của core, không phải module nghiệp vụ: mọi bản cài Autoxide đều
//! cần nó. Ở bước này nó có bảng người dùng và việc hash mật khẩu bằng Argon2id;
//! các mục sau của lộ trình bước 2 thêm đăng nhập, role, permission và API key vào
//! chính module này (docs/DECISIONS.md D-044).
//!
//! Module khác KHÔNG đọc bảng `auth_…`; chúng lấy người dùng qua trait
//! [`api::Users`] mà module này công bố lúc khởi động.

pub mod api;
mod migration;
mod password;
mod user;

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};

use crate::password::Passwords;
use crate::user::UserService;

/// Module `auth`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    AuthModule
}

struct AuthModule;

impl Module for AuthModule {
    fn name(&self) -> &'static str {
        "auth"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let users = Arc::new(UserService::new(context.database().clone(), Passwords::new()));
        // `Arc<UserService>` tự đổi thành `Arc<dyn Users>`: module khác chỉ thấy trait.
        context.provide::<dyn api::Users>(users)
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! Đồ dùng chung cho test trong crate này.

    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::database::Database;
    use autoxide_kernel::migration::{Migrator, ModuleMigrations};
    use autoxide_testkit::TestDatabase;

    /// Kết nối tới database trống của test và áp dụng mọi migration của module.
    pub async fn database(test: &TestDatabase) -> Database {
        let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
        let mut reader = ConfigReader::new(&source);
        let config = Config::read(&mut reader);
        let (config, _) = reader.finish(config).unwrap();
        let database = Database::connect(&config.database).await.unwrap();
        Migrator::new(vec![ModuleMigrations::new("auth", super::migration::all())])
            .unwrap()
            .up(&database)
            .await
            .unwrap();
        database
    }
}
