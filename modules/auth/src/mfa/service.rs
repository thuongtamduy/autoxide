//! Service MFA: bật, xác nhận, kiểm và tắt TOTP.
//!
//! Tham số TOTP theo mặc định mà mọi ứng dụng authenticator hiểu: SHA-1, 6 chữ số,
//! bước 30 giây. Nhận mã của bước trước và bước sau (lệch đồng hồ tới 30 giây). Mỗi
//! mã chỉ nhận một lần: lưu bước của mã vừa dùng, chỉ nhận bước lớn hơn — kẻ nhìn
//! trộm một mã không dùng lại được nó (RFC 6238 mục 5.2).

use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::time::Timestamp;
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
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use serde_json::json;

/// Độ dài secret: 160 bit, như RFC 4226 khuyến nghị cho HMAC-SHA1.
const SECRET_BYTES: usize = 20;
const DIGITS: u8 = 6;
const STEP_SECONDS: u64 = 30;
/// Số bước lệch cho phép mỗi phía.
const SKEW: u16 = 1;
/// Mục đích khi mã hoá secret TOTP (`Sealer`, D-257): secret lưu đã mã hoá, gắn với khách và dòng
/// `auth_totp`; chỉ mở lúc kiểm mã.
pub(crate) const SECRET_PURPOSE: &str = "auth.totp_secret";

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
    /// Khoá gốc: mã hoá secret TOTP khi lưu (bước 33).
    secrets: SecretKey,
    issuer: String,
    // Đếm mã sai khi tắt TOTP / tạo lại recovery code, theo người dùng (D-079): người
    // cầm access token bị lộ không đoán mã vô hạn được.
    throttle: LoginThrottle,
    outbox: Outbox,
    /// Tên trong app xác thực theo khách (`auth.totp_issuer`).
    policy: crate::policy::TenantPolicy,
}

impl MfaService {
    pub(crate) fn new(
        secrets: SecretKey,
        issuer: String,
        throttle: LoginThrottle,
        outbox: Outbox,
    ) -> Self {
        MfaService { secrets, issuer, throttle, outbox, policy: Default::default() }
    }

    /// Đọc tên trong app xác thực theo khách.
    pub(crate) fn with_policy(mut self, policy: crate::policy::TenantPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Kiểm mã cho một thao tác của chính người dùng, có giới hạn số lần sai: xin lượt
    /// trước, mã đúng thì quên các lần sai.
    async fn check_code_limited(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        code: &str,
        allow_recovery: bool,
    ) -> Result<(), Error> {
        let key = format!("mfa:{user_id}");
        self.throttle.reserve(tenant, &key).await?;
        let valid = if allow_recovery {
            self.verify_second_factor(tenant, user_id, code).await?
        } else {
            self.verify_totp(tenant, user_id, code).await?
        };
        if !valid {
            return Err(invalid_code());
        }
        self.throttle.clear(tenant, &key).await
    }

    /// Xác nhận lại người đang đăng nhập bằng mã TOTP (D-272), thay cho mật khẩu ở thao tác nhạy
    /// cảm. Như mọi lần nhập mã: tính vào giới hạn sai, mã đã dùng không dùng lại được. Chưa bật
    /// TOTP thì báo `auth.mfa_not_enabled`.
    pub(crate) async fn reauthenticate(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        code: &str,
    ) -> Result<(), Error> {
        if !self.status(tenant, user_id).await?.totp_enabled {
            return Err(not_enabled());
        }
        // Lỗi "mã sai" của MFA chỉ ô `code`; mọi chỗ xác nhận lại (`/me/reauth`, tạo API key)
        // nhận mã ở ô `totp_code`, nên lỗi phải chỉ đúng ô client đã gửi.
        self.check_code_limited(tenant, user_id, code, false).await.map_err(|error| {
            if error.kind() == ErrorKind::Validation {
                let fields = error
                    .fields()
                    .iter()
                    .map(|field| {
                        let mut field = field.clone();
                        field.field = "totp_code".to_owned();
                        field
                    })
                    .collect();
                Error::validation(fields)
            } else {
                error
            }
        })
    }

    pub(crate) async fn status(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<MfaStatus, Error> {
        let row = self.row(tenant, user_id).await?;
        let remaining = recovery::remaining(tenant.database().write(), tenant, user_id).await?;
        Ok(MfaStatus {
            totp_enabled: row.is_some_and(|row| row.confirmed_at.is_some()),
            recovery_codes_remaining: remaining,
        })
    }

    /// Bắt đầu bật TOTP cho người dùng `user_id` (email `email`, hiện trong ứng dụng):
    /// sinh secret mới. Chưa có hiệu lực tới khi [`confirm_totp`](Self::confirm_totp).
    /// Gọi lại khi đang bật dở thì thay secret cũ.
    pub(crate) async fn start_totp(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        email: &str,
    ) -> Result<TotpSetup, Error> {
        let mut bytes = vec![0u8; SECRET_BYTES];
        getrandom::fill(&mut bytes)
            .map_err(|error| Error::internal(format!("random number generator: {error}")))?;
        let issuer = self.policy.totp_issuer(tenant, &self.issuer).await?;
        let totp = self.totp(bytes, email, issuer)?;
        let secret = totp.secret().to_base32();
        let otpauth_url = totp
            .to_url()
            .map_err(|error| Error::internal(format!("totp: cannot build the URL: {error:?}")))?;

        // Mã hoá TRƯỚC khi mở transaction ghi (đọc khoá gốc có thể chạm database).
        let id = Uuid::now_v7();
        let sealer = self.secrets.sealer().await?;
        let sealed =
            sealer.seal(tenant.id(), SECRET_PURPOSE, &id.to_string(), secret.as_bytes())?;
        let transaction = tenant.database().begin_write().await?;
        let existing = Entity::find()
            .filter(Column::UserId.eq(user_id))
            .scoped(tenant)
            .one(&transaction)
            .await?;
        if existing.as_ref().is_some_and(|row| row.confirmed_at.is_some()) {
            return Err(Error::conflict(
                "auth.mfa_already_enabled",
                "Two-step sign-in is already on. Turn it off first to set up a new device.",
            ));
        }
        Entity::delete_many()
            .filter(Column::UserId.eq(user_id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        totp::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            user_id: Set(user_id),
            secret: Set(sealed),
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
        tenant: &TenantContext,
        user_id: Uuid,
        code: &str,
    ) -> Result<Vec<String>, Error> {
        let row = self.row(tenant, user_id).await?.ok_or_else(setup_not_started)?;
        if row.confirmed_at.is_some() {
            return Err(Error::conflict(
                "auth.mfa_already_enabled",
                "Two-step sign-in is already on.",
            ));
        }
        let step =
            self.matching_step(tenant, &row, code, unix_now()).await?.ok_or_else(invalid_code)?;
        let now = now();
        let transaction = tenant.database().begin_write().await?;
        let confirmed = Entity::update_many()
            .col_expr(Column::ConfirmedAt, Expr::value(Some(now)))
            .col_expr(Column::LastUsedStep, Expr::value(Some(step)))
            .filter(Column::Id.eq(row.id))
            .filter(Column::ConfirmedAt.is_null())
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if confirmed.rows_affected == 0 {
            // Một request khác vừa xác nhận (hay bắt đầu lại) cùng lúc.
            transaction.rollback().await?;
            return Err(setup_not_started());
        }
        let codes = recovery::replace(&transaction, tenant, user_id, now).await?;
        let event = events::MfaEnabled { before: None, after: Some(json!({"method": "totp"})) };
        let subject = events::user_subject(user_id);
        self.outbox.record(&transaction, tenant, Actor::user(user_id), subject, &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, "two-step sign-in turned on");
        Ok(codes)
    }

    /// Kiểm lớp thứ hai: mã TOTP hoặc một recovery code chưa dùng. `true` nếu đúng;
    /// mã đúng thì không dùng lại được.
    pub(crate) async fn verify_second_factor(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        code: &str,
    ) -> Result<bool, Error> {
        if !self.status(tenant, user_id).await?.totp_enabled {
            return Ok(false);
        }
        if recovery::looks_like_code(code) {
            let write = tenant.database().write();
            let used = recovery::redeem(write, tenant, user_id, code, now()).await?;
            if used {
                tracing::warn!(user_id = %user_id, "recovery code used");
            }
            return Ok(used);
        }
        self.verify_totp(tenant, user_id, code).await
    }

    /// Thay bộ recovery code bằng bộ mới (mã cũ hết dùng được). Cần một mã TOTP: ai
    /// chỉ cầm recovery code không tự đẻ thêm được.
    pub(crate) async fn regenerate_recovery_codes(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        code: &str,
    ) -> Result<Vec<String>, Error> {
        if !self.status(tenant, user_id).await?.totp_enabled {
            return Err(not_enabled());
        }
        self.check_code_limited(tenant, user_id, code, false).await?;
        let transaction = tenant.database().begin_write().await?;
        let codes = recovery::replace(&transaction, tenant, user_id, now()).await?;
        let event = events::RecoveryCodesReplaced::default();
        let subject = events::user_subject(user_id);
        self.outbox.record(&transaction, tenant, Actor::user(user_id), subject, &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, "recovery codes replaced");
        Ok(codes)
    }

    /// Kiểm một mã TOTP của người dùng đã bật MFA. `true` nếu đúng và chưa dùng; mã
    /// đúng thì không dùng lại được nữa.
    pub(crate) async fn verify_totp(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        code: &str,
    ) -> Result<bool, Error> {
        self.verify_totp_at(tenant, user_id, code, unix_now()).await
    }

    async fn verify_totp_at(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        code: &str,
        now: u64,
    ) -> Result<bool, Error> {
        let row = self.row(tenant, user_id).await?;
        let Some(row) = row.filter(|row| row.confirmed_at.is_some()) else {
            return Ok(false);
        };
        let Some(step) = self.matching_step(tenant, &row, code, now).await? else {
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
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        Ok(used.rows_affected == 1)
    }

    /// Tắt TOTP, sau khi kiểm một mã TOTP hoặc recovery code. Dùng cho chính người dùng.
    pub(crate) async fn disable_totp(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        code: &str,
    ) -> Result<(), Error> {
        if !self.status(tenant, user_id).await?.totp_enabled {
            return Err(not_enabled());
        }
        self.check_code_limited(tenant, user_id, code, true).await?;
        self.remove(tenant, Actor::user(user_id), user_id, false).await
    }

    /// Xoá MFA của người dùng mà không cần mã — cho quản trị viên khi người dùng mất
    /// thiết bị. Chưa bật thì không có gì để xoá, vẫn coi là xong.
    pub(crate) async fn reset(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        user_id: Uuid,
    ) -> Result<(), Error> {
        self.remove(tenant, actor, user_id, true).await
    }

    async fn remove(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        user_id: Uuid,
        by_admin: bool,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        remove_in(&transaction, tenant, &self.outbox, actor, user_id, by_admin).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, "two-step sign-in turned off");
        Ok(())
    }

    async fn row(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Option<totp::Model>, Error> {
        // Kết nối ghi: vừa bật xong mà đọc từ read replica chậm thì như chưa bật.
        Ok(Entity::find()
            .filter(Column::UserId.eq(user_id))
            .scoped(tenant)
            .one(tenant.database().write())
            .await?)
    }

    /// Bước thời gian khớp với `code`, nếu có. Secret chỉ được mở (giải mã) ở đây.
    async fn matching_step(
        &self,
        tenant: &TenantContext,
        row: &totp::Model,
        code: &str,
        now: u64,
    ) -> Result<Option<i64>, Error> {
        let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        if code.len() != usize::from(DIGITS) || !code.chars().all(|c| c.is_ascii_digit()) {
            return Ok(None);
        }
        // Secret cũ chưa mã hoá (trước upgrade `u0001_seal_totp`) được `open` trả nguyên văn.
        let sealer = self.secrets.sealer().await?;
        let opened = sealer.open(tenant.id(), SECRET_PURPOSE, &row.id.to_string(), &row.secret)?;
        let base32 = std::str::from_utf8(&opened)
            .map_err(|_| Error::internal("totp: stored secret is not text"))?;
        let secret = Secret::try_from_base32(base32).map_err(|error| {
            Error::internal(format!("totp: stored secret is invalid: {error:?}"))
        })?;
        // Tên trong app không ảnh hưởng mã: dùng tên của máy chủ khi chỉ kiểm mã.
        let totp = self.totp(secret.as_bytes().to_vec(), "", self.issuer.clone())?;
        // `check` trả về số thứ tự của bước khớp.
        Ok(totp.check(&code, now).map(|step| i64::try_from(step).unwrap_or(i64::MAX)))
    }

    fn totp(&self, secret: Vec<u8>, email: &str, issuer: String) -> Result<Totp, Error> {
        // Dấu `:` ngăn cách tên hệ thống và tài khoản trong URL: không được có trong email.
        let account = email.replace(':', "_");
        Builder::new()
            .with_algorithm(Algorithm::SHA1)
            .with_digits(DIGITS)
            .with_step_duration(STEP_SECONDS)
            .with_skew(SKEW)
            .with_secret(secret)
            .with_account_name(account)
            .with_issuer(Some(issuer))
            .build()
            .map_err(|error| Error::internal(format!("totp: {error:?}")))
    }
}

/// Tắt TOTP và xoá recovery code của `user_id` trong `transaction` (kèm event
/// `auth.mfa_disabled` khi đã bật). Dùng chung cho tắt MFA và cho báo "không phải tôi" khi đổi
/// email (`email_report.rs`).
pub(crate) async fn remove_in(
    transaction: &DatabaseTransaction,
    tenant: &TenantContext,
    outbox: &Outbox,
    actor: Actor,
    user_id: Uuid,
    by_admin: bool,
) -> Result<(), Error> {
    let removed = Entity::delete_many()
        .filter(Column::UserId.eq(user_id))
        .scoped(tenant)
        .exec(transaction)
        .await?;
    recovery::remove(transaction, tenant, user_id).await?;
    // Chưa bật thì không có gì thay đổi: không ghi event.
    if removed.rows_affected > 0 {
        let event = events::MfaDisabled {
            before: Some(json!({"method": "totp"})),
            after: Some(json!({"by_administrator": by_admin})),
        };
        outbox.record(transaction, tenant, actor, events::user_subject(user_id), &event).await?;
    }
    Ok(())
}

fn unix_now() -> u64 {
    u64::try_from(Timestamp::now().as_unix_seconds()).unwrap_or(0)
}

fn now() -> Timestamp {
    Timestamp::now()
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
        "There is no two-step sign-in setup to confirm. Start the setup again.",
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

    async fn setup(test: &TestDatabase) -> (Arc<MfaService>, Uuid, TenantContext) {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let ada = NewAccount {
            email: "ada@example.com".to_owned(),
            name: "Ada".to_owned(),
            password: None,
            admin: false,
        };
        let ada = Accounts::create(services.users.as_ref(), &tenant, ada).await.unwrap();
        (services.mfa, ada.id, tenant)
    }

    #[tokio::test]
    async fn bat_xac_nhan_roi_tat() {
        TestDatabase::run(async |test| {
            let (mfa, user, tenant) = setup(test).await;

            let setup = mfa.start_totp(&tenant, user, "ada@example.com").await.unwrap();
            assert_eq!(setup.secret.len(), 32);
            assert!(setup.otpauth_url.starts_with("otpauth://totp/Autoxide:ada%40example.com?"));
            assert!(!mfa.status(&tenant, user).await.unwrap().totp_enabled);

            let wrong = mfa.confirm_totp(&tenant, user, "000000").await.unwrap_err();
            assert_eq!(wrong.fields()[0].field, "code");
            let code = code_at(&setup.secret, unix_now());
            mfa.confirm_totp(&tenant, user, &code).await.unwrap();
            assert!(mfa.status(&tenant, user).await.unwrap().totp_enabled);
            let again = mfa.start_totp(&tenant, user, "ada@example.com").await.unwrap_err();
            assert_eq!(again.code(), "auth.mfa_already_enabled");

            // Mã vừa dùng để xác nhận không tắt được (đã dùng); mã của bước sau thì được.
            let next = code_at(&setup.secret, unix_now() + STEP_SECONDS);
            assert!(mfa.disable_totp(&tenant, user, &code).await.is_err());
            mfa.disable_totp(&tenant, user, &next).await.unwrap();
            assert!(!mfa.status(&tenant, user).await.unwrap().totp_enabled);
            let off = mfa.disable_totp(&tenant, user, &next).await.unwrap_err();
            assert_eq!(off.code(), "auth.mfa_not_enabled");
        })
        .await
        .unwrap();
    }

    /// Đoán mã để tắt TOTP: sau 5 lần sai thì bị khoá, kể cả khi gửi mã đúng (D-079).
    #[tokio::test]
    async fn doan_ma_tat_totp_bi_khoa() {
        TestDatabase::run(async |test| {
            let (mfa, user, tenant) = setup(test).await;
            let setup = mfa.start_totp(&tenant, user, "ada@example.com").await.unwrap();
            let start = unix_now();
            mfa.confirm_totp(&tenant, user, &code_at(&setup.secret, start)).await.unwrap();

            for _ in 0..5 {
                let wrong = mfa.disable_totp(&tenant, user, "000000").await.unwrap_err();
                assert_eq!(wrong.fields()[0].field, "code");
            }
            let right = code_at(&setup.secret, start + STEP_SECONDS);
            let locked = mfa.disable_totp(&tenant, user, &right).await.unwrap_err();
            assert_eq!(locked.code(), "auth.login_throttled");
            let locked = mfa.regenerate_recovery_codes(&tenant, user, &right).await.unwrap_err();
            assert_eq!(locked.code(), "auth.login_throttled");
            assert!(mfa.status(&tenant, user).await.unwrap().totp_enabled);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn moi_ma_chi_dung_mot_lan_va_lech_toi_mot_buoc() {
        TestDatabase::run(async |test| {
            let (mfa, user, tenant) = setup(test).await;
            let setup = mfa.start_totp(&tenant, user, "ada@example.com").await.unwrap();
            let start = unix_now();
            mfa.confirm_totp(&tenant, user, &code_at(&setup.secret, start)).await.unwrap();
            let later = start + 10 * STEP_SECONDS;

            let current = code_at(&setup.secret, later);
            assert!(mfa.verify_totp_at(&tenant, user, &current, later).await.unwrap());
            assert!(!mfa.verify_totp_at(&tenant, user, &current, later).await.unwrap());
            // Mã cũ hơn mã vừa dùng cũng bị từ chối.
            let older = code_at(&setup.secret, later - STEP_SECONDS);
            assert!(!mfa.verify_totp_at(&tenant, user, &older, later).await.unwrap());
            // Mã của bước kế được nhận (đồng hồ điện thoại nhanh 30 giây).
            let ahead = code_at(&setup.secret, later + STEP_SECONDS);
            assert!(mfa.verify_totp_at(&tenant, user, &ahead, later).await.unwrap());
            // Lệch quá một bước thì không.
            let far = code_at(&setup.secret, later + 3 * STEP_SECONDS);
            assert!(!mfa.verify_totp_at(&tenant, user, &far, later).await.unwrap());
            assert!(!mfa.verify_totp_at(&tenant, user, "12345", later).await.unwrap());
        })
        .await
        .unwrap();
    }

    /// Nhiều request gửi CÙNG một mã cùng lúc: đúng một bên được nhận — trên cả hai
    /// database.
    #[tokio::test]
    async fn gui_cung_ma_dong_thoi_chi_mot_ben_duoc_nhan() {
        TestDatabase::run(async |test| {
            let (mfa, user, tenant) = setup(test).await;
            let setup = mfa.start_totp(&tenant, user, "ada@example.com").await.unwrap();
            let start = unix_now();
            mfa.confirm_totp(&tenant, user, &code_at(&setup.secret, start)).await.unwrap();
            let later = start + 10 * STEP_SECONDS;
            let code = code_at(&setup.secret, later);

            let attempts: Vec<_> = (0..6)
                .map(|_| {
                    let mfa = mfa.clone();
                    let code = code.clone();
                    let tenant = tenant.clone();
                    tokio::spawn(
                        async move { mfa.verify_totp_at(&tenant, user, &code, later).await },
                    )
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
            let (mfa, user, tenant) = setup(test).await;
            let setup = mfa.start_totp(&tenant, user, "ada@example.com").await.unwrap();
            mfa.confirm_totp(&tenant, user, &code_at(&setup.secret, unix_now())).await.unwrap();

            mfa.reset(&tenant, Actor::system(), user).await.unwrap();
            mfa.reset(&tenant, Actor::system(), user).await.unwrap();

            assert!(!mfa.status(&tenant, user).await.unwrap().totp_enabled);
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
    async fn enabled(
        test: &TestDatabase,
    ) -> (Arc<MfaService>, Uuid, String, Vec<String>, TenantContext) {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let ada = NewAccount {
            email: "ada@example.com".to_owned(),
            name: "Ada".to_owned(),
            password: None,
            admin: false,
        };
        let ada = Accounts::create(services.users.as_ref(), &tenant, ada).await.unwrap();
        let setup = services.mfa.start_totp(&tenant, ada.id, "ada@example.com").await.unwrap();
        let codes = services
            .mfa
            .confirm_totp(&tenant, ada.id, &code_at(&setup.secret, unix_now()))
            .await
            .unwrap();
        (services.mfa, ada.id, setup.secret, codes, tenant)
    }

    #[tokio::test]
    async fn moi_recovery_code_dung_mot_lan() {
        TestDatabase::run(async |test| {
            let (mfa, user, _, codes, tenant) = enabled(test).await;
            assert_eq!(codes.len(), recovery::COUNT);

            // Gõ hoa, có khoảng trắng vẫn nhận.
            let typed = format!(" {} ", codes[0].to_uppercase());
            assert!(mfa.verify_second_factor(&tenant, user, &typed).await.unwrap());
            assert!(!mfa.verify_second_factor(&tenant, user, &codes[0]).await.unwrap());
            assert!(!mfa.verify_second_factor(&tenant, user, "aaaa-aaaa-aaaa-aaaa").await.unwrap());
            assert_eq!(mfa.status(&tenant, user).await.unwrap().recovery_codes_remaining, 9);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn sinh_lai_thi_ma_cu_het_dung_duoc() {
        TestDatabase::run(async |test| {
            let (mfa, user, secret, old, tenant) = enabled(test).await;
            let next = code_at(&secret, unix_now() + STEP_SECONDS);

            // Recovery code không dùng để sinh lại được.
            let by_recovery =
                mfa.regenerate_recovery_codes(&tenant, user, &old[0]).await.unwrap_err();
            assert_eq!(by_recovery.code(), "validation_failed");
            let new = mfa.regenerate_recovery_codes(&tenant, user, &next).await.unwrap();

            assert_eq!(new.len(), recovery::COUNT);
            assert!(!mfa.verify_second_factor(&tenant, user, &old[1]).await.unwrap());
            assert!(mfa.verify_second_factor(&tenant, user, &new[1]).await.unwrap());
            mfa.reset(&tenant, Actor::system(), user).await.unwrap();
            assert_eq!(mfa.status(&tenant, user).await.unwrap().recovery_codes_remaining, 0);
        })
        .await
        .unwrap();
    }

    /// Nhiều request dùng CÙNG một recovery code cùng lúc: đúng một bên được nhận —
    /// trên cả hai database.
    #[tokio::test]
    async fn dung_cung_recovery_code_dong_thoi() {
        TestDatabase::run(async |test| {
            let (mfa, user, _, codes, tenant) = enabled(test).await;

            let attempts: Vec<_> = (0..6)
                .map(|_| {
                    let mfa = mfa.clone();
                    let code = codes[3].clone();
                    let tenant = tenant.clone();
                    tokio::spawn(
                        async move { mfa.verify_second_factor(&tenant, user, &code).await },
                    )
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
