//! Service của tài khoản người dùng: mọi logic về người dùng nằm ở đây. Handler REST,
//! lệnh CLI và tool MCP (ở các mục sau) chỉ gọi vào đây.

use autoxide_kernel::database::Database;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, Set, SqlErr,
};
use uuid::Uuid;
use validator::ValidateEmail;

use super::entity::{self, Column, Entity};
use crate::api::{Accounts, NewAccount, User, Users};
use crate::events;
use crate::password::{self, Passwords, Verification};
use crate::session::SessionService;
use autoxide_kernel::event::{Actor, Outbox};
use serde_json::json;

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

/// Thay đổi cho một người dùng; trường `None` giữ nguyên.
#[derive(Debug, Clone, Default)]
pub(crate) struct UserChanges {
    pub name: Option<String>,
    /// `true`: khoá; `false`: mở khoá.
    pub disabled: Option<bool>,
}

pub(crate) struct UserService {
    database: Database,
    passwords: Passwords,
    outbox: Outbox,
}

impl UserService {
    pub(crate) fn new(database: Database, passwords: Passwords, outbox: Outbox) -> Self {
        UserService { database, passwords, outbox }
    }

    /// Người dùng có ID `id`.
    pub(crate) async fn get(&self, id: Uuid) -> Result<Option<User>, Error> {
        let row = Entity::find_by_id(id).one(self.database.read()).await?;
        Ok(row.map(User::from))
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
            return Err(user_disabled());
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

    /// Tạo người dùng. Email được chuẩn hoá (bỏ khoảng trắng hai đầu, chữ thường)
    /// và phải chưa ai dùng.
    pub(crate) async fn create(&self, actor: Actor, input: NewUser) -> Result<User, Error> {
        self.create_with_roles(actor, input, false).await
    }

    /// Như [`create`](Self::create); `admin` thì gán luôn role `admin` trong CÙNG
    /// transaction — không bao giờ có người dùng "admin dở dang" khi một bước hỏng.
    pub(crate) async fn create_with_roles(
        &self,
        actor: Actor,
        input: NewUser,
        admin: bool,
    ) -> Result<User, Error> {
        let email = normalize_email(&input.email);
        let name = input.name.trim().to_owned();
        let mut problems = Vec::new();
        problems.extend(check_email_field("email", &email));
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
        let transaction = self.database.begin_write().await?;
        let row = entity::ActiveModel {
            id: Set(Uuid::now_v7()),
            email: Set(email.clone()),
            search_text: Set(search_text(&email, &name)),
            name: Set(name),
            password_hash: Set(password_hash),
            disabled_at: Set(None),
            email_verified_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await
        .map_err(|error| match error.sql_err() {
            // Hai request tạo cùng email đồng thời: index unique quyết bên nào thắng.
            Some(SqlErr::UniqueConstraintViolation(_)) => email_taken(&email),
            _ => Error::from(error),
        })?;
        if admin {
            crate::role::grant_admin(&transaction, row.id).await?;
        }
        let after = Some(json!({"email": row.email, "name": row.name, "admin": admin}));
        let event = events::UserCreated { before: None, after };
        self.outbox.record(&transaction, actor, events::user_subject(row.id), &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %row.id, admin, "user created");
        Ok(row.into())
    }

    /// Đổi tên và/hoặc khoá, mở khoá người dùng `id`. Khoá thì đăng xuất người đó ở
    /// mọi nơi; không khoá được admin cuối cùng còn hoạt động.
    /// `expected`: phiên bản (`updated_at`) người gọi đã đọc; khác thì lỗi 412 (D-084).
    pub(crate) async fn update(
        &self,
        actor: Actor,
        id: Uuid,
        changes: UserChanges,
        expected: Option<DateTime<Utc>>,
    ) -> Result<User, Error> {
        let name = changes.name.map(|name| name.trim().to_owned());
        if let Some(problem) = name.as_deref().and_then(check_name) {
            return Err(Error::validation(vec![problem]));
        }
        let transaction = self.database.begin_write().await?;
        let before =
            Entity::find_by_id(id).one(&transaction).await?.ok_or_else(|| user_not_found(id))?;
        if expected.is_some_and(|expected| expected != before.updated_at) {
            transaction.rollback().await?;
            return Err(autoxide_kernel::version::changed());
        }
        let admins_before = crate::role::count_active_admins(&transaction).await?;
        let now = now();
        let mut update = Entity::update_many().col_expr(Column::UpdatedAt, Expr::value(now));
        if let Some(name) = name {
            update =
                update.col_expr(Column::SearchText, Expr::value(search_text(&before.email, &name)));
            update = update.col_expr(Column::Name, Expr::value(name));
        }
        match changes.disabled {
            // Đã khoá rồi thì giữ nguyên thời điểm khoá cũ.
            Some(true) => {
                update = update.col_expr(
                    Column::DisabledAt,
                    Expr::col(Column::DisabledAt).if_null(Expr::value(now)),
                );
            }
            Some(false) => {
                update = update.col_expr(Column::DisabledAt, Expr::value(None::<DateTime<Utc>>));
            }
            None => {}
        }
        if let Some(expected) = expected {
            // Kiểm phiên bản ngay trong câu `UPDATE`: người khác vừa sửa thì 0 dòng.
            update = update.filter(Column::UpdatedAt.eq(expected));
        }
        // Câu `UPDATE` này cũng khoá dòng người dùng tới hết transaction.
        let updated = update.filter(Column::Id.eq(id)).exec(&transaction).await?;
        if updated.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(match expected {
                Some(_) => autoxide_kernel::version::changed(),
                None => user_not_found(id),
            });
        }
        if changes.disabled == Some(true) {
            SessionService::revoke_user(&transaction, id).await?;
            if admins_before > 0 && crate::role::count_active_admins(&transaction).await? == 0 {
                transaction.rollback().await?;
                return Err(crate::role::last_admin());
            }
        }
        let row =
            Entity::find_by_id(id).one(&transaction).await?.ok_or_else(|| user_not_found(id))?;
        let event =
            events::UserUpdated { before: Some(snapshot(&before)), after: Some(snapshot(&row)) };
        self.outbox.record(&transaction, actor, events::user_subject(id), &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %id, disabled = ?changes.disabled, "user changed");
        Ok(row.into())
    }

    /// Xoá hẳn tài khoản `id` cùng mọi dữ liệu của nó trong module `auth` (phiên, key,
    /// passkey, MFA… xoá theo foreign key). Phát event [`USER_DELETED`] trong cùng
    /// transaction để các module khác xoá phần dữ liệu của người đó. Không xoá được
    /// admin cuối cùng còn hoạt động.
    ///
    /// [`USER_DELETED`]: autoxide_kernel::privacy::USER_DELETED
    pub(crate) async fn delete(&self, actor: Actor, id: Uuid) -> Result<(), Error> {
        let transaction = self.database.begin_write().await?;
        let admins_before = crate::role::count_active_admins(&transaction).await?;
        let deleted = Entity::delete_by_id(id).exec(&transaction).await?;
        if deleted.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(user_not_found(id));
        }
        if admins_before > 0 && crate::role::count_active_admins(&transaction).await? == 0 {
            transaction.rollback().await?;
            return Err(crate::role::last_admin());
        }
        // Chỉ ghi id: audit log không giữ lại email hay tên của người đã xin xoá.
        let event =
            events::UserDeleted { before: Some(serde_json::json!({"id": id})), after: None };
        self.outbox.record(&transaction, actor, events::user_subject(id), &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %id, "user deleted");
        Ok(())
    }

    /// Người dùng có email `email` (so sánh sau khi chuẩn hoá).
    pub(crate) async fn find_by_email(&self, email: &str) -> Result<Option<User>, Error> {
        let row = Entity::find()
            .filter(Column::Email.eq(normalize_email(email)))
            .one(self.database.read())
            .await?;
        Ok(row.map(User::from))
    }

    /// Ghi nhận người dùng `id` đã xác minh `email`, trong `transaction` của bên gọi.
    /// `email` khác email hiện tại thì nó thay email hiện tại (đổi email). Trả email cũ.
    pub(crate) async fn apply_verified_email(
        &self,
        transaction: &DatabaseTransaction,
        id: Uuid,
        email: &str,
        at: DateTime<Utc>,
    ) -> Result<String, Error> {
        let row =
            Entity::find_by_id(id).one(transaction).await?.ok_or_else(|| user_not_found(id))?;
        Entity::update_many()
            .col_expr(Column::Email, Expr::value(email))
            .col_expr(Column::SearchText, Expr::value(search_text(email, &row.name)))
            .col_expr(Column::EmailVerifiedAt, Expr::value(Some(at)))
            .col_expr(Column::UpdatedAt, Expr::value(at))
            .filter(Column::Id.eq(id))
            .exec(transaction)
            .await
            .map_err(|error| match error.sql_err() {
                // Trong lúc chờ xác minh, người khác đã lấy email này.
                Some(SqlErr::UniqueConstraintViolation(_)) => email_taken(email),
                _ => Error::from(error),
            })?;
        Ok(row.email)
    }

    /// Một trang người dùng, mới tạo trước (mặc định). `search`: chỉ những người có
    /// email hoặc tên chứa chuỗi đó (không phân biệt hoa thường).
    pub(crate) async fn list(
        &self,
        search: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<User>, Error> {
        let mut select = Entity::find();
        if let Some(search) = search.map(str::trim).filter(|search| !search.is_empty()) {
            // Không có index nào giúp được tìm "chứa": quét bảng người dùng.
            select = select.filter(autoxide_kernel::search::contains(Column::SearchText, search));
        }
        let page = fetch_page(select, Column::Id, |row| row.id, page, self.database.read()).await?;
        Ok(page.map(User::from))
    }

    /// Đặt mật khẩu mới cho người dùng `id`.
    pub(crate) async fn set_password(
        &self,
        actor: Actor,
        id: Uuid,
        password: String,
    ) -> Result<(), Error> {
        self.write_password(actor, id, "password", password, false).await
    }

    /// Quản trị viên đặt mật khẩu cho tài khoản tạo không kèm mật khẩu. Tài khoản đã
    /// có mật khẩu thì từ chối (`auth.password_already_set`): người giữ quyền quản lý
    /// người dùng không được chiếm tài khoản của người khác bằng cách đổi mật khẩu của họ.
    pub(crate) async fn set_initial_password(
        &self,
        actor: Actor,
        id: Uuid,
        password: String,
    ) -> Result<(), Error> {
        self.write_password(actor, id, "password", password, true).await
    }

    /// Hash và lưu mật khẩu. `field`: tên trường báo trong lỗi luật mật khẩu.
    /// `only_if_unset`: chỉ ghi khi tài khoản chưa có mật khẩu — kiểm ngay trong câu
    /// `UPDATE` nên hai request đồng thời không cùng ghi được.
    pub(crate) async fn write_password(
        &self,
        actor: Actor,
        id: Uuid,
        field: &str,
        password: String,
        only_if_unset: bool,
    ) -> Result<(), Error> {
        if let Some(problem) = password::check_rules(field, &password) {
            return Err(Error::validation(vec![problem]));
        }
        let hash = self.passwords.hash(password).await?;
        // Đổi mật khẩu và thu hồi mọi phiên đăng nhập trong CÙNG một transaction: kẻ
        // giữ mật khẩu cũ (hay refresh token cũ) mất quyền ngay khi mật khẩu đổi.
        let transaction = self.database.begin_write().await?;
        let mut update = Entity::update_many()
            .col_expr(Column::PasswordHash, Expr::value(Some(hash)))
            .col_expr(Column::UpdatedAt, Expr::value(now()))
            .filter(Column::Id.eq(id));
        if only_if_unset {
            update = update.filter(Column::PasswordHash.is_null());
        }
        let updated = update.exec(&transaction).await?;
        if updated.rows_affected == 0 {
            transaction.rollback().await?;
            if only_if_unset && self.get(id).await?.is_some() {
                return Err(password_already_set());
            }
            return Err(user_not_found(id));
        }
        SessionService::revoke_user(&transaction, id).await?;
        let event = events::PasswordChanged::default();
        self.outbox.record(&transaction, actor, events::user_subject(id), &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %id, "password changed");
        Ok(())
    }
}

#[async_trait::async_trait]
impl Accounts for UserService {
    async fn create(&self, account: NewAccount) -> Result<User, Error> {
        let input =
            NewUser { email: account.email, name: account.name, password: account.password };
        // Chỉ lệnh CLI dùng trait này: người vận hành máy chủ, tức "hệ thống".
        self.create_with_roles(Actor::system(), input, account.admin).await
    }
}

#[async_trait::async_trait]
impl Users for UserService {
    async fn find(&self, id: Uuid) -> Result<Option<User>, Error> {
        self.get(id).await
    }
}

/// Ảnh chụp những gì audit cần biết về một người dùng (không có hash mật khẩu).
fn snapshot(row: &entity::Model) -> serde_json::Value {
    json!({"email": row.email, "name": row.name, "disabled_at": row.disabled_at})
}

/// Bây giờ, cắt tới micro giây: Postgres lưu tới micro giây, SQLite lưu nguyên.
/// Cắt trước để hai database trả về cùng một giá trị.
/// Email và tên viết thường, bỏ dấu, lưu ở `auth_user.search_text` để tìm (m0016, m0017):
/// "dat" tìm ra "Đạt".
fn search_text(email: &str, name: &str) -> String {
    autoxide_kernel::search::fold(&format!("{email} {name}"))
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

pub(crate) fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

pub(crate) fn email_taken(email: &str) -> Error {
    Error::conflict(
        "auth.email_taken",
        "A user with the email `{email}` already exists. Use another email, or sign in with \
         this one.",
    )
    .with_param("email", email)
}

/// Lỗi của trường `field` nếu `email` (đã chuẩn hoá) không phải địa chỉ email hợp lệ.
pub(crate) fn check_email_field(field: &str, email: &str) -> Option<FieldError> {
    let valid = email.chars().count() <= EMAIL_MAX_LENGTH && email.validate_email();
    (!valid).then(|| {
        FieldError::new(
            field,
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

/// Lỗi khi tài khoản đã bị khoá.
pub(crate) fn user_disabled() -> Error {
    Error::forbidden(
        "auth.user_disabled",
        "This account is disabled. Ask an administrator to enable it.",
    )
}

fn invalid_credentials() -> Error {
    Error::unauthenticated("auth.invalid_credentials", "The email or password is incorrect.")
}

fn password_already_set() -> Error {
    Error::conflict(
        "auth.password_already_set",
        "This account already has a password. Only its owner can change it \
         (`POST /api/v1/auth/me/password`), or reset it by email \
         (`POST /api/v1/auth/password-reset`).",
    )
}

/// Lỗi khi không có người dùng `id`.
pub(crate) fn user_not_found(id: Uuid) -> Error {
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
        UserService::new(
            crate::testing::database(test).await,
            Passwords::new(),
            Outbox::standalone("auth"),
        )
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

            let created =
                users.create(Actor::system(), new_user("  Ada@Example.COM ")).await.unwrap();

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
            let created = users.create(Actor::system(), new_user("ada@example.com")).await.unwrap();

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
            users.create(Actor::system(), new_user("ada@example.com")).await.unwrap();

            let error =
                users.create(Actor::system(), new_user("ADA@example.com")).await.unwrap_err();

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
                    tokio::spawn(async move {
                        users.create(Actor::system(), new_user("ada@example.com")).await
                    })
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

            let error = users.create(Actor::system(), input).await.unwrap_err();

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
            let created = users.create(Actor::system(), new_user("ada@example.com")).await.unwrap();

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
            users.create(Actor::system(), input).await.unwrap();

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
            let created = users.create(Actor::system(), new_user("ada@example.com")).await.unwrap();
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
            let created = users.create(Actor::system(), new_user("ada@example.com")).await.unwrap();
            let new_password = "a brand new passphrase";

            users.set_password(Actor::system(), created.id, new_password.to_owned()).await.unwrap();

            assert!(users.check_credentials("ada@example.com", PASSWORD).await.is_err());
            let user = users.check_credentials("ada@example.com", new_password).await.unwrap();
            assert!(user.updated_at >= created.updated_at);

            let missing =
                users.set_password(Actor::system(), Uuid::now_v7(), new_password.to_owned()).await;
            assert_eq!(missing.unwrap_err().code(), "auth.user_not_found");
            let short = users.set_password(Actor::system(), created.id, "short".to_owned()).await;
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
            let created = users.create(Actor::system(), new_user("ada@example.com")).await.unwrap();
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
                created.push(users.create(Actor::system(), input).await.unwrap().id);
            }

            let first = users.list(None, &PageRequest::first(2, Order::Asc)).await.unwrap();
            let cursor: Uuid = first.pagination.next_cursor.as_deref().unwrap().parse().unwrap();
            let second =
                users.list(None, &PageRequest::first(2, Order::Asc).after(cursor)).await.unwrap();

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
            let created =
                service.create(Actor::system(), new_user("ada@example.com")).await.unwrap();
            let users: std::sync::Arc<dyn Users> = service;

            assert_eq!(users.find(created.id).await.unwrap(), Some(created));
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn quan_tri_vien_chi_dat_duoc_mat_khau_dau_tien() {
        TestDatabase::run(async |test| {
            let users = service(test).await;
            let bob = NewUser { password: None, ..new_user("bob@example.com") };
            let bob = users.create(Actor::system(), bob).await.unwrap();
            let ada = users.create(Actor::system(), new_user("ada@example.com")).await.unwrap();

            let short = users.set_initial_password(Actor::system(), bob.id, "short".to_owned());
            assert_eq!(short.await.unwrap_err().code(), "validation_failed");
            users.set_initial_password(Actor::system(), bob.id, PASSWORD.to_owned()).await.unwrap();
            users.check_credentials("bob@example.com", PASSWORD).await.unwrap();

            for id in [bob.id, ada.id] {
                let taken = users.set_initial_password(Actor::system(), id, PASSWORD.to_owned());
                let taken = taken.await.unwrap_err();
                assert_eq!(taken.kind(), ErrorKind::Conflict);
                assert_eq!(taken.code(), "auth.password_already_set");
            }
            let ghost =
                users.set_initial_password(Actor::system(), Uuid::now_v7(), PASSWORD.into());
            assert_eq!(ghost.await.unwrap_err().code(), "auth.user_not_found");
        })
        .await
        .unwrap();
    }

    /// Hai quản trị viên cùng đặt mật khẩu đầu tiên: đúng một người được — trên cả hai
    /// database.
    #[tokio::test]
    async fn dat_mat_khau_dau_tien_dong_thoi_chi_mot_ben_duoc() {
        TestDatabase::run(async |test| {
            let users = std::sync::Arc::new(service(test).await);
            let bob = NewUser { password: None, ..new_user("bob@example.com") };
            let bob = users.create(Actor::system(), bob).await.unwrap();

            let attempts: Vec<_> = (0..4)
                .map(|index| {
                    let users = users.clone();
                    tokio::spawn(async move {
                        let password = format!("{PASSWORD} number {index}");
                        users.set_initial_password(Actor::system(), bob.id, password).await
                    })
                })
                .collect();
            let mut succeeded = 0;
            for attempt in attempts {
                if attempt.await.unwrap().is_ok() {
                    succeeded += 1;
                }
            }

            assert_eq!(succeeded, 1);
        })
        .await
        .unwrap();
    }
}

#[cfg(test)]
mod account_tests {
    use super::*;
    use autoxide_testkit::TestDatabase;

    fn account(email: &str, admin: bool) -> NewAccount {
        NewAccount {
            email: email.to_owned(),
            name: "Ada".to_owned(),
            password: Some("correct horse battery staple".to_owned()),
            admin,
        }
    }

    #[tokio::test]
    async fn tao_tai_khoan_admin_co_moi_quyen() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let accounts: &dyn Accounts = services.users.as_ref();

            let admin = accounts.create(account("ada@example.com", true)).await.unwrap();
            let plain = accounts.create(account("bob@example.com", false)).await.unwrap();

            assert!(services.roles.principal(admin.id).await.unwrap().has_all_permissions());
            let bob = services.roles.principal(plain.id).await.unwrap();
            assert!(!bob.has_all_permissions());
            assert!(bob.permissions().is_empty());
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tao_hong_thi_khong_de_lai_gi() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let accounts: &dyn Accounts = services.users.as_ref();
            accounts.create(account("ada@example.com", true)).await.unwrap();

            let error = accounts.create(account("ADA@example.com", true)).await.unwrap_err();

            assert_eq!(error.code(), "auth.email_taken");
            let page = services
                .users
                .list(None, &PageRequest::first(10, autoxide_kernel::pagination::Order::Asc))
                .await
                .unwrap();
            assert_eq!(page.data.len(), 1);
        })
        .await
        .unwrap();
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;
    use autoxide_kernel::error::ErrorKind;
    use autoxide_testkit::TestDatabase;

    const PASSWORD: &str = "correct horse battery staple";

    fn account(email: &str, admin: bool) -> NewAccount {
        NewAccount {
            email: email.to_owned(),
            name: "Ada".to_owned(),
            password: Some(PASSWORD.to_owned()),
            admin,
        }
    }

    fn disable(disabled: bool) -> UserChanges {
        UserChanges { name: None, disabled: Some(disabled) }
    }

    #[tokio::test]
    async fn doi_ten_khoa_va_mo_khoa() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let users = &services.users;
            Accounts::create(users.as_ref(), account("root@example.com", true)).await.unwrap();
            let ada =
                Accounts::create(users.as_ref(), account("ada@example.com", false)).await.unwrap();
            let tokens =
                services.sessions.login("ada@example.com", PASSWORD, None).await.unwrap().tokens();

            let renamed = users
                .update(
                    Actor::system(),
                    ada.id,
                    UserChanges { name: Some("  Ada L. ".to_owned()), disabled: None },
                    None,
                )
                .await
                .unwrap();
            assert_eq!(renamed.name, "Ada L.");
            assert!(renamed.is_active());

            let disabled =
                users.update(Actor::system(), ada.id, disable(true), None).await.unwrap();
            assert!(!disabled.is_active());
            // Khoá lần nữa giữ thời điểm khoá cũ.
            let again = users.update(Actor::system(), ada.id, disable(true), None).await.unwrap();
            assert_eq!(again.disabled_at, disabled.disabled_at);
            // Đăng xuất ở mọi nơi: refresh token cũ hết dùng được.
            let refresh = services.sessions.refresh(&tokens.refresh_token).await.unwrap_err();
            assert_eq!(refresh.code(), "auth.invalid_refresh_token");
            let login =
                services.sessions.login("ada@example.com", PASSWORD, None).await.unwrap_err();
            assert_eq!(login.code(), "auth.user_disabled");

            let enabled =
                users.update(Actor::system(), ada.id, disable(false), None).await.unwrap();
            assert!(enabled.is_active());
            services.sessions.login("ada@example.com", PASSWORD, None).await.unwrap().tokens();

            let missing = users
                .update(Actor::system(), Uuid::now_v7(), disable(true), None)
                .await
                .unwrap_err();
            assert_eq!(missing.code(), "auth.user_not_found");
            let empty = UserChanges { name: Some(" ".to_owned()), disabled: None };
            assert_eq!(
                users.update(Actor::system(), ada.id, empty, None).await.unwrap_err().code(),
                "validation_failed"
            );
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn khong_khoa_duoc_admin_cuoi_cung() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let users = &services.users;
            let root =
                Accounts::create(users.as_ref(), account("root@example.com", true)).await.unwrap();

            let error =
                users.update(Actor::system(), root.id, disable(true), None).await.unwrap_err();

            assert_eq!(error.kind(), ErrorKind::Conflict);
            assert_eq!(error.code(), "auth.last_admin");
            assert!(users.get(root.id).await.unwrap().unwrap().is_active());
            // Có admin khác còn hoạt động thì khoá được.
            Accounts::create(users.as_ref(), account("second@example.com", true)).await.unwrap();
            users.update(Actor::system(), root.id, disable(true), None).await.unwrap();
        })
        .await
        .unwrap();
    }

    /// Hai admin cuối cùng xoá (hay khoá) nhau cùng lúc: đúng một bên thành công, luôn còn
    /// một admin hoạt động — trên cả hai database (D-112).
    #[tokio::test]
    async fn hai_admin_cuoi_xoa_nhau_cung_luc_van_con_mot() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let users = services.users.clone();
            let a = Accounts::create(users.as_ref(), account("a@example.com", true)).await.unwrap();
            let b = Accounts::create(users.as_ref(), account("b@example.com", true)).await.unwrap();
            let (first, second) = (users.clone(), users.clone());
            let delete_a = tokio::spawn(async move { first.delete(Actor::system(), a.id).await });
            let disable_b = tokio::spawn(async move {
                second.update(Actor::system(), b.id, disable(true), None).await.map(|_| ())
            });
            let outcomes = [delete_a.await.unwrap(), disable_b.await.unwrap()];
            let refused: Vec<_> =
                outcomes.iter().filter_map(|outcome| outcome.as_ref().err()).collect();
            assert_eq!(refused.len(), 1, "{outcomes:?}");
            assert_eq!(refused[0].code(), "auth.last_admin");
            let left = crate::role::count_active_admins(services.database.write()).await.unwrap();
            assert_eq!(left, 1);
        })
        .await
        .unwrap();
    }
}
