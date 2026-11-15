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
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, DatabaseTransaction, EntityTrait, QueryFilter, Set,
};
use totp_rs::{Algorithm, Builder, Secret, Totp};
use uuid::Uuid;

use super::entity::totp::{self, Column, Entity};
use super::recovery;
use crate::events;
use crate::session::LoginThrottle;
use autoxide_kernel::event::{Actor, Outbox};
use serde_json::json;

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
    /// Số recovery code chưa dùng.
    pub recovery_codes_remaining: u64,
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
    // Đếm mã sai khi tắt TOTP / tạo lại recovery code, theo người dùng (D-079): người
    // cầm access token bị lộ không đoán mã vô hạn được.
    throttle: LoginThrottle,
    outbox: Outbox,
}

impl MfaService {
    pub(crate) fn new(
        database: Database,
        issuer: String,
        throttle: LoginThrottle,
        outbox: Outbox,
    ) -> Self {
        MfaService { database, issuer, throttle, outbox }
    }

    /// Kiểm mã cho một thao tác của chính người dùng, có giới hạn số lần sai: xin lượt
    /// trước, mã đúng thì quên các lần sai.
    async fn check_code_limited(
        &self,
        user_id: Uuid,
        code: &str,
        allow_recovery: bool,
    ) -> Result<(), Error> {
        let key = format!("mfa:{user_id}");
        self.throttle.reserve(&key).await?;
        let valid = if allow_recovery {
            self.verify_second_factor(user_id, code).await?
        } else {
            self.verify_totp(user_id, code).await?
        };
        if !valid {
            return Err(invalid_code());
        }
        self.throttle.clear(&key).await
    }

    pub(crate) async fn status(&self, user_id: Uuid) -> Result<MfaStatus, Error> {
        let row = self.row(user_id).await?;
        let remaining = recovery::remaining(self.database.write(), user_id).await?;
        Ok(MfaStatus {
            totp_enabled: row.is_some_and(|row| row.confirmed_at.is_some()),
            recovery_codes_remaining: remaining,
        })
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
    /// secret đúng. Từ đây đăng nhập cần mã. Trả về bộ recovery code (chỉ lần này).
    pub(crate) async fn confirm_totp(
        &self,
        user_id: Uuid,
        code: &str,
    ) -> Result<Vec<String>, Error> {
        let row = self.row(user_id).await?.ok_or_else(setup_not_started)?;
        if row.confirmed_at.is_some() {
            return Err(Error::conflict(
                "auth.mfa_already_enabled",
                "Two-step sign-in is already on.",
            ));
        }
        let step = self.matching_step(&row, code, unix_now())?.ok_or_else(invalid_code)?;
        let now = now();
        let transaction = self.database.begin_write().await?;
        let confirmed = Entity::update_many()
            .col_expr(Column::ConfirmedAt, Expr::value(Some(now)))
            .col_expr(Column::LastUsedStep, Expr::value(Some(step)))
            .filter(Column::Id.eq(row.id))
            .filter(Column::ConfirmedAt.is_null())
            .exec(&transaction)
            .await?;
        if confirmed.rows_affected == 0 {
            // Một request khác vừa xác nhận (hay bắt đầu lại) cùng lúc.
            transaction.rollback().await?;
            return Err(setup_not_started());
        }
        let codes = recovery::replace(&transaction, user_id, now).await?;
        let event = events::MfaEnabled { before: None, after: Some(json!({"method": "totp"})) };
        let subject = events::user_subject(user_id);
        self.outbox.record(&transaction, Actor::user(user_id), subject, &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, "two-step sign-in turned on");
        Ok(codes)
    }

    /// Kiểm lớp thứ hai: mã TOTP hoặc một recovery code chưa dùng. `true` nếu đúng;
    /// mã đúng thì không dùng lại được.
    pub(crate) async fn verify_second_factor(
        &self,
        user_id: Uuid,
        code: &str,
    ) -> Result<bool, Error> {
        if !self.status(user_id).await?.totp_enabled {
            return Ok(false);
        }
        if recovery::looks_like_code(code) {
            let used = recovery::redeem(self.database.write(), user_id, code, now()).await?;
            if used {
                tracing::warn!(user_id = %user_id, "recovery code used");
            }
            return Ok(used);
        }
        self.verify_totp(user_id, code).await
    }

    /// Thay bộ recovery code bằng bộ mới (mã cũ hết dùng được). Cần một mã TOTP: ai
    /// chỉ cầm recovery code không tự đẻ thêm được.
    pub(crate) async fn regenerate_recovery_codes(
        &self,
        user_id: Uuid,
        code: &str,
    ) -> Result<Vec<String>, Error> {
        if !self.status(user_id).await?.totp_enabled {
            return Err(not_enabled());
        }
        self.check_code_limited(user_id, code, false).await?;
        let transaction = self.database.begin_write().await?;
        let codes = recovery::replace(&transaction, user_id, now()).await?;
        let event = events::RecoveryCodesReplaced::default();
        let subject = events::user_subject(user_id);
        self.outbox.record(&transaction, Actor::user(user_id), subject, &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, "recovery codes replaced");
        Ok(codes)
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

    /// Tắt TOTP, sau khi kiểm một mã TOTP hoặc recovery code. Dùng cho chính người dùng.
    pub(crate) async fn disable_totp(&self, user_id: Uuid, code: &str) -> Result<(), Error> {
        if !self.status(user_id).await?.totp_enabled {
            return Err(not_enabled());
        }
        self.check_code_limited(user_id, code, true).await?;
        self.remove(Actor::user(user_id), user_id, false).await
    }

    /// Xoá MFA của người dùng mà không cần mã — cho quản trị viên khi người dùng mất
    /// thiết bị. Chưa bật thì không có gì để xoá, vẫn coi là xong.
    pub(crate) async fn reset(&self, actor: Actor, user_id: Uuid) -> Result<(), Error> {
        self.remove(actor, user_id, true).await
    }

    async fn remove(&self, actor: Actor, user_id: Uuid, by_admin: bool) -> Result<(), Error> {
        let transaction = self.database.begin_write().await?;
        remove_in(&transaction, &self.outbox, actor, user_id, by_admin).await?;
        transaction.commit().await?;
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

/// Tắt TOTP và xoá recovery code của `user_id` trong `transaction` (kèm event
/// `auth.mfa_disabled` khi đã bật). Dùng chung cho tắt MFA và cho báo "không phải tôi" khi đổi
/// email (`email_report.rs`).
pub(crate) async fn remove_in(
    transaction: &DatabaseTransaction,
    outbox: &Outbox,
    actor: Actor,
    user_id: Uuid,
    by_admin: bool,
) -> Result<(), Error> {
    let removed =
        Entity::delete_many().filter(Column::UserId.eq(user_id)).exec(transaction).await?;
    recovery::remove(transaction, user_id).await?;
    // Chưa bật thì không có gì thay đổi: không ghi event.
    if removed.rows_affected > 0 {
        let event = events::MfaDisabled {
            before: Some(json!({"method": "totp"})),
            after: Some(json!({"by_administrator": by_admin})),
        };
        outbox.record(transaction, actor, events::user_subject(user_id), &event).await?;
    }
    Ok(())
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
        "is not a current code from your authenticator app or an unused recovery code, or was \
         already used; wait for the next code and try again",
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

    /// Đoán mã để tắt TOTP: sau 5 lần sai thì bị khoá, kể cả khi gửi mã đúng (D-079).
    #[tokio::test]
    async fn doan_ma_tat_totp_bi_khoa() {
        TestDatabase::run(async |test| {
            let (mfa, user) = setup(test).await;
            let setup = mfa.start_totp(user, "ada@example.com").await.unwrap();
            let start = unix_now();
            mfa.confirm_totp(user, &code_at(&setup.secret, start)).await.unwrap();

            for _ in 0..5 {
                let wrong = mfa.disable_totp(user, "000000").await.unwrap_err();
                assert_eq!(wrong.fields()[0].field, "code");
            }
            let right = code_at(&setup.secret, start + STEP_SECONDS);
            let locked = mfa.disable_totp(user, &right).await.unwrap_err();
            assert_eq!(locked.code(), "auth.login_throttled");
            let locked = mfa.regenerate_recovery_codes(user, &right).await.unwrap_err();
            assert_eq!(locked.code(), "auth.login_throttled");
            assert!(mfa.status(user).await.unwrap().totp_enabled);
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

            mfa.reset(Actor::system(), user).await.unwrap();
            mfa.reset(Actor::system(), user).await.unwrap();

            assert!(!mfa.status(user).await.unwrap().totp_enabled);
        })
        .await
        .unwrap();
    }
}

#[cfg(test)]
mod recovery_tests {
    use std::sync::Arc;

    use super::tests::code_at;
    use super::*;
    use autoxide_testkit::TestDatabase;

    use crate::api::{Accounts, NewAccount};

    /// Người dùng đã bật MFA; trả về service, ID, secret và bộ recovery code.
    async fn enabled(test: &TestDatabase) -> (Arc<MfaService>, Uuid, String, Vec<String>) {
        let services = crate::testing::services(test).await;
        let ada = NewAccount {
            email: "ada@example.com".to_owned(),
            name: "Ada".to_owned(),
            password: None,
            admin: false,
        };
        let ada = Accounts::create(services.users.as_ref(), ada).await.unwrap();
        let setup = services.mfa.start_totp(ada.id, "ada@example.com").await.unwrap();
        let codes =
            services.mfa.confirm_totp(ada.id, &code_at(&setup.secret, unix_now())).await.unwrap();
        (services.mfa, ada.id, setup.secret, codes)
    }

    #[tokio::test]
    async fn moi_recovery_code_dung_mot_lan() {
        TestDatabase::run(async |test| {
            let (mfa, user, _, codes) = enabled(test).await;
            assert_eq!(codes.len(), recovery::COUNT);

            // Gõ hoa, có khoảng trắng vẫn nhận.
            let typed = format!(" {} ", codes[0].to_uppercase());
            assert!(mfa.verify_second_factor(user, &typed).await.unwrap());
            assert!(!mfa.verify_second_factor(user, &codes[0]).await.unwrap());
            assert!(!mfa.verify_second_factor(user, "aaaa-aaaa-aaaa-aaaa").await.unwrap());
            assert_eq!(mfa.status(user).await.unwrap().recovery_codes_remaining, 9);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn sinh_lai_thi_ma_cu_het_dung_duoc() {
        TestDatabase::run(async |test| {
            let (mfa, user, secret, old) = enabled(test).await;
            let next = code_at(&secret, unix_now() + STEP_SECONDS);

            // Recovery code không dùng để sinh lại được.
            let by_recovery = mfa.regenerate_recovery_codes(user, &old[0]).await.unwrap_err();
            assert_eq!(by_recovery.code(), "validation_failed");
            let new = mfa.regenerate_recovery_codes(user, &next).await.unwrap();

            assert_eq!(new.len(), recovery::COUNT);
            assert!(!mfa.verify_second_factor(user, &old[1]).await.unwrap());
            assert!(mfa.verify_second_factor(user, &new[1]).await.unwrap());
            mfa.reset(Actor::system(), user).await.unwrap();
            assert_eq!(mfa.status(user).await.unwrap().recovery_codes_remaining, 0);
        })
        .await
        .unwrap();
    }

    /// Nhiều request dùng CÙNG một recovery code cùng lúc: đúng một bên được nhận —
    /// trên cả hai database.
    #[tokio::test]
    async fn dung_cung_recovery_code_dong_thoi() {
        TestDatabase::run(async |test| {
            let (mfa, user, _, codes) = enabled(test).await;

            let attempts: Vec<_> = (0..6)
                .map(|_| {
                    let mfa = mfa.clone();
                    let code = codes[3].clone();
                    tokio::spawn(async move { mfa.verify_second_factor(user, &code).await })
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
}
