//! Service của tài khoản người dùng: mọi logic về người dùng nằm ở đây. Handler REST,
//! lệnh CLI và tool MCP (ở các mục sau) chỉ gọi vào đây.

use autoxide_kernel::access::Realm;
use autoxide_kernel::time::Timestamp;
use std::collections::HashMap;

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::rows;
use autoxide_kernel::tokens::OneTimeTokens;
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, QuerySelect, Set,
    SqlErr,
};
use uuid::Uuid;
use validator::ValidateEmail;

use super::entity::{self, Column, Entity};
use crate::api::{Accounts, NewAccount, User, Users};
use crate::events;
use crate::password::{self, Passwords, Verification};
use crate::session::SessionService;
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use serde_json::json;

mod credentials;

/// Độ dài tối đa của email (RFC 5321) và của tên hiển thị, theo ký tự.
const EMAIL_MAX_LENGTH: usize = 254;
const NAME_MAX_LENGTH: usize = 200;

/// Vì sao mật khẩu được ghi (xem [`UserService::write_password`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PasswordWrite {
    /// Người dùng tự đổi, hay hệ thống đặt.
    Change,
    /// Người dùng tự đổi và chọn thu hồi mọi thông tin đăng nhập khác: xoá thêm passkey, API key,
    /// liên kết đăng nhập ngoài, như `Recovery` (L-7).
    ChangeRevokingAll,
    /// Quản trị viên đặt mật khẩu đầu cho tài khoản chưa có.
    Initial,
    /// Lấy lại tài khoản bằng mail: xoá thêm passkey, API key, token đặt lại khác.
    Recovery,
}

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
    /// Ngôn ngữ ưa thích: `Some(None)` bỏ đặt (D-308).
    pub locale: Option<Option<String>>,
    /// Múi giờ IANA ưa thích: `Some(None)` bỏ đặt.
    pub time_zone: Option<Option<String>>,
}

/// Lọc danh sách người dùng theo trạng thái.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UserStatus {
    Active,
    Disabled,
}

/// Một dòng của [`UserService::facts`]: ID, cờ có mật khẩu, cờ bật TOTP (`x` / `NULL`), lần đăng nhập cuối.
type FactsRow = (Uuid, Option<String>, Option<String>, Option<Timestamp>);

/// Tài khoản của chính người gọi, kèm dữ kiện `GET /me` cần (xem [`UserService::own_account`]).
#[derive(Debug, Clone)]
pub(crate) struct OwnAccount {
    pub user: User,
    pub has_password: bool,
    /// Có role bắt buộc MFA mà chưa bật MFA: quyền của các role đó đang bị giữ.
    pub mfa_pending: bool,
}

/// Dữ kiện của một tài khoản cho trang quản trị (không có trong `api::User`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AccountFacts {
    pub has_password: bool,
    pub mfa_enabled: bool,
    pub last_sign_in_at: Option<Timestamp>,
}

// Service không giữ database: mỗi hàm nhận khách của request (`TenantContext`) và làm việc trên
// database của khách đó, chỉ với dòng của khách đó (bước 33).
pub(crate) struct UserService {
    passwords: Passwords,
    outbox: Outbox,
}

impl UserService {
    pub(crate) fn new(passwords: Passwords, outbox: Outbox) -> Self {
        UserService { passwords, outbox }
    }

    /// Người dùng có ID `id` của khách `tenant`.
    pub(crate) async fn get(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Option<User>, Error> {
        let row = Entity::find_by_id(id).scoped(tenant).one(tenant.database().read()).await?;
        Ok(row.map(User::from))
    }

    /// Tạo người dùng. Email được chuẩn hoá (bỏ khoảng trắng hai đầu, chữ thường)
    /// và phải chưa ai dùng.
    pub(crate) async fn create(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        input: NewUser,
    ) -> Result<User, Error> {
        self.create_with_roles(tenant, actor, input, false).await
    }

    /// Như [`create`](Self::create); `admin` thì gán luôn role `admin` trong CÙNG
    /// transaction — không bao giờ có người dùng "admin dở dang" khi một bước hỏng.
    pub(crate) async fn create_with_roles(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        input: NewUser,
        admin: bool,
    ) -> Result<User, Error> {
        self.create_in(tenant, actor, input, admin, Realm::Staff).await
    }

    /// Tạo tài khoản người mua của khách `tenant` (D-309): không role, không permission.
    pub(crate) async fn create_customer(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        input: NewUser,
    ) -> Result<User, Error> {
        self.create_in(tenant, actor, input, false, Realm::Customer).await
    }

    async fn create_in(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        input: NewUser,
        admin: bool,
        realm: Realm,
    ) -> Result<User, Error> {
        let email = autoxide_kernel::email::normalize(&input.email);
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
        let transaction = tenant.database().begin_write().await?;
        let row = entity::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            email: Set(email.clone()),
            search_text: Set(search_text(&email, &name)),
            name: Set(name),
            password_hash: Set(password_hash),
            disabled_at: Set(None),
            email_verified_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            last_sign_in_at: Set(None),
            locale: Set(None),
            time_zone: Set(None),
            realm: Set(entity::realm_str(realm).to_owned()),
            phone_key: Set(None),
        }
        .insert(&transaction)
        .await
        .map_err(|error| match error.sql_err() {
            // Hai request tạo cùng email đồng thời: index unique quyết bên nào thắng.
            Some(SqlErr::UniqueConstraintViolation(_)) => email_taken(&email),
            _ => Error::from(error),
        })?;
        if admin {
            crate::role::grant_admin(&transaction, tenant, row.id).await?;
        }
        let after = Some(json!({"email": row.email, "name": row.name, "admin": admin}));
        let event = events::UserCreated { before: None, after };
        self.outbox
            .record(&transaction, tenant, actor, events::user_subject(row.id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(user_id = %row.id, admin, "user created");
        Ok(row.into())
    }

    /// Đổi tên và/hoặc khoá, mở khoá người dùng `id`. Khoá thì đăng xuất người đó ở
    /// mọi nơi; không khoá được admin cuối cùng còn hoạt động.
    /// `expected`: phiên bản (`updated_at`) người gọi đã đọc; khác thì lỗi 412 (D-084).
    pub(crate) async fn update(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        changes: UserChanges,
        expected: Option<Timestamp>,
    ) -> Result<User, Error> {
        let name = changes.name.as_ref().map(|name| name.trim().to_owned());
        let mut problems: Vec<FieldError> =
            name.as_deref().and_then(check_name).into_iter().collect();
        let locale = match &changes.locale {
            Some(Some(raw)) => match check_locale(raw) {
                Ok(locale) => Some(Some(locale)),
                Err(problem) => {
                    problems.push(problem);
                    None
                }
            },
            Some(None) => Some(None),
            None => None,
        };
        let time_zone = match &changes.time_zone {
            Some(Some(raw)) => match autoxide_kernel::time::zone(raw.trim()) {
                Some(_) => Some(Some(raw.trim().to_owned())),
                None => {
                    problems.push(FieldError::new(
                        "time_zone",
                        "invalid",
                        "must be an IANA time zone name, e.g. `Asia/Ho_Chi_Minh` or `UTC`",
                    ));
                    None
                }
            },
            Some(None) => Some(None),
            None => None,
        };
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let transaction = tenant.database().begin_write().await?;
        let before = Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| user_not_found(id))?;
        if expected.is_some_and(|expected| expected != before.updated_at) {
            transaction.rollback().await?;
            return Err(autoxide_kernel::version::changed());
        }
        // Chỉ việc khoá mới có thể làm mất admin cuối. Khoá dòng người dùng TRƯỚC rồi mới khoá
        // các role toàn quyền (trong `count_active_admins`) — cùng thứ tự với
        // `RoleService::set_user_roles`, nên hai việc chạy cùng lúc trên Postgres chờ nhau thay
        // vì deadlock (D-156).
        let admins_before = if changes.disabled == Some(true) {
            lock_row(&transaction, tenant, id).await?;
            crate::role::count_active_admins(&transaction, tenant).await?
        } else {
            0
        };
        let now = now();
        let mut update = Entity::update_many().col_expr(Column::UpdatedAt, Expr::value(now));
        if let Some(name) = name {
            update =
                update.col_expr(Column::SearchText, Expr::value(search_text(&before.email, &name)));
            update = update.col_expr(Column::Name, Expr::value(name));
        }
        if let Some(locale) = locale {
            update = update.col_expr(Column::Locale, Expr::value(locale));
        }
        if let Some(time_zone) = time_zone {
            update = update.col_expr(Column::TimeZone, Expr::value(time_zone));
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
                update = update.col_expr(Column::DisabledAt, Expr::value(None::<Timestamp>));
            }
            None => {}
        }
        if let Some(expected) = expected {
            // Kiểm phiên bản ngay trong câu `UPDATE`: người khác vừa sửa thì 0 dòng.
            update = update.filter(Column::UpdatedAt.eq(expected));
        }
        // Câu `UPDATE` này cũng khoá dòng người dùng tới hết transaction.
        let updated = update.filter(Column::Id.eq(id)).scoped(tenant).exec(&transaction).await?;
        if updated.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(match expected {
                Some(_) => autoxide_kernel::version::changed(),
                None => user_not_found(id),
            });
        }
        if changes.disabled == Some(true) {
            SessionService::revoke_user(&transaction, tenant, id).await?;
            revoke_durable_grants_in(&transaction, tenant, &self.outbox, actor, id).await?;
            if admins_before > 0
                && crate::role::count_active_admins(&transaction, tenant).await? == 0
            {
                transaction.rollback().await?;
                return Err(crate::role::last_admin());
            }
        }
        let row = Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| user_not_found(id))?;
        let event =
            events::UserUpdated { before: Some(snapshot(&before)), after: Some(snapshot(&row)) };
        self.outbox.record(&transaction, tenant, actor, events::user_subject(id), &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %id, disabled = ?changes.disabled, "user changed");
        Ok(row.into())
    }

    /// Xoá hẳn tài khoản `id` cùng mọi dữ liệu của nó trong module `auth` (phiên, key,
    /// passkey, MFA… xoá theo foreign key; token đặt lại mật khẩu và xác minh email trong
    /// bảng `auth_tokens` của kernel thì xoá ở đây, cùng transaction). Phát event [`USER_DELETED`] trong cùng
    /// transaction để các module khác xoá phần dữ liệu của người đó. Không xoá được
    /// admin cuối cùng còn hoạt động.
    ///
    /// [`USER_DELETED`]: autoxide_kernel::privacy::USER_DELETED
    pub(crate) async fn delete(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        // Dòng người dùng trước, role toàn quyền sau (xem `update`).
        lock_row(&transaction, tenant, id).await?;
        let admins_before = crate::role::count_active_admins(&transaction, tenant).await?;
        let deleted = Entity::delete_by_id(id).scoped(tenant).exec(&transaction).await?;
        if deleted.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(user_not_found(id));
        }
        if admins_before > 0 && crate::role::count_active_admins(&transaction, tenant).await? == 0 {
            transaction.rollback().await?;
            return Err(crate::role::last_admin());
        }
        autoxide_kernel::tokens::OneTimeTokens::new("auth")
            .forget(&transaction, tenant, &id.to_string())
            .await?;
        // Liên kết đăng nhập ngoài (bước 40 P3): không có foreign key, xoá cùng transaction.
        crate::oidc::identity::delete_for_user(&transaction, tenant, id).await?;
        // Liên kết SCIM (bước 66): không có foreign key, xoá cùng transaction.
        crate::scim::forget_user(&transaction, tenant, id).await?;
        // Chỉ ghi id: audit log không giữ lại email hay tên của người đã xin xoá.
        let event =
            events::UserDeleted { before: Some(serde_json::json!({"id": id})), after: None };
        self.outbox.record(&transaction, tenant, actor, events::user_subject(id), &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %id, "user deleted");
        Ok(())
    }

    /// Nhân sự có email `email` (so sánh sau khi chuẩn hoá).
    pub(crate) async fn find_by_email(
        &self,
        tenant: &TenantContext,
        email: &str,
    ) -> Result<Option<User>, Error> {
        self.find_by_email_in(tenant, Realm::Staff, email).await
    }

    /// Tài khoản `realm` có email `email` (D-309: cùng email có thể là nhân sự và người mua).
    pub(crate) async fn find_by_email_in(
        &self,
        tenant: &TenantContext,
        realm: Realm,
        email: &str,
    ) -> Result<Option<User>, Error> {
        let row = Entity::find()
            .filter(Column::Realm.eq(entity::realm_str(realm)))
            .filter(Column::Email.eq(autoxide_kernel::email::normalize(email)))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        Ok(row.map(User::from))
    }

    /// Ghi nhận người dùng `id` đã xác minh `email`, trong `transaction` của bên gọi.
    /// `email` khác email hiện tại thì nó thay email hiện tại (đổi email). Trả email cũ.
    pub(crate) async fn apply_verified_email(
        &self,
        tenant: &TenantContext,
        transaction: &DatabaseTransaction,
        id: Uuid,
        email: &str,
        at: Timestamp,
    ) -> Result<String, Error> {
        let row = Entity::find_by_id(id)
            .scoped(tenant)
            .one(transaction)
            .await?
            .ok_or_else(|| user_not_found(id))?;
        Entity::update_many()
            .col_expr(Column::Email, Expr::value(email))
            .col_expr(Column::SearchText, Expr::value(search_text(email, &row.name)))
            .col_expr(Column::EmailVerifiedAt, Expr::value(Some(at)))
            .col_expr(Column::UpdatedAt, Expr::value(at))
            .filter(Column::Id.eq(id))
            .scoped(tenant)
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
        tenant: &TenantContext,
        search: Option<&str>,
        status: Option<UserStatus>,
        page: &PageRequest,
    ) -> Result<Page<User>, Error> {
        self.list_in(tenant, Realm::Staff, search, status, page).await
    }

    /// Như [`list`](Self::list), cho tài khoản `realm` (người mua: `GET /customers`).
    pub(crate) async fn list_in(
        &self,
        tenant: &TenantContext,
        realm: Realm,
        search: Option<&str>,
        status: Option<UserStatus>,
        page: &PageRequest,
    ) -> Result<Page<User>, Error> {
        let mut select =
            Entity::find().filter(Column::Realm.eq(entity::realm_str(realm))).scoped(tenant);
        if let Some(search) = search.map(str::trim).filter(|search| !search.is_empty()) {
            // Không có index nào giúp được tìm "chứa": quét bảng người dùng.
            select = select.filter(autoxide_kernel::search::contains(Column::SearchText, search));
        }
        select = match status {
            Some(UserStatus::Active) => select.filter(Column::DisabledAt.is_null()),
            Some(UserStatus::Disabled) => select.filter(Column::DisabledAt.is_not_null()),
            None => select,
        };
        let read = tenant.database().read();
        let page = fetch_page(select, Column::Id, |row| row.id, page, read).await?;
        Ok(page.map(User::from))
    }

    /// Những người `ids` (để hiện tên, email chủ của bản ghi) — một câu cho cả trang. Người đã
    /// bị xoá: vắng.
    pub(crate) async fn by_ids(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, User>, Error> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = Entity::find()
            .filter(Column::Id.is_in(ids.iter().copied()))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        Ok(rows.into_iter().map(|row| (row.id, User::from(row))).collect())
    }

    /// Dữ kiện tài khoản của những người `ids` cho trang quản trị (có mật khẩu chưa, đã bật đăng
    /// nhập hai bước chưa, lần đăng nhập cuối) — hai câu truy vấn cho cả trang, không theo từng
    /// người (đề bài giao diện B1).
    /// Ngôn ngữ cho mail gửi chậm tới người dùng `id`: ngôn ngữ ưa thích nếu đã đặt, không thì
    /// `fallback` (ngôn ngữ của request gốc) (D-308).
    pub(crate) async fn mail_language(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        fallback: autoxide_kernel::i18n::Language,
    ) -> Result<autoxide_kernel::i18n::Language, Error> {
        let preferred = self.get(tenant, id).await?.and_then(|found| found.locale);
        Ok(preferred
            .as_deref()
            .and_then(autoxide_kernel::i18n::Language::from_tag)
            .unwrap_or(fallback))
    }

    /// Tài khoản `id` kèm dữ kiện cho chính chủ (`GET /me`, tool `whoami`) trong MỘT câu: có mật
    /// khẩu chưa, và có role bắt buộc MFA mà chưa bật MFA (D-308). Đường nóng:
    /// trước đây là bốn câu (dòng người dùng, `facts` hai câu, `mfa_pending`) — D-319.
    pub(crate) async fn own_account(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Option<OwnAccount>, Error> {
        use sea_orm::{ConnectionTrait as _, FromQueryResult as _, QueryTrait as _};
        let pending = Expr::case(
            crate::role::holds_mfa_role(tenant)
                .and(crate::role::has_second_factor_condition(tenant).not()),
            "x",
        )
        .finally(Expr::val(Option::<String>::None));
        let read = tenant.database().read();
        let statement = Entity::find_by_id(id)
            .scoped(tenant)
            .column_as(Into::<Expr>::into(pending), "mfa_pending")
            .build(read.get_database_backend());
        let Some(row) = read.query_one_raw(statement).await? else {
            return Ok(None);
        };
        let model = entity::Model::from_query_result(&row, "")?;
        let mfa_pending = row.try_get::<Option<String>>("", "mfa_pending")?.is_some();
        Ok(Some(OwnAccount {
            has_password: model.password_hash.is_some(),
            mfa_pending,
            user: User::from(model),
        }))
    }

    pub(crate) async fn facts(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, AccountFacts>, Error> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        use crate::mfa::entity::totp;
        // Một câu cho cả trang (D-319): cờ MFA là subquery tương quan theo `auth_user.id`, không
        // phải câu thứ hai. Mỗi cờ chỉ là `x` hay `NULL` — chạy như nhau trên SQLite và Postgres.
        let flag = |condition: Expr| -> Expr {
            Expr::case(condition, "x").finally(Expr::val(Option::<String>::None)).into()
        };
        let confirmed_totp = sea_orm::sea_query::Query::select()
            .expr(Expr::val(1))
            .from(totp::Entity)
            .and_where(Expr::col((totp::Entity, totp::Column::UserId)).equals((Entity, Column::Id)))
            .and_where(Expr::col((totp::Entity, totp::Column::TenantId)).eq(tenant.id().as_uuid()))
            .and_where(Expr::col((totp::Entity, totp::Column::ConfirmedAt)).is_not_null())
            .to_owned();
        // Chỉ cần biết có hay không: không đọc hash ra.
        let rows: Vec<FactsRow> = Entity::find()
            .select_only()
            .column(Column::Id)
            .column_as(
                flag(Expr::col((Entity, Column::PasswordHash)).is_not_null()),
                "has_password",
            )
            .column_as(flag(Expr::exists(confirmed_totp)), "mfa_enabled")
            .column(Column::LastSignInAt)
            .filter(Column::Id.is_in(ids.iter().copied()))
            .scoped(tenant)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        Ok(rows
            .into_iter()
            .map(|(id, password, mfa, last_sign_in_at)| {
                let facts = AccountFacts {
                    has_password: password.is_some(),
                    mfa_enabled: mfa.is_some(),
                    last_sign_in_at,
                };
                (id, facts)
            })
            .collect())
    }
}

impl UserService {
    /// Khoá tài khoản `id` trong `transaction` khi chủ của nó báo "không phải tôi" sau một lần
    /// đổi email (`email_report.rs`): kẻ chiếm đã biết mật khẩu (đổi email cần nó) và có thể đã
    /// gắn MFA hay passkey của nó. Bỏ mật khẩu (không đăng nhập bằng mật khẩu được nữa; chủ
    /// tài khoản đặt lại qua mail), thu hồi mọi phiên, refresh token, API key, passkey, mọi
    /// token xác minh email / đặt lại mật khẩu / link đăng nhập / xác nhận lại, liên kết nhà
    /// cung cấp ngoài, và tắt TOTP kèm recovery code.
    pub(crate) async fn lock_out_in(
        &self,
        tenant: &TenantContext,
        transaction: &DatabaseTransaction,
        actor: Actor,
        id: Uuid,
    ) -> Result<(), Error> {
        Entity::update_many()
            .col_expr(Column::PasswordHash, Expr::value(Option::<String>::None))
            .col_expr(Column::UpdatedAt, Expr::value(now()))
            .filter(Column::Id.eq(id))
            .scoped(tenant)
            .exec(transaction)
            .await?;
        SessionService::revoke_user(transaction, tenant, id).await?;
        SessionService::drop_mfa_challenges(transaction, tenant, id).await?;
        crate::passkey::drop_ceremonies_in(transaction, tenant, id).await?;
        crate::passkey::remove_all_in(transaction, tenant, &self.outbox, actor, id, false).await?;
        crate::api_key::revoke_all_in(transaction, tenant, &self.outbox, actor, id).await?;
        crate::mfa::service::remove_in(transaction, tenant, &self.outbox, actor, id, false).await?;
        let subject = id.to_string();
        // Link đăng nhập và token xác nhận lại đang chờ cũng là cửa vào của kẻ chiếm (link gửi tới
        // email của nó trước khi bị khoá); liên kết nhà cung cấp ngoài nó đã gắn cũng vậy — gỡ như
        // đặt lại mật khẩu kiểu `Recovery` (rà soát bảo mật 2026-11).
        for purpose in ["email_verification", "password_reset", "login_link", "reauth"] {
            OneTimeTokens::new("auth").revoke(transaction, tenant, purpose, &subject).await?;
        }
        self.forget_identities(transaction, tenant, actor, id).await?;
        revoke_durable_grants_in(transaction, tenant, &self.outbox, actor, id).await?;
        Ok(())
    }
}

/// Thu hồi các uỷ quyền BỀN tài khoản `id` đã tạo — thứ có sức mạnh đo bằng quyền hiện tại (hay
/// sau này) của tài khoản: token SCIM nó tạo, và vai "người cấu hình" của nhà cung cấp đăng nhập ngoài
/// (tự liên kết). Gọi ở mọi đường khoá hết cửa vào (`lock_out_in`, nhận lại tài khoản chưa xác minh)
/// và khi khoá tài khoản — không thì mở khoá lại làm chúng sống lại (D-20261125-durable-grants).
pub(crate) async fn revoke_durable_grants_in<C: sea_orm::ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    outbox: &autoxide_kernel::event::Outbox,
    actor: Actor,
    id: Uuid,
) -> Result<(), Error> {
    crate::scim::revoke_created_by_in(connection, tenant, outbox, actor, id).await?;
    crate::oidc::provider::forget_configurer_in(connection, tenant, outbox, actor, id).await
}

#[async_trait::async_trait]
impl Accounts for UserService {
    async fn create(&self, tenant: &TenantContext, account: NewAccount) -> Result<User, Error> {
        let input =
            NewUser { email: account.email, name: account.name, password: account.password };
        // Chỉ lệnh CLI dùng trait này: người vận hành máy chủ, tức "hệ thống".
        self.create_with_roles(tenant, Actor::system(), input, account.admin).await
    }
}

#[async_trait::async_trait]
impl Users for UserService {
    async fn find(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<User>, Error> {
        self.get(tenant, id).await
    }
}

#[async_trait::async_trait]
impl crate::api::TenantAdmins for UserService {
    async fn admins(&self, tenant: &TenantContext) -> Result<Vec<User>, Error> {
        let read = tenant.database().read();
        let ids = crate::role::active_admin_ids(read, tenant).await?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = Entity::find().scoped(tenant).filter(Column::Id.is_in(ids)).all(read).await?;
        let mut users: Vec<User> = rows.into_iter().map(User::from).collect();
        users.sort_unstable_by_key(|user| user.id);
        Ok(users)
    }
}

/// Ảnh chụp những gì audit cần biết về một người dùng (không có hash mật khẩu).
fn snapshot(row: &entity::Model) -> serde_json::Value {
    json!({
        "email": row.email,
        "name": row.name,
        "disabled_at": row.disabled_at,
        "locale": row.locale,
        "time_zone": row.time_zone,
    })
}

/// Ngôn ngữ ưa thích: mã BCP 47 mà máy chủ có bản dịch (`en`, `vi`), dạng chuẩn.
fn check_locale(raw: &str) -> Result<String, FieldError> {
    autoxide_kernel::i18n::canonical_locale(raw.trim())
        .and_then(|locale| autoxide_kernel::i18n::Language::from_tag(&locale))
        .map(|language| language.tag().to_owned())
        .ok_or_else(|| FieldError::new("locale", "invalid", "must be `en` or `vi`"))
}

/// Bây giờ, cắt tới micro giây: Postgres lưu tới micro giây, SQLite lưu nguyên.
/// Cắt trước để hai database trả về cùng một giá trị.
/// Email và tên viết thường, bỏ dấu, lưu ở `auth_user.search_text` để tìm (m0016, m0017):
/// "dat" tìm ra "Đạt".
pub(crate) fn search_text(email: &str, name: &str) -> String {
    autoxide_kernel::search::fold(&format!("{email} {name}"))
}

fn now() -> Timestamp {
    Timestamp::now()
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
            "must be a valid email address of at most {email_max_length} characters",
        )
        .with_param("email_max_length", EMAIL_MAX_LENGTH)
    })
}

fn check_name(name: &str) -> Option<FieldError> {
    let length = name.chars().count();
    (!(1..=NAME_MAX_LENGTH).contains(&length)).then(|| {
        FieldError::new("name", "length", "must be between 1 and {name_max_length} characters long")
            .with_param("name_max_length", NAME_MAX_LENGTH)
    })
}

/// Lỗi khi tài khoản đã bị khoá.
pub(crate) fn user_disabled() -> Error {
    Error::forbidden(
        "auth.user_disabled",
        "This account is disabled. Ask an administrator to enable it.",
    )
}

/// Khoá dòng người dùng `id` tới hết transaction (`rows::lock_one`, D-181).
async fn lock_row(
    transaction: &DatabaseTransaction,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<(), Error> {
    rows::lock_one(transaction, Entity::find_by_id(id).scoped(tenant)).await?;
    Ok(())
}

pub(crate) fn invalid_credentials() -> Error {
    Error::unauthenticated("auth.invalid_credentials", "The email or password is incorrect.")
}

/// Lỗi khi không có người dùng `id`.
pub(crate) fn user_not_found(id: Uuid) -> Error {
    Error::not_found("auth.user_not_found", "User `{id}` does not exist.").with_param("id", id)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod account_tests;

#[cfg(test)]
mod update_tests;
