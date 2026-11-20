//! Giới thiệu bạn (bước 47 mục 5, 6): mỗi hồ sơ một mã; người được giới thiệu nhập mã trong
//! `REFERRAL_APPLY_DAYS` ngày sau khi tạo hồ sơ và trước đơn đầu tiên; đơn đầu hoàn tất với tiền hàng
//! tối thiểu thì thưởng cả hai bên theo quy tắc có ngày hiệu lực — điểm (mặc định) và / hoặc voucher qua
//! `Promotions::grant_voucher` (`source_ref = referral:<id>`). Chống gian lận: cùng số điện thoại hay
//! email với người giới thiệu → `rejected`.

use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_promotions::api::{GrantVoucher, VoucherSource};
use chrono::Duration;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{order_fact, profile, referral, referral_code, referral_rule};
use crate::events::{Change, ReferralRewarded};
use crate::loyalty::points::NewLot;
use crate::profiles::subject;
use crate::service::{CustomersService, now};

const PENDING: &str = "pending";
const REWARDED: &str = "rewarded";
const REJECTED: &str = "rejected";
/// Chữ dễ đọc, không nhập nhằng (bỏ 0/O, 1/I/L).
const ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";

fn code_invalid() -> Error {
    Error::new(
        ErrorKind::Validation,
        "customers.referral_code_invalid",
        "This referral code does not exist.",
    )
}

fn not_eligible() -> Error {
    Error::new(
        ErrorKind::Conflict,
        "customers.referral_not_eligible",
        "This account can no longer enter a referral code (already referred, already ordered, or too long after signing up).",
    )
}

fn self_referral() -> Error {
    Error::new(ErrorKind::Validation, "customers.self_referral", "You cannot refer yourself.")
}

fn new_code() -> Result<String, Error> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|error| Error::internal(format!("random: {error}")))?;
    Ok(bytes.iter().map(|byte| char::from(ALPHABET[usize::from(*byte) % ALPHABET.len()])).collect())
}

impl CustomersService {
    /// Mã giới thiệu của chính người mua (tạo khi chưa có) và số người đã được thưởng nhờ mã.
    pub(crate) async fn my_referral(
        &self,
        tenant: &TenantContext,
        user: Uuid,
    ) -> Result<(String, u64), Error> {
        let profile = self.me(tenant, user).await?;
        let write = tenant.database().write();
        let existing = referral_code::Entity::find_by_id((tenant.id().as_uuid(), profile.id))
            .one(write)
            .await?;
        let code = match existing {
            Some(row) => row.code,
            None => {
                let mut code = String::new();
                for _ in 0..5 {
                    let candidate = new_code()?;
                    let inserted = referral_code::Entity::insert(referral_code::ActiveModel {
                        tenant_id: Set(tenant.id().as_uuid()),
                        profile_id: Set(profile.id),
                        code: Set(candidate.clone()),
                        created_at: Set(now()),
                    })
                    .on_conflict(OnConflict::new().do_nothing().to_owned())
                    .exec_without_returning(write)
                    .await?;
                    if inserted == 1 {
                        code = candidate;
                        break;
                    }
                    // Trùng hồ sơ (request song song) hay trùng mã: đọc lại hay thử mã khác.
                    if let Some(row) =
                        referral_code::Entity::find_by_id((tenant.id().as_uuid(), profile.id))
                            .one(write)
                            .await?
                    {
                        code = row.code;
                        break;
                    }
                }
                if code.is_empty() {
                    return Err(Error::internal("could not create a unique referral code"));
                }
                code
            }
        };
        use sea_orm::PaginatorTrait;
        let rewarded = referral::Entity::find()
            .filter(referral::Column::ReferrerProfileId.eq(profile.id))
            .filter(referral::Column::Status.eq(REWARDED))
            .scoped(tenant)
            .count(tenant.database().read())
            .await?;
        Ok((code, rewarded))
    }

    /// Người mua nhập mã của người giới thiệu.
    pub(crate) async fn apply_referral(
        &self,
        tenant: &TenantContext,
        user: Uuid,
        code: &str,
    ) -> Result<referral::Model, Error> {
        let profile = self.me(tenant, user).await?;
        if profile.orders_count > 0
            || profile.created_at < now() - Duration::days(self.config.referral_apply_days)
        {
            return Err(not_eligible());
        }
        let code = code.trim().to_ascii_uppercase();
        let read = tenant.database().read();
        let owner = referral_code::Entity::find()
            .filter(referral_code::Column::Code.eq(code.as_str()))
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(code_invalid)?;
        if owner.profile_id == profile.id {
            return Err(self_referral());
        }
        let referrer =
            self.find_profile(read, tenant, owner.profile_id).await?.ok_or_else(code_invalid)?;
        let same_phone = referrer.phone_key.is_some() && referrer.phone_key == profile.phone_key;
        let same_email =
            referrer.email_lower.is_some() && referrer.email_lower == profile.email_lower;
        let at = now();
        let row = referral::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            referrer_profile_id: Set(referrer.id),
            referee_profile_id: Set(profile.id),
            status: Set(if same_phone || same_email { REJECTED } else { PENDING }.to_owned()),
            reason: Set((same_phone || same_email).then(|| "same_contact".to_owned())),
            qualifying_order_id: Set(None),
            created_at: Set(at),
            updated_at: Set(at),
        };
        // Unique theo người được giới thiệu: nhập lần hai (hay hai request cùng lúc) → không đủ điều kiện.
        let inserted = referral::Entity::insert(row.clone())
            .on_conflict(
                OnConflict::columns([
                    referral::Column::TenantId,
                    referral::Column::RefereeProfileId,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(tenant.database().write())
            .await?;
        if inserted == 0 {
            return Err(not_eligible());
        }
        if same_phone || same_email {
            return Err(self_referral());
        }
        referral::Entity::find()
            .filter(referral::Column::RefereeProfileId.eq(profile.id))
            .scoped(tenant)
            .one(tenant.database().write())
            .await?
            .ok_or_else(|| Error::internal("referral vanished"))
    }

    pub(crate) async fn add_referral_rule(
        &self,
        tenant: &TenantContext,
        rule: referral_rule::ActiveModel,
    ) -> Result<referral_rule::Model, Error> {
        let mut rule = rule;
        let mut problems = Vec::new();
        for (field, value) in
            [("referrer_points", &rule.referrer_points), ("referee_points", &rule.referee_points)]
        {
            if let sea_orm::ActiveValue::Set(points) = value
                && !(0..=1_000_000).contains(points)
            {
                problems.push(autoxide_kernel::error::FieldError::new(
                    field,
                    "range",
                    "must be from 0 to 1000000",
                ));
            }
        }
        if let sea_orm::ActiveValue::Set(min) = &rule.min_first_order_minor
            && *min < 0
        {
            problems.push(autoxide_kernel::error::FieldError::new(
                "min_first_order_minor",
                "range",
                "must not be negative",
            ));
        }
        if let sea_orm::ActiveValue::Set(code) = &rule.currency
            && autoxide_kernel::money::Currency::parse(code).is_err()
        {
            problems.push(autoxide_kernel::error::FieldError::new(
                "currency",
                "invalid",
                "must be an ISO 4217 currency code like `VND`",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        rule.id = Set(Uuid::now_v7());
        rule.tenant_id = Set(tenant.id().as_uuid());
        rule.effective_from = Set(now());
        rule.created_at = Set(now());
        Ok(rule.insert(tenant.database().write()).await?)
    }

    pub(crate) async fn referral_rules(
        &self,
        tenant: &TenantContext,
    ) -> Result<Vec<referral_rule::Model>, Error> {
        Ok(referral_rule::Entity::find()
            .scoped(tenant)
            .order_by_desc(referral_rule::Column::EffectiveFrom)
            .all(tenant.database().read())
            .await?)
    }

    /// Đơn vừa hoàn tất (trong transaction của `facts`): đơn đầu tiên của người được giới thiệu đạt tiền
    /// tối thiểu → thưởng điểm hai bên. Trả `(id giới thiệu, quy tắc)` để cấp voucher sau commit.
    pub(crate) async fn qualify_referral<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        fact: &order_fact::Model,
    ) -> Result<Option<(referral::Model, referral_rule::Model)>, Error> {
        let Some(row) = referral::Entity::find()
            .filter(referral::Column::RefereeProfileId.eq(fact.profile_id))
            .filter(referral::Column::Status.eq(PENDING))
            .scoped(tenant)
            .one(connection)
            .await?
        else {
            return Ok(None);
        };
        let Some(rule) = referral_rule::Entity::find()
            .filter(referral_rule::Column::EffectiveFrom.lte(now()))
            .scoped(tenant)
            .order_by_desc(referral_rule::Column::EffectiveFrom)
            .one(connection)
            .await?
        else {
            return Ok(None);
        };
        let net = (fact.net_minor - fact.refunded_minor).max(0);
        if rule.currency != fact.currency || net < rule.min_first_order_minor {
            return Ok(None);
        }
        let changed = referral::Entity::update_many()
            .col_expr(referral::Column::Status, Expr::value(REWARDED))
            .col_expr(referral::Column::QualifyingOrderId, Expr::value(fact.order_id))
            .col_expr(referral::Column::UpdatedAt, Expr::value(now()))
            .filter(referral::Column::Id.eq(row.id))
            .filter(referral::Column::Status.eq(PENDING))
            .scoped(tenant)
            .exec(connection)
            .await?
            .rows_affected;
        if changed == 0 {
            return Ok(None);
        }
        if let Some(points_rule) = crate::loyalty::current_rule(connection, tenant, now()).await? {
            for (profile_id, points, side) in [
                (row.referrer_profile_id, rule.referrer_points, "referrer"),
                (row.referee_profile_id, rule.referee_points, "referee"),
            ] {
                let lot = NewLot {
                    profile_id,
                    points,
                    value: points_rule.point_value_minor,
                    currency: &points_rule.currency,
                    valid_months: points_rule.points_valid_months,
                    source_ref: format!("referral:{}:{side}", row.id),
                    kind: "earn",
                    actor: Actor::system(),
                    reason: Some("referral".to_owned()),
                };
                self.add_lot(connection, tenant, lot).await?;
            }
        }
        self.outbox
            .record(
                connection,
                tenant,
                Actor::system(),
                Some(subject(row.referrer_profile_id)),
                &ReferralRewarded(Change::after(json!({
                    "referral_id": row.id, "referrer_profile_id": row.referrer_profile_id,
                    "referee_profile_id": row.referee_profile_id, "order_id": fact.order_id,
                }))),
            )
            .await?;
        Ok(Some((row, rule)))
    }

    /// Voucher thưởng giới thiệu (sau commit, ngoài transaction ghi).
    pub(crate) async fn referral_vouchers(
        &self,
        tenant: &TenantContext,
        row: &referral::Model,
        rule: &referral_rule::Model,
    ) -> Result<(), Error> {
        let Some(promotions) = &self.promotions else { return Ok(()) };
        let read = tenant.database().read();
        for (profile_id, promotion) in [
            (row.referrer_profile_id, rule.referrer_promotion_id),
            (row.referee_profile_id, rule.referee_promotion_id),
        ] {
            let (Some(promotion), Some(user)) = (
                promotion,
                profile::Entity::find_by_id(profile_id)
                    .one(read)
                    .await?
                    .and_then(|profile| profile.user_id),
            ) else {
                continue;
            };
            let request = GrantVoucher::new(
                promotion,
                user,
                VoucherSource::Referral,
                format!("referral:{}", row.id),
            );
            if let Err(error) = promotions.grant_voucher(tenant, &request).await {
                if error.kind().status() >= 500 {
                    return Err(error);
                }
                tracing::warn!(code = error.code(), referral_id = %row.id, "referral voucher not granted");
            }
        }
        Ok(())
    }
}
