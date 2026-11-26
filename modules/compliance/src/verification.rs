//! Xác thực tài khoản đăng nội dung (NĐ 147/2024/NĐ-CP, D-356 — cần đối chiếu văn bản gốc / người có
//! chuyên môn pháp lý): số điện thoại di động xác nhận bằng mã một lần qua `dyn SmsSender`, mức
//! "định danh" đọc từ hồ sơ KYC của chính người dùng. Cổng đăng nội dung so mức đó với chính sách
//! `posting.<chỗ>` của khách.
//!
//! Mã một lần không bao giờ nằm trong job hay log: gửi SMS đồng bộ NGOÀI transaction (bảng job đọc
//! được qua API quản trị và dead-letter queue), database chỉ giữ bản băm có pepper riêng của khách.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::net::IpAddr;
use std::time::Duration;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::rows;
use autoxide_kernel::secret::sha256;
use autoxide_kernel::sms::{PhoneNumber, SmsMessage, SmsPurpose, sms_disabled};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::throttle::{Outcome, Throttle};
use sea_orm::sea_query::{Expr, ExprTrait as _, OnConflict};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{PostingSurface, Subject, VerificationLevel};
use crate::entity::{account_verification, phone_challenge};
use crate::events;
use crate::service::{ComplianceService, now};

/// Nhãn dẫn pepper của mã một lần từ khoá gốc (`SecretKey::derive_for`).
pub(crate) const OTP_PURPOSE: &str = "compliance.otp";
/// Mã nước mặc định khi người dùng gõ số bắt đầu bằng `0`.
const DEFAULT_COUNTRY: &str = "84";
/// Chờ nhà cung cấp SMS tối đa chừng này.
pub(crate) const SMS_TIMEOUT: Duration = Duration::from_secs(10);
/// Mã đã hết hạn / đã dùng giữ lại chừng này rồi job hằng ngày xoá.
pub(crate) const CHALLENGE_KEEP_HOURS: i64 = 24;
pub(crate) const HOUR: Duration = Duration::from_secs(3_600);
const DAY: Duration = Duration::from_secs(86_400);

/// Trạng thái xác thực của một tài khoản.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Verification {
    pub level: VerificationLevel,
    /// Số đã xác thực (E.164).
    pub phone: Option<PhoneNumber>,
    pub phone_verified_at: Option<Timestamp>,
    pub identity_verified_at: Option<Timestamp>,
}

pub(crate) fn verification_required(level: VerificationLevel) -> Error {
    Error::forbidden(
        "compliance.verification_required",
        "Your account needs {level} verification before posting here.",
    )
    .with_param("level", level.as_str())
}

pub(crate) fn otp_invalid() -> Error {
    Error::new(
        ErrorKind::Validation,
        "compliance.otp_invalid",
        "The code is wrong or has expired; ask for a new code if it has expired.",
    )
}

pub(crate) fn otp_attempts_exceeded() -> Error {
    Error::too_many_requests(
        "compliance.otp_attempts_exceeded",
        "Too many wrong codes; ask for a new code.",
    )
}

fn phone_not_allowed() -> Error {
    Error::new(
        ErrorKind::Validation,
        "compliance.phone_not_allowed",
        "The phone number must be a mobile number of a country this service accepts (Vietnam).",
    )
}

pub(crate) fn phone_in_use() -> Error {
    Error::conflict(
        "compliance.phone_in_use",
        "This phone number is already verified for another account.",
    )
}

pub(crate) fn challenge_not_found() -> Error {
    Error::not_found(
        "compliance.challenge_not_found",
        "There is no such phone verification for your account.",
    )
}

pub(crate) fn otp_throttled(retry_after: u64) -> Error {
    Error::too_many_requests(
        "compliance.otp_throttled",
        "Too many codes were asked for. Wait {seconds} seconds, then try again.",
    )
    .with_param("seconds", retry_after)
    .with_retry_after(retry_after)
}

/// Băm `code` của mã `id` bằng pepper của khách. Mã 6 số chỉ có một triệu khả năng: không có pepper
/// thì ai đọc được bảng sẽ thử hết trong tích tắc.
pub(crate) fn code_hash(pepper: &[u8; 32], id: Uuid, code: &str) -> String {
    let pepper: String = pepper.iter().map(|byte| format!("{byte:02x}")).collect();
    sha256(&format!("{pepper}|{id}|{code}"))
}

/// So hai chuỗi trong thời gian không phụ thuộc chỗ khác nhau đầu tiên.
pub(crate) fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Mã 6 chữ số ngẫu nhiên đều (loại bỏ phần dư để không lệch về số nhỏ).
pub(crate) fn random_code() -> Result<String, Error> {
    // 4_294_000_000 là bội lớn nhất của 1_000_000 không vượt `u32::MAX`.
    const LIMIT: u32 = 4_294_000_000;
    loop {
        let mut bytes = [0u8; 4];
        getrandom::fill(&mut bytes)
            .map_err(|error| Error::internal(format!("random number generator: {error}")))?;
        let value = u32::from_le_bytes(bytes);
        if value < LIMIT {
            return Ok(format!("{:06}", value % 1_000_000));
        }
    }
}

/// Số `phone` có thuộc nước được nhận và đúng dạng số di động theo quy tắc `posting.phone_countries`
/// (`{"countries": ["84"], "mobile": {"84": {"length": 9, "prefixes": ["3", …]}}}`) không.
pub(crate) fn phone_allowed(phone: &PhoneNumber, rule: &Value) -> bool {
    let countries: Vec<&str> = rule["countries"]
        .as_array()
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_else(|| vec![DEFAULT_COUNTRY]);
    let Some(code) = phone.calling_code(&countries) else {
        return false;
    };
    let national = phone.as_str().get(1 + code.len()..).unwrap_or_default();
    let mobile = &rule["mobile"][code];
    if let Some(length) = mobile["length"].as_u64()
        && u64::try_from(national.len()).ok() != Some(length)
    {
        return false;
    }
    match mobile["prefixes"].as_array() {
        Some(prefixes) => {
            prefixes.iter().filter_map(Value::as_str).any(|prefix| national.starts_with(prefix))
        }
        None => true,
    }
}

impl ComplianceService {
    /// Mức xác thực và số đã xác thực của `user_id`.
    pub(crate) async fn verification(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Verification, Error> {
        let row = account_verification::Entity::find_by_id((tenant.id().as_uuid(), user_id))
            .one(tenant.database().read())
            .await?;
        let (phone, phone_verified_at) = match row {
            Some(row) => match (row.phone, row.phone_verified_at) {
                (Some(phone), Some(at)) => (PhoneNumber::parse(&phone, DEFAULT_COUNTRY), Some(at)),
                _ => (None, None),
            },
            None => (None, None),
        };
        let kyc = self.kyc_status(tenant, &Subject::User(user_id)).await?;
        let identity_verified_at = (kyc.status == "verified").then_some(kyc.verified_at).flatten();
        let level = if kyc.status == "verified" {
            VerificationLevel::Identity
        } else if phone.is_some() {
            VerificationLevel::Phone
        } else {
            VerificationLevel::None
        };
        Ok(Verification { level, phone, phone_verified_at, identity_verified_at })
    }

    /// Mức khách đặt cho chỗ đăng `surface` (chính sách `posting.<chỗ>`, mặc định `none`).
    pub(crate) async fn posting_level(
        &self,
        tenant: &TenantContext,
        surface: PostingSurface,
    ) -> Result<VerificationLevel, Error> {
        Ok(self
            .policy(tenant, surface.policy_key())
            .await?
            .and_then(|value| value.as_str().and_then(VerificationLevel::parse))
            .unwrap_or(VerificationLevel::None))
    }

    /// Cổng đăng nội dung: tài khoản phải đạt ít nhất mức khách đặt cho `surface`.
    pub(crate) async fn check_posting(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        surface: PostingSurface,
    ) -> Result<(), Error> {
        let required = self.posting_level(tenant, surface).await?;
        self.require_level(tenant, principal.map(Principal::user_id), required).await
    }

    /// `user_id` (None = khách vãng lai) đạt mức `required` chưa; chưa → `verification_required`.
    pub(crate) async fn require_level(
        &self,
        tenant: &TenantContext,
        user_id: Option<Uuid>,
        required: VerificationLevel,
    ) -> Result<(), Error> {
        if required == VerificationLevel::None {
            return Ok(());
        }
        let level = match user_id {
            Some(user_id) => self.verification(tenant, user_id).await?.level,
            None => VerificationLevel::None,
        };
        if level >= required { Ok(()) } else { Err(verification_required(required)) }
    }

    /// Số tài khoản tối đa dùng chung một số đã xác thực (chính sách `posting.max_accounts_per_phone`,
    /// mặc định 1).
    pub(crate) async fn max_accounts_per_phone(
        &self,
        tenant: &TenantContext,
    ) -> Result<u64, Error> {
        Ok(self
            .policy(tenant, "posting.max_accounts_per_phone")
            .await?
            .and_then(|value| value.as_u64())
            .filter(|count| *count >= 1)
            .unwrap_or(1))
    }

    /// Số tài khoản KHÁC `user_id` đã xác thực số có khoá `phone_key`.
    pub(crate) async fn others_with_phone<C: sea_orm::ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        phone_key: &str,
        user_id: Uuid,
    ) -> Result<u64, Error> {
        Ok(account_verification::Entity::find()
            .scoped(tenant)
            .filter(account_verification::Column::PhoneKey.eq(phone_key))
            .filter(account_verification::Column::PhoneVerifiedAt.is_not_null())
            .filter(account_verification::Column::UserId.ne(user_id))
            .count(connection)
            .await?)
    }

    /// Chuẩn hoá và kiểm số `raw` theo quy tắc `posting.phone_countries` đang hiệu lực.
    pub(crate) async fn allowed_phone(
        &self,
        tenant: &TenantContext,
        raw: &str,
    ) -> Result<PhoneNumber, Error> {
        let Some(phone) = PhoneNumber::parse(raw, DEFAULT_COUNTRY) else {
            return Err(Error::validation(vec![FieldError::new(
                "phone",
                "invalid",
                "is not a phone number",
            )]));
        };
        let rule = self
            .rule_for(tenant, "posting.phone_countries", Timestamp::now())
            .await?
            .and_then(|row| serde_json::from_str::<Value>(&row.body).ok())
            .unwrap_or_else(|| json!({"countries": [DEFAULT_COUNTRY]}));
        if phone_allowed(&phone, &rule) { Ok(phone) } else { Err(phone_not_allowed()) }
    }

    /// Gửi mã một lần tới số `raw` để `principal` xác thực số đó.
    pub(crate) async fn start_phone_verification(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        raw: &str,
        ip: Option<IpAddr>,
    ) -> Result<phone_challenge::Model, Error> {
        let user_id = principal.user_id();
        let phone = self.allowed_phone(tenant, raw).await?;
        let Some(sender) = self.sms.clone() else {
            return Err(sms_disabled());
        };
        let phone_key = sha256(phone.as_str());
        // Kiểm sớm để không gửi tin vô ích; xác nhận kiểm lại dưới khoá.
        let max = self.max_accounts_per_phone(tenant).await?;
        if Self::others_with_phone(tenant.database().read(), tenant, &phone_key, user_id).await?
            >= max
        {
            return Err(phone_in_use());
        }
        let otp = self.otp;
        let mut limits = vec![
            (
                Throttle::new("compliance", otp.per_phone_per_hour, HOUR)?,
                format!("otp-phone:{}", phone.as_str()),
            ),
            (
                Throttle::new("compliance", otp.per_user_per_day, DAY)?,
                format!("otp-user:{user_id}"),
            ),
        ];
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
        let row = phone_challenge::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            user_id: Set(user_id),
            phone: Set(phone.as_str().to_owned()),
            code_hash: Set(code_hash(&pepper, id, &code)),
            attempts: Set(0),
            expires_at: Set(at + SignedDuration::from_secs(otp.ttl_seconds)),
            consumed_at: Set(None),
            created_at: Set(at),
            requester_block: Set(None),
        };
        let saved = phone_challenge::Entity::insert(row)
            .exec_with_returning(tenant.database().write())
            .await?;

        // Gửi NGOÀI transaction, chờ có hạn; lỗi thì xoá mã vừa tạo để người dùng xin lại.
        let minutes = Ord::max(otp.ttl_seconds.div_euclid(60), 1);
        let message = SmsMessage::new(
            tenant.id(),
            phone.clone(),
            self.otp_text(tenant, &crate::sms_templates::PHONE_VERIFICATION, &code, minutes).await,
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
            tracing::warn!(phone = %phone.masked(), code = error.code(), "phone verification sms failed");
            return Err(error);
        }
        tracing::info!(challenge_id = %id, phone = %phone.masked(), "phone verification code sent");
        Ok(saved)
    }

    /// Mã `id` của chính `principal`.
    pub(crate) async fn phone_challenge(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<phone_challenge::Model, Error> {
        phone_challenge::Entity::find_by_id(id)
            .scoped(tenant)
            .filter(phone_challenge::Column::UserId.eq(principal.user_id()))
            .one(tenant.database().read())
            .await?
            .ok_or_else(challenge_not_found)
    }

    /// Nhập mã: đúng, còn hạn, chưa dùng, chưa quá số lần sai → số của mã thành số đã xác thực.
    pub(crate) async fn confirm_phone(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        code: &str,
    ) -> Result<Verification, Error> {
        let user_id = principal.user_id();
        let found = self.phone_challenge(tenant, principal, id).await?;
        let pepper = self.secret.derive_for(tenant.id(), OTP_PURPOSE).await?;
        let max = self.max_accounts_per_phone(tenant).await?;
        let phone_key = sha256(&found.phone);

        let transaction = tenant.database().begin_write().await?;
        // Khoá MỌI mã của cùng số (theo ID): hai lần xác nhận cùng mã, hay hai người cùng nhận một
        // số, nối đuôi nhau — bên sau đọc lại thấy kết quả của bên trước (Postgres `FOR UPDATE`;
        // SQLite vốn một bên ghi).
        let locked = rows::lock(
            &transaction,
            phone_challenge::Entity::find()
                .scoped(tenant)
                .filter(phone_challenge::Column::Phone.eq(found.phone.clone())),
            phone_challenge::Column::Id,
        )
        .await?;
        let Some(row) = locked.into_iter().find(|row| row.id == id) else {
            return Err(challenge_not_found());
        };
        let at = now();
        if row.consumed_at.is_some() || row.expires_at <= at {
            return Err(otp_invalid());
        }
        if row.attempts >= self.otp.max_attempts {
            return Err(otp_attempts_exceeded());
        }
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
        if Self::others_with_phone(&transaction, tenant, &phone_key, user_id).await? >= max {
            return Err(phone_in_use());
        }
        // Dùng một lần: chỉ đổi dòng còn chưa dùng.
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
        // Chủ số chứng minh lại được số từ tài khoản đã đăng nhập (NIST SP 800-63B-4 §4.1, gắn lại bộ xác thực): mở khoá
        // mã đăng nhập SMS của số (D-20261126-otp-nist).
        if let Some(number) = PhoneNumber::parse(&row.phone, "84") {
            let key = self.secret.phone_key(tenant.id(), &number).await?;
            crate::phone_lock::succeeded(&transaction, tenant, &row.phone, &key, id, at).await?;
        }
        account_verification::Entity::insert(account_verification::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            user_id: Set(user_id),
            phone: Set(Some(row.phone.clone())),
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
        let event = events::PhoneVerified(json!({"user_id": user_id}));
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("compliance.account:{user_id}")),
                &event,
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(challenge_id = %id, "phone verified");
        self.verification(tenant, user_id).await
    }

    /// Gỡ số đã xác thực của `principal` (gọi lại vô hại).
    pub(crate) async fn remove_phone(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<(), Error> {
        let user_id = principal.user_id();
        let transaction = tenant.database().begin_write().await?;
        let changed = account_verification::Entity::update_many()
            .col_expr(account_verification::Column::Phone, Expr::value(Option::<String>::None))
            .col_expr(account_verification::Column::PhoneKey, Expr::value(""))
            .col_expr(
                account_verification::Column::PhoneVerifiedAt,
                Expr::value(Option::<Timestamp>::None),
            )
            .col_expr(account_verification::Column::UpdatedAt, Expr::value(now()))
            .filter(account_verification::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(account_verification::Column::UserId.eq(user_id))
            .filter(account_verification::Column::PhoneVerifiedAt.is_not_null())
            .exec(&transaction)
            .await?;
        if changed.rows_affected > 0 {
            let event = events::PhoneRemoved(json!({"user_id": user_id}));
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::from(principal),
                    Some(format!("compliance.account:{user_id}")),
                    &event,
                )
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Xoá mã đã quá hạn giữ (job hằng ngày, mọi database).
    pub(crate) async fn purge_challenges(
        &self,
        database: &autoxide_kernel::database::Database,
    ) -> Result<u64, Error> {
        let before = Timestamp::now() - SignedDuration::from_hours(CHALLENGE_KEEP_HOURS);
        // tenant-scope: việc nền / kiểm tra quét mọi khách của database, chủ ý không lọc khách (rà soát bảo mật 2026-11)
        let challenges = phone_challenge::Entity::delete_many()
            .filter(phone_challenge::Column::CreatedAt.lt(before))
            .exec(database.write())
            .await?
            .rows_affected;
        // Client quen của số (D-20261126-otp-nist): giữ `KNOWN_CLIENT_DAYS` ngày kể từ lần đăng nhập đúng cuối.
        let forgotten = Timestamp::now()
            - SignedDuration::from_hours(crate::sign_in_codes::KNOWN_CLIENT_DAYS * 24);
        // tenant-scope: việc nền quét mọi khách của database, chủ ý không lọc khách
        let clients = crate::entity::phone_known_client::Entity::delete_many()
            .filter(crate::entity::phone_known_client::Column::LastSuccessAt.lt(forgotten))
            .exec(database.write())
            .await?
            .rows_affected;
        Ok(challenges + clients)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn so_di_dong_viet_nam_theo_quy_tac() {
        let rule = json!({"countries": ["84"], "mobile": {"84": {"length": 9, "prefixes": ["3", "5", "7", "8", "9"]}}});
        let ok = PhoneNumber::parse("0912 345 678", "84").unwrap();
        assert!(phone_allowed(&ok, &rule));
        // Số cố định Hà Nội (024…) không phải số di động.
        let landline = PhoneNumber::parse("024 3825 1234", "84").unwrap();
        assert!(!phone_allowed(&landline, &rule));
        // Nước khác.
        let foreign = PhoneNumber::parse("+1 415 555 0100", "84").unwrap();
        assert!(!phone_allowed(&foreign, &rule));
        // Quy tắc cũ chỉ có danh sách nước.
        assert!(phone_allowed(&landline, &json!({"countries": ["84"]})));
    }

    #[test]
    fn ma_ngau_nhien_va_bam() {
        let code = random_code().unwrap();
        assert_eq!(code.len(), 6);
        assert!(code.bytes().all(|b| b.is_ascii_digit()));
        let id = Uuid::now_v7();
        let pepper = [7u8; 32];
        let hash = code_hash(&pepper, id, &code);
        assert!(same(&hash, &code_hash(&pepper, id, &code)));
        assert!(!same(&hash, &code_hash(&[8u8; 32], id, &code)));
        assert!(!same(&hash, &code_hash(&pepper, Uuid::now_v7(), &code)));
        assert!(!same("abc", "abcd"));
    }
}
