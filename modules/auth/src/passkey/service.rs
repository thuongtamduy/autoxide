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
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, SqlErr};
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
        let state = self.take_ceremony(ceremony_id, Some(user_id)).await?;
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
            .record(&transaction, Actor::user(user_id), events::user_subject(user_id), &event)
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

    /// Xoá passkey `id` của người dùng `user_id`.
    pub(crate) async fn delete(&self, user_id: Uuid, id: Uuid) -> Result<(), Error> {
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
        let event = events::PasskeyRemoved { before: Some(json!({"name": row.name})), after: None };
        self.outbox
            .record(&transaction, Actor::user(user_id), events::user_subject(user_id), &event)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Xoá mọi passkey của người dùng `user_id` (vd khi chủ tài khoản lấy lại tài khoản bằng
    /// cách đặt lại mật khẩu). Trả số passkey đã xoá.
    pub(crate) async fn remove_all(&self, actor: Actor, user_id: Uuid) -> Result<u64, Error> {
        let transaction = self.database.begin_write().await?;
        let rows = passkey::Entity::find()
            .filter(passkey::Column::UserId.eq(user_id))
            .all(&transaction)
            .await?;
        for row in &rows {
            passkey::Entity::delete_by_id(row.id).exec(&transaction).await?;
            let event =
                events::PasskeyRemoved { before: Some(json!({"name": row.name})), after: None };
            self.outbox.record(&transaction, actor, events::user_subject(user_id), &event).await?;
        }
        transaction.commit().await?;
        Ok(rows.len() as u64)
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
    ) -> Result<Tokens, Error> {
        let engine = self.engine()?;
        // Đọc câu trả lời trước khi dùng nghi thức: JSON hỏng thì nghi thức còn dùng được.
        let assertion = engine.read_assertion(credential).map_err(|Rejected(reason)| {
            Error::validation(vec![FieldError::new("credential", "invalid", reason)])
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
        self.sessions.login_with_passkey(stored.user_id).await
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

    /// Lấy và xoá nghi thức `id` (của `user_id`, nếu là đăng ký). Xoá có điều kiện: nhiều
    /// request cùng dùng một nghi thức thì đúng một bên được.
    async fn take_ceremony(&self, id: Uuid, user_id: Option<Uuid>) -> Result<Vec<u8>, Error> {
        let row = ceremony::Entity::find_by_id(id)
            .one(self.database.write())
            .await?
            .filter(|row| row.user_id == user_id && row.expires_at > now())
            .ok_or_else(invalid_ceremony)?;
        let deleted = ceremony::Entity::delete_many()
            .filter(ceremony::Column::Id.eq(id))
            .exec(self.database.write())
            .await?;
        if deleted.rows_affected == 0 {
            return Err(invalid_ceremony());
        }
        Ok(row.state)
    }
}

fn check_name(name: &str) -> Result<String, Error> {
    let name = name.trim();
    let length = name.chars().count();
    if (1..=NAME_MAX_LENGTH).contains(&length) && !name.chars().any(char::is_control) {
        return Ok(name.to_owned());
    }
    Err(Error::validation(vec![FieldError::new(
        "name",
        "length",
        format!("must be 1 to {NAME_MAX_LENGTH} characters"),
    )]))
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
