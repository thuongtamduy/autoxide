//! Quản trị chương trình điểm (bước 47 mục 3, 6): quy tắc và hạng là dữ liệu có ngày hiệu lực, chỉ
//! thêm (không sửa dòng đã hiệu lực, không thêm dòng hiệu lực trong quá khứ); chương trình voucher
//! tặng; điều chỉnh điểm tay có `reference` + lý do (D-184); xem điểm và sổ điểm.

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use chrono::{DateTime, Duration, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use uuid::Uuid;

use super::current_rule;
use super::ledger::{EXPENSE, LIABILITY, Line};
use super::points::NewLot;
use crate::entity::{loyalty_rule, points_entry, points_lot, tier, voucher_program};
use crate::permissions::{LOYALTY_ADJUST, LOYALTY_MANAGE};
use crate::service::{CustomersService, now};

/// Quy tắc mới.
#[derive(Debug, Clone)]
pub(crate) struct NewRule {
    pub earn_per_minor: i64,
    pub points_per_unit: i64,
    pub point_value_minor: i64,
    pub currency: String,
    pub points_valid_months: i32,
    pub min_redeem_points: i64,
    pub max_redeem_ratio_bp: i32,
    pub effective_from: Option<DateTime<Utc>>,
}

/// Hạng mới (hay bản mới của một hạng).
#[derive(Debug, Clone)]
pub(crate) struct NewTier {
    pub code: String,
    pub name: String,
    pub threshold_minor: i64,
    pub currency: String,
    pub earn_multiplier_bp: i32,
    pub rank: i32,
    pub effective_from: Option<DateTime<Utc>>,
}

/// Chương trình voucher tặng.
#[derive(Debug, Clone)]
pub(crate) struct NewProgram {
    pub kind: String,
    pub promotion_id: Uuid,
    pub points_cost: Option<i64>,
    pub min_order_minor: Option<i64>,
}

/// Điểm của một hồ sơ.
#[derive(Debug, Clone)]
pub(crate) struct PointsView {
    pub balance: i64,
    pub held: i64,
    /// Lô còn điểm, hết hạn sớm nhất trước.
    pub lots: Vec<points_lot::Model>,
    pub rule: Option<loyalty_rule::Model>,
}

fn past_effective() -> Error {
    Error::new(
        ErrorKind::Validation,
        "customers.rule_effective_in_past",
        "A rule cannot take effect in the past; give `effective_from` now or later.",
    )
}

fn positive(field: &str, value: i64, problems: &mut Vec<FieldError>) {
    if value <= 0 {
        problems.push(FieldError::new(field, "range", "must be greater than 0"));
    }
}

fn currency_ok(field: &str, code: &str, problems: &mut Vec<FieldError>) {
    if autoxide_kernel::money::Currency::parse(code).is_err() {
        problems.push(FieldError::new(
            field,
            "invalid",
            "must be an ISO 4217 currency code like `VND`",
        ));
    }
}

/// Mốc hiệu lực: bỏ trống = ngay; trước lúc này quá 1 phút = lỗi (đồng hồ hai máy lệch nhẹ).
fn effective(from: Option<DateTime<Utc>>) -> Result<DateTime<Utc>, Error> {
    let at = now();
    match from {
        None => Ok(at),
        Some(from) if from < at - Duration::minutes(1) => Err(past_effective()),
        Some(from) => Ok(from),
    }
}

impl CustomersService {
    /// Mọi quyền quản trị điểm là quyền ở cả khách (chương trình chung của khách).
    async fn require_tenant_wide(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        permission: &autoxide_kernel::access::Permission,
    ) -> Result<(), Error> {
        principal
            .require_at(
                tenant.database().read(),
                permission,
                OrgTarget::new(autoxide_kernel::tenancy::root_of(tenant.id()), None),
            )
            .await
    }

    pub(crate) async fn add_rule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewRule,
    ) -> Result<loyalty_rule::Model, Error> {
        self.require_tenant_wide(tenant, principal, &LOYALTY_MANAGE).await?;
        let mut problems = Vec::new();
        positive("earn_per_minor", input.earn_per_minor, &mut problems);
        positive("points_per_unit", input.points_per_unit, &mut problems);
        positive("point_value_minor", input.point_value_minor, &mut problems);
        currency_ok("currency", &input.currency, &mut problems);
        if !(1..=120).contains(&input.points_valid_months) {
            problems.push(FieldError::new("points_valid_months", "range", "must be from 1 to 120"));
        }
        if input.min_redeem_points < 0 {
            problems.push(FieldError::new("min_redeem_points", "range", "must not be negative"));
        }
        if !(1..=10_000).contains(&input.max_redeem_ratio_bp) {
            problems.push(FieldError::new(
                "max_redeem_ratio_bp",
                "range",
                "must be from 1 to 10000",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let from = effective(input.effective_from)?;
        let row = loyalty_rule::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            earn_per_minor: Set(input.earn_per_minor),
            points_per_unit: Set(input.points_per_unit),
            point_value_minor: Set(input.point_value_minor),
            currency: Set(input.currency),
            points_valid_months: Set(input.points_valid_months),
            min_redeem_points: Set(input.min_redeem_points),
            max_redeem_ratio_bp: Set(input.max_redeem_ratio_bp),
            effective_from: Set(from),
            created_at: Set(now()),
        };
        Ok(row.insert(tenant.database().write()).await?)
    }

    pub(crate) async fn rules(
        &self,
        tenant: &TenantContext,
        page: &PageRequest,
    ) -> Result<Page<loyalty_rule::Model>, Error> {
        let select = loyalty_rule::Entity::find().scoped(tenant);
        fetch_page(select, loyalty_rule::Column::Id, |row| row.id, page, tenant.database().read())
            .await
    }

    pub(crate) async fn add_tier(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewTier,
    ) -> Result<tier::Model, Error> {
        self.require_tenant_wide(tenant, principal, &LOYALTY_MANAGE).await?;
        let mut problems = Vec::new();
        let code_ok = !input.code.is_empty()
            && input.code.len() <= 64
            && input
                .code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !code_ok {
            problems.push(FieldError::new(
                "code",
                "invalid",
                "must be up to 64 characters of `a-z`, `0-9` and `_`",
            ));
        }
        if !(1..=100).contains(&input.name.trim().chars().count()) {
            problems.push(FieldError::new("name", "length", "must be 1 to 100 characters"));
        }
        if input.threshold_minor < 0 {
            problems.push(FieldError::new("threshold_minor", "range", "must not be negative"));
        }
        currency_ok("currency", &input.currency, &mut problems);
        if !(0..=100_000).contains(&input.earn_multiplier_bp) {
            problems.push(FieldError::new(
                "earn_multiplier_bp",
                "range",
                "must be from 0 to 100000",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let from = effective(input.effective_from)?;
        let row = tier::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            code: Set(input.code),
            name: Set(input.name.trim().to_owned()),
            threshold_minor: Set(input.threshold_minor),
            currency: Set(input.currency),
            earn_multiplier_bp: Set(input.earn_multiplier_bp),
            rank: Set(input.rank),
            effective_from: Set(from),
            created_at: Set(now()),
        };
        Ok(row.insert(tenant.database().write()).await?)
    }

    pub(crate) async fn tiers(
        &self,
        tenant: &TenantContext,
        page: &PageRequest,
    ) -> Result<Page<tier::Model>, Error> {
        let select = tier::Entity::find().scoped(tenant);
        fetch_page(select, tier::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    pub(crate) async fn add_program(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewProgram,
    ) -> Result<voucher_program::Model, Error> {
        self.require_tenant_wide(tenant, principal, &LOYALTY_MANAGE).await?;
        let mut problems = Vec::new();
        if ![super::vouchers::BIRTHDAY, super::vouchers::PURCHASE, super::vouchers::POINTS]
            .contains(&input.kind.as_str())
        {
            problems.push(FieldError::new(
                "kind",
                "invalid",
                "must be `birthday`, `purchase` or `points`",
            ));
        }
        match (input.kind.as_str(), input.points_cost) {
            (super::vouchers::POINTS, None) => {
                problems.push(FieldError::new(
                    "points_cost",
                    "required",
                    "is required for `points`",
                ));
            }
            (_, Some(cost)) if cost <= 0 => {
                problems.push(FieldError::new("points_cost", "range", "must be greater than 0"));
            }
            _ => {}
        }
        if input.min_order_minor.is_some_and(|min| min < 0) {
            problems.push(FieldError::new("min_order_minor", "range", "must not be negative"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let at = now();
        let row = voucher_program::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            kind: Set(input.kind),
            promotion_id: Set(input.promotion_id),
            points_cost: Set(input.points_cost),
            min_order_minor: Set(input.min_order_minor),
            active: Set(true),
            created_at: Set(at),
            updated_at: Set(at),
        };
        Ok(row.insert(tenant.database().write()).await?)
    }

    /// Bật / tắt một chương trình voucher.
    pub(crate) async fn set_program_active(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        active: bool,
    ) -> Result<voucher_program::Model, Error> {
        self.require_tenant_wide(tenant, principal, &LOYALTY_MANAGE).await?;
        let row = voucher_program::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| {
                Error::not_found(
                    "customers.voucher_program_not_found",
                    "No active voucher reward with this id.",
                )
            })?;
        let mut row: voucher_program::ActiveModel = row.into();
        row.active = Set(active);
        row.updated_at = Set(now());
        Ok(row.update(tenant.database().write()).await?)
    }

    pub(crate) async fn programs_page(
        &self,
        tenant: &TenantContext,
        page: &PageRequest,
        only_active: bool,
    ) -> Result<Page<voucher_program::Model>, Error> {
        let mut select = voucher_program::Entity::find().scoped(tenant);
        if only_active {
            select = select.filter(voucher_program::Column::Active.eq(true));
        }
        fetch_page(
            select,
            voucher_program::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await
    }

    /// Điểm của hồ sơ `profile_id` (không kiểm quyền).
    pub(crate) async fn points_view(
        &self,
        tenant: &TenantContext,
        profile_id: Uuid,
    ) -> Result<PointsView, Error> {
        let read = tenant.database().read();
        let (balance, held) = self.balance_of(read, tenant, profile_id).await?;
        let lots = points_lot::Entity::find()
            .filter(points_lot::Column::ProfileId.eq(profile_id))
            .filter(points_lot::Column::Remaining.gt(0))
            .scoped(tenant)
            .order_by_asc(points_lot::Column::ExpiresAt)
            .all(read)
            .await?;
        let rule = current_rule(read, tenant, now()).await?;
        Ok(PointsView { balance, held, lots, rule })
    }

    /// Sổ điểm của hồ sơ.
    pub(crate) async fn points_entries(
        &self,
        tenant: &TenantContext,
        profile_id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<points_entry::Model>, Error> {
        let select = points_entry::Entity::find()
            .filter(points_entry::Column::ProfileId.eq(profile_id))
            .scoped(tenant);
        fetch_page(select, points_entry::Column::Id, |row| row.id, page, tenant.database().read())
            .await
    }

    /// Điều chỉnh tay (`customers.loyalty.adjust` ở nút của hồ sơ): cộng thành lô mới theo quy tắc
    /// hiện hành, trừ thì tiêu lô cũ nhất trước. `reference` chống làm hai lần.
    pub(crate) async fn adjust_points(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        profile_id: Uuid,
        points: i64,
        reference: &str,
        reason: &str,
    ) -> Result<PointsView, Error> {
        let row = self.get(tenant, principal, profile_id).await?;
        principal
            .require_at(
                tenant.database().read(),
                &LOYALTY_ADJUST,
                OrgTarget::new(OrgId::from_uuid(row.org_id), None),
            )
            .await?;
        let mut problems = Vec::new();
        if points == 0 {
            problems.push(FieldError::new("points", "range", "must not be 0"));
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
        let reference = format!("adjust:{}", reference.trim());
        let actor = Actor::user(principal.user_id());
        let read = tenant.database().read();
        let rule = current_rule(read, tenant, now()).await?.ok_or_else(|| {
            Error::new(
                ErrorKind::Conflict,
                "customers.no_loyalty_rule",
                "Add a loyalty rule before adjusting points.",
            )
        })?;
        let done = points_entry::Entity::find()
            .filter(points_entry::Column::Reference.eq(reference.as_str()))
            .scoped(tenant)
            .one(read)
            .await?
            .is_some();
        if !done {
            let transaction = tenant.database().begin_write().await?;
            if points > 0 {
                let lot = NewLot {
                    profile_id,
                    points,
                    value: rule.point_value_minor,
                    currency: &rule.currency,
                    valid_months: rule.points_valid_months,
                    source_ref: reference.clone(),
                    kind: "adjust",
                    actor,
                    reason: Some(reason.trim().to_owned()),
                };
                self.add_lot(&transaction, tenant, lot).await?;
            } else {
                let taken = self
                    .spend_now(
                        &transaction,
                        tenant,
                        profile_id,
                        -points,
                        "adjust",
                        &reference,
                        Some(reason.trim().to_owned()),
                        actor,
                    )
                    .await?;
                let value: i64 =
                    taken.iter().map(|part| part.points.saturating_mul(part.value)).sum();
                self.queue_posting(
                    &transaction,
                    tenant,
                    format!("customers:points:{reference}"),
                    "Loyalty points adjusted down",
                    &rule.currency,
                    vec![Line::debit(LIABILITY, value), Line::credit(EXPENSE, value)],
                )
                .await?;
            }
            transaction.commit().await?;
        }
        self.points_view(tenant, profile_id).await
    }
}
