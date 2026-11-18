//! Kiểm tuân thủ trước khi bật (D-222, D-225; luật số hai): trần giảm theo luật đang hiệu lực,
//! giới hạn số ngày giảm giá, thông báo cho từng nơi nhận, mục tiêu, sàn giá của `catalog`, xung
//! đột. `activation-check` chỉ đọc; `activate` chạy cùng bộ kiểm, lỗi → `errors[]` của `Problem`,
//! cảnh báo không chặn. Hạn mức là dữ liệu (`promotions_legal_limits`); không có dữ liệu thì không
//! tự cho qua (`promotions.no_legal_data`).

use std::collections::{BTreeMap, BTreeSet};

use autoxide_catalog::api::{FloorAction, PriceQuery};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::money::Currency;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{notice, promotion};
use crate::legal::legal_kind;
use crate::model::{Exclusivity, Kind, Program, RewardSelection, Scope, Status};
use crate::service::PromotionsService;

/// How serious a failed check is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = PromotionsCheckSeverity)]
pub(crate) enum Severity {
    /// Blocks `activate`.
    Error,
    /// Only a warning.
    Warning,
}

/// One check made before a promotion is turned on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = PromotionsActivationCheck)]
pub(crate) struct Check {
    /// What was checked, or the error code when it failed: `legal_limit` /
    /// `promotions.exceeds_legal_limit`, `duration` / `promotions.exceeds_duration_limit`,
    /// `notice` / `promotions.notice_required`, `legal_data` / `promotions.no_legal_data`,
    /// `targets`, `window`, `price_floor`, `conflicts`.
    pub code: String,
    pub ok: bool,
    pub severity: Severity,
    /// Values of the check (limit, legal basis, authority…).
    pub params: BTreeMap<String, String>,
}

fn check(code: &str, ok: bool, severity: Severity) -> Check {
    Check { code: code.to_owned(), ok, severity, params: BTreeMap::new() }
}

impl Check {
    fn with(mut self, name: &str, value: impl ToString) -> Self {
        self.params.insert(name.to_owned(), value.to_string());
        self
    }
}

/// Ngày theo giờ của vùng luật (Việt Nam: UTC+7).
pub(crate) fn legal_day(at: DateTime<Utc>) -> NaiveDate {
    at.with_timezone(&chrono_tz::Asia::Ho_Chi_Minh).date_naive()
}

/// Số ngày (theo lịch) chương trình chạy trong năm `year`.
fn days_in_year(starts_at: DateTime<Utc>, ends_at: DateTime<Utc>, year: i32) -> i64 {
    let (Some(first), Some(last)) =
        (NaiveDate::from_ymd_opt(year, 1, 1), NaiveDate::from_ymd_opt(year, 12, 31))
    else {
        return 0;
    };
    let from = legal_day(starts_at).max(first);
    let to = legal_day(ends_at - chrono::Duration::microseconds(1)).min(last);
    if to < from { 0 } else { (to - from).num_days() + 1 }
}

/// Mức giảm phần trăm lớn nhất chương trình có thể cho (biết trước, không cần giá).
fn static_percent(program: &Program) -> Option<i32> {
    let mut best: Option<i32> = program.value_bp.filter(|_| program.kind != Kind::BuyXGetY);
    let mut take = |bp: Option<i32>| {
        if let Some(bp) = bp {
            best = Some(best.map_or(bp, |current| current.max(bp)));
        }
    };
    for tier in &program.tiers {
        take(tier.value_bp);
    }
    for item in &program.flash_items {
        take(item.value_bp);
    }
    if program.kind == Kind::BuyXGetY {
        // Mua X tặng Y: mức giảm bình quân trên một đơn vị của nhóm.
        let buy = i64::from(program.buy_quantity.unwrap_or(1).max(1));
        let get = i64::from(program.get_quantity.unwrap_or(1).max(1));
        let reward = i64::from(program.reward_value_bp.unwrap_or(10_000));
        let average = if program.reward_selection == Some(RewardSelection::Specified) {
            reward
        } else {
            reward * get / (buy + get)
        };
        take(i32::try_from(average).ok());
    }
    best
}

impl PromotionsService {
    /// Mọi mục kiểm trước khi bật `program` (không ghi gì).
    pub(crate) async fn activation_checks(
        &self,
        tenant: &TenantContext,
        program: &Program,
    ) -> Result<Vec<Check>, Error> {
        let mut checks = Vec::new();
        let now = crate::service::now();
        checks.push(
            check(
                "window",
                program.ends_at > now && program.ends_at > program.starts_at,
                Severity::Error,
            )
            .with("starts_at", program.starts_at.to_rfc3339())
            .with("ends_at", program.ends_at.to_rfc3339()),
        );
        checks.push(self.targets_check(program));
        let legal = self.legal().await?;
        let jurisdiction = self.config.jurisdiction.clone();
        let kind = legal_kind(program);
        let start_day = legal_day(program.starts_at.max(now));
        let Some(limit) = legal.limit(&jurisdiction, kind, program.program_kind, start_day) else {
            checks.push(
                check("promotions.no_legal_data", false, Severity::Error)
                    .with("jurisdiction", &jurisdiction)
                    .with("promotion_kind", kind),
            );
            return Ok(checks);
        };
        checks.push(
            check("legal_data", true, Severity::Error).with("pack_basis", &limit.legal_basis),
        );
        // Trần giảm: phần trăm biết trước, và giảm theo số tiền / giá sốc của từng biến thể so
        // với giá tham chiếu (giá ngay trước khuyến mại) của `catalog`.
        let mut worst = static_percent(program).unwrap_or(0);
        let variants: Vec<Uuid> = program
            .flash_items
            .iter()
            .map(|item| item.variant_id)
            .chain(program.conditions_targets.variants.iter().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .take(autoxide_catalog::api::MAX_IDS)
            .collect();
        let currency = Currency::parse(&program.currency)?;
        let amount_based = matches!(program.kind, Kind::AmountOff | Kind::FixedPrice)
            && program.scope == Scope::Items
            || program.flash_items.iter().any(|item| item.sale_price_minor.is_some());
        if amount_based && !variants.is_empty() {
            let query = PriceQuery::new(
                OrgId::from_uuid(program.org_id),
                currency,
                program.starts_at.max(now),
                variants.clone(),
            );
            let references = self.catalog.reference_prices(tenant, &query).await?;
            for reference in &references {
                let price = reference.amount.minor();
                if price <= 0 {
                    continue;
                }
                let discount = per_unit_discount(program, reference.variant_id, price);
                let bp = i32::try_from(discount.saturating_mul(10_000) / price).unwrap_or(i32::MAX);
                worst = worst.max(bp);
            }
        }
        let over = worst > limit.max_bp;
        checks.push(
            check(
                if over { "promotions.exceeds_legal_limit" } else { "legal_limit" },
                !over,
                Severity::Error,
            )
            .with("value", format!("{}%", crate::engine::show_percent(worst)))
            .with("max", format!("{}%", crate::engine::show_percent(limit.max_bp)))
            .with("legal_basis", &limit.legal_basis),
        );
        checks.push(self.duration_check(tenant, program, limit).await?);
        checks.extend(self.notice_checks(tenant, program, &legal, start_day).await?);
        if !variants.is_empty() {
            checks.extend(self.floor_checks(tenant, program, currency, &variants).await?);
        }
        checks.push(self.conflict_check(tenant, program).await?);
        Ok(checks)
    }

    fn targets_check(&self, program: &Program) -> Check {
        let ok = match program.kind {
            Kind::FlashSale => !program.flash_items.is_empty(),
            Kind::AddOn => {
                !program.flash_items.is_empty() && !program.conditions_targets.is_empty()
            }
            Kind::BuyXGetY if program.reward_selection == Some(RewardSelection::Specified) => {
                !program.reward_targets.is_empty()
            }
            Kind::GiftChoice => !program.gift_options.is_empty(),
            _ => true,
        };
        check("targets", ok, Severity::Error).with("kind", program.kind.as_str())
    }

    /// Số ngày giảm giá: của chương trình và cộng các chương trình giảm giá khác trên cùng mục
    /// tiêu trong năm (D-225).
    async fn duration_check(
        &self,
        tenant: &TenantContext,
        program: &Program,
        limit: &crate::legal::LegalLimit,
    ) -> Result<Check, Error> {
        let own = ((program.ends_at - program.starts_at).num_seconds() + 86_399) / 86_400;
        if let Some(max) = limit.max_days_per_program
            && own > i64::from(max)
        {
            return Ok(check("promotions.exceeds_duration_limit", false, Severity::Error)
                .with("max", max)
                .with("days", own)
                .with("legal_basis", &limit.legal_basis));
        }
        let Some(max) = limit.max_days_per_year else {
            return Ok(check("duration", true, Severity::Error).with("days", own));
        };
        let year = legal_day(program.starts_at).year();
        let others = self.overlapping_programs(tenant, program, false).await?;
        let mut days = days_in_year(program.starts_at, program.ends_at, year);
        for other in others.iter().filter(|other| legal_kind(other) == "price_reduction") {
            days += days_in_year(other.starts_at, other.ends_at, year);
        }
        let ok = days <= i64::from(max);
        Ok(check(
            if ok { "duration" } else { "promotions.exceeds_duration_limit" },
            ok,
            Severity::Error,
        )
        .with("max", max)
        .with("days", days)
        .with("legal_basis", &limit.legal_basis))
    }

    /// Một mục mỗi nơi nhận thông báo theo quy tắc đang hiệu lực.
    async fn notice_checks(
        &self,
        tenant: &TenantContext,
        program: &Program,
        legal: &crate::legal::LegalData,
        day: NaiveDate,
    ) -> Result<Vec<Check>, Error> {
        let Some(rule) = legal.rule(&self.config.jurisdiction, legal_kind(program), day) else {
            return Ok(Vec::new());
        };
        if !rule.notice_required {
            return Ok(vec![check("notice", true, Severity::Error).with("required", false)]);
        }
        let authorities = self.authorities(tenant, program, &rule.authority).await?;
        let recorded = notice::Entity::find()
            .filter(notice::Column::PromotionId.eq(program.id))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let mut checks = Vec::new();
        for authority in authorities {
            let done = recorded.iter().any(|found| {
                found.authority_code == authority
                    && matches!(found.status.as_str(), "submitted" | "not_required")
            });
            checks.push(
                check(
                    if done { "notice" } else { "promotions.notice_required" },
                    done,
                    Severity::Error,
                )
                .with("authority", &authority)
                .with("legal_basis", &rule.legal_basis),
            );
        }
        Ok(checks)
    }

    /// Nơi nhận thông báo của chương trình: `province_of_store` → `province:<mã tỉnh>` từ cài đặt
    /// kế thừa `promotions.notice_province` của nút chạy chương trình; quy tắc khác → mã cố định.
    pub(crate) async fn authorities(
        &self,
        tenant: &TenantContext,
        program: &Program,
        authority: &str,
    ) -> Result<Vec<String>, Error> {
        if authority != "province_of_store" {
            return Ok(vec![authority.to_owned()]);
        }
        let province = self
            .setting(
                tenant,
                OrgId::from_uuid(program.org_id),
                crate::settings::NOTICE_PROVINCE.key(),
            )
            .await?
            .and_then(|value| value.as_str().map(str::to_owned));
        Ok(vec![
            province
                .map_or_else(|| "province:unknown".to_owned(), |code| format!("province:{code}")),
        ])
    }

    async fn floor_checks(
        &self,
        tenant: &TenantContext,
        program: &Program,
        currency: Currency,
        variants: &[Uuid],
    ) -> Result<Vec<Check>, Error> {
        let org = OrgId::from_uuid(program.org_id);
        let floors = self.catalog.price_floors(tenant, org, currency, variants).await?;
        if floors.is_empty() {
            return Ok(Vec::new());
        }
        let query = PriceQuery::new(
            org,
            currency,
            program.starts_at.max(crate::service::now()),
            variants.to_vec(),
        );
        let prices = self.catalog.prices(tenant, &query).await?;
        let mut checks = Vec::new();
        for floor in floors {
            let (Some(minimum), Some(price)) =
                (floor.floor, prices.iter().find(|price| price.variant_id == floor.variant_id))
            else {
                continue;
            };
            let unit = price.amount.minor();
            let after = unit - per_unit_discount(program, floor.variant_id, unit);
            if after >= minimum.minor() {
                continue;
            }
            let severity = if floor.action == FloorAction::Block {
                Severity::Error
            } else {
                Severity::Warning
            };
            checks.push(
                check("price_floor", false, severity)
                    .with("variant_id", floor.variant_id)
                    .with("floor", minimum.minor())
                    .with("price_after", after),
            );
        }
        Ok(checks)
    }

    async fn conflict_check(
        &self,
        tenant: &TenantContext,
        program: &Program,
    ) -> Result<Check, Error> {
        let others = self.overlapping_programs(tenant, program, true).await?;
        let conflicts: Vec<String> = others
            .iter()
            .filter(|other| {
                other.class() == program.class()
                    && (other.exclusivity != Exclusivity::None
                        || program.exclusivity != Exclusivity::None)
            })
            .map(|other| other.id.to_string())
            .collect();
        Ok(check("conflicts", conflicts.is_empty(), Severity::Warning)
            .with("promotions", conflicts.join(",")))
    }

    /// Chương trình khác của cùng nút, trùng thời gian (`same_time`) hay cùng năm, chung mục tiêu.
    pub(crate) async fn overlapping_programs(
        &self,
        tenant: &TenantContext,
        program: &Program,
        same_time: bool,
    ) -> Result<Vec<Program>, Error> {
        let read = tenant.database().read();
        let mut select = promotion::Entity::find()
            .filter(promotion::Column::OrgId.eq(program.org_id))
            .filter(promotion::Column::Id.ne(program.id))
            .filter(promotion::Column::Status.is_in([
                Status::Scheduled.as_str(),
                Status::Active.as_str(),
                Status::Paused.as_str(),
                Status::Ended.as_str(),
            ]))
            .scoped(tenant);
        if same_time {
            select = select
                .filter(promotion::Column::StartsAt.lt(program.ends_at))
                .filter(promotion::Column::EndsAt.gt(program.starts_at));
        } else {
            let year = legal_day(program.starts_at).year();
            let first = NaiveDate::from_ymd_opt(year, 1, 1)
                .and_then(|day| day.and_hms_opt(0, 0, 0))
                .map(|day| day.and_utc() - chrono::Duration::hours(7));
            if let Some(first) = first {
                select = select
                    .filter(promotion::Column::EndsAt.gt(first))
                    .filter(promotion::Column::StartsAt.lt(first + chrono::Duration::days(366)));
            }
        }
        let rows = select.all(read).await?;
        let views = crate::programs::load_children(read, tenant, rows).await?;
        Ok(views
            .iter()
            .map(crate::compiled::program_of)
            .filter(|other| shares_targets(program, other))
            .collect())
    }

    /// Ghi nhận thông báo đã nộp (hay không cần) cho một nơi nhận.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn record_notice<C: ConnectionTrait>(
        &self,
        transaction: &C,
        tenant: &TenantContext,
        promotion_id: Uuid,
        authority: &str,
        status: &str,
        reference: Option<String>,
        submitted_at: Option<DateTime<Utc>>,
        by: Option<Uuid>,
    ) -> Result<notice::Model, Error> {
        use sea_orm::{ActiveModelTrait, Set};
        let existing = notice::Entity::find()
            .filter(notice::Column::PromotionId.eq(promotion_id))
            .filter(notice::Column::AuthorityCode.eq(authority))
            .scoped(tenant)
            .one(transaction)
            .await?;
        let now = crate::service::now();
        let row = match existing {
            Some(found) => {
                let mut row: notice::ActiveModel = found.into();
                row.status = Set(status.to_owned());
                row.reference = Set(reference);
                row.submitted_at = Set(submitted_at);
                row.submitted_by = Set(by);
                row.updated_at = Set(now);
                row.update(transaction).await?
            }
            None => {
                notice::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    promotion_id: Set(promotion_id),
                    authority_code: Set(authority.to_owned()),
                    status: Set(status.to_owned()),
                    reference: Set(reference),
                    submitted_at: Set(submitted_at),
                    submitted_by: Set(by),
                    updated_at: Set(now),
                }
                .insert(transaction)
                .await?
            }
        };
        Ok(row)
    }
}

impl PromotionsService {
    /// Ghi nhận thông báo cho một nơi nhận (người nộp là con người; ở đây chỉ ghi nhận).
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn submit_notice(
        &self,
        tenant: &TenantContext,
        principal: &autoxide_kernel::access::Principal,
        id: Uuid,
        authority: &str,
        status: &str,
        reference: Option<String>,
        submitted_at: Option<DateTime<Utc>>,
    ) -> Result<notice::Model, Error> {
        let mut problems = Vec::new();
        if !matches!(status, "submitted" | "not_required") {
            problems.push(FieldError::new(
                "status",
                "invalid",
                "must be `submitted` or `not_required`",
            ));
        }
        if authority.is_empty()
            || authority.len() > 32
            || !authority
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'))
        {
            problems.push(FieldError::new(
                "authority_code",
                "invalid",
                "1 to 32 letters, digits, `:`, `_` or `-`",
            ));
        }
        if reference.as_ref().is_some_and(|text| text.chars().count() > 100) {
            problems.push(FieldError::new(
                "reference",
                "length",
                "must be at most 100 characters long",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let actor = autoxide_kernel::event::Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let program = promotion::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| crate::programs::not_found(id))?;
        crate::service::writable(
            principal,
            &transaction,
            &crate::permissions::COMPLIANCE,
            program.org_id,
            || crate::programs::not_found(id),
        )
        .await?;
        let when = (status == "submitted")
            .then(|| submitted_at.map_or_else(crate::service::now, crate::service::micros));
        let row = self
            .record_notice(
                &transaction,
                tenant,
                id,
                authority,
                status,
                reference,
                when,
                Some(principal.user_id()),
            )
            .await?;
        self.outbox
            .record(
                &transaction,
                tenant,
                actor,
                crate::events::promotion_subject(id),
                &crate::events::NoticeSubmitted(crate::events::Change::after(serde_json::json!({
                    "id": id, "authority_code": authority, "status": status, "reference": row.reference,
                }))),
            )
            .await?;
        transaction.commit().await?;
        Ok(row)
    }
}

/// Tiền giảm trên một đơn vị giá `price` của biến thể (giảm theo số tiền, giá cố định, giá sốc).
fn per_unit_discount(program: &Program, variant: Uuid, price: i64) -> i64 {
    if let Some(item) = program.flash_item(variant) {
        if let Some(sale) = item.sale_price_minor {
            return (price - sale).max(0);
        }
        if let Some(bp) = item.value_bp {
            return price * i64::from(bp) / 10_000;
        }
    }
    match program.kind {
        Kind::AmountOff => program.value_minor.unwrap_or(0).min(price),
        Kind::FixedPrice => (price - program.value_minor.unwrap_or(price)).max(0),
        Kind::PercentOff => price * i64::from(program.value_bp.unwrap_or(0)) / 10_000,
        _ => 0,
    }
}

/// Hai chương trình chung mục tiêu (không mục tiêu = mọi hàng).
fn shares_targets(a: &Program, b: &Program) -> bool {
    let all_a = a.conditions_targets.is_empty() && a.flash_items.is_empty();
    let all_b = b.conditions_targets.is_empty() && b.flash_items.is_empty();
    if all_a || all_b {
        return true;
    }
    let variants = |program: &Program| -> BTreeSet<Uuid> {
        program
            .conditions_targets
            .variants
            .iter()
            .copied()
            .chain(program.flash_items.iter().map(|item| item.variant_id))
            .collect()
    };
    !variants(a).is_disjoint(&variants(b))
        || !a.conditions_targets.products.is_disjoint(&b.conditions_targets.products)
        || !a.conditions_targets.categories.is_disjoint(&b.conditions_targets.categories)
}

/// Lỗi của `activate` từ các mục không đạt: mã của mục đầu, mọi mục trong `errors[]`.
pub(crate) fn activation_error(failed: &[&Check]) -> Error {
    let first: &str =
        failed.first().map(|found| found.code.as_str()).unwrap_or("promotions.invalid_transition");
    let mut error = match first {
        "promotions.no_legal_data" => Error::conflict(
            "promotions.no_legal_data",
            "No promotion law limit is in force; run `autoxide task promotions.load_legal_data`.",
        ),
        "promotions.notice_required" => Error::conflict(
            "promotions.notice_required",
            "The promotion must be notified to {authority} before it starts; record the notice as \
             submitted.",
        ),
        "promotions.exceeds_duration_limit" => Error::new(
            ErrorKind::Validation,
            "promotions.exceeds_duration_limit",
            "The discount days exceed the limit of {max} days under {legal_basis} in force.",
        ),
        "promotions.exceeds_legal_limit" => Error::new(
            ErrorKind::Validation,
            "promotions.exceeds_legal_limit",
            "The discount {value} exceeds the limit {max} under {legal_basis} in force.",
        ),
        _ => Error::new(
            ErrorKind::Validation,
            "promotions.activation_failed",
            "The promotion cannot be turned on: see `errors`.",
        ),
    };
    if let Some(found) = failed.first() {
        for (name, value) in &found.params {
            error = error.with_param(name.clone(), value);
        }
    }
    for found in failed {
        let mut field = FieldError::new(&found.code, &found.code, "check `{code}` failed")
            .with_param("code", &found.code);
        for (name, value) in &found.params {
            field = field.with_param(name.clone(), value);
        }
        error = error.with_field(field);
    }
    error
}

#[cfg(test)]
mod tests {
    use super::days_in_year;
    use chrono::{TimeZone, Utc};

    #[test]
    fn dem_ngay_theo_gio_viet_nam() {
        // 17:00 UTC 31/12/2025 = 00:00 1/1/2026 giờ Việt Nam.
        let start = Utc.with_ymd_and_hms(2025, 12, 31, 17, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 1, 10, 17, 0, 0).unwrap();
        assert_eq!(days_in_year(start, end, 2026), 10);
        assert_eq!(days_in_year(start, end, 2025), 0);
    }
}
