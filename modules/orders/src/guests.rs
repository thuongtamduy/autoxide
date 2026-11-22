//! Mua không cần tài khoản (đặc tả 45 mục "Mua không cần tài khoản", D-201, D-202, D-204):
//!
//! - **Tra đơn** bằng số đơn + số điện thoại → mã một lần 6 số qua SMS → token xem đơn mở đúng một
//!   đơn (header `Order-Token`). "Không thấy đơn" và "sai số điện thoại" trả lời giống hệt nhau.
//! - **Thao tác** trên đơn bằng token: huỷ, đổi địa chỉ (cần mã lần hai), mở hỗ trợ — gọi CÙNG
//!   service với đường có tài khoản, chống trùng theo `Idempotency-Key` ở `orders_guest_requests`.
//! - **Gắn đơn** vãng lai vào tài khoản: người mua đăng nhập đưa token xem đơn (chứng minh số điện
//!   thoại) → mọi đơn vãng lai cùng số trong khách về tài khoản đó.
//!
//! Mã và token chỉ lưu bản băm; mã không bao giờ vào job hay log (gửi SMS đồng bộ, NGOÀI
//! transaction). Số điện thoại so bằng khoá liên hệ theo khách (`SecretKey::phone_key`).

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::net::IpAddr;

use autoxide_address::api::AddressInput;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::reference::fingerprint;
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::secret::{random_token, sha256};
use autoxide_kernel::sms::{PhoneNumber, SmsMessage, SmsPurpose};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_kernel::throttle::Throttle;
use autoxide_shipping::api::{QuoteItem, QuoteRequest};
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter,
    QueryOrder, Set,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::OrderStatus;
use crate::carts::ShipTo;
use crate::checkout::Contact;
use crate::entity::{fulfillment, guest_request, line, lookup_code, lookup_token, order};
use crate::errors;
use crate::events::{Change, DeliveryAddressChanged, OrdersClaimed};
use crate::lifecycle;
use crate::service::{OrdersService, now};

/// Mã nước mặc định khi chuẩn hoá số điện thoại người mua nhập (`0…` → `+84…`).
pub(crate) const DEFAULT_COUNTRY: &str = "84";
/// Nhãn dẫn pepper của mã một lần từ khoá gốc.
const OTP_PURPOSE: &str = "orders.lookup_otp";
/// Mã sống chừng này.
const CODE_TTL_MINUTES: i64 = 10;
/// Sai chừng này lần thì mã hỏng.
const MAX_ATTEMPTS: i32 = 5;
/// Mục đích của mã: mở token xem đơn.
const PURPOSE_LOOKUP: &str = "lookup";
/// Mục đích của mã: đổi địa chỉ giao (lần hai).
const PURPOSE_ADDRESS: &str = "address";
/// Header mang token xem đơn.
pub(crate) const ORDER_TOKEN: &str = "order-token";

/// Câu SMS (khoá dịch `text:<câu>`): macro chỉ trả lại chuỗi, để `cargo xtask check` thấy câu và
/// đòi bản dịch tiếng Việt.
macro_rules! answer_text {
    ($text:literal) => {
        $text
    };
}

const LOOKUP_SMS: &str =
    answer_text!("Your code to view order {number} is {code}. It expires in {minutes} minutes.");
const ADDRESS_SMS: &str = answer_text!(
    "Your code to change the delivery address of order {number} is {code}. It expires in \
     {minutes} minutes."
);

/// Mã vừa gửi.
#[derive(Debug, Clone)]
pub(crate) struct CodeSent {
    pub id: Uuid,
    pub order_id: Uuid,
    pub expires_at: Timestamp,
    /// Gửi lại được sau chừng này giây.
    pub resend_after: i64,
    /// Số đã nhận mã, dạng che.
    pub masked_phone: String,
}

/// Token xem đơn vừa cấp (chỉ trả một lần).
#[derive(Debug, Clone)]
pub(crate) struct AccessIssued {
    pub token: String,
    pub order_id: Uuid,
    pub expires_at: Timestamp,
}

/// Token xem đơn đã kiểm.
#[derive(Debug, Clone)]
pub(crate) struct GuestAccess {
    pub token_id: Uuid,
    pub order_id: Uuid,
    pub phone_hash: String,
}

/// Địa chỉ giao mới của người mua vãng lai.
#[derive(Debug, Clone)]
pub(crate) struct AddressChange {
    pub address: AddressInput,
    pub recipient_name: Option<String>,
    pub phone: Option<String>,
    /// Mã lần hai (`POST /public/orders/{id}/address/otp`).
    pub code: String,
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

/// Băm `code` của mã `id` bằng pepper của khách: mã 6 số chỉ có một triệu khả năng.
fn code_hash(pepper: &[u8; 32], id: Uuid, code: &str) -> String {
    let pepper: String = pepper.iter().map(|byte| format!("{byte:02x}")).collect();
    sha256(&format!("{pepper}|{id}|{code}"))
}

/// So hai chuỗi trong thời gian không phụ thuộc chỗ khác nhau đầu tiên.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Số điện thoại che cho response xem đơn bằng token: giữ 3 số đầu, 3 số cuối (`090•••567`).
pub(crate) fn mask_phone(raw: &str) -> String {
    let digits: Vec<char> = raw.chars().filter(char::is_ascii_digit).collect();
    // Số dạng `+84…`: hiện như người mua quen nhìn (`0…`).
    let local: Vec<char> = match (raw.trim_start().starts_with('+'), digits.as_slice()) {
        (true, ['8', '4', rest @ ..]) => std::iter::once('0').chain(rest.iter().copied()).collect(),
        _ => digits,
    };
    if local.len() <= 6 {
        return "•••".to_owned();
    }
    let head: String = local[..3].iter().collect();
    let tail: String = local[local.len() - 3..].iter().collect();
    format!("{head}•••{tail}")
}

/// Tên che: họ và tên đệm chỉ còn chữ cái đầu, giữ tên (`Nguyễn Văn An` → `N. V. An`).
pub(crate) fn mask_name(raw: &str) -> String {
    let words: Vec<&str> = raw.split_whitespace().collect();
    match words.split_last() {
        Some((last, rest)) => {
            let mut parts: Vec<String> = rest
                .iter()
                .filter_map(|word| word.chars().next())
                .map(|first| format!("{first}."))
                .collect();
            parts.push((*last).to_owned());
            parts.join(" ")
        }
        None => String::new(),
    }
}

/// Địa chỉ chỉ tới cấp phường / xã (bỏ số nhà, đường).
pub(crate) fn mask_ship_to(raw: &str) -> Option<Value> {
    let ship_to: ShipTo = serde_json::from_str(raw).ok()?;
    let address = ship_to.address.map(|address| {
        json!({"country": address.country, "province_code": address.province_code,
            "province_name": address.province_name, "commune_code": address.commune_code,
            "commune_name": address.commune_name})
    });
    Some(json!({
        "recipient_name": ship_to.recipient_name.as_deref().map(mask_name),
        "phone": ship_to.phone.as_deref().map(mask_phone),
        "address": address,
    }))
}

impl OrdersService {
    /// Khoá liên hệ của số `raw` trong khách (`None` khi không phải số điện thoại).
    pub(crate) async fn buyer_key(
        &self,
        tenant: &TenantContext,
        raw: &str,
    ) -> Result<Option<String>, Error> {
        let Some(phone) = PhoneNumber::parse(raw, DEFAULT_COUNTRY) else { return Ok(None) };
        Ok(Some(self.secret.phone_key(tenant.id(), &phone).await?))
    }

    async fn pepper(&self, tenant: &TenantContext) -> Result<[u8; 32], Error> {
        self.secret.derive_for(tenant.id(), OTP_PURPOSE).await
    }

    /// Đơn số `number` có số liên hệ `phone`; không thấy hay sai số → cùng một lỗi.
    async fn order_for_lookup(
        &self,
        tenant: &TenantContext,
        number: &str,
        phone: &PhoneNumber,
    ) -> Result<order::Model, Error> {
        let row = order::Entity::find()
            .filter(order::Column::Number.eq(number.trim()))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(errors::lookup_failed)?;
        let contact: Contact =
            serde_json::from_str(&row.contact).map_err(|_| errors::lookup_failed())?;
        let matches = PhoneNumber::parse(&contact.phone, DEFAULT_COUNTRY)
            .is_some_and(|own| own.as_str() == phone.as_str());
        if matches { Ok(row) } else { Err(errors::lookup_failed()) }
    }

    /// `POST /public/orders/lookup`: gửi mã một lần tới số của đơn. Giới hạn theo (số đơn, số
    /// điện thoại) và theo IP băm; đếm TRƯỚC khi tìm đơn, nên dò số đơn cũng bị chặn.
    pub(crate) async fn start_lookup(
        &self,
        tenant: &TenantContext,
        number: &str,
        raw_phone: &str,
        client: Option<IpAddr>,
    ) -> Result<CodeSent, Error> {
        let mut problems = Vec::new();
        if !(1..=40).contains(&number.trim().chars().count()) {
            problems.push(FieldError::new("number", "length", "must be 1 to 40 characters"));
        }
        let phone = PhoneNumber::parse(raw_phone, DEFAULT_COUNTRY);
        if phone.is_none() {
            problems.push(FieldError::new("phone", "invalid", "is not a phone number"));
        }
        let Some(phone) = phone.filter(|_| problems.is_empty()) else {
            return Err(Error::validation(problems));
        };
        let hour = std::time::Duration::from_secs(3600);
        if let Some(ip) = client {
            let by_ip = Throttle::new("orders", self.config.lookup_ip_per_hour, hour)?;
            if let autoxide_kernel::throttle::Outcome::Limited { retry_after } =
                by_ip.hit(tenant, &format!("lookup-ip:{ip}")).await?
            {
                return Err(errors::lookup_too_many(retry_after));
            }
        }
        let by_pair = Throttle::new("orders", self.config.lookup_per_hour, hour)?;
        let pair = format!("lookup:{}|{}", number.trim(), phone.as_str());
        if let autoxide_kernel::throttle::Outcome::Limited { retry_after } =
            by_pair.hit(tenant, &pair).await?
        {
            return Err(errors::lookup_too_many(retry_after));
        }
        let row = self.order_for_lookup(tenant, number, &phone).await?;
        self.send_code(tenant, &row, PURPOSE_LOOKUP, None, &phone).await
    }

    /// Gửi một mã mục đích `purpose` cho đơn `row` tới `phone`; mã trước còn mới quá thì 429.
    async fn send_code(
        &self,
        tenant: &TenantContext,
        row: &order::Model,
        purpose: &str,
        token_id: Option<Uuid>,
        phone: &PhoneNumber,
    ) -> Result<CodeSent, Error> {
        let sender = self.sms.clone().ok_or_else(errors::sms_unavailable)?;
        let at = now();
        let resend = self.config.lookup_resend_seconds;
        let mut last = lookup_code::Entity::find()
            .filter(lookup_code::Column::OrderId.eq(row.id))
            .filter(lookup_code::Column::Purpose.eq(purpose))
            .scoped(tenant);
        if let Some(token_id) = token_id {
            last = last.filter(lookup_code::Column::TokenId.eq(token_id));
        }
        let last = last
            .order_by_desc(lookup_code::Column::CreatedAt)
            .one(tenant.database().read())
            .await?;
        if let Some(last) = last {
            let wait = resend - (at - last.created_at).as_secs();
            if wait > 0 {
                return Err(errors::lookup_resend_too_soon(u64::try_from(wait).unwrap_or(1)));
            }
        }
        let code = random_code()?;
        let id = Uuid::now_v7();
        let pepper = self.pepper(tenant).await?;
        let expires_at = at + SignedDuration::from_mins(CODE_TTL_MINUTES);
        lookup_code::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            order_id: Set(row.id),
            purpose: Set(purpose.to_owned()),
            token_id: Set(token_id),
            code_hash: Set(code_hash(&pepper, id, &code)),
            attempts: Set(0),
            expires_at: Set(expires_at),
            consumed_at: Set(None),
            created_at: Set(at),
        }
        .insert(tenant.database().write())
        .await?;
        let template = if purpose == PURPOSE_ADDRESS { ADDRESS_SMS } else { LOOKUP_SMS };
        let language = Language::from_tag(&row.locale).unwrap_or(Language::En);
        let minutes = CODE_TTL_MINUTES.to_string();
        let text = self.translations.describe_with(
            template,
            &[("number", &row.number), ("code", &code), ("minutes", &minutes)],
            language,
        );
        sender
            .send(SmsMessage::new(
                tenant.id(),
                phone.clone(),
                text,
                SmsPurpose::Otp,
                format!("orders.otp:{id}"),
            ))
            .await?;
        tracing::info!(order_id = %row.id, purpose, phone = %phone.masked(), "order code sent");
        Ok(CodeSent {
            id,
            order_id: row.id,
            expires_at,
            resend_after: resend,
            masked_phone: mask_phone(phone.as_str()),
        })
    }

    /// Kiểm mã `code` của lần gửi `id` (mục đích `purpose`); đúng thì dùng hết mã, trả dòng mã.
    async fn consume_code(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        purpose: &str,
        code: &str,
    ) -> Result<lookup_code::Model, Error> {
        let pepper = self.pepper(tenant).await?;
        let write = tenant.database().write();
        let row = lookup_code::Entity::find_by_id(id)
            .filter(lookup_code::Column::Purpose.eq(purpose))
            .scoped(tenant)
            .one(write)
            .await?
            .ok_or_else(errors::lookup_code_invalid)?;
        let at = now();
        if row.consumed_at.is_some() || row.expires_at <= at {
            return Err(errors::lookup_code_invalid());
        }
        // Giành một lượt thử TRƯỚC khi so mã, bằng một câu cộng có trần (D-181): N lần đoán cùng
        // lúc thì chỉ 5 lần đầu được so. Đọc số lượt rồi mới so (như trước) thì cả N cùng thấy dưới
        // giới hạn — trên cả SQLite lẫn Postgres, vì không có transaction nào giữ dòng.
        let counted = rows::adjust::<lookup_code::Entity, _>(
            write,
            lookup_code::Column::Attempts,
            Condition::all()
                .add(lookup_code::Column::Id.eq(id))
                .add(lookup_code::Column::TenantId.eq(tenant.id().as_uuid()))
                .add(lookup_code::Column::ConsumedAt.is_null()),
            1,
            Bounds { floor: 0, ceiling: i64::from(MAX_ATTEMPTS) },
        )
        .await?;
        if counted == 0 || !same(&row.code_hash, &code_hash(&pepper, id, code.trim())) {
            return Err(errors::lookup_code_invalid());
        }
        // Dùng một lần: hai request cùng mã thì chỉ một bên được.
        let used = lookup_code::Entity::update_many()
            .col_expr(lookup_code::Column::ConsumedAt, Expr::value(at))
            .filter(lookup_code::Column::Id.eq(id))
            .filter(lookup_code::Column::ConsumedAt.is_null())
            .scoped(tenant)
            .exec(write)
            .await?;
        if used.rows_affected == 0 {
            return Err(errors::lookup_code_invalid());
        }
        Ok(row)
    }

    /// `POST /public/orders/lookup/{id}/confirm`: mã đúng → token xem đơn (trả một lần).
    pub(crate) async fn confirm_lookup(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        code: &str,
    ) -> Result<AccessIssued, Error> {
        let used = self.consume_code(tenant, id, PURPOSE_LOOKUP, code).await?;
        let row = self.order(tenant, used.order_id).await?;
        let contact: Contact = serde_json::from_str(&row.contact).map_err(Error::internal)?;
        let phone_hash = self
            .buyer_key(tenant, &contact.phone)
            .await?
            .ok_or_else(errors::lookup_code_invalid)?;
        let token = random_token(32)?;
        let at = now();
        let expires_at = at + SignedDuration::from_mins(self.config.lookup_token_minutes);
        lookup_token::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            order_id: Set(row.id),
            phone_hash: Set(phone_hash),
            token_hash: Set(sha256(&token)),
            expires_at: Set(expires_at),
            otp_confirmed_at: Set(at),
            created_at: Set(at),
        }
        .insert(tenant.database().write())
        .await?;
        Ok(AccessIssued { token, order_id: row.id, expires_at })
    }

    /// Token xem đơn còn hạn (header `Order-Token`); không có hay sai → 401.
    pub(crate) async fn guest_access(
        &self,
        tenant: &TenantContext,
        token: Option<&str>,
    ) -> Result<GuestAccess, Error> {
        let token = token.map(str::trim).filter(|token| !token.is_empty());
        let Some(token) = token else { return Err(errors::lookup_token_invalid()) };
        let row = lookup_token::Entity::find()
            .filter(lookup_token::Column::TokenHash.eq(sha256(token)))
            .filter(lookup_token::Column::ExpiresAt.gt(now()))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(errors::lookup_token_invalid)?;
        Ok(GuestAccess { token_id: row.id, order_id: row.order_id, phone_hash: row.phone_hash })
    }

    /// Đơn mà token mở được; đơn khác → 404 (token chỉ mở đúng một đơn).
    pub(crate) async fn guest_order(
        &self,
        tenant: &TenantContext,
        access: &GuestAccess,
        id: Uuid,
    ) -> Result<order::Model, Error> {
        if access.order_id != id {
            return Err(errors::order_not_found());
        }
        self.order(tenant, id).await
    }

    /// Đã làm thao tác `action` với khoá `reference` chưa: cùng nội dung → `true` (gọi lại), khác
    /// nội dung → 422.
    async fn guest_replayed(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        action: &str,
        reference: &str,
        fingerprint: &str,
    ) -> Result<bool, Error> {
        let found = guest_request::Entity::find()
            .filter(guest_request::Column::OrderId.eq(order_id))
            .filter(guest_request::Column::Action.eq(action))
            .filter(guest_request::Column::Reference.eq(reference))
            .scoped(tenant)
            .one(tenant.database().write())
            .await?;
        match found {
            Some(found) if found.fingerprint == fingerprint => Ok(true),
            Some(_) => Err(errors::guest_key_reused(reference)),
            None => Ok(false),
        }
    }

    /// Ghi thao tác đã làm (bỏ qua nếu một request song song đã ghi).
    async fn remember_guest_request(
        connection: &impl ConnectionTrait,
        tenant: &TenantContext,
        order_id: Uuid,
        action: &str,
        reference: &str,
        fingerprint: &str,
    ) -> Result<(), Error> {
        let row = guest_request::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            order_id: Set(order_id),
            action: Set(action.to_owned()),
            reference: Set(reference.to_owned()),
            fingerprint: Set(fingerprint.to_owned()),
            created_at: Set(now()),
        };
        guest_request::Entity::insert(row)
            .on_conflict_do_nothing()
            .exec_without_returning(connection)
            .await?;
        Ok(())
    }

    /// `POST /public/orders/{id}/cancel`: cùng chính sách `buyer_cancel_until` với người mua có
    /// tài khoản.
    pub(crate) async fn guest_cancel(
        &self,
        tenant: &TenantContext,
        access: &GuestAccess,
        id: Uuid,
        reason: Option<String>,
        reference: &str,
    ) -> Result<order::Model, Error> {
        let row = self.guest_order(tenant, access, id).await?;
        let print = fingerprint(&json!({"reason": reason}))?;
        if self.guest_replayed(tenant, id, "cancel", reference, &print).await? {
            return Ok(row);
        }
        let row = self.cancel_as_buyer(tenant, row, Actor::public(), reason).await?;
        Self::remember_guest_request(
            tenant.database().write(),
            tenant,
            id,
            "cancel",
            reference,
            &print,
        )
        .await?;
        Ok(row)
    }

    /// `POST /public/orders/{id}/address/otp`: mã lần hai để đổi địa chỉ, gửi tới số của đơn.
    pub(crate) async fn guest_address_code(
        &self,
        tenant: &TenantContext,
        access: &GuestAccess,
        id: Uuid,
    ) -> Result<CodeSent, Error> {
        let row = self.guest_order(tenant, access, id).await?;
        self.ensure_address_editable(tenant, &row).await?;
        let contact: Contact = serde_json::from_str(&row.contact).map_err(Error::internal)?;
        let phone = PhoneNumber::parse(&contact.phone, DEFAULT_COUNTRY)
            .ok_or_else(errors::lookup_token_invalid)?;
        self.send_code(tenant, &row, PURPOSE_ADDRESS, Some(access.token_id), &phone).await
    }

    /// Đơn còn đổi được địa chỉ giao: có hàng phải giao, trước `shipping`, chưa yêu cầu vận đơn.
    async fn ensure_address_editable(
        &self,
        tenant: &TenantContext,
        row: &order::Model,
    ) -> Result<(), Error> {
        let status = OrderStatus::parse(&row.status).unwrap_or(OrderStatus::Cancelled);
        let early = matches!(
            status,
            OrderStatus::PendingPayment | OrderStatus::Confirmed | OrderStatus::Preparing
        );
        if !early || row.ship_to.is_none() || row.pickup_org_id.is_some() {
            return Err(errors::address_locked());
        }
        let requested = fulfillment::Entity::find()
            .filter(fulfillment::Column::OrderId.eq(row.id))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        if requested.is_some() {
            return Err(errors::address_locked());
        }
        Ok(())
    }

    /// `POST /public/orders/{id}/address`: đổi địa chỉ giao bằng mã lần hai. Phí giao tính lại cho
    /// cùng cách giao; khác phí (hay cách giao không tới được) → 409, không đổi gì.
    pub(crate) async fn guest_change_address(
        &self,
        tenant: &TenantContext,
        access: &GuestAccess,
        id: Uuid,
        change: AddressChange,
        reference: &str,
    ) -> Result<order::Model, Error> {
        let row = self.guest_order(tenant, access, id).await?;
        let print = fingerprint(&json!({"address": change.address,
            "recipient_name": change.recipient_name, "phone": change.phone}))?;
        if self.guest_replayed(tenant, id, "address", reference, &print).await? {
            return Ok(row);
        }
        let mut problems = Vec::new();
        if change.phone.as_deref().is_some_and(|phone| !crate::checkout::phone_valid(phone)) {
            problems.push(FieldError::new("phone", "invalid", "is not a phone number"));
        }
        if change
            .recipient_name
            .as_deref()
            .is_some_and(|name| !(1..=200).contains(&name.trim().chars().count()))
        {
            problems.push(FieldError::new(
                "recipient_name",
                "length",
                "must be 1 to 200 characters",
            ));
        }
        let address = match &self.units {
            Some(units) => match units.validate(change.address.clone(), "address").await {
                Ok(address) => Some(address),
                Err(fields) => {
                    problems.extend(fields);
                    None
                }
            },
            None => Some(crate::carts::unchecked_address(change.address.clone())?),
        };
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        self.ensure_address_editable(tenant, &row).await?;
        // Phí trước, mã sau: phí khác thì mã chưa bị dùng, người mua sửa địa chỉ rồi gửi lại.
        self.check_fee_unchanged(tenant, &row, address.as_ref()).await?;
        self.address_code_of(tenant, access, &change.code).await?;
        let old: ShipTo = row
            .ship_to
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .ok_or_else(errors::address_locked)?;
        let ship_to = ShipTo {
            address_id: None,
            recipient_name: change.recipient_name.or(old.recipient_name),
            phone: change.phone.or(old.phone),
            address,
        };
        let ship_to = serde_json::to_string(&ship_to).map_err(Error::internal)?;
        let (database, outbox) = (tenant.database(), &self.outbox);
        let reference = reference.to_owned();
        database
            .write_with_retry(move || {
                let (ship_to, reference, print) =
                    (ship_to.clone(), reference.clone(), print.clone());
                async move {
                    let transaction = database.begin_write().await?;
                    let current = lifecycle::lock(&transaction, tenant, id).await?;
                    let status =
                        OrderStatus::parse(&current.status).unwrap_or(OrderStatus::Cancelled);
                    if !matches!(
                        status,
                        OrderStatus::PendingPayment
                            | OrderStatus::Confirmed
                            | OrderStatus::Preparing
                    ) {
                        return Err(errors::address_locked());
                    }
                    let mut edit: order::ActiveModel = current.clone().into();
                    edit.ship_to = Set(Some(ship_to));
                    edit.version = Set(current.version + 1);
                    edit.updated_at = Set(now());
                    let saved = edit.update(&transaction).await?;
                    Self::remember_guest_request(
                        &transaction,
                        tenant,
                        id,
                        "address",
                        &reference,
                        &print,
                    )
                    .await?;
                    let event = DeliveryAddressChanged(Change::after(json!({
                        "order_id": id, "tenant_id": saved.tenant_id, "org_id": saved.org_id,
                        "channel": saved.channel, "number": saved.number, "by": "buyer",
                    })));
                    outbox
                        .record(&transaction, tenant, Actor::public(), subject(id), &event)
                        .await?;
                    transaction.commit().await?;
                    Ok(saved)
                }
            })
            .await
    }

    /// Mã đổi địa chỉ phải do chính token này xin (người khác cầm mã cũng không dùng được).
    async fn address_code_of(
        &self,
        tenant: &TenantContext,
        access: &GuestAccess,
        code: &str,
    ) -> Result<lookup_code::Model, Error> {
        let latest = lookup_code::Entity::find()
            .filter(lookup_code::Column::OrderId.eq(access.order_id))
            .filter(lookup_code::Column::Purpose.eq(PURPOSE_ADDRESS))
            .filter(lookup_code::Column::TokenId.eq(access.token_id))
            .filter(lookup_code::Column::ConsumedAt.is_null())
            .scoped(tenant)
            .order_by_desc(lookup_code::Column::CreatedAt)
            .one(tenant.database().read())
            .await?
            .ok_or_else(errors::lookup_code_invalid)?;
        self.consume_code(tenant, latest.id, PURPOSE_ADDRESS, code).await
    }

    /// Phí giao của cùng cách giao tới địa chỉ mới phải bằng phí đã tính trong đơn.
    async fn check_fee_unchanged(
        &self,
        tenant: &TenantContext,
        row: &order::Model,
        address: Option<&autoxide_shipping::api::Address>,
    ) -> Result<(), Error> {
        let Some(rates) = &self.sources.shipping else { return Ok(()) };
        let Some(option_key) = row
            .shipping_option
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .and_then(|option| option["option_key"].as_str().map(str::to_owned))
        else {
            return Ok(());
        };
        let lines = line::Entity::find()
            .filter(line::Column::OrderId.eq(row.id))
            .filter(line::Column::ParentLineId.is_null())
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let ids: Vec<Uuid> = lines.iter().map(|line| line.variant_id).collect();
        let variants = self.sources.catalog.variants(tenant, &ids, None).await?;
        let items = lines
            .iter()
            .filter_map(|line| {
                let variant =
                    variants.iter().find(|variant| variant.variant_id == line.variant_id)?;
                Some(
                    QuoteItem::new(line.variant_id, line.quantity)
                        .sku(variant.sku.clone())
                        .requires_shipping(variant.requires_shipping)
                        .weight(variant.weight_grams)
                        .dimensions(variant.length_mm, variant.width_mm, variant.height_mm),
                )
            })
            .collect();
        let subtotal =
            autoxide_kernel::money::Money::from_columns(row.subtotal_minor, &row.currency)?;
        let mut request = QuoteRequest::new(OrgId::from_uuid(row.org_id), items, subtotal, now())
            .locale(row.locale.clone());
        if let Some(address) = address {
            request = request.destination(address.clone());
        }
        let quoted = rates.quote(tenant, &request).await?;
        let same_fee = quoted
            .options
            .iter()
            .find(|option| option.option_key == option_key)
            .is_some_and(|option| option.amount.minor() == row.shipping_fee_minor);
        if same_fee { Ok(()) } else { Err(errors::address_change_needs_payment()) }
    }

    /// `POST /public/orders/{id}/support` (và bản có tài khoản): mở phiếu hỗ trợ gắn với đơn ở
    /// `support` (D-204); chưa cài `support` → 404 `orders.support_unavailable`.
    pub(crate) async fn open_support(
        &self,
        tenant: &TenantContext,
        row: &order::Model,
        requester: autoxide_support::api::Requester,
        message: &str,
        reference: &str,
    ) -> Result<autoxide_support::api::TicketRef, Error> {
        let Some(intake) = &self.support else { return Err(errors::support_unavailable()) };
        let message = message.trim();
        if !(1..=5000).contains(&message.chars().count()) {
            return Err(Error::validation(vec![FieldError::new(
                "message",
                "length",
                "must be 1 to 5000 characters",
            )]));
        }
        let contact: Option<Contact> = serde_json::from_str(&row.contact).ok();
        let ticket = autoxide_support::api::NewTicket {
            org_id: row.org_id,
            requester,
            requester_name: contact.map(|contact| contact.name).unwrap_or_default(),
            category: "order".to_owned(),
            subject: format!("Order {}", row.number),
            body: message.to_owned(),
            links: vec![format!("orders.order:{}", row.id)],
            reference: format!("orders.order:{}:{reference}", row.id),
        };
        intake.open(tenant, ticket).await
    }

    /// Mở hỗ trợ bằng token xem đơn: người hỏi là số điện thoại đã qua OTP.
    pub(crate) async fn guest_support(
        &self,
        tenant: &TenantContext,
        access: &GuestAccess,
        id: Uuid,
        message: &str,
        reference: &str,
    ) -> Result<autoxide_support::api::TicketRef, Error> {
        let row = self.guest_order(tenant, access, id).await?;
        let contact: Contact = serde_json::from_str(&row.contact).map_err(Error::internal)?;
        let phone = PhoneNumber::parse(&contact.phone, DEFAULT_COUNTRY)
            .ok_or_else(errors::lookup_token_invalid)?;
        let requester = autoxide_support::api::Requester::VerifiedPhone(phone.as_str().to_owned());
        self.open_support(tenant, &row, requester, message, reference).await
    }

    /// Mở hỗ trợ cho đơn của người mua có tài khoản.
    pub(crate) async fn buyer_support(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        id: Uuid,
        message: &str,
        reference: &str,
    ) -> Result<autoxide_support::api::TicketRef, Error> {
        let row = self.order(tenant, id).await?;
        if row.buyer_user_id != Some(buyer) {
            return Err(errors::order_not_found());
        }
        let requester = autoxide_support::api::Requester::User(buyer);
        self.open_support(tenant, &row, requester, message, reference).await
    }

    /// `POST /me/orders/claim`: người mua đăng nhập đưa token xem đơn (đã chứng minh số điện
    /// thoại) → mọi đơn vãng lai cùng số trong khách về tài khoản. Gọi lại không đổi gì thêm.
    pub(crate) async fn claim_orders(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        token: &str,
    ) -> Result<Vec<Uuid>, Error> {
        // Người gọi đã đăng nhập: token sai là lỗi của trường (422), không phải 401 — 401 nghĩa là
        // phải đăng nhập lại.
        let access = match self.guest_access(tenant, Some(token)).await {
            Err(error) if error.code() == "orders.lookup_token_invalid" => {
                return Err(Error::validation(vec![FieldError::new(
                    "lookup_token",
                    "invalid",
                    "is wrong or expired; look the order up again",
                )]));
            }
            other => other?,
        };
        let read = tenant.database().read();
        let candidates: Vec<order::Model> = order::Entity::find()
            .filter(order::Column::BuyerKey.eq(access.phone_hash.as_str()))
            .filter(order::Column::BuyerUserId.is_null())
            .scoped(tenant)
            .all(read)
            .await?;
        // Đơn của token luôn được gắn (kể cả đơn cũ chưa có khoá liên hệ).
        let mut ids: Vec<Uuid> = candidates.iter().map(|row| row.id).collect();
        let own = self.order(tenant, access.order_id).await?;
        if own.buyer_user_id.is_none() && !ids.contains(&own.id) {
            ids.push(own.id);
        }
        if ids.is_empty() {
            return Ok(ids);
        }
        let (database, outbox) = (tenant.database(), &self.outbox);
        database
            .write_with_retry(move || {
                let ids = ids.clone();
                async move {
                    let transaction = database.begin_write().await?;
                    let at = now();
                    let claimed = order::Entity::update_many()
                        .col_expr(order::Column::BuyerUserId, Expr::value(buyer))
                        .col_expr(order::Column::Version, Expr::col(order::Column::Version).add(1))
                        .col_expr(order::Column::UpdatedAt, Expr::value(at))
                        .filter(order::Column::Id.is_in(ids.clone()))
                        .filter(order::Column::BuyerUserId.is_null())
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                    if claimed.rows_affected > 0 {
                        let event = OrdersClaimed(Change::after(json!({
                            "tenant_id": tenant.id(), "buyer_user_id": buyer, "order_ids": ids,
                        })));
                        outbox
                            .record(&transaction, tenant, Actor::user(buyer), None, &event)
                            .await?;
                    }
                    transaction.commit().await?;
                    Ok(ids)
                }
            })
            .await
    }
}

fn subject(order_id: Uuid) -> Option<String> {
    Some(format!("orders.order:{order_id}"))
}

// `auth` (bước 34) hỏi số điện thoại đã qua OTP của một token xem đơn để tạo tài khoản (D-202):
// qua trait của kernel, vì `auth` không phụ thuộc `orders` (D-20261122-late-apis).
#[async_trait::async_trait]
impl autoxide_kernel::sms::VerifiedPhoneTokens for OrdersService {
    async fn verified_phone(
        &self,
        tenant: &TenantContext,
        token: &str,
    ) -> Result<Option<PhoneNumber>, Error> {
        let access = match self.guest_access(tenant, Some(token)).await {
            Ok(access) => access,
            Err(error) if error.code() == "orders.lookup_token_invalid" => return Ok(None),
            Err(error) => return Err(error),
        };
        let row = self.order(tenant, access.order_id).await?;
        let contact: Contact = serde_json::from_str(&row.contact).map_err(Error::internal)?;
        let Some(phone) = PhoneNumber::parse(&contact.phone, DEFAULT_COUNTRY) else {
            return Ok(None);
        };
        // Số trong đơn phải đúng số đã qua OTP lúc cấp token.
        let key = self.secret.phone_key(tenant.id(), &phone).await?;
        Ok((key == access.phone_hash).then_some(phone))
    }
}

#[async_trait::async_trait]
impl crate::api::OrderLookups for OrdersService {
    async fn verified_phone(
        &self,
        tenant: &TenantContext,
        token: &str,
    ) -> Result<Option<String>, Error> {
        let phone =
            autoxide_kernel::sms::VerifiedPhoneTokens::verified_phone(self, tenant, token).await?;
        Ok(phone.map(|phone| phone.as_str().to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn che_so_dien_thoai_ten_va_dia_chi() {
        assert_eq!(mask_phone("0901234567"), "090•••567");
        assert_eq!(mask_phone("+84901234567"), "090•••567");
        assert_eq!(mask_phone("12"), "•••");
        assert_eq!(mask_name("Nguyễn Văn An"), "N. V. An");
        assert_eq!(mask_name("An"), "An");
        let raw = json!({"address_id": null, "recipient_name": "Trần Thị Bình", "phone": "0912345678",
            "address": {"country": "VN", "province_code": "01", "province_name": "Hà Nội",
                "commune_code": "00004", "commune_name": "Phường Ba Đình", "line": "12 Tràng Tiền"}});
        let masked = mask_ship_to(&raw.to_string()).unwrap();
        assert_eq!(masked["recipient_name"], "T. T. Bình");
        assert_eq!(masked["phone"], "091•••678");
        assert_eq!(masked["address"]["commune_name"], "Phường Ba Đình");
        assert!(masked["address"].get("line").is_none());
    }
}
