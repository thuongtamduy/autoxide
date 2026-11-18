//! Bảng giá suy ra và quy tắc làm tròn giá đẹp (D-222).
//!
//! Bảng suy ra (`derived_from`) lấy giá của bảng gốc (chỉ một tầng — bảng gốc không được là bảng
//! suy ra, `catalog.derived_chain`), điều chỉnh `adjust_bp` (phần vạn, có dấu) rồi `adjust_minor`,
//! làm tròn theo `rounding_rule_id`. Job `catalog.derive_prices` **ghi dòng giá thật** vào bảng suy
//! ra (`source = derived`, `reference = derived:<id dòng gốc>`, cùng `valid_from` / `valid_to`):
//! đọc giá không tính công thức, và chạy lại không nhân đôi. Dòng tay (`manual`) đang hiệu lực trong
//! bảng suy ra ghim giá biến thể đó: job bỏ qua tới khi dòng tay hết `valid_to`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entity::{price, price_list, rounding_rule};
use crate::permissions::PRICES_WRITE;
use crate::rounding::{RoundingMode, round};
use crate::service::{CatalogService, check_length, fail_if_any, now};

/// Loại job ghi dòng giá của bảng suy ra.
pub(crate) const DERIVE_PRICES: &str = "catalog.derive_prices";
/// Số dòng gốc mỗi transaction.
const BATCH: u64 = 500;

/// Job: ghi dòng giá của mọi bảng suy ra từ `source` (hay chỉ `target`).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct DerivePrices {
    pub source: Uuid,
    pub target: Option<Uuid>,
}

impl Job for DerivePrices {
    const KIND: &'static str = DERIVE_PRICES;
}

/// Quy tắc làm tròn mới.
#[derive(Debug, Clone)]
pub(crate) struct NewRoundingRule {
    pub name: String,
    pub step_minor: i64,
    pub mode: String,
    pub ending_minor: Option<i64>,
}

/// Giá suy ra từ giá gốc `amount`: `amount × (10000 + bp) / 10000` (làm tròn nửa lên), cộng
/// `adjust_minor`, làm tròn theo `rule`; không âm. Số nguyên, không số thực.
pub(crate) fn derive_amount(
    amount: i64,
    adjust_bp: i64,
    adjust_minor: Option<i64>,
    rule: Option<&rounding_rule::Model>,
) -> i64 {
    let scaled = i128::from(amount) * i128::from(10_000 + adjust_bp);
    let mut value = (scaled + 5_000).div_euclid(10_000) + i128::from(adjust_minor.unwrap_or(0));
    if value < 0 {
        value = 0;
    }
    let value = i64::try_from(value).unwrap_or(i64::MAX);
    match rule {
        Some(rule) => round(
            value,
            rule.step_minor,
            rule.ending_minor.unwrap_or(0),
            RoundingMode::parse(&rule.mode).unwrap_or(RoundingMode::Nearest),
        ),
        None => value,
    }
}

impl CatalogService {
    /// Tạo quy tắc làm tròn của khách (cần `catalog.prices.write`).
    pub(crate) async fn create_rounding_rule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewRoundingRule,
    ) -> Result<rounding_rule::Model, Error> {
        principal.require(&PRICES_WRITE)?;
        let mut problems = Vec::new();
        check_length(&mut problems, "name", input.name.trim(), 1, 200);
        if input.step_minor <= 0 {
            problems.push(FieldError::new("step_minor", "range", "must be greater than 0"));
        }
        if RoundingMode::parse(&input.mode).is_none() {
            problems.push(FieldError::new("mode", "one_of", "must be `up`, `down` or `nearest`"));
        }
        if input.ending_minor.is_some_and(|ending| !(0..input.step_minor.max(1)).contains(&ending))
        {
            problems.push(FieldError::new(
                "ending_minor",
                "range",
                "must be 0 or more and below `step_minor`",
            ));
        }
        fail_if_any(problems)?;
        Ok(rounding_rule::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            name: Set(input.name.trim().to_owned()),
            step_minor: Set(input.step_minor),
            mode: Set(input.mode),
            ending_minor: Set(input.ending_minor),
            created_at: Set(now()),
        }
        .insert(tenant.database().write())
        .await?)
    }

    /// Mọi quy tắc làm tròn của khách.
    pub(crate) async fn list_rounding_rules(
        &self,
        tenant: &TenantContext,
    ) -> Result<Vec<rounding_rule::Model>, Error> {
        Ok(rounding_rule::Entity::find()
            .scoped(tenant)
            .order_by_asc(rounding_rule::Column::Id)
            .all(tenant.database().read())
            .await?)
    }

    /// Xếp job ghi giá cho các bảng suy ra từ `source` (trong transaction của thay đổi). Không có
    /// bảng suy ra thì không xếp gì.
    pub(crate) async fn queue_derivation<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        source: Uuid,
        target: Option<Uuid>,
    ) -> Result<(), Error> {
        let mut targets = price_list::Entity::find()
            .filter(price_list::Column::DerivedFrom.eq(source))
            .scoped(tenant);
        if let Some(target) = target {
            targets = targets.filter(price_list::Column::Id.eq(target));
        }
        if targets.one(connection).await?.is_none() {
            return Ok(());
        }
        self.jobs
            .enqueue(
                connection,
                tenant,
                &DerivePrices { source, target },
                EnqueueOptions::default(),
            )
            .await?;
        Ok(())
    }

    /// Ghi dòng giá của bảng suy ra `target` từ mọi dòng của bảng gốc, theo lô [`BATCH`] dòng,
    /// mỗi lô một transaction ngắn. Chạy lại không nhân đôi (khoá `reference`).
    pub(crate) async fn derive_into(
        &self,
        tenant: &TenantContext,
        target: &price_list::Model,
    ) -> Result<u64, Error> {
        let Some(source) = target.derived_from else { return Ok(0) };
        let rule = match target.rounding_rule_id {
            Some(id) => {
                rounding_rule::Entity::find_by_id(id)
                    .scoped(tenant)
                    .one(tenant.database().read())
                    .await?
            }
            None => None,
        };
        let mut after: Option<Uuid> = None;
        let mut written = 0;
        loop {
            let mut select = price::Entity::find()
                .filter(price::Column::PriceListId.eq(source))
                .scoped(tenant)
                .order_by_asc(price::Column::Id)
                .limit(BATCH);
            if let Some(after) = after {
                select = select.filter(price::Column::Id.gt(after));
            }
            let rows = select.all(tenant.database().read()).await?;
            let Some(last) = rows.last() else { break };
            after = Some(last.id);
            let transaction = tenant.database().begin_write().await?;
            let references: Vec<String> =
                rows.iter().map(|row| format!("derived:{}", row.id)).collect();
            let done: Vec<String> = price::Entity::find()
                .filter(price::Column::PriceListId.eq(target.id))
                .filter(price::Column::Reference.is_in(references))
                .scoped(tenant)
                .all(&transaction)
                .await?
                .into_iter()
                .filter_map(|row| row.reference)
                .collect();
            let variants: Vec<Uuid> = rows.iter().map(|row| row.variant_id).collect();
            let pins = price::Entity::find()
                .filter(price::Column::PriceListId.eq(target.id))
                .filter(price::Column::VariantId.is_in(variants))
                .filter(price::Column::Source.eq("manual"))
                .scoped(tenant)
                .all(&transaction)
                .await?;
            for row in &rows {
                let reference = format!("derived:{}", row.id);
                if done.contains(&reference) {
                    continue;
                }
                // Dòng tay đang hiệu lực lúc dòng gốc bắt đầu: ghim giá, bỏ qua.
                let pinned = pins.iter().any(|pin| {
                    pin.variant_id == row.variant_id
                        && pin.valid_from <= row.valid_from
                        && pin.valid_to.is_none_or(|to| to > row.valid_from)
                });
                if pinned {
                    continue;
                }
                let amount = derive_amount(
                    row.amount_minor,
                    target.adjust_bp,
                    target.adjust_minor,
                    rule.as_ref(),
                );
                let compare_at = row.compare_at_minor.map(|minor| {
                    derive_amount(minor, target.adjust_bp, target.adjust_minor, rule.as_ref())
                });
                price::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    price_list_id: Set(target.id),
                    variant_id: Set(row.variant_id),
                    amount_minor: Set(amount),
                    amount_currency: Set(target.currency.clone()),
                    compare_at_minor: Set(compare_at.filter(|compare| *compare > amount)),
                    valid_from: Set(row.valid_from),
                    valid_to: Set(row.valid_to),
                    source: Set("derived".to_owned()),
                    reference: Set(Some(reference)),
                    reference_fingerprint: Set(None),
                    created_by: Set(None),
                    created_at: Set(now()),
                }
                .insert(&transaction)
                .await?;
                written += 1;
            }
            transaction.commit().await?;
        }
        Ok(written)
    }
}

/// Bộ chạy job [`DerivePrices`].
pub(crate) struct Deriver(pub(crate) Arc<CatalogService>);

#[async_trait::async_trait]
impl JobHandler for Deriver {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let Some(tenant) = &job.tenant else { return Ok(()) };
        let input: DerivePrices = serde_json::from_value(job.payload.clone())
            .map_err(|error| Error::internal(format!("bad derive_prices payload: {error}")))?;
        let mut targets = price_list::Entity::find()
            .filter(price_list::Column::DerivedFrom.eq(input.source))
            .filter(Condition::all())
            .scoped(tenant);
        if let Some(target) = input.target {
            targets = targets.filter(price_list::Column::Id.eq(target));
        }
        for target in targets.all(tenant.database().read()).await? {
            let written = self.0.derive_into(tenant, &target).await?;
            tracing::info!(price_list_id = %target.id, written, "derived prices written");
        }
        Ok(())
    }
}
