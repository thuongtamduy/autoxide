//! Thẻ quà tặng và số dư cửa hàng (bước 47 mục 4, D-194, D-20261120-step47-stored-value) — TẮT mặc định
//! bằng HAI lớp: biến `AUTOXIDE_CUSTOMERS_STORED_VALUE_ENABLED` (người vận hành chỉ bật sau khi đối
//! chiếu NĐ 52/2024/NĐ-CP với người có chuyên môn) **và** tính năng theo khách
//! `customers.gift_cards` / `customers.store_credit` (bước 35).
//!
//! Luật cứng giữ tính chất "phương tiện thanh toán khép kín" (cần đối chiếu): chỉ mua hàng của chính
//! khách phát hành; KHÔNG có đường rút tiền, chuyển số dư cho người khác hay đổi ra tiền mặt — số dư
//! chỉ giảm qua giữ / dùng cho đơn, huỷ thẻ, hết hạn, điều chỉnh của nhân viên có lý do. Mã thẻ chỉ lưu
//! băm SHA-256 + 4 ký tự cuối; tra mã có giới hạn số lần (chống dò).

use autoxide_kernel::access::Principal;
use autoxide_kernel::entitlement::Feature;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::throttle::{Outcome, Throttle};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter,
    QueryOrder, Set,
};
use uuid::Uuid;

use crate::entity::{gift_card, store_credit, stored_value_entry};
use crate::loyalty::ledger::{Line, PREPAYMENTS};
use crate::service::{CustomersService, now};

mod holds;
mod tender;

pub(crate) use tender::{GiftCardTender, StoreCreditTender, StoredValueCheck};

pub(crate) const GIFT_CARDS: Feature =
    Feature::new("customers.gift_cards", "Gift cards usable only in the shop that issued them.");
pub(crate) const STORE_CREDIT: Feature = Feature::new(
    "customers.store_credit",
    "Store credit from refunds and compensation, usable only in the shop.",
);

pub(crate) const GIFT_CARD: &str = "gift_card";
pub(crate) const CREDIT: &str = "store_credit";
pub(crate) const GIFT_LIABILITY: &str = "gift_card.liability";
pub(crate) const CREDIT_LIABILITY: &str = "store_credit.liability";
pub(crate) const BREAKAGE: &str = "stored_value.breakage";
pub(crate) const CREDIT_EXPENSE: &str = "store_credit.expense";

pub(super) const HELD: &str = "held";
/// Chữ dễ đọc, không nhập nhằng (bỏ 0/O, 1/I/L).
const ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
const CODE_LENGTH: usize = 16;

pub(crate) fn disabled() -> Error {
    Error::forbidden(
        "customers.stored_value_disabled",
        "Gift cards and store credit are not turned on for this shop.",
    )
}

pub(super) fn card_not_found() -> Error {
    Error::not_found("customers.gift_card_not_found", "This gift card code is wrong.")
}

pub(super) fn card_inactive() -> Error {
    Error::new(
        ErrorKind::Conflict,
        "customers.gift_card_inactive",
        "This gift card has expired or was voided.",
    )
}

pub(super) fn insufficient(available: i64) -> Error {
    Error::new(
        ErrorKind::Conflict,
        "customers.insufficient_balance",
        "Not enough balance ({available} left).",
    )
    .with_param("available", available)
}

pub(super) fn lookup_throttled(retry_after: u64) -> Error {
    Error::too_many_requests(
        "customers.lookup_throttled",
        "Too many wrong gift card codes. Wait {seconds} seconds, then try again.",
    )
    .with_param("seconds", retry_after)
    .with_retry_after(retry_after)
}

pub(super) fn need_account() -> Error {
    Error::new(
        ErrorKind::Validation,
        "customers.stored_value_need_account",
        "Sign in to pay with a gift card or store credit.",
    )
}

/// Mã thẻ chuẩn hoá: chữ hoa, bỏ khoảng trắng và gạch.
fn normalize(code: &str) -> String {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase()
}

/// 16 ký tự ngẫu nhiên (~79 bit). Bỏ byte rơi vào phần dư của phép chia để mọi ký tự có xác suất
/// như nhau (256 không chia hết cho 31).
fn new_code() -> Result<String, Error> {
    let fair = 256 - 256 % ALPHABET.len();
    let mut code = String::with_capacity(CODE_LENGTH);
    while code.len() < CODE_LENGTH {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|error| Error::internal(format!("random: {error}")))?;
        for byte in bytes.into_iter().map(usize::from).filter(|byte| *byte < fair) {
            if code.len() < CODE_LENGTH {
                code.push(char::from(ALPHABET[byte % ALPHABET.len()]));
            }
        }
    }
    Ok(code)
}

/// Thẻ mới phát hành.
#[derive(Debug, Clone)]
pub(crate) struct NewCard {
    pub amount: Money,
    /// Khoá chống phát hành hai lần (vd số phiếu thu ở quầy).
    pub reference: String,
    pub valid_months: Option<i32>,
}

impl CustomersService {
    /// Lớp tắt thứ nhất (biến môi trường) và thứ hai (tính năng của khách).
    pub(crate) async fn stored_value_allowed(
        &self,
        tenant: &TenantContext,
        feature: &Feature,
    ) -> Result<(), Error> {
        if !self.config.stored_value_enabled {
            return Err(disabled());
        }
        if !self.entitlements.has_feature(tenant.id(), feature).await? {
            return Err(disabled());
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "một dòng sổ cần đủ các trường; gom struct không rõ hơn"
    )]
    pub(super) async fn sv_entry<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        instrument: &str,
        instrument_id: Uuid,
        kind: &str,
        amount: Money,
        reference: &str,
        actor: Actor,
    ) -> Result<(), Error> {
        stored_value_entry::Entity::insert(stored_value_entry::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            instrument: Set(instrument.to_owned()),
            instrument_id: Set(instrument_id),
            kind: Set(kind.to_owned()),
            amount_minor: Set(amount.minor()),
            currency: Set(amount.currency().code().to_owned()),
            reference: Set(reference.to_owned()),
            reason: Set(None),
            actor_id: Set(actor.user_id),
            created_at: Set(now()),
        })
        .exec_without_returning(connection)
        .await?;
        Ok(())
    }

    /// Phát hành thẻ (`customers.stored_value.manage`). Trả thẻ và mã (mã chỉ thấy lúc phát hành;
    /// gọi lại cùng `reference` trả thẻ cũ không kèm mã).
    pub(crate) async fn issue_gift_card(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        card: NewCard,
    ) -> Result<(gift_card::Model, Option<String>), Error> {
        self.stored_value_allowed(tenant, &GIFT_CARDS).await?;
        let mut problems = Vec::new();
        if !card.amount.is_positive() {
            problems.push(FieldError::new("amount", "range", "must be greater than 0"));
        }
        if card.amount.minor() > self.config.gift_card_max_minor {
            return Err(Error::new(
                ErrorKind::Validation,
                "customers.face_value_too_high",
                "A gift card is worth at most {max}.",
            )
            .with_param("max", self.config.gift_card_max_minor));
        }
        if !(1..=200).contains(&card.reference.trim().chars().count()) {
            problems.push(FieldError::new("reference", "length", "must be 1 to 200 characters"));
        }
        if card.valid_months.is_some_and(|months| !(1..=120).contains(&months)) {
            problems.push(FieldError::new("valid_months", "range", "must be from 1 to 120"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let read = tenant.database().read();
        if let Some(existing) = gift_card::Entity::find()
            .filter(gift_card::Column::IssuedReference.eq(card.reference.trim()))
            .scoped(tenant)
            .one(read)
            .await?
        {
            return Ok((existing, None));
        }
        let code = new_code()?;
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        let id = Uuid::now_v7();
        let row = gift_card::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            // Cột để dành: `Tender` chưa biết đơn thuộc cửa hàng nào nên chưa giới hạn được.
            org_scope: Set(None),
            code_hash: Set(autoxide_kernel::secret::sha256(&code)),
            code_last4: Set(code[code.len() - 4..].to_owned()),
            initial_minor: Set(card.amount.minor()),
            balance_minor: Set(card.amount.minor()),
            currency: Set(card.amount.currency().code().to_owned()),
            status: Set("active".to_owned()),
            expires_at: Set(card
                .valid_months
                .map(|months| crate::loyalty::expiry_after(at, months))),
            owner_profile_id: Set(None),
            issued_reference: Set(card.reference.trim().to_owned()),
            issued_by: Set(Some(principal.user_id())),
            created_at: Set(at),
            updated_at: Set(at),
        };
        // "Trùng thì bỏ qua" theo `reference`: hai lần phát hành cùng lúc chỉ ra một thẻ.
        let inserted = gift_card::Entity::insert(row)
            .on_conflict(OnConflict::new().do_nothing().to_owned())
            .exec_without_returning(&transaction)
            .await?;
        if inserted == 0 {
            transaction.rollback().await?;
            let existing = gift_card::Entity::find()
                .filter(gift_card::Column::IssuedReference.eq(card.reference.trim()))
                .scoped(tenant)
                .one(tenant.database().write())
                .await?
                .ok_or_else(|| Error::internal("gift card code collided"))?;
            return Ok((existing, None));
        }
        let row = gift_card::Entity::find_by_id(id)
            .one(&transaction)
            .await?
            .ok_or_else(|| Error::internal("new gift card vanished"))?;
        let actor = Actor::user(principal.user_id());
        self.sv_entry(
            &transaction,
            tenant,
            GIFT_CARD,
            row.id,
            "issue",
            card.amount,
            &row.issued_reference,
            actor,
        )
        .await?;
        // Bán thẻ là nhận trước (không xuất hoá đơn bán hàng lúc bán — cần đối chiếu).
        self.queue_posting(
            &transaction,
            tenant,
            format!("customers:gift_card:{}:issue", row.id),
            "Gift card issued",
            card.amount.currency().code(),
            vec![
                Line::debit(PREPAYMENTS, card.amount.minor()),
                Line::credit(GIFT_LIABILITY, card.amount.minor()),
            ],
        )
        .await?;
        transaction.commit().await?;
        Ok((row, Some(code)))
    }

    /// Tra thẻ theo mã. Chỉ lần tra SAI mới bị đếm (người đoán mã), theo người tra: quá
    /// `AUTOXIDE_CUSTOMERS_GIFT_CARD_LOOKUPS_PER_HOUR` lần sai trong một giờ thì bị chặn cả tra đúng.
    pub(crate) async fn lookup_gift_card(
        &self,
        tenant: &TenantContext,
        who: Uuid,
        code: &str,
    ) -> Result<gift_card::Model, Error> {
        self.stored_value_allowed(tenant, &GIFT_CARDS).await?;
        let throttle = Throttle::new(
            "customers",
            self.config.lookup_per_hour,
            std::time::Duration::from_secs(3600),
        )?;
        let key = format!("gift_card_lookup:{who}");
        if let Outcome::Limited { retry_after } = throttle.check(tenant, &key).await? {
            return Err(lookup_throttled(retry_after));
        }
        let found = gift_card::Entity::find()
            .filter(
                gift_card::Column::CodeHash.eq(autoxide_kernel::secret::sha256(&normalize(code))),
            )
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        match found {
            Some(row) => Ok(row),
            None => {
                throttle.record(tenant, &key).await?;
                Err(card_not_found())
            }
        }
    }

    pub(crate) async fn gift_card(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<gift_card::Model, Error> {
        self.stored_value_allowed(tenant, &GIFT_CARDS).await?;
        gift_card::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(card_not_found)
    }

    /// Huỷ thẻ (mất, gian lận): số dư còn lại về 0, ghi vào thu nhập khác.
    pub(crate) async fn void_gift_card(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<gift_card::Model, Error> {
        self.stored_value_allowed(tenant, &GIFT_CARDS).await?;
        let transaction = tenant.database().begin_write().await?;
        let row = rows::lock_one(&transaction, gift_card::Entity::find_by_id(id).scoped(tenant))
            .await?
            .ok_or_else(card_not_found)?;
        if row.status != "active" {
            return Err(card_inactive());
        }
        let left = row.balance_minor;
        let currency = Currency::parse(&row.currency)?;
        let mut active: gift_card::ActiveModel = row.into();
        active.status = Set("void".to_owned());
        active.balance_minor = Set(0);
        active.updated_at = Set(now());
        let row = active.update(&transaction).await?;
        self.sv_entry(
            &transaction,
            tenant,
            GIFT_CARD,
            id,
            "void",
            Money::new(-left, currency),
            "void",
            Actor::user(principal.user_id()),
        )
        .await?;
        self.queue_posting(
            &transaction,
            tenant,
            format!("customers:gift_card:{id}:void"),
            "Gift card voided",
            &row.currency,
            vec![Line::debit(GIFT_LIABILITY, left), Line::credit(BREAKAGE, left)],
        )
        .await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Người mua gắn thẻ vào tài khoản của mình (vẫn chỉ dùng trong khách phát hành).
    pub(crate) async fn attach_gift_card(
        &self,
        tenant: &TenantContext,
        user: Uuid,
        code: &str,
    ) -> Result<gift_card::Model, Error> {
        let row = self.lookup_gift_card(tenant, user, code).await?;
        let profile = self.me(tenant, user).await?;
        gift_card::Entity::update_many()
            .col_expr(gift_card::Column::OwnerProfileId, Expr::value(profile.id))
            .filter(gift_card::Column::Id.eq(row.id))
            .filter(gift_card::Column::OwnerProfileId.is_null())
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        self.gift_card(tenant, row.id).await
    }

    pub(crate) async fn my_gift_cards(
        &self,
        tenant: &TenantContext,
        user: Uuid,
    ) -> Result<Vec<gift_card::Model>, Error> {
        self.stored_value_allowed(tenant, &GIFT_CARDS).await?;
        let profile = self.me(tenant, user).await?;
        Ok(gift_card::Entity::find()
            .filter(gift_card::Column::OwnerProfileId.eq(profile.id))
            .scoped(tenant)
            .order_by_desc(gift_card::Column::CreatedAt)
            .all(tenant.database().read())
            .await?)
    }

    pub(crate) async fn store_credit_of(
        &self,
        tenant: &TenantContext,
        profile_id: Uuid,
    ) -> Result<Option<store_credit::Model>, Error> {
        self.stored_value_allowed(tenant, &STORE_CREDIT).await?;
        Ok(store_credit::Entity::find_by_id((tenant.id().as_uuid(), profile_id))
            .one(tenant.database().read())
            .await?)
    }

    /// Nhân viên cộng (bồi thường) hay trừ số dư cửa hàng của hồ sơ, có `reference` + lý do. Không
    /// nạp bằng tiền: số dư chỉ đến từ bồi thường hay hoàn tiền người mua chọn nhận dạng này.
    pub(crate) async fn adjust_store_credit(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        profile_id: Uuid,
        amount: Money,
        reference: &str,
        reason: &str,
    ) -> Result<store_credit::Model, Error> {
        self.stored_value_allowed(tenant, &STORE_CREDIT).await?;
        let profile = self.get(tenant, principal, profile_id).await?;
        let mut problems = Vec::new();
        if amount.is_zero() {
            problems.push(FieldError::new("amount", "range", "must not be 0"));
        }
        if !(1..=200).contains(&reference.trim().chars().count()) {
            problems.push(FieldError::new("reference", "length", "must be 1 to 200 characters"));
        }
        if !(1..=500).contains(&reason.trim().chars().count()) {
            problems.push(FieldError::new("reason", "length", "must be 1 to 500 characters"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let transaction = tenant.database().begin_write().await?;
        // Ghi dòng sổ trước, "trùng thì bỏ qua" theo (hồ sơ, `reference`): gọi lại hay hai lần cùng
        // lúc chỉ đổi số dư một lần, không thành lỗi 500 trên Postgres.
        let recorded = stored_value_entry::Entity::insert(stored_value_entry::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            instrument: Set(CREDIT.to_owned()),
            instrument_id: Set(profile.id),
            kind: Set("adjust".to_owned()),
            amount_minor: Set(amount.minor()),
            currency: Set(amount.currency().code().to_owned()),
            reference: Set(reference.trim().to_owned()),
            reason: Set(Some(reason.trim().to_owned())),
            actor_id: Set(Some(principal.user_id())),
            created_at: Set(now()),
        })
        .on_conflict(OnConflict::new().do_nothing().to_owned())
        .exec_without_returning(&transaction)
        .await?;
        if recorded > 0 {
            store_credit::Entity::insert(store_credit::ActiveModel {
                tenant_id: Set(tenant.id().as_uuid()),
                profile_id: Set(profile.id),
                balance_minor: Set(0),
                currency: Set(amount.currency().code().to_owned()),
                updated_at: Set(now()),
            })
            .on_conflict(
                OnConflict::columns([
                    store_credit::Column::TenantId,
                    store_credit::Column::ProfileId,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
            let filter = Condition::all()
                .add(store_credit::Column::TenantId.eq(tenant.id().as_uuid()))
                .add(store_credit::Column::ProfileId.eq(profile.id))
                .add(store_credit::Column::Currency.eq(amount.currency().code()));
            if rows::adjust::<store_credit::Entity, _>(
                &transaction,
                store_credit::Column::BalanceMinor,
                filter,
                amount.minor(),
                Bounds::NON_NEGATIVE,
            )
            .await?
                == 0
            {
                let available = self.credit_balance(&transaction, tenant, profile.id).await?;
                return Err(insufficient(available));
            }
            let value = i64::abs(amount.minor());
            let lines = if amount.is_positive() {
                vec![Line::debit(CREDIT_EXPENSE, value), Line::credit(CREDIT_LIABILITY, value)]
            } else {
                vec![Line::debit(CREDIT_LIABILITY, value), Line::credit(CREDIT_EXPENSE, value)]
            };
            self.queue_posting(
                &transaction,
                tenant,
                format!("customers:store_credit:{}:{}", profile.id, reference.trim()),
                "Store credit adjusted",
                amount.currency().code(),
                lines,
            )
            .await?;
        }
        transaction.commit().await?;
        store_credit::Entity::find_by_id((tenant.id().as_uuid(), profile.id))
            .one(tenant.database().write())
            .await?
            .ok_or_else(|| Error::internal("store credit vanished"))
    }

    pub(super) async fn credit_balance<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        profile: Uuid,
    ) -> Result<i64, Error> {
        Ok(store_credit::Entity::find_by_id((tenant.id().as_uuid(), profile))
            .one(connection)
            .await?
            .map_or(0, |row| row.balance_minor))
    }
}
