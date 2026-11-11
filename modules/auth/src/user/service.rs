//! Service của tài khoản người dùng: mọi logic về người dùng nằm ở đây. Handler REST,
//! lệnh CLI và tool MCP (ở các mục sau) chỉ gọi vào đây.

use autoxide_kernel::database::Database;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, SqlErr};
use uuid::Uuid;
use validator::ValidateEmail;

use super::entity::{self, Column, Entity};
use crate::api::{User, Users};
use crate::password::{self, Passwords, Verification};

/// Độ dài tối đa của email (RFC 5321) và của tên hiển thị, theo ký tự.
const EMAIL_MAX_LENGTH: usize = 254;
const NAME_MAX_LENGTH: usize = 200;

/// Dữ liệu để tạo một người dùng.
#[derive(Debug, Clone)]
pub(crate) struct NewUser {
    pub email: String,
    pub name: String,
    /// `None`: tài khoản chưa có mật khẩu (đặt sau, vd qua đường dẫn đặt lại mật khẩu).
    pub password: Option<String>,
}

pub(crate) struct UserService {
    database: Database,
    passwords: Passwords,
}

impl UserService {
    pub(crate) fn new(database: Database, passwords: Passwords) -> Self {
        UserService { database, passwords }
    }

    /// Người dùng có ID `id`.
    pub(crate) async fn get(&self, id: Uuid) -> Result<Option<User>, Error> {
        let row = Entity::find_by_id(id).one(self.database.read()).await?;
        Ok(row.map(User::from))
    }
}

// Các hàm dưới đây được đăng nhập, lệnh CLI tạo người dùng và API quản trị dùng — đều
// là các mục kế tiếp của lộ trình bước 2. Tới lúc đó chỉ test gọi chúng.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by login and user management, next items of roadmap step 2")
)]
impl UserService {
    /// Tạo người dùng. Email được chuẩn hoá (bỏ khoảng trắng hai đầu, chữ thường)
    /// và phải chưa ai dùng.
    pub(crate) async fn create(&self, input: NewUser) -> Result<User, Error> {
        let email = normalize_email(&input.email);
        let name = input.name.trim().to_owned();
        let mut problems = Vec::new();
        problems.extend(check_email(&email));
        problems.extend(check_name(&name));
        if let Some(password) = &input.password {
            problems.extend(password::check_rules("password", password));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }

        // Hash TRƯỚC khi ghi: không giữ kết nối ghi trong lúc hash (luật dữ liệu).
        let password_hash = match input.password {
            Some(password) => Some(self.passwords.hash(password).await?),
            None => None,
        };
        let now = now();
        let row = entity::ActiveModel {
            id: Set(Uuid::now_v7()),
            email: Set(email.clone()),
            name: Set(name),
            password_hash: Set(password_hash),
            disabled_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(self.database.write())
        .await
        .map_err(|error| match error.sql_err() {
            // Hai request tạo cùng email đồng thời: index unique quyết bên nào thắng.
            Some(SqlErr::UniqueConstraintViolation(_)) => Error::conflict(
                "auth.email_taken",
                "A user with the email `{email}` already exists. Use another email, or sign in \
                 with this one.",
            )
            .with_param("email", &email),
            _ => Error::from(error),
        })?;
        tracing::info!(user_id = %row.id, "user created");
        Ok(row.into())
    }

    /// Người dùng có email `email` (so sánh sau khi chuẩn hoá).
    pub(crate) async fn find_by_email(&self, email: &str) -> Result<Option<User>, Error> {
        let row = Entity::find()
            .filter(Column::Email.eq(normalize_email(email)))
            .one(self.database.read())
            .await?;
        Ok(row.map(User::from))
    }

    /// Một trang người dùng, mới tạo trước (mặc định).
    pub(crate) async fn list(&self, page: &PageRequest) -> Result<Page<User>, Error> {
        let page = fetch_page(Entity::find(), Column::Id, |row| row.id, page, self.database.read())
            .await?;
        Ok(page.map(User::from))
    }

    /// Đặt mật khẩu mới cho người dùng `id`.
    pub(crate) async fn set_password(&self, id: Uuid, password: String) -> Result<(), Error> {
        if let Some(problem) = password::check_rules("password", &password) {
            return Err(Error::validation(vec![problem]));
        }
        let hash = self.passwords.hash(password).await?;
        let updated = Entity::update_many()
            .col_expr(Column::PasswordHash, Expr::value(Some(hash)))
            .col_expr(Column::UpdatedAt, Expr::value(now()))
            .filter(Column::Id.eq(id))
            .exec(self.database.write())
            .await?;
        if updated.rows_affected == 0 {
            return Err(user_not_found(id));
        }
        tracing::info!(user_id = %id, "password changed");
        Ok(())
    }

    /// Kiểm email và mật khẩu; đúng thì trả người dùng.
    ///
    /// Email không tồn tại và mật khẩu sai trả CÙNG một lỗi, sau CÙNG một khoảng
    /// thời gian: không ai dò được email nào đã đăng ký. Hash lưu theo tham số cũ
    /// được hash lại theo tham số hiện hành.
    pub(crate) async fn check_credentials(
        &self,
        email: &str,
        password: &str,
    ) -> Result<User, Error> {
        if password.chars().count() > password::MAX_LENGTH {
            return Err(invalid_credentials());
        }
        let row = Entity::find()
            .filter(Column::Email.eq(normalize_email(email)))
            .one(self.database.read())
            .await?;
        let stored = row.as_ref().and_then(|row| row.password_hash.clone());
        let verification = self.passwords.verify(password.to_owned(), stored.clone()).await?;

        let (row, rehash) = match (row, verification) {
            (Some(row), Verification::Valid { rehash }) => (row, rehash),
            _ => return Err(invalid_credentials()),
        };
        if row.disabled_at.is_some() {
            return Err(Error::forbidden(
                "auth.user_disabled",
                "This account is disabled. Ask an administrator to enable it.",
            ));
        }
        if rehash && let Some(stored) = stored {
            self.upgrade_hash(row.id, &stored, password).await;
        }
        Ok(row.into())
    }

    /// Hash lại mật khẩu theo tham số hiện hành. Hỏng thì chỉ ghi log: người dùng
    /// đã đăng nhập đúng, lần sau thử lại.
    async fn upgrade_hash(&self, id: Uuid, stored: &str, password: &str) {
        let result = async {
            let hash = self.passwords.hash(password.to_owned()).await?;
            // Chỉ thay nếu hash vẫn là hash vừa kiểm: không đè mật khẩu vừa được
            // đổi bởi một request khác.
            Entity::update_many()
                .col_expr(Column::PasswordHash, Expr::value(Some(hash)))
                .filter(Column::Id.eq(id))
                .filter(Column::PasswordHash.eq(stored))
                .exec(self.database.write())
                .await?;
            Ok::<(), Error>(())
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(user_id = %id, error = %error, "cannot upgrade the password hash");
        }
    }
}

#[async_trait::async_trait]
impl Users for UserService {
    async fn find(&self, id: Uuid) -> Result<Option<User>, Error> {
        self.get(id).await
    }
}

/// Bây giờ, cắt tới micro giây: Postgres lưu tới micro giây, SQLite lưu nguyên.
/// Cắt trước để hai database trả về cùng một giá trị.
fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

fn check_email(email: &str) -> Option<FieldError> {
    let valid = email.chars().count() <= EMAIL_MAX_LENGTH && email.validate_email();
    (!valid).then(|| {
        FieldError::new(
            "email",
            "email",
            format!("must be a valid email address of at most {EMAIL_MAX_LENGTH} characters"),
        )
    })
}

fn check_name(name: &str) -> Option<FieldError> {
    let length = name.chars().count();
    (!(1..=NAME_MAX_LENGTH).contains(&length)).then(|| {
        FieldError::new(
            "name",
            "length",
            format!("must be between 1 and {NAME_MAX_LENGTH} characters long"),
        )
    })
}

fn invalid_credentials() -> Error {
    Error::unauthenticated("auth.invalid_credentials", "The email or password is incorrect.")
}

#[cfg_attr(not(test), expect(dead_code, reason = "used by `set_password`"))]
fn user_not_found(id: Uuid) -> Error {
    Error::not_found("auth.user_not_found", "User `{id}` does not exist.").with_param("id", id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::error::ErrorKind;
    use autoxide_kernel::pagination::Order;
    use autoxide_testkit::TestDatabase;

    const PASSWORD: &str = "correct horse battery staple";

    async fn service(test: &TestDatabase) -> UserService {
        UserService::new(crate::testing::database(test).await, Passwords::new())
    }

    fn new_user(email: &str) -> NewUser {
        NewUser {
            email: email.to_owned(),
            name: "Ada Lovelace".to_owned(),
            password: Some(PASSWORD.to_owned()),
        }
    }

    #[tokio::test]
    async fn tao_roi_doc_lai_nguoi_dung() {
        TestDatabase::run(async |test| {
            let users = service(test).await;

            let created = users.create(new_user("  Ada@Example.COM ")).await.unwrap();

            assert_eq!(created.email, "ada@example.com");
            assert_eq!(created.name, "Ada Lovelace");
            assert!(created.is_active());
            assert_eq!(created.id.get_version_num(), 7);
            assert_eq!(users.get(created.id).await.unwrap(), Some(created.clone()));
            assert_eq!(users.find_by_email("ADA@example.com").await.unwrap(), Some(created));
            assert_eq!(users.get(Uuid::now_v7()).await.unwrap(), None);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn mat_khau_luu_duoi_dang_hash_argon2id() {
        TestDatabase::run(async |test| {
            let users = service(test).await;
            let created = users.create(new_user("ada@example.com")).await.unwrap();

            let row = Entity::find_by_id(created.id).one(users.database.read()).await.unwrap();

            let hash = row.unwrap().password_hash.unwrap();
            assert!(hash.starts_with("$argon2id$"), "{hash}");
            assert!(!hash.contains(PASSWORD));
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn email_trung_khong_phan_biet_hoa_thuong_bi_tu_choi() {
        TestDatabase::run(async |test| {
            let users = service(test).await;
            users.create(new_user("ada@example.com")).await.unwrap();

            let error = users.create(new_user("ADA@example.com")).await.unwrap_err();

            assert_eq!(error.kind(), ErrorKind::Conflict);
            assert_eq!(error.code(), "auth.email_taken");
            assert_eq!(
                error.message(),
                "A user with the email `ada@example.com` already exists. Use another email, or \
                 sign in with this one."
            );
        })
        .await
        .unwrap();
    }

    /// Nhiều request tạo cùng một email cùng lúc: đúng một cái thành công, còn lại
    /// nhận `auth.email_taken` — trên cả hai database.
    #[tokio::test]
    async fn tao_dong_thoi_cung_email_chi_mot_thanh_cong() {
        TestDatabase::run(async |test| {
            let users = std::sync::Arc::new(service(test).await);

            let attempts: Vec<_> = (0..6)
                .map(|_| {
                    let users = users.clone();
                    tokio::spawn(async move { users.create(new_user("ada@example.com")).await })
                })
                .collect();
            let mut created = 0;
            for attempt in attempts {
                match attempt.await.unwrap() {
                    Ok(_) => created += 1,
                    Err(error) => assert_eq!(error.code(), "auth.email_taken", "{error:?}"),
                }
            }

            assert_eq!(created, 1);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn du_lieu_sai_bao_du_moi_truong() {
        TestDatabase::run(async |test| {
            let users = service(test).await;
            let input = NewUser {
                email: "not an email".to_owned(),
                name: "   ".to_owned(),
                password: Some("short".to_owned()),
            };

            let error = users.create(input).await.unwrap_err();

            assert_eq!(error.code(), "validation_failed");
            let fields: Vec<(&str, &str)> = error
                .fields()
                .iter()
                .map(|field| (field.field.as_str(), field.code.as_str()))
                .collect();
            assert_eq!(fields, [("email", "email"), ("name", "length"), ("password", "length")]);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn kiem_thong_tin_dang_nhap() {
        TestDatabase::run(async |test| {
            let users = service(test).await;
            let created = users.create(new_user("ada@example.com")).await.unwrap();

            let signed_in = users.check_credentials(" ADA@example.com", PASSWORD).await.unwrap();
            let wrong = users.check_credentials("ada@example.com", "wrong password").await;
            let unknown = users.check_credentials("bob@example.com", PASSWORD).await;
            let huge = users.check_credentials("ada@example.com", &"a".repeat(10_000)).await;

            assert_eq!(signed_in, created);
            // Email lạ và mật khẩu sai: cùng một lỗi, không lộ email nào có thật.
            for error in [wrong, unknown, huge] {
                let error = error.unwrap_err();
                assert_eq!(error.kind(), ErrorKind::Unauthenticated);
                assert_eq!(error.code(), "auth.invalid_credentials");
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tai_khoan_chua_co_mat_khau_khong_dang_nhap_duoc() {
        TestDatabase::run(async |test| {
            let users = service(test).await;
            let input = NewUser { password: None, ..new_user("ada@example.com") };
            users.create(input).await.unwrap();

            let error = users.check_credentials("ada@example.com", "").await.unwrap_err();

            assert_eq!(error.code(), "auth.invalid_credentials");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tai_khoan_bi_khoa_khong_dang_nhap_duoc() {
        TestDatabase::run(async |test| {
            let users = service(test).await;
            let created = users.create(new_user("ada@example.com")).await.unwrap();
            Entity::update_many()
                .col_expr(Column::DisabledAt, Expr::value(Some(now())))
                .filter(Column::Id.eq(created.id))
                .exec(users.database.write())
                .await
                .unwrap();

            let error = users.check_credentials("ada@example.com", PASSWORD).await.unwrap_err();

            assert_eq!(error.kind(), ErrorKind::Forbidden);
            assert_eq!(error.code(), "auth.user_disabled");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn doi_mat_khau() {
        TestDatabase::run(async |test| {
            let users = service(test).await;
            let created = users.create(new_user("ada@example.com")).await.unwrap();
            let new_password = "a brand new passphrase";

            users.set_password(created.id, new_password.to_owned()).await.unwrap();

            assert!(users.check_credentials("ada@example.com", PASSWORD).await.is_err());
            let user = users.check_credentials("ada@example.com", new_password).await.unwrap();
            assert!(user.updated_at >= created.updated_at);

            let missing = users.set_password(Uuid::now_v7(), new_password.to_owned()).await;
            assert_eq!(missing.unwrap_err().code(), "auth.user_not_found");
            let short = users.set_password(created.id, "short".to_owned()).await;
            assert_eq!(short.unwrap_err().code(), "validation_failed");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn hash_cu_duoc_hash_lai_khi_dang_nhap_dung() {
        use argon2::password_hash::PasswordHasher as _;
        use argon2::{Algorithm, Argon2, Params, Version};

        TestDatabase::run(async |test| {
            let users = service(test).await;
            let created = users.create(new_user("ada@example.com")).await.unwrap();
            let weak = Argon2::new(
                Algorithm::Argon2id,
                Version::V0x13,
                Params::new(8 * 1024, 1, 1, None).unwrap(),
            );
            let old = weak.hash_password(PASSWORD.as_bytes()).unwrap().to_string();
            Entity::update_many()
                .col_expr(Column::PasswordHash, Expr::value(Some(old.clone())))
                .filter(Column::Id.eq(created.id))
                .exec(users.database.write())
                .await
                .unwrap();

            users.check_credentials("ada@example.com", PASSWORD).await.unwrap();

            let row = Entity::find_by_id(created.id).one(users.database.read()).await.unwrap();
            let upgraded = row.unwrap().password_hash.unwrap();
            assert_ne!(upgraded, old);
            assert!(upgraded.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{upgraded}");
            users.check_credentials("ada@example.com", PASSWORD).await.unwrap();
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn danh_sach_theo_trang() {
        TestDatabase::run(async |test| {
            let users = service(test).await;
            let mut created = Vec::new();
            for index in 0..3 {
                let input =
                    NewUser { password: None, ..new_user(&format!("u{index}@example.com")) };
                created.push(users.create(input).await.unwrap().id);
            }

            let first = users.list(&PageRequest::first(2, Order::Asc)).await.unwrap();
            let cursor: Uuid = first.pagination.next_cursor.as_deref().unwrap().parse().unwrap();
            let second =
                users.list(&PageRequest::first(2, Order::Asc).after(cursor)).await.unwrap();

            let ids: Vec<Uuid> =
                first.data.iter().chain(&second.data).map(|user| user.id).collect();
            assert_eq!(ids, created);
            assert_eq!(second.pagination.next_cursor, None);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn trait_users_cho_module_khac() {
        TestDatabase::run(async |test| {
            let service = std::sync::Arc::new(service(test).await);
            let created = service.create(new_user("ada@example.com")).await.unwrap();
            let users: std::sync::Arc<dyn Users> = service;

            assert_eq!(users.find(created.id).await.unwrap(), Some(created));
        })
        .await
        .unwrap();
    }
}
