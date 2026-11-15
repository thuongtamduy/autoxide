//! Service passkey: đăng ký, liệt kê, xoá passkey của người dùng; đăng nhập bằng passkey.
//!
//! Mỗi nghi thức (đăng ký hay đăng nhập) gồm hai request: bắt đầu (máy chủ tạo challenge,
//! lưu trạng thái vào `auth_passkey_ceremony`, trả tuỳ chọn cho trình duyệt) và hoàn tất
//! (trình duyệt gửi câu trả lời của authenticator). Trạng thái nằm trong database nên máy
//! chủ nào hoàn tất cũng được; mỗi nghi thức dùng đúng một lần (câu `DELETE` có điều kiện
//! quyết khi hai request cùng dùng).

use std::sync::Arc;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::TenantContext;
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set, SqlErr,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::entity::{ceremony, passkey};
use super::webauthn::{Engine, Rejected};
use crate::config::PasskeyConfig;
use crate::events;
use crate::session::{SessionService, Tokens};
use crate::user::UserService;

/// Nghi thức sống chừng này — bằng thời gian chờ thư viện đặt cho trình duyệt (5 phút).
const CEREMONY_MINUTES: i64 = 5;
/// Độ dài tối đa của tên passkey.
const NAME_MAX_LENGTH: usize = 100;
/// Mỗi người tối đa chừng này passkey.
const MAX_PASSKEYS: u64 = 20;

/// Nghi thức vừa bắt đầu: gửi `options` cho `navigator.credentials.create()` /
/// `.get()` của trình duyệt, giữ `id` để hoàn tất.
pub(crate) struct Ceremony {
    pub id: Uuid,
    pub options: Value,
}

pub(crate) struct PasskeyService {
    database: Database,
    users: Arc<UserService>,
    sessions: Arc<SessionService>,
    outbox: Outbox,
    /// `None`: passkey tắt (chưa cấu hình RP ID).
    engine: Option<Engine>,
}

impl PasskeyService {
    pub(crate) fn new(
        database: Database,
        users: Arc<UserService>,
        sessions: Arc<SessionService>,
        outbox: Outbox,
        config: Option<&PasskeyConfig>,
    ) -> Result<Self, Error> {
        let engine = config.map(Engine::new).transpose().map_err(|Rejected(reason)| {
            Error::internal(format!("passkey configuration: {reason}"))
        })?;
        Ok(PasskeyService { database, users, sessions, outbox, engine })
    }

    fn engine(&self) -> Result<&Engine, Error> {
        self.engine.as_ref().ok_or_else(|| {
            Error::forbidden(
                "auth.passkeys_disabled",
                "Passkey sign-in is not enabled on this server. Its operator turns it on by \
                 setting AUTOXIDE_AUTH_PASSKEY_RP_ID.",
            )
        })
    }

    /// Bắt đầu đăng ký một passkey cho người dùng `user_id` (đã đăng nhập).
    /// Đòi mật khẩu hiện tại: access token bị lộ không đủ để gắn passkey của kẻ gian vào
    /// tài khoản (D-079).
    pub(crate) async fn start_registration(
        &self,
        user_id: Uuid,
        current_password: &str,
    ) -> Result<Ceremony, Error> {
        let engine = self.engine()?;
        self.sessions.check_password(user_id, current_password).await?;
        let user =
            self.users.get(user_id).await?.ok_or_else(|| crate::user::user_not_found(user_id))?;
        let existing = passkey::Entity::find()
            .filter(passkey::Column::UserId.eq(user_id))
            .all(self.database.read())
            .await?;
        if existing.len() as u64 >= MAX_PASSKEYS {
            return Err(Error::conflict(
                "auth.passkey_limit",
                "You already have {limit} passkeys. Remove one you no longer use first.",
            )
            .with_param("limit", MAX_PASSKEYS));
        }
        // Mọi passkey của một người dùng chung một user handle.
        let handle = existing
            .first()
            .map(|row| row.user_handle.clone())
            .unwrap_or_else(Engine::new_user_handle);
        let ids: Vec<Vec<u8>> = existing.into_iter().map(|row| row.credential_id).collect();
        let started = engine.start_registration(&handle, &user.email, &user.name, &ids).map_err(
            |Rejected(reason)| Error::internal(format!("passkey registration: {reason}")),
        )?;
        self.save_ceremony(Some(user_id), started.state, started.options).await
    }

    /// Hoàn tất đăng ký: kiểm câu trả lời `credential` rồi lưu passkey tên `name`.
    pub(crate) async fn finish_registration(
        &self,
        user_id: Uuid,
        ceremony_id: Uuid,
        name: &str,
        credential: &Value,
    ) -> Result<super::Passkey, Error> {
        let engine = self.engine()?;
        let name = check_name(name)?;
        // Đọc nghi thức, kiểm câu trả lời, rồi dùng nghi thức và lưu passkey trong CÙNG một
        // transaction: đổi / đặt lại mật khẩu xoá nghi thức đang dở trong transaction của nó,
        // nên passkey mở bằng mật khẩu cũ không lọt vào sau khi mật khẩu đã đổi.
        let state = self.read_ceremony(ceremony_id, Some(user_id)).await?;
        let registered =
            engine.finish_registration(&state, credential).map_err(|Rejected(reason)| {
                Error::bad_request(
                    "auth.passkey_rejected",
                    "The passkey was not accepted: {reason}. Start again with `POST \
                 /api/v1/auth/me/passkeys/registration`.",
                )
                .with_param("reason", reason)
            })?;
        let transaction = self.database.begin_write().await?;
        if !take_ceremony_in(&transaction, ceremony_id).await? {
            transaction.rollback().await?;
            return Err(invalid_ceremony());
        }
        let row = passkey::ActiveModel {
            id: Set(Uuid::now_v7()),
            user_id: Set(user_id),
            credential_id: Set(registered.credential_id),
            user_handle: Set(registered.user_handle),
            static_state: Set(registered.static_state),
            dynamic_state: Set(registered.dynamic_state),
            name: Set(name),
            created_at: Set(now()),
            last_used_at: Set(None),
        }
        .insert(&transaction)
        .await
        .map_err(|error| match error.sql_err() {
            Some(SqlErr::UniqueConstraintViolation(_)) => Error::conflict(
                "auth.passkey_already_registered",
                "This passkey is already registered.",
            ),
            _ => Error::from(error),
        })?;
        let event = events::PasskeyAdded { before: None, after: Some(json!({"name": row.name})) };
        self.outbox
            .record(
                &transaction,
                &TenantContext::legacy_default(&self.database),
                Actor::user(user_id),
                events::user_subject(user_id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, passkey_id = %row.id, "passkey added");
        Ok(row)
    }

    /// Passkey của người dùng `user_id`, mới thêm trước (mặc định).
    pub(crate) async fn list(
        &self,
        user_id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<super::Passkey>, Error> {
        let select = passkey::Entity::find().filter(passkey::Column::UserId.eq(user_id));
        fetch_page(select, passkey::Column::Id, |row| row.id, page, self.database.read()).await
    }

    /// Xoá passkey `id` của người dùng `user_id` (chính người đó gỡ).
    pub(crate) async fn delete(&self, user_id: Uuid, id: Uuid) -> Result<(), Error> {
        self.remove(Actor::user(user_id), user_id, id, false).await
    }

    /// Quản trị viên (`actor`) gỡ passkey `id` của người dùng `user_id`; event ghi
    /// `by_administrator`. Bên gọi đã kiểm permission và `require_covers`.
    pub(crate) async fn remove_for(
        &self,
        actor: Actor,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<(), Error> {
        self.remove(actor, user_id, id, true).await
    }

    /// Quản trị viên (hay người vận hành qua CLI) gỡ mọi passkey của `user_id`. Trả số passkey
    /// đã gỡ.
    pub(crate) async fn remove_all_for(&self, actor: Actor, user_id: Uuid) -> Result<u64, Error> {
        let transaction = self.database.begin_write().await?;
        let removed = remove_all_in(
            &transaction,
            &TenantContext::legacy_default(&self.database),
            &self.outbox,
            actor,
            user_id,
            true,
        )
        .await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, removed, "passkeys removed by an administrator");
        Ok(removed)
    }

    async fn remove(
        &self,
        actor: Actor,
        user_id: Uuid,
        id: Uuid,
        by_admin: bool,
    ) -> Result<(), Error> {
        let transaction = self.database.begin_write().await?;
        let row = passkey::Entity::find_by_id(id)
            .one(&transaction)
            .await?
            .filter(|row| row.user_id == user_id)
            .ok_or_else(|| passkey_not_found(id))?;
        let deleted = passkey::Entity::delete_by_id(id).exec(&transaction).await?;
        if deleted.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(passkey_not_found(id));
        }
        let event = removed_event(&row.name, by_admin);
        self.outbox
            .record(
                &transaction,
                &TenantContext::legacy_default(&self.database),
                actor,
                events::user_subject(user_id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Bắt đầu đăng nhập bằng passkey (chưa biết là ai).
    pub(crate) async fn start_login(&self) -> Result<Ceremony, Error> {
        let started = self
            .engine()?
            .start_login()
            .map_err(|Rejected(reason)| Error::internal(format!("passkey sign-in: {reason}")))?;
        self.save_ceremony(None, started.state, started.options).await
    }

    /// Hoàn tất đăng nhập: kiểm câu trả lời `credential`, cấp cặp token như đăng nhập
    /// bằng mật khẩu. Passkey đã xác minh người dùng (vân tay, PIN…) nên không hỏi thêm
    /// mã TOTP.
    pub(crate) async fn finish_login(
        &self,
        ceremony_id: Uuid,
        credential: &Value,
        user_agent: Option<&str>,
    ) -> Result<Tokens, Error> {
        let engine = self.engine()?;
        // Đọc câu trả lời trước khi dùng nghi thức: JSON hỏng thì nghi thức còn dùng được.
        let assertion = engine.read_assertion(credential).map_err(|Rejected(reason)| {
            Error::validation(vec![
                FieldError::new("credential", "invalid", "is not valid: {reason}")
                    .with_param("reason", reason),
            ])
        })?;
        let state = self.take_ceremony(ceremony_id, None).await?;
        let stored = passkey::Entity::find()
            .filter(passkey::Column::CredentialId.eq(assertion.credential_id()))
            .one(self.database.read())
            .await?;
        let Some(stored) = stored else {
            tracing::info!("passkey sign-in with an unknown credential");
            return Err(sign_in_failed());
        };
        let verified = engine.finish_login(
            &state,
            &assertion,
            &stored.user_handle,
            &stored.static_state,
            &stored.dynamic_state,
        );
        let dynamic = match verified {
            Ok(dynamic) => dynamic,
            Err(Rejected(reason)) => {
                tracing::info!(passkey_id = %stored.id, reason, "passkey sign-in rejected");
                return Err(sign_in_failed());
            }
        };
        let mut update = passkey::Entity::update_many()
            .col_expr(passkey::Column::LastUsedAt, Expr::value(Some(now())))
            .filter(passkey::Column::Id.eq(stored.id));
        if let Some(dynamic) = dynamic {
            update = update.col_expr(passkey::Column::DynamicState, Expr::value(dynamic));
        }
        update.exec(self.database.write()).await?;
        self.sessions.login_with_passkey(stored.user_id, stored.id, user_agent).await
    }

    async fn save_ceremony(
        &self,
        user_id: Option<Uuid>,
        state: Vec<u8>,
        options: Value,
    ) -> Result<Ceremony, Error> {
        let id = Uuid::now_v7();
        ceremony::ActiveModel {
            id: Set(id),
            user_id: Set(user_id),
            state: Set(state),
            expires_at: Set(now() + chrono::Duration::minutes(CEREMONY_MINUTES)),
        }
        .insert(self.database.write())
        .await?;
        Ok(Ceremony { id, options })
    }

    /// Trạng thái của nghi thức `id` (của `user_id`, nếu là đăng ký), còn hạn; chưa dùng nó.
    async fn read_ceremony(&self, id: Uuid, user_id: Option<Uuid>) -> Result<Vec<u8>, Error> {
        let row = ceremony::Entity::find_by_id(id)
            .one(self.database.write())
            .await?
            .filter(|row| row.user_id == user_id && row.expires_at > now())
            .ok_or_else(invalid_ceremony)?;
        Ok(row.state)
    }

    /// Lấy và xoá nghi thức `id` (của `user_id`, nếu là đăng ký). Xoá có điều kiện: nhiều
    /// request cùng dùng một nghi thức thì đúng một bên được.
    async fn take_ceremony(&self, id: Uuid, user_id: Option<Uuid>) -> Result<Vec<u8>, Error> {
        let state = self.read_ceremony(id, user_id).await?;
        if !take_ceremony_in(self.database.write(), id).await? {
            return Err(invalid_ceremony());
        }
        Ok(state)
    }
}

/// Xoá nghi thức `id` trong `connection`; `false` nếu request khác đã dùng (hay đã bị xoá).
async fn take_ceremony_in<C: ConnectionTrait>(connection: &C, id: Uuid) -> Result<bool, Error> {
    let deleted = ceremony::Entity::delete_many()
        .filter(ceremony::Column::Id.eq(id))
        .exec(connection)
        .await?;
    Ok(deleted.rows_affected == 1)
}

fn check_name(name: &str) -> Result<String, Error> {
    let name = name.trim();
    let length = name.chars().count();
    if (1..=NAME_MAX_LENGTH).contains(&length) && !name.chars().any(char::is_control) {
        return Ok(name.to_owned());
    }
    Err(Error::validation(vec![
        FieldError::new("name", "length", "must be 1 to {name_max_length} characters")
            .with_param("name_max_length", NAME_MAX_LENGTH),
    ]))
}

fn invalid_ceremony() -> Error {
    Error::bad_request(
        "auth.invalid_passkey_ceremony",
        "This passkey ceremony is unknown, expired (after 5 minutes) or already used. Start a new \
         one.",
    )
}

fn sign_in_failed() -> Error {
    Error::unauthenticated(
        "auth.passkey_sign_in_failed",
        "Signing in with this passkey failed: it is not registered here, belongs to another \
         account, or the response is not valid. Sign in with your password, or add the passkey \
         again.",
    )
}

fn passkey_not_found(id: Uuid) -> Error {
    Error::not_found("auth.passkey_not_found", "You have no passkey `{id}`.").with_param("id", id)
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Xoá mọi passkey của người dùng `user_id` trong `connection` (transaction của bên gọi, vd đặt
/// lại mật khẩu — chủ tài khoản lấy lại tài khoản thì mọi cửa khác đóng cùng lúc). Mỗi passkey
/// một event. Trả số passkey đã xoá.
pub(crate) async fn remove_all_in<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    outbox: &Outbox,
    actor: Actor,
    user_id: Uuid,
    by_admin: bool,
) -> Result<u64, Error> {
    let rows =
        passkey::Entity::find().filter(passkey::Column::UserId.eq(user_id)).all(connection).await?;
    for row in &rows {
        passkey::Entity::delete_by_id(row.id).exec(connection).await?;
        let event = removed_event(&row.name, by_admin);
        outbox.record(connection, tenant, actor, events::user_subject(user_id), &event).await?;
    }
    Ok(rows.len() as u64)
}

/// Event `auth.passkey_removed`; quản trị viên gỡ thì `after.by_administrator` (như
/// `auth.mfa_disabled`, D-159).
fn removed_event(name: &str, by_admin: bool) -> events::PasskeyRemoved {
    events::PasskeyRemoved {
        before: Some(json!({"name": name})),
        after: by_admin.then(|| json!({"by_administrator": true})),
    }
}

/// Bỏ mọi nghi thức đăng ký passkey đang dở của `user_id` trong `connection`: đổi mật khẩu thì
/// nghi thức mở bằng mật khẩu cũ hết dùng được.
pub(crate) async fn drop_ceremonies_in<C: ConnectionTrait>(
    connection: &C,
    user_id: Uuid,
) -> Result<(), Error> {
    ceremony::Entity::delete_many()
        .filter(ceremony::Column::UserId.eq(user_id))
        .exec(connection)
        .await?;
    Ok(())
}

/// Passkey `id` của `user_id` còn đó không, trong `connection` (transaction cấp phiên): đặt lại
/// mật khẩu xoá passkey trong transaction của nó, nên passkey vừa bị xoá không mở được phiên.
pub(crate) async fn still_registered_in<C: ConnectionTrait>(
    connection: &C,
    id: Uuid,
    user_id: Uuid,
) -> Result<bool, Error> {
    let touched = passkey::Entity::update_many()
        .col_expr(passkey::Column::LastUsedAt, Expr::value(Some(now())))
        .filter(passkey::Column::Id.eq(id))
        .filter(passkey::Column::UserId.eq(user_id))
        .exec(connection)
        .await?;
    Ok(touched.rows_affected == 1)
}
