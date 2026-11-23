//! Khách vãng lai xác nhận số điện thoại bằng mã một lần (D-20261118-step60-bookings mục 11), như tra
//! đơn vãng lai: mã 6 số gửi qua `dyn SmsSender`, database chỉ giữ bản băm có pepper riêng của khách,
//! hết hạn sau 10 phút, sai quá 5 lần thì phải xin mã mới, mỗi mã dùng một lần.
//!
//! Mã không bao giờ nằm trong job hay log: gửi SMS đồng bộ NGOÀI transaction.

use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::secret::sha256;
use autoxide_kernel::sms::{PhoneNumber, SmsMessage, SmsPurpose, sms_disabled};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::throttle::{Outcome, Throttle};
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use std::net::IpAddr;
use uuid::Uuid;

use crate::booking::DEFAULT_COUNTRY;
use crate::entity::guest_challenge;
use crate::service::{BookingsService, now};

/// Nhãn dẫn pepper của mã một lần từ khoá gốc.
const OTP_PURPOSE: &str = "bookings.otp";
/// Mã sống chừng này.
const TTL_MINUTES: i64 = 10;
/// Sai chừng này lần thì mã hỏng.
const MAX_ATTEMPTS: i32 = 5;
/// Tối đa chừng này mã cho một số điện thoại mỗi giờ.
const PER_PHONE_PER_HOUR: u64 = 5;
/// Tối đa chừng này mã từ một địa chỉ client mỗi giờ (mọi số cộng lại): chặn một máy xin mã cho
/// hàng loạt số khác nhau, mỗi tin tốn tiền SMS của cửa hàng (pentest 2026-11; như tra đơn vãng lai).
const PER_CLIENT_PER_HOUR: u32 = 20;

/// Mã vừa gửi.
#[derive(Debug, Clone)]
pub(crate) struct Challenge {
    pub id: Uuid,
    pub expires_at: Timestamp,
}

pub(crate) fn code_invalid() -> Error {
    Error::new(
        ErrorKind::Validation,
        "bookings.code_invalid",
        "The code is wrong, used or expired; ask for a new code if it has expired.",
    )
}

fn too_many_codes(retry_after: u64) -> Error {
    Error::too_many_requests(
        "bookings.too_many_codes",
        "Too many codes were asked for. Wait {seconds} seconds, then try again.",
    )
    .with_param("seconds", retry_after)
    .with_retry_after(retry_after)
}

fn phone_invalid() -> Error {
    Error::validation(vec![FieldError::new("phone", "invalid", "is not a phone number")])
}

/// Băm `code` của mã `id` bằng pepper của khách: mã 6 số chỉ có một triệu khả năng.
fn code_hash(pepper: &[u8; 32], id: Uuid, code: &str) -> String {
    let pepper: String = pepper.iter().map(|byte| format!("{byte:02x}")).collect();
    sha256(&format!("{pepper}|{id}|{code}"))
}

/// So hai chuỗi trong thời gian không phụ thuộc chỗ khác nhau đầu tiên.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Mã 6 chữ số ngẫu nhiên đều (loại bỏ phần dư để không lệch về số nhỏ).
fn random_code() -> Result<String, Error> {
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

impl BookingsService {
    async fn pepper(&self, tenant: &TenantContext) -> Result<[u8; 32], Error> {
        let secret = self
            .secret
            .as_ref()
            .ok_or_else(|| Error::internal("module `bookings` has no secret key"))?;
        secret.derive_for(tenant.id(), OTP_PURPOSE).await
    }

    /// Gửi mã một lần tới số `raw` để khách vãng lai đặt / huỷ lịch.
    pub(crate) async fn send_code(
        &self,
        tenant: &TenantContext,
        raw: &str,
        client: Option<IpAddr>,
    ) -> Result<Challenge, Error> {
        let phone = PhoneNumber::parse(raw, DEFAULT_COUNTRY).ok_or_else(phone_invalid)?;
        let Some(sender) = self.sms.clone() else {
            return Err(sms_disabled());
        };
        if let Some(ip) = client {
            let hour = std::time::Duration::from_secs(3600);
            let by_client = Throttle::new("bookings", PER_CLIENT_PER_HOUR, hour)?;
            if let Outcome::Limited { retry_after } =
                by_client.hit(tenant, &format!("otp-ip:{ip}")).await?
            {
                return Err(too_many_codes(retry_after));
            }
        }
        let at = now();
        let recent = guest_challenge::Entity::find()
            .filter(guest_challenge::Column::Phone.eq(phone.as_str()))
            .filter(guest_challenge::Column::CreatedAt.gt(at - SignedDuration::from_hours(1)))
            .scoped(tenant)
            .count(tenant.database().read())
            .await?;
        if recent >= PER_PHONE_PER_HOUR {
            return Err(too_many_codes(3600));
        }
        let code = random_code()?;
        let id = Uuid::now_v7();
        let pepper = self.pepper(tenant).await?;
        let expires_at = at + SignedDuration::from_mins(TTL_MINUTES);
        let transaction = tenant.database().begin_write().await?;
        guest_challenge::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            phone: Set(phone.as_str().to_owned()),
            code_hash: Set(code_hash(&pepper, id, &code)),
            attempts: Set(0),
            expires_at: Set(expires_at),
            consumed_at: Set(None),
            created_at: Set(at),
        }
        .insert(&transaction)
        .await?;
        transaction.commit().await?;
        let text = format!("Your booking code is {code}. It expires in {TTL_MINUTES} minutes.");
        sender
            .send(SmsMessage::new(
                tenant.id(),
                phone.clone(),
                text,
                SmsPurpose::Otp,
                format!("bookings.otp:{id}"),
            ))
            .await?;
        tracing::info!(phone = %phone.masked(), "booking code sent");
        Ok(Challenge { id, expires_at })
    }

    /// Kiểm mã `code` của lần gửi `id`; đúng thì dùng hết mã và trả số điện thoại (E.164).
    pub(crate) async fn verify_code(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        code: &str,
    ) -> Result<String, Error> {
        let pepper = self.pepper(tenant).await?;
        let transaction = tenant.database().begin_write().await?;
        // Khoá lần gửi mã (D-181): N lần đoán cùng lúc nối đuôi nhau, mỗi lần thấy số lần thử của
        // lần trước — không thì Postgres cho cả N cùng thấy dưới giới hạn và cùng được so mã.
        let Some(row) = autoxide_kernel::rows::lock_one(
            &transaction,
            guest_challenge::Entity::find_by_id(id).scoped(tenant),
        )
        .await?
        else {
            return Err(code_invalid());
        };
        let at = now();
        if row.consumed_at.is_some() || row.expires_at <= at || row.attempts >= MAX_ATTEMPTS {
            return Err(code_invalid());
        }
        if !same(&row.code_hash, &code_hash(&pepper, id, code.trim())) {
            guest_challenge::Entity::update_many()
                .col_expr(
                    guest_challenge::Column::Attempts,
                    Expr::col(guest_challenge::Column::Attempts).add(1),
                )
                .filter(guest_challenge::Column::Id.eq(id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
            transaction.commit().await?;
            return Err(code_invalid());
        }
        // Dùng một lần: hai request cùng mã thì chỉ một bên được.
        let used = guest_challenge::Entity::update_many()
            .col_expr(guest_challenge::Column::ConsumedAt, Expr::value(at))
            .filter(guest_challenge::Column::Id.eq(id))
            .filter(guest_challenge::Column::ConsumedAt.is_null())
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if used.rows_affected == 0 {
            return Err(code_invalid());
        }
        transaction.commit().await?;
        Ok(row.phone)
    }
}
