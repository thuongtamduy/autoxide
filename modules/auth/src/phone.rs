//! Đăng nhập / tạo tài khoản người mua bằng số điện thoại + mã một lần qua SMS (bước 47,
//! D-20261120-phone-accounts).
//!
//! - `auth` không gửi SMS: bộ mã ([`PhoneCodes`]) do `compliance` cài và cắm vào lúc khởi động (đăng ký
//!   sau `auth`). Chưa cắm → 503 `auth.phone_sign_in_unavailable`.
//! - Chỉ người mua (`realm = customer`). Số đã có tài khoản → đăng nhập (vẫn hỏi TOTP nếu bật); chưa có
//!   và khách cho người mua tự tạo tài khoản → tạo tài khoản chỉ có số (`email = ''`), không mật khẩu.
//! - Tài khoản được tìm theo `phone_key` (HMAC của số theo khách): database không giữ số rõ.

use autoxide_kernel::time::Timestamp;
use std::sync::{Arc, OnceLock};

use autoxide_kernel::access::Realm;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::module::Later;
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::sms::{PhoneNumber, VerifiedPhoneTokens};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::throttle::{Outcome, Throttle};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, SqlErr};
use serde_json::json;
use uuid::Uuid;

use crate::api::{AuthExtensions, PhoneCodeSent, PhoneCodes};
use crate::events;
use crate::session::{LoginOutcome, SessionService};
use crate::user::entity;

/// Cách đăng nhập ghi trên phiên.
const METHOD: &str = "phone_otp";

fn unavailable() -> Error {
    Error::unavailable(
        "auth.phone_sign_in_unavailable",
        "This server cannot send sign-in codes by SMS.",
    )
}

fn registration_closed() -> Error {
    Error::forbidden(
        "auth.customer_registration_closed",
        "This shop does not let customers create accounts yet.",
    )
}

/// Cách đăng nhập ghi trên phiên của tài khoản tạo từ token đã xác thực số.
const SIGNUP_METHOD: &str = "phone_token";

/// Số lần tạo tài khoản từ token mỗi giờ theo một địa chỉ IP.
const SIGNUPS_PER_IP_PER_HOUR: u32 = 10;

fn signup_unavailable() -> Error {
    Error::unavailable(
        "auth.phone_signup_unavailable",
        "This server has no module that issues verified phone tokens, such as order lookups.",
    )
}

fn phone_taken() -> Error {
    Error::conflict(
        "auth.phone_taken",
        "This phone number already has an account. Sign in with a code sent to it instead.",
    )
}

fn lookup_token_invalid() -> Error {
    Error::validation(vec![FieldError::new(
        "lookup_token",
        "invalid",
        "is not valid or has expired; look the order up again",
    )])
}

fn signups_throttled(retry_after: u64) -> Error {
    Error::too_many_requests(
        "auth.phone_signup_throttled",
        "Too many accounts created from this address. Try again in {retry_after} seconds.",
    )
    .with_param("retry_after", retry_after)
    .with_retry_after(retry_after)
}

fn check_name(name: Option<&str>) -> Result<Option<&str>, Error> {
    let name = name.map(str::trim).filter(|name| !name.is_empty());
    if name.is_some_and(|name| name.chars().count() > 100) {
        return Err(Error::validation(vec![
            FieldError::new(
                "name",
                "length",
                "must be between 1 and {name_max_length} characters long",
            )
            .with_param("name_max_length", 100),
        ]));
    }
    Ok(name)
}

/// Kết quả của một lần đăng nhập bằng số.
pub(crate) struct PhoneLogin {
    pub outcome: LoginOutcome,
    /// Tài khoản vừa được tạo (lần đầu dùng số này).
    pub created: bool,
}

pub(crate) struct PhoneSignIn {
    codes: OnceLock<Arc<dyn PhoneCodes>>,
    /// Token đã qua mã một lần do module khác cấp (token xem đơn của `orders`), cho
    /// `POST /phone-signups`. Module đó đăng ký sau `auth` nên lấy bằng chỗ nối muộn của kernel.
    tokens: Later<dyn VerifiedPhoneTokens>,
    sessions: Arc<SessionService>,
    secret: SecretKey,
    outbox: Outbox,
    orgs: Option<Arc<dyn autoxide_organizations::api::OrgDirectory>>,
}

impl PhoneSignIn {
    pub(crate) fn new(
        sessions: Arc<SessionService>,
        secret: SecretKey,
        outbox: Outbox,
        orgs: Option<Arc<dyn autoxide_organizations::api::OrgDirectory>>,
        tokens: Later<dyn VerifiedPhoneTokens>,
    ) -> Self {
        PhoneSignIn { codes: OnceLock::new(), tokens, sessions, secret, outbox, orgs }
    }

    fn codes(&self) -> Result<&Arc<dyn PhoneCodes>, Error> {
        self.codes.get().ok_or_else(unavailable)
    }

    /// Gửi mã tới số `phone`. Trả lời giống nhau dù số đã có tài khoản hay chưa.
    pub(crate) async fn send_code(
        &self,
        tenant: &TenantContext,
        phone: &str,
        ip: Option<std::net::IpAddr>,
    ) -> Result<PhoneCodeSent, Error> {
        self.codes()?.send(tenant, phone, ip).await
    }

    /// Nhập mã: số đã có tài khoản người mua → đăng nhập; chưa có → tạo (khi khách cho phép) rồi đăng
    /// nhập. `name`: tên hiển thị cho tài khoản mới (không có thì số đã che).
    pub(crate) async fn login(
        &self,
        tenant: &TenantContext,
        challenge: Uuid,
        code: &str,
        name: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<PhoneLogin, Error> {
        let name = check_name(name)?;
        let codes = self.codes()?;
        let phone = codes.verify(tenant, challenge, code).await?;
        let key = self.secret.phone_key(tenant.id(), &phone).await?;
        let (user, created) = match self.find(tenant, &key).await? {
            Some(user) => (user, false),
            None => {
                let open = crate::settings::enabled(
                    self.orgs.as_ref(),
                    tenant,
                    &crate::settings::CUSTOMER_REGISTRATION,
                )
                .await?;
                if !open {
                    return Err(registration_closed());
                }
                self.create(tenant, &phone, &key, name, METHOD).await?
            }
        };
        // Số này giờ là số đã xác thực của tài khoản (module khác thấy qua `compliance`).
        codes.record(tenant, user, &phone).await?;
        let outcome = self.sessions.login_verified(tenant, user, METHOD, user_agent).await?;
        Ok(PhoneLogin { outcome, created })
    }

    /// Tạo tài khoản người mua từ số đã qua mã một lần ở module khác (token xem đơn vãng lai của
    /// `orders`, D-202), rồi đăng nhập. Không mật khẩu, không email: lần sau đăng nhập bằng mã SMS hay
    /// passkey thêm sau. Số đã có tài khoản → 409 `auth.phone_taken` (không đăng nhập hộ: token xem đơn
    /// không phải bằng chứng đủ để vào tài khoản sẵn có).
    pub(crate) async fn sign_up(
        &self,
        tenant: &TenantContext,
        token: &str,
        name: Option<&str>,
        ip: Option<std::net::IpAddr>,
        user_agent: Option<&str>,
    ) -> Result<LoginOutcome, Error> {
        let name = check_name(name)?;
        let tokens = self.tokens.get().ok_or_else(signup_unavailable)?;
        // Đếm TRƯỚC khi hỏi token: dò token cũng bị giới hạn.
        if let Some(ip) = ip {
            let hour = std::time::Duration::from_secs(3600);
            let throttle = Throttle::new("auth", SIGNUPS_PER_IP_PER_HOUR, hour)?;
            let key = format!("phone-signup-ip:{}", autoxide_kernel::ip_hash::prefix(ip));
            if let Outcome::Limited { retry_after } = throttle.hit(tenant, &key).await? {
                return Err(signups_throttled(retry_after));
            }
        }
        let phone = tokens.verified_phone(tenant, token).await?.ok_or_else(lookup_token_invalid)?;
        let key = self.secret.phone_key(tenant.id(), &phone).await?;
        if self.find(tenant, &key).await?.is_some() {
            return Err(phone_taken());
        }
        let open = crate::settings::enabled(
            self.orgs.as_ref(),
            tenant,
            &crate::settings::CUSTOMER_REGISTRATION,
        )
        .await?;
        if !open {
            return Err(registration_closed());
        }
        let (user, created) = self.create(tenant, &phone, &key, name, SIGNUP_METHOD).await?;
        if !created {
            // Một request khác vừa tạo tài khoản cho cùng số.
            return Err(phone_taken());
        }
        // Ghi số đã xác thực ở `compliance` khi có (module khác thấy qua `AccountVerification`).
        if let Some(codes) = self.codes.get() {
            codes.record(tenant, user, &phone).await?;
        }
        self.sessions.login_verified(tenant, user, SIGNUP_METHOD, user_agent).await
    }

    /// Xác nhận lại (D-308) bằng mã gửi tới số đăng nhập của chính tài khoản `user_id`. Số khác: lỗi
    /// của ô `phone_code`.
    pub(crate) async fn reauthenticate(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        challenge: Uuid,
        code: &str,
    ) -> Result<(), Error> {
        let phone = self.codes()?.verify(tenant, challenge, code).await?;
        let key = self.secret.phone_key(tenant.id(), &phone).await?;
        let own = entity::Entity::find_by_id(user_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .and_then(|row| row.phone_key);
        if own.as_deref() != Some(key.as_str()) {
            return Err(Error::validation(vec![FieldError::new(
                "phone_code",
                "mismatch",
                "was not sent to the number this account signs in with",
            )]));
        }
        Ok(())
    }

    async fn find(&self, tenant: &TenantContext, key: &str) -> Result<Option<Uuid>, Error> {
        Ok(entity::Entity::find()
            .filter(entity::Column::PhoneKey.eq(key))
            .filter(entity::Column::Realm.eq(entity::realm_str(Realm::Customer)))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .map(|row| row.id))
    }

    /// Tạo tài khoản người mua chỉ có số. Hai lần cùng lúc cho cùng số: index unique quyết, bên thua
    /// dùng tài khoản của bên thắng. Trả (ID, có phải vừa tạo).
    async fn create(
        &self,
        tenant: &TenantContext,
        phone: &PhoneNumber,
        key: &str,
        name: Option<&str>,
        method: &str,
    ) -> Result<(Uuid, bool), Error> {
        let name = name.map_or_else(|| phone.masked(), str::to_owned);
        // Lưu tới micro giây như mọi mốc thời gian của module.
        let at = Timestamp::now();
        let transaction = tenant.database().begin_write().await?;
        let inserted = entity::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            email: Set(String::new()),
            search_text: Set(crate::user::search_text("", &name)),
            name: Set(name.clone()),
            password_hash: Set(None),
            disabled_at: Set(None),
            email_verified_at: Set(None),
            created_at: Set(at),
            updated_at: Set(at),
            last_sign_in_at: Set(None),
            locale: Set(None),
            time_zone: Set(None),
            realm: Set(entity::realm_str(Realm::Customer).to_owned()),
            phone_key: Set(Some(key.to_owned())),
        }
        .insert(&transaction)
        .await;
        let row = match inserted {
            Ok(row) => row,
            Err(error) if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) => {
                transaction.rollback().await?;
                let id = self
                    .find(tenant, key)
                    .await?
                    .ok_or_else(|| Error::internal("phone account vanished"))?;
                return Ok((id, false));
            }
            Err(error) => return Err(error.into()),
        };
        let event = events::UserCreated {
            before: None,
            after: Some(json!({"email": "", "name": row.name, "admin": false, "sign_in": method})),
        };
        self.outbox
            .record(&transaction, tenant, Actor::system(), events::user_subject(row.id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(user_id = %row.id, "phone-only customer account created");
        Ok((row.id, true))
    }
}

impl AuthExtensions for PhoneSignIn {
    fn set_phone_codes(&self, codes: Arc<dyn PhoneCodes>) -> Result<(), Error> {
        self.codes.set(codes).map_err(|_| Error::internal("phone codes are already plugged in"))
    }
}
