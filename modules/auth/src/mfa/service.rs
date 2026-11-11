//! Service MFA: bật, xác nhận, kiểm và tắt TOTP.
//!
//! Tham số TOTP theo mặc định mà mọi ứng dụng authenticator hiểu: SHA-1, 6 chữ số,
//! bước 30 giây. Nhận mã của bước trước và bước sau (lệch đồng hồ tới 30 giây). Mỗi
//! mã chỉ nhận một lần: lưu bước của mã vừa dùng, chỉ nhận bước lớn hơn — kẻ nhìn
//! trộm một mã không dùng lại được nó (RFC 6238 mục 5.2).

use autoxide_kernel::database::Database;
use autoxide_kernel::error::{Error, FieldError};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, Condition, EntityTrait, QueryFilter, Set};
use totp_rs::{Algorithm, Builder, Secret, Totp};
use uuid::Uuid;

use super::entity::totp::{self, Column, Entity};

/// Độ dài secret: 160 bit, như RFC 4226 khuyến nghị cho HMAC-SHA1.
const SECRET_BYTES: usize = 20;
const DIGITS: u8 = 6;
const STEP_SECONDS: u64 = 30;
/// Số bước lệch cho phép mỗi phía.
const SKEW: u16 = 1;

/// Trạng thái MFA của một người dùng.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MfaStatus {
    /// TOTP đã bật và xác nhận.
    pub totp_enabled: bool,
}

/// Thứ người dùng nhập vào ứng dụng authenticator khi bắt đầu bật TOTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TotpSetup {
    /// Secret base32, để gõ tay.
    pub secret: String,
    /// Đường dẫn `otpauth://` (định dạng Key URI), để tạo mã QR.
    pub otpauth_url: String,
}

pub(crate) struct MfaService {
    database: Database,
    issuer: String,
}

impl MfaService {
    pub(crate) fn new(database: Database, issuer: String) -> Self {
        MfaService { database, issuer }
    }

    pub(crate) async fn status(&self, user_id: Uuid) -> Result<MfaStatus, Error> {
        let row = self.row(user_id).await?;
        Ok(MfaStatus { totp_enabled: row.is_some_and(|row| row.confirmed_at.is_some()) })
    }

    /// Bắt đầu bật TOTP cho người dùng `user_id` (email `email`, hiện trong ứng dụng):
    /// sinh secret mới. Chưa có hiệu lực tới khi [`confirm_totp`](Self::confirm_totp).
    /// Gọi lại khi đang bật dở thì thay secret cũ.
    pub(crate) async fn start_totp(&self, user_id: Uuid, email: &str) -> Result<TotpSetup, Error> {
        let mut bytes = vec![0u8; SECRET_BYTES];
        getrandom::fill(&mut bytes)
            .map_err(|error| Error::internal(format!("random number generator: {error}")))?;
        let totp = self.totp(bytes, email)?;
        let secret = totp.secret().to_base32();
        let otpauth_url = totp
            .to_url()
            .map_err(|error| Error::internal(format!("totp: cannot build the URL: {error:?}")))?;

        let transaction = self.database.begin_write().await?;
        let existing = Entity::find().filter(Column::UserId.eq(user_id)).one(&transaction).await?;
        if existing.as_ref().is_some_and(|row| row.confirmed_at.is_some()) {
            return Err(Error::conflict(
                "auth.mfa_already_enabled",
                "Two-step sign-in is already on. Turn it off first to set up a new device.",
            ));
        }
        Entity::delete_many().filter(Column::UserId.eq(user_id)).exec(&transaction).await?;
        totp::ActiveModel {
            id: Set(Uuid::now_v7()),
            user_id: Set(user_id),
            secret: Set(secret.clone()),
            confirmed_at: Set(None),
            last_used_step: Set(None),
            created_at: Set(now()),
        }
        .insert(&transaction)
        .await?;
        transaction.commit().await?;
        Ok(TotpSetup { secret, otpauth_url })
    }

    /// Xác nhận việc bật TOTP bằng một mã từ ứng dụng: chứng tỏ người dùng đã lưu
    /// secret đúng. Từ đây đăng nhập cần mã.
    pub(crate) async fn confirm_totp(&self, user_id: Uuid, code: &str) -> Result<(), Error> {
        let row = self.row(user_id).await?.ok_or_else(setup_not_started)?;
        if row.confirmed_at.is_some() {
            return Err(Error::conflict(
                "auth.mfa_already_enabled",
                "Two-step sign-in is already on.",
            ));
        }
        let step = self.matching_step(&row, code, unix_now())?.ok_or_else(invalid_code)?;
        let confirmed = Entity::update_many()
            .col_expr(Column::ConfirmedAt, Expr::value(Some(now())))
            .col_expr(Column::LastUsedStep, Expr::value(Some(step)))
            .filter(Column::Id.eq(row.id))
            .filter(Column::ConfirmedAt.is_null())
            .exec(self.database.write())
            .await?;
        if confirmed.rows_affected == 0 {
            // Một request khác vừa xác nhận (hay bắt đầu lại) cùng lúc.
            return Err(setup_not_started());
        }
        tracing::info!(user_id = %user_id, "two-step sign-in turned on");
        Ok(())
    }

    /// Kiểm một mã TOTP của người dùng đã bật MFA. `true` nếu đúng và chưa dùng; mã
    /// đúng thì không dùng lại được nữa.
    pub(crate) async fn verify_totp(&self, user_id: Uuid, code: &str) -> Result<bool, Error> {
        self.verify_totp_at(user_id, code, unix_now()).await
    }

    async fn verify_totp_at(&self, user_id: Uuid, code: &str, now: u64) -> Result<bool, Error> {
        let Some(row) = self.row(user_id).await?.filter(|row| row.confirmed_at.is_some()) else {
            return Ok(false);
        };
        let Some(step) = self.matching_step(&row, code, now)? else {
            return Ok(false);
        };
        // Có điều kiện: hai request cùng gửi một mã thì chỉ một bên được nhận.
        let used = Entity::update_many()
            .col_expr(Column::LastUsedStep, Expr::value(Some(step)))
            .filter(Column::Id.eq(row.id))
            .filter(
                Condition::any()
                    .add(Column::LastUsedStep.is_null())
                    .add(Column::LastUsedStep.lt(step)),
            )
            .exec(self.database.write())
            .await?;
        Ok(used.rows_affected == 1)
    }

    /// Tắt TOTP, sau khi kiểm một mã. Dùng cho chính người dùng.
    pub(crate) async fn disable_totp(&self, user_id: Uuid, code: &str) -> Result<(), Error> {
        if !self.status(user_id).await?.totp_enabled {
            return Err(not_enabled());
        }
        if !self.verify_totp(user_id, code).await? {
            return Err(invalid_code());
        }
        self.remove(user_id).await
    }

    /// Xoá MFA của người dùng mà không cần mã — cho quản trị viên khi người dùng mất
    /// thiết bị. Chưa bật thì không có gì để xoá, vẫn coi là xong.
    pub(crate) async fn reset(&self, user_id: Uuid) -> Result<(), Error> {
        self.remove(user_id).await
    }

    async fn remove(&self, user_id: Uuid) -> Result<(), Error> {
        Entity::delete_many()
            .filter(Column::UserId.eq(user_id))
            .exec(self.database.write())
            .await?;
        tracing::info!(user_id = %user_id, "two-step sign-in turned off");
        Ok(())
    }

    async fn row(&self, user_id: Uuid) -> Result<Option<totp::Model>, Error> {
        // Kết nối ghi: vừa bật xong mà đọc từ read replica chậm thì như chưa bật.
        Ok(Entity::find().filter(Column::UserId.eq(user_id)).one(self.database.write()).await?)
    }

    /// Bước thời gian khớp với `code`, nếu có.
    fn matching_step(&self, row: &totp::Model, code: &str, now: u64) -> Result<Option<i64>, Error> {
        let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        if code.len() != usize::from(DIGITS) || !code.chars().all(|c| c.is_ascii_digit()) {
            return Ok(None);
        }
        let secret = Secret::try_from_base32(&row.secret).map_err(|error| {
            Error::internal(format!("totp: stored secret is invalid: {error:?}"))
        })?;
        let totp = self.totp(secret.as_bytes().to_vec(), "")?;
        // `check` trả về số thứ tự của bước khớp.
        Ok(totp.check(&code, now).map(|step| i64::try_from(step).unwrap_or(i64::MAX)))
    }

    fn totp(&self, secret: Vec<u8>, email: &str) -> Result<Totp, Error> {
        // Dấu `:` ngăn cách tên hệ thống và tài khoản trong URL: không được có trong email.
        let account = email.replace(':', "_");
        Builder::new()
            .with_algorithm(Algorithm::SHA1)
            .with_digits(DIGITS)
            .with_step_duration(STEP_SECONDS)
            .with_skew(SKEW)
            .with_secret(secret)
            .with_account_name(account)
            .with_issuer(Some(self.issuer.clone()))
            .build()
            .map_err(|error| Error::internal(format!("totp: {error:?}")))
    }
}

fn unix_now() -> u64 {
    u64::try_from(Utc::now().timestamp()).unwrap_or(0)
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Mã sai, hết hạn hay đã dùng: một lỗi chung (không cho biết mã gần đúng tới đâu).
pub(crate) fn invalid_code() -> Error {
    Error::validation(vec![FieldError::new(
        "code",
        "invalid",
        "is not a current code from your authenticator app, or was already used; wait for the \
         next code and try again",
    )])
}

fn setup_not_started() -> Error {
    Error::conflict(
        "auth.mfa_setup_not_started",
        "There is no two-step sign-in setup to confirm. Start one with \
         `POST /api/v1/auth/me/mfa/totp`.",
    )
}

fn not_enabled() -> Error {
    Error::conflict("auth.mfa_not_enabled", "Two-step sign-in is not on.")
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use super::*;
    use autoxide_testkit::TestDatabase;

    use crate::api::{Accounts, NewAccount};

    /// Mã TOTP của `secret` (base32) tại thời điểm `unix` — như ứng dụng authenticator.
    pub(crate) fn code_at(secret: &str, unix: u64) -> String {
        let bytes = Secret::try_from_base32(secret).unwrap().as_bytes().to_vec();
        Builder::new().with_secret(bytes).build().unwrap().generate(unix).to_string()
    }

    async fn setup(test: &TestDatabase) -> (Arc<MfaService>, Uuid) {
        let services = crate::testing::services(test).await;
        let ada = NewAccount {
            email: "ada@example.com".to_owned(),
            name: "Ada".to_owned(),
            password: None,
            admin: false,
        };
        let ada = Accounts::create(services.users.as_ref(), ada).await.unwrap();
        (services.mfa, ada.id)
    }

    #[tokio::test]
    async fn bat_xac_nhan_roi_tat() {
        TestDatabase::run(async |test| {
            let (mfa, user) = setup(test).await;

            let setup = mfa.start_totp(user, "ada@example.com").await.unwrap();
            assert_eq!(setup.secret.len(), 32);
            assert!(setup.otpauth_url.starts_with("otpauth://totp/Autoxide:ada%40example.com?"));
            assert!(!mfa.status(user).await.unwrap().totp_enabled);

            let wrong = mfa.confirm_totp(user, "000000").await.unwrap_err();
            assert_eq!(wrong.fields()[0].field, "code");
            let code = code_at(&setup.secret, unix_now());
            mfa.confirm_totp(user, &code).await.unwrap();
            assert!(mfa.status(user).await.unwrap().totp_enabled);
            let again = mfa.start_totp(user, "ada@example.com").await.unwrap_err();
            assert_eq!(again.code(), "auth.mfa_already_enabled");

            // Mã vừa dùng để xác nhận không tắt được (đã dùng); mã của bước sau thì được.
            let next = code_at(&setup.secret, unix_now() + STEP_SECONDS);
            assert!(mfa.disable_totp(user, &code).await.is_err());
            mfa.disable_totp(user, &next).await.unwrap();
            assert!(!mfa.status(user).await.unwrap().totp_enabled);
            let off = mfa.disable_totp(user, &next).await.unwrap_err();
            assert_eq!(off.code(), "auth.mfa_not_enabled");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn moi_ma_chi_dung_mot_lan_va_lech_toi_mot_buoc() {
        TestDatabase::run(async |test| {
            let (mfa, user) = setup(test).await;
            let setup = mfa.start_totp(user, "ada@example.com").await.unwrap();
            let start = unix_now();
            mfa.confirm_totp(user, &code_at(&setup.secret, start)).await.unwrap();
            let later = start + 10 * STEP_SECONDS;

            let current = code_at(&setup.secret, later);
            assert!(mfa.verify_totp_at(user, &current, later).await.unwrap());
            assert!(!mfa.verify_totp_at(user, &current, later).await.unwrap());
            // Mã cũ hơn mã vừa dùng cũng bị từ chối.
            let older = code_at(&setup.secret, later - STEP_SECONDS);
            assert!(!mfa.verify_totp_at(user, &older, later).await.unwrap());
            // Mã của bước kế được nhận (đồng hồ điện thoại nhanh 30 giây).
            let ahead = code_at(&setup.secret, later + STEP_SECONDS);
            assert!(mfa.verify_totp_at(user, &ahead, later).await.unwrap());
            // Lệch quá một bước thì không.
            let far = code_at(&setup.secret, later + 3 * STEP_SECONDS);
            assert!(!mfa.verify_totp_at(user, &far, later).await.unwrap());
            assert!(!mfa.verify_totp_at(user, "12345", later).await.unwrap());
        })
        .await
        .unwrap();
    }

    /// Nhiều request gửi CÙNG một mã cùng lúc: đúng một bên được nhận — trên cả hai
    /// database.
    #[tokio::test]
    async fn gui_cung_ma_dong_thoi_chi_mot_ben_duoc_nhan() {
        TestDatabase::run(async |test| {
            let (mfa, user) = setup(test).await;
            let setup = mfa.start_totp(user, "ada@example.com").await.unwrap();
            let start = unix_now();
            mfa.confirm_totp(user, &code_at(&setup.secret, start)).await.unwrap();
            let later = start + 10 * STEP_SECONDS;
            let code = code_at(&setup.secret, later);

            let attempts: Vec<_> = (0..6)
                .map(|_| {
                    let mfa = mfa.clone();
                    let code = code.clone();
                    tokio::spawn(async move { mfa.verify_totp_at(user, &code, later).await })
                })
                .collect();
            let mut accepted = 0;
            for attempt in attempts {
                if attempt.await.unwrap().unwrap() {
                    accepted += 1;
                }
            }

            assert_eq!(accepted, 1);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn quan_tri_vien_xoa_mfa() {
        TestDatabase::run(async |test| {
            let (mfa, user) = setup(test).await;
            let setup = mfa.start_totp(user, "ada@example.com").await.unwrap();
            mfa.confirm_totp(user, &code_at(&setup.secret, unix_now())).await.unwrap();

            mfa.reset(user).await.unwrap();
            mfa.reset(user).await.unwrap();

            assert!(!mfa.status(user).await.unwrap().totp_enabled);
        })
        .await
        .unwrap();
    }
}
