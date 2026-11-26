//! Mã một lần qua SMS cho **đăng nhập** bằng số điện thoại (bước 47, D-20261120-phone-accounts): cài
//! `autoxide_auth::api::PhoneCodes` và cắm vào `auth` (đăng ký trước) qua `AuthExtensions`.
//!
//! Dùng chung bảng mã, cách sinh / băm mã, quy tắc số được nhận, giới hạn gửi và bên gửi SMS với xác
//! thực số của người đã đăng nhập ([`crate::verification`]). Khác: chưa có tài khoản nên mã có
//! `user_id` = UUID rỗng (chỉ đọc được qua ID mã, không lộ qua `/me/phone-verifications`), không giới
//! hạn theo người, và việc gắn số vào tài khoản ([`ComplianceService::record_sign_in_phone`]) làm sau,
//! khi `auth` đã biết tài khoản nào.

use jiff::SignedDuration;
use std::net::IpAddr;
use std::sync::Arc;

use autoxide_auth::api::{PhoneCodeSent, PhoneCodes};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::rows;
use autoxide_kernel::secret::sha256;
use autoxide_kernel::sms::{PhoneNumber, SmsMessage, SmsPurpose, sms_disabled};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::throttle::{Outcome, Throttle};
use autoxide_kernel::time::Timestamp;
use sea_orm::sea_query::{Expr, ExprTrait as _, OnConflict};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{account_verification, phone_challenge, phone_known_client};
use crate::events;
use crate::service::{ComplianceService, now};
use crate::verification::{
    CHALLENGE_KEEP_HOURS, HOUR, OTP_PURPOSE, SMS_TIMEOUT, challenge_not_found, code_hash,
    otp_attempts_exceeded, otp_invalid, otp_throttled, random_code, same,
};

/// Chủ của mã đăng nhập: chưa có tài khoản.
const NOBODY: Uuid = Uuid::nil();
/// Cửa sổ đếm lần nhập sai của một số.
const WRONG_WINDOW: SignedDuration = SignedDuration::from_hours(CHALLENGE_KEEP_HOURS);
/// Số ngày nhớ một client đã đăng nhập đúng bằng mã của một số (D-20261126-otp-nist): client quen không bị trần
/// chung của số chặn.
pub(crate) const KNOWN_CLIENT_DAYS: i64 = 90;
/// Phạm vi của dấu số trong bảng client quen.
const KNOWN_PHONE_PURPOSE: &str = "compliance.otp.known-phone";
/// Phạm vi khoá dấu khối địa chỉ.
const BLOCK_SCOPE: &str = "compliance.otp";

/// Số điện thoại đã nhập sai mã quá hạn mức trong 24 giờ.
pub(crate) fn otp_daily_limit(retry_after: u64) -> Error {
    Error::too_many_requests(
        "compliance.otp_daily_limit",
        "Too many wrong codes for this phone number today. Wait {seconds} seconds, then try again.",
    )
    .with_param("seconds", retry_after)
    .with_retry_after(retry_after)
}

impl ComplianceService {
    /// Gửi mã đăng nhập tới số `raw`.
    pub(crate) async fn start_sign_in_code(
        &self,
        tenant: &TenantContext,
        raw: &str,
        ip: Option<IpAddr>,
    ) -> Result<PhoneCodeSent, Error> {
        let phone = self.allowed_phone(tenant, raw).await?;
        let Some(sender) = self.sms.clone() else {
            return Err(sms_disabled());
        };
        let otp = self.otp;
        // Khối địa chỉ của client (PT-7): lần sai đếm theo (số, khối) — người lạ đoán sai trên mã họ xin
        // không khoá chủ số ở khối khác.
        let block = match ip {
            Some(ip) => Some(
                autoxide_kernel::ip_hash::client_fingerprint(&self.secret, BLOCK_SCOPE, ip).await?,
            ),
            None => None,
        };
        // Số đã nhập sai đủ hạn mức trong 24 giờ: không gửi thêm mã (mã gửi đi cũng không kiểm được).
        let recent = phone_challenge::Entity::find()
            .filter(phone_challenge::Column::Phone.eq(phone.as_str()))
            .filter(phone_challenge::Column::CreatedAt.gt(now() - WRONG_WINDOW))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let known = self.known_client(tenant, phone.as_str(), block.as_deref()).await?;
        self.check_wrong_today(&recent, block.as_deref(), known, now())?;
        let mut limits = vec![(
            Throttle::new("compliance", otp.per_phone_per_hour, HOUR)?,
            format!("otp-phone:{}", phone.as_str()),
        )];
        if let Some(ip) = ip {
            limits.push((
                Throttle::new("compliance", otp.per_ip_per_hour, HOUR)?,
                autoxide_kernel::throttle::address_key("otp-ip", ip),
            ));
        }
        for (throttle, key) in &limits {
            if let Outcome::Limited { retry_after } = throttle.hit(tenant, key).await? {
                return Err(otp_throttled(retry_after));
            }
        }
        let code = random_code()?;
        let id = Uuid::now_v7();
        let pepper = self.secret.derive_for(tenant.id(), OTP_PURPOSE).await?;
        let at = now();
        let expires_at = at + SignedDuration::from_secs(otp.ttl_seconds);
        phone_challenge::Entity::insert(phone_challenge::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            user_id: Set(NOBODY),
            phone: Set(phone.as_str().to_owned()),
            code_hash: Set(code_hash(&pepper, id, &code)),
            attempts: Set(0),
            expires_at: Set(expires_at),
            consumed_at: Set(None),
            created_at: Set(at),
            requester_block: Set(block),
        })
        .exec_without_returning(tenant.database().write())
        .await?;
        // Gửi NGOÀI transaction, chờ có hạn; lỗi thì xoá mã vừa tạo để người dùng xin lại.
        let minutes = Ord::max(otp.ttl_seconds.div_euclid(60), 1);
        let message = SmsMessage::new(
            tenant.id(),
            phone.clone(),
            self.otp_text(tenant, &crate::sms_templates::SIGN_IN, &code, minutes).await,
            SmsPurpose::Otp,
            id.to_string(),
        );
        let outcome = match tokio::time::timeout(SMS_TIMEOUT, sender.send(message)).await {
            Ok(result) => result,
            Err(_) => Err(Error::unavailable(
                "compliance.sms_timeout",
                "The SMS provider did not answer in time; try again.",
            )),
        };
        if let Err(error) = outcome {
            phone_challenge::Entity::delete_by_id(id)
                .scoped(tenant)
                .exec(tenant.database().write())
                .await?;
            tracing::warn!(phone = %phone.masked(), code = error.code(), "sign-in code sms failed");
            return Err(error);
        }
        tracing::info!(challenge_id = %id, phone = %phone.masked(), "sign-in code sent");
        Ok(PhoneCodeSent { id, expires_at })
    }

    /// Kiểm mã đăng nhập: đúng, còn hạn, chưa dùng, chưa quá số lần sai → số đã xác thực.
    pub(crate) async fn confirm_sign_in_code(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        code: &str,
    ) -> Result<PhoneNumber, Error> {
        let found = phone_challenge::Entity::find_by_id(id)
            .filter(phone_challenge::Column::UserId.eq(NOBODY))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(challenge_not_found)?;
        let pepper = self.secret.derive_for(tenant.id(), OTP_PURPOSE).await?;
        let transaction = tenant.database().begin_write().await?;
        // Khoá mọi mã của cùng số: hai lần nhập cùng mã nối đuôi nhau, bên sau thấy mã đã dùng.
        let locked = rows::lock(
            &transaction,
            phone_challenge::Entity::find()
                .scoped(tenant)
                .filter(phone_challenge::Column::Phone.eq(found.phone.clone())),
            phone_challenge::Column::Id,
        )
        .await?;
        let Some(row) = locked.iter().find(|row| row.id == id).cloned() else {
            return Err(challenge_not_found());
        };
        let at = now();
        if row.consumed_at.is_some() || row.expires_at <= at {
            return Err(otp_invalid());
        }
        if row.attempts >= self.otp.max_attempts {
            return Err(otp_attempts_exceeded());
        }
        // Hạn mức sai mỗi ngày của số (cộng mọi mã của số trong 24 giờ xin từ cùng khối địa chỉ với mã
        // này, và trần chung của mọi client lạ; đã khoá ở trên): xin mã mới liên tục không mở thêm lượt đoán.
        let known = self.known_client(tenant, &row.phone, row.requester_block.as_deref()).await?;
        self.check_wrong_today(&locked, row.requester_block.as_deref(), known, at)?;
        let digits: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        if !same(&code_hash(&pepper, id, &digits), &row.code_hash) {
            phone_challenge::Entity::update_many()
                .scoped(tenant)
                .col_expr(
                    phone_challenge::Column::Attempts,
                    Expr::col(phone_challenge::Column::Attempts).add(1),
                )
                .filter(phone_challenge::Column::Id.eq(id))
                .exec(&transaction)
                .await?;
            transaction.commit().await?;
            return Err(if row.attempts + 1 >= self.otp.max_attempts {
                otp_attempts_exceeded()
            } else {
                otp_invalid()
            });
        }
        let consumed = phone_challenge::Entity::update_many()
            .scoped(tenant)
            .col_expr(phone_challenge::Column::ConsumedAt, Expr::value(at))
            .filter(phone_challenge::Column::Id.eq(id))
            .filter(phone_challenge::Column::ConsumedAt.is_null())
            .exec(&transaction)
            .await?;
        if consumed.rows_affected != 1 {
            return Err(otp_invalid());
        }
        // Client vừa đăng nhập đúng thành client quen của số (NIST SP 800-63B-4 §3.2.2: sau khi xác thực đúng, bỏ qua
        // các lần sai trước — `check_wrong_today` chỉ đếm mã tạo sau lần đúng gần nhất).
        if let Some(block) = &row.requester_block {
            let phone_hash = self.known_phone_hash(tenant, &row.phone).await?;
            phone_known_client::Entity::insert(phone_known_client::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                phone_hash: Set(phone_hash),
                requester_block: Set(block.clone()),
                last_success_at: Set(at),
            })
            .on_conflict(
                OnConflict::columns([
                    phone_known_client::Column::TenantId,
                    phone_known_client::Column::PhoneHash,
                    phone_known_client::Column::RequesterBlock,
                ])
                .update_column(phone_known_client::Column::LastSuccessAt)
                .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
        }
        transaction.commit().await?;
        PhoneNumber::parse(&row.phone, "84")
            .ok_or_else(|| Error::internal("stored phone is not E.164"))
    }

    /// Dấu của số `phone` trong bảng client quen: SHA-256 có pepper riêng của khách — không lưu số.
    async fn known_phone_hash(&self, tenant: &TenantContext, phone: &str) -> Result<String, Error> {
        let pepper = self.secret.derive_for(tenant.id(), KNOWN_PHONE_PURPOSE).await?;
        let pepper: String = pepper.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(sha256(&format!("{pepper}|{phone}")))
    }

    /// Client `block` đã đăng nhập đúng bằng mã của số `phone` trong [`KNOWN_CLIENT_DAYS`] ngày qua.
    async fn known_client(
        &self,
        tenant: &TenantContext,
        phone: &str,
        block: Option<&str>,
    ) -> Result<bool, Error> {
        let Some(block) = block else { return Ok(false) };
        let phone_hash = self.known_phone_hash(tenant, phone).await?;
        let since = now() - SignedDuration::from_hours(KNOWN_CLIENT_DAYS * 24);
        Ok(phone_known_client::Entity::find()
            .filter(phone_known_client::Column::PhoneHash.eq(phone_hash))
            .filter(phone_known_client::Column::RequesterBlock.eq(block))
            .filter(phone_known_client::Column::LastSuccessAt.gt(since))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .is_some())
    }

    /// Báo [`otp_daily_limit`] khi các mã `rows` (của cùng một số) tạo trong 24 giờ trước `at` — và sau lần đăng nhập
    /// đúng gần nhất của số (NIST SP 800-63B-4 §3.2.2) — đã bị nhập sai:
    ///
    /// - `AUTOXIDE_COMPLIANCE_OTP_WRONG_PER_PHONE_PER_DAY` lần trên những mã xin từ client `block` (trống: không biết
    ///   địa chỉ — một nhóm riêng); hoặc
    /// - `AUTOXIDE_COMPLIANCE_OTP_WRONG_PER_PHONE_ALL_CLIENTS_PER_DAY` lần trên mọi mã của số, trừ khi `block` là client
    ///   quen (`known`): người lạ nhập sai từ nhiều nơi không khoá được chủ số ở mạng họ đã đăng nhập.
    ///
    /// Tạo mã mới không xoá lần sai đã đếm (§3.1.3.2). Tính từ bảng mã (giữ [`CHALLENGE_KEEP_HOURS`] giờ) nên chạy như
    /// nhau trên cả hai database.
    fn check_wrong_today(
        &self,
        rows: &[phone_challenge::Model],
        block: Option<&str>,
        known: bool,
        at: Timestamp,
    ) -> Result<(), Error> {
        let since = at - WRONG_WINDOW;
        // `Iterator::max` / `min` viết rõ: `ExprTrait` (sea-query) dùng trong file cũng có hàm `max` / `min`.
        let last_success = Iterator::max(rows.iter().filter_map(|row| row.consumed_at));
        let recent: Vec<&phone_challenge::Model> = rows
            .iter()
            .filter(|row| row.created_at > since && row.attempts > 0)
            .filter(|row| last_success.is_none_or(|success| row.created_at > success))
            .collect();
        let same_block: Vec<&phone_challenge::Model> =
            recent.iter().copied().filter(|row| row.requester_block.as_deref() == block).collect();
        let mut groups = vec![(&same_block, i64::from(self.otp.wrong_per_phone_per_day))];
        if !known {
            groups.push((&recent, i64::from(self.otp.wrong_per_phone_all_clients_per_day)));
        }
        for (group, cap) in groups {
            let wrong: i64 = group.iter().map(|row| i64::from(row.attempts)).sum();
            if wrong < cap {
                continue;
            }
            // Chờ tới khi mã cũ nhất có lần sai ra khỏi cửa sổ 24 giờ.
            let oldest = Iterator::min(group.iter().map(|row| row.created_at)).unwrap_or(at);
            let wait = Ord::max((oldest + WRONG_WINDOW - at).as_secs(), 1);
            return Err(otp_daily_limit(u64::try_from(wait).unwrap_or(1)));
        }
        Ok(())
    }

    /// Ghi `phone` là số đã xác thực của tài khoản `user_id`. Quy tắc của khách không cho thêm tài
    /// khoản dùng số này (`max_accounts_per_phone`) thì bỏ qua: tài khoản vẫn đăng nhập được, chỉ không
    /// được tính là đã xác thực số.
    pub(crate) async fn record_sign_in_phone(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        phone: &PhoneNumber,
    ) -> Result<(), Error> {
        let max = self.max_accounts_per_phone(tenant).await?;
        let phone_key = sha256(phone.as_str());
        let transaction = tenant.database().begin_write().await?;
        if Self::others_with_phone(&transaction, tenant, &phone_key, user_id).await? >= max {
            tracing::info!(user_id = %user_id, "phone sign-in not recorded: number used by another account");
            return Ok(());
        }
        let at = now();
        account_verification::Entity::insert(account_verification::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            user_id: Set(user_id),
            phone: Set(Some(phone.as_str().to_owned())),
            phone_key: Set(phone_key),
            phone_verified_at: Set(Some(at)),
            updated_at: Set(at),
        })
        .on_conflict(
            OnConflict::columns([
                account_verification::Column::TenantId,
                account_verification::Column::UserId,
            ])
            .update_columns([
                account_verification::Column::Phone,
                account_verification::Column::PhoneKey,
                account_verification::Column::PhoneVerifiedAt,
                account_verification::Column::UpdatedAt,
            ])
            .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::user(user_id),
                Some(format!("compliance.account:{user_id}")),
                &events::PhoneVerified(json!({"user_id": user_id})),
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}

/// Bộ mã cắm vào `auth`.
pub(crate) struct SignInCodes(pub Arc<ComplianceService>);

#[async_trait::async_trait]
impl PhoneCodes for SignInCodes {
    async fn send(
        &self,
        tenant: &TenantContext,
        phone: &str,
        ip: Option<IpAddr>,
    ) -> Result<PhoneCodeSent, Error> {
        self.0.start_sign_in_code(tenant, phone, ip).await
    }

    async fn verify(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        code: &str,
    ) -> Result<PhoneNumber, Error> {
        self.0.confirm_sign_in_code(tenant, id, code).await
    }

    async fn record(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        phone: &PhoneNumber,
    ) -> Result<(), Error> {
        self.0.record_sign_in_phone(tenant, user_id, phone).await
    }
}
